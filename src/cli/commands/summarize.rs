use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config::load_config;
use crate::jobs::JobManager;
use crate::summarizer::SummarizerEngine;
use crate::transcript::TranscriptParser;

/// Manually trigger summarization of a transcript
pub async fn run(
    transcript: PathBuf,
    task_name: Option<String>,
    cwd: Option<PathBuf>,
    foreground: bool,
    job_id: Option<String>,
) -> Result<()> {
    let config = load_config()?;

    // Generate task name if not provided
    let task_name = task_name.unwrap_or_else(|| {
        let timestamp = chrono::Local::now().format("%H%M%S");
        format!("session-{}", timestamp)
    });

    // Use provided cwd, or fallback to current dir
    let cwd = cwd
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

    if !foreground {
        // Background mode: spawn detached process
        eprintln!(
            "[daybook] Starting background summarization for: {}",
            task_name
        );

        // Re-invoke ourselves in foreground mode as a detached process
        let exe = std::env::current_exe().context("Failed to get current executable")?;

        let transcript_str = transcript.to_string_lossy().to_string();

        // Build args with cwd
        let args = vec![
            "summarize".to_string(),
            "--transcript".to_string(),
            transcript_str,
            "--task-name".to_string(),
            task_name.clone(),
            "--cwd".to_string(),
            cwd.clone(),
            "--foreground".to_string(),
        ];

        // Spawn detached background process
        #[cfg(unix)]
        {
            // Use nohup-style spawning on Unix
            Command::new(&exe)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("Failed to spawn background process")?;
        }

        #[cfg(windows)]
        {
            Command::new(&exe)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("Failed to spawn background process")?;
        }

        // Suppress unused variable warning
        let _ = args;

        eprintln!("[daybook] Background summarization started");
        return Ok(());
    }

    // Foreground mode: do the actual summarization
    eprintln!("[daybook] Summarizing session: {}", task_name);

    // Initialize job manager for status updates
    let job_manager = JobManager::new(&config).ok();

    // Run summarization with job status tracking
    let result = run_summarization(&config, &transcript, &task_name, &cwd).await;

    // Update job status based on result
    if let (Some(ref manager), Some(ref id)) = (&job_manager, &job_id) {
        match &result {
            Ok(_) => {
                if let Err(e) = manager.mark_completed(id) {
                    eprintln!("[daybook] Warning: Failed to update job status: {}", e);
                }
            }
            Err(e) => {
                if let Err(update_err) = manager.mark_failed(id, &e.to_string()) {
                    eprintln!(
                        "[daybook] Warning: Failed to update job status: {}",
                        update_err
                    );
                }
            }
        }

        // Truncate log if needed
        let _ = manager.truncate_log_if_needed(id);
    }

    result
}

/// Run the actual summarization logic
pub(crate) async fn run_summarization(
    config: &crate::config::Config,
    transcript: &PathBuf,
    task_name: &str,
    cwd: &str,
) -> Result<()> {
    // Check if transcript file exists before attempting to parse
    if !transcript.exists() {
        eprintln!(
            "[daybook] Transcript file not found, skipping: {}",
            transcript.display()
        );
        return Ok(());
    }

    // Runtime-aware parse: known session trees go through the runtime
    // adapters (Claude Code, Pi, Codex); any other explicit path falls
    // back to the legacy Claude-format parser (upstream CLI contract).
    let (data, session_id, agent, start) =
        match crate::transcript::adapters::meta_for_path(transcript) {
            Some(meta) => {
                let parsed = crate::transcript::adapters::parse_session(&meta)
                    .context("Failed to parse transcript")?;
                let start = parsed.start.clone();
                (
                    parsed.data,
                    meta.session_id,
                    Some(meta.agent.as_str().to_string()),
                    start,
                )
            }
            None => {
                let parsed =
                    TranscriptParser::parse(transcript).context("Failed to parse transcript")?;
                let session_id = transcript
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                (parsed, session_id, None, None)
            }
        };

    if data.is_empty() {
        eprintln!("[daybook] Session is empty, skipping summarization");
        return Ok(());
    }

    let engine = SummarizerEngine::new(config.clone());

    // Summarize the session
    let mut archive = engine
        .summarize_session(
            transcript,
            task_name,
            cwd,
            &data,
            &session_id,
            start.as_deref(),
        )
        .await
        .context("Failed to summarize session")?;
    archive.agent = agent;

    // Save the archive
    let archive_path = archive.save(config)?;
    eprintln!("[daybook] Session archived: {}", archive_path.display());

    // Auto-evaluate skill extraction (沉淀三问 quality gate)
    if should_extract_skill(&archive.skill_hints) {
        eprintln!("[daybook] Skill candidate detected, attempting extraction...");
        match auto_extract_skill(&engine, &archive, config).await {
            Ok(Some(skill_path)) => {
                eprintln!("[daybook] Pending skill saved: {}", skill_path.display());
            }
            Ok(None) => {
                eprintln!("[daybook] Skill did not pass quality gate, skipped");
            }
            Err(e) => {
                eprintln!("[daybook] Skill extraction failed: {}", e);
            }
        }
    }

    // Note: Daily summary is now generated via `daily digest` command
    // either manually or auto-triggered on session start

    eprintln!("[daybook] Summarization complete!");

    Ok(())
}

/// Check if skill_hints suggest extractable global skill knowledge
fn should_extract_skill(skill_hints: &str) -> bool {
    let hints_lower = skill_hints.to_lowercase();

    // Skip if explicitly marked as none
    if hints_lower.contains("none identified")
        || hints_lower.contains("no skills")
        || hints_lower.contains("no potential")
        || skill_hints.contains("未发现可沉淀技能")
        || skill_hints.contains("未发现可复用候选项")
        || skill_hints.trim().is_empty()
    {
        return false;
    }

    // Versioned v2 candidates: only extract when at least one global-skill exists
    if skill_hints.contains("<!-- daybook:workflow-candidates:v2 -->") {
        let normalized = hints_lower.replace('*', "");
        return normalized.contains("scope: global-skill")
            || normalized.contains("scope:global-skill");
    }

    // Legacy skill markers (name, trigger, etc.)
    hints_lower.contains("**") || hints_lower.contains("trigger:") || hints_lower.contains("- ")
}

/// Auto-extract skill from session archive
async fn auto_extract_skill(
    engine: &SummarizerEngine,
    archive: &crate::archive::SessionArchive,
    config: &crate::config::Config,
) -> Result<Option<PathBuf>> {
    // Build context from archive
    let session_content = archive.to_markdown();

    // Extract skill (will apply 沉淀三问 quality gate)
    let skill_content = engine
        .extract_skill(&session_content, Some(&archive.skill_hints))
        .await?;

    // Check if extraction was rejected by quality gate
    if skill_content.trim().starts_with("NOT_EXTRACTABLE:") {
        return Ok(None);
    }

    // Save to pending-skills directory
    let pending_dir = config
        .storage
        .path
        .join("pending-skills")
        .join(&archive.date);
    fs::create_dir_all(&pending_dir)?;

    // Extract skill name from content
    let skill_name = extract_skill_name(&skill_content);
    // Path traversal guard: the LLM-derived name must stay a single path
    // segment inside pending-skills, never escape it.
    if skill_name.contains('/')
        || skill_name.contains('\\')
        || skill_name.contains("..")
        || skill_name.contains('\0')
    {
        eprintln!(
            "[daybook] Skipping skill with unsafe name: {}",
            skill_name.replace(char::is_control, "?")
        );
        return Ok(None);
    }
    let skill_file = pending_dir.join(format!("{}.md", skill_name));

    fs::write(&skill_file, &skill_content)?;

    Ok(Some(skill_file))
}

/// Extract skill name from YAML frontmatter
fn extract_skill_name(content: &str) -> String {
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("name:") {
            let name = line.trim_start_matches("name:").trim();
            let name = name.trim_matches('"').trim_matches('\'');
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    // Fallback with timestamp
    let timestamp = chrono::Local::now().format("%H%M%S");
    format!("skill-{}", timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::Prompts;

    #[test]
    fn should_extract_skill_respects_v2_scope_and_prompt_contract() {
        let en = Prompts::default_session_summary_template("en");
        let zh = Prompts::default_session_summary_template("zh");
        assert!(en.contains("<!-- daybook:workflow-candidates:v2 -->"));
        assert!(zh.contains("<!-- daybook:workflow-candidates:v2 -->"));

        assert!(!should_extract_skill(""));
        assert!(!should_extract_skill("None identified in this session."));
        assert!(!should_extract_skill("本次会话未发现可复用候选项。"));

        let v2_repo_only = "<!-- daybook:workflow-candidates:v2 -->\n- **[home-assistant-dependency-audit]**: Check indirect YAML references.\n  - Scope: repo-rule\n  - Trigger: When removing a HACS component.";
        assert!(!should_extract_skill(v2_repo_only));

        let v2_with_global = "<!-- daybook:workflow-candidates:v2 -->\n- **[validate-investigation-evidence]**: Verify tool HTTP status.\n  - Scope: global-skill\n  - Trigger: When evaluating incident hypotheses.";
        assert!(should_extract_skill(v2_with_global));
    }
}
