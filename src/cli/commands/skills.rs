use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Review pending skills
use super::init::{finish_write, stage_write};
use crate::config::load_config;
pub async fn run_review(install: Option<String>, delete: Option<String>) -> Result<()> {
    let config = load_config()?;
    let pending_dir = config.storage.path.join("pending-skills");
    // Handle install action
    if let Some(skill_path) = install {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        return install_skill(&pending_dir, &skill_path, &home);
    }

    // Handle delete action
    if let Some(skill_path) = delete {
        return delete_skill(&pending_dir, &skill_path);
    }

    list_pending_skills(&pending_dir)?;
    list_decisions(&pending_dir)
}

/// List all pending skills
fn list_pending_skills(pending_dir: &Path) -> Result<()> {
    let mut skills: Vec<(String, String, PathBuf)> = Vec::new();

    if let Ok(entries) = fs::read_dir(pending_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let date = entry.file_name().to_string_lossy().to_string();
                if let Ok(files) = fs::read_dir(entry.path()) {
                    for file in files.flatten() {
                        if file.path().extension().is_some_and(|e| e == "md") {
                            let name = file
                                .path()
                                .file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_default();
                            skills.push((date.clone(), name, file.path()));
                        }
                    }
                }
            }
        }
    }

    if skills.is_empty() {
        println!("No pending skills to review.");
        return Ok(());
    }

    println!("Pending Skills ({} total):", skills.len());
    println!("{}", "─".repeat(60));

    for (date, name, path) in &skills {
        println!();
        println!("📦 {}/{}", date, name);

        // Read and show preview
        if let Ok(content) = fs::read_to_string(path) {
            // Extract description from frontmatter
            if let Some(desc) = extract_description(&content) {
                println!("   {}", desc);
            }

            // Show trigger conditions if present
            if let Some(trigger) = extract_section(&content, "## When to Use") {
                let preview: String = trigger.lines().take(3).collect::<Vec<_>>().join("\n   ");
                println!("   Trigger: {}", preview.trim());
            }
        }

        println!();
        println!("   Actions:");
        println!("     daybook review-skills --install {}/{}", date, name);
        println!("     daybook review-skills --delete {}/{}", date, name);
    }

    println!();
    println!("{}", "─".repeat(60));
    println!(
        "Or ask your agent: \"install skill {}/{}\"",
        skills[0].0, skills[0].1
    );

    Ok(())
}

/// Install a skill to user's skills directory
fn install_skill(pending_dir: &Path, skill_ref: &str, home: &Path) -> Result<()> {
    let (date, name) = parse_skill_ref(skill_ref)?;
    let skill_path = pending_dir.join(&date).join(format!("{}.md", name));

    if !skill_path.exists() {
        anyhow::bail!("Skill not found: {}/{}", date, name);
    }

    // Read skill content
    let content = fs::read_to_string(&skill_path)?;

    // ~/.agents is shared by Pi and Codex; Claude keeps its native destination too.
    let target_dirs = [
        home.join(".agents/skills").join(&name),
        home.join(".claude/skills").join(&name),
    ];
    let mut destinations = Vec::new();
    for target_dir in &target_dirs {
        let target_file = target_dir.join("SKILL.md");
        if fs::read_to_string(&target_file).is_ok_and(|existing| existing != content) {
            anyhow::bail!(
                "Refusing to overwrite edited skill {}; move it aside first",
                target_file.display()
            );
        }
        destinations.push(target_file);
    }
    let mut staged = Vec::new();
    for target_file in &destinations {
        match stage_write(target_file, &content) {
            Ok(temp) => staged.push(temp),
            Err(error) => {
                for temp in staged {
                    let _ = fs::remove_file(temp);
                }
                return Err(error);
            }
        }
    }
    let decision_path = decision_log_path(pending_dir);
    let decision_len = fs::metadata(&decision_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let mut backups: Vec<Option<PathBuf>> = vec![None; destinations.len()];
    let mut installed = 0;
    let install_result: Result<()> = (|| {
        for (index, (temp, target_file)) in staged.iter().zip(&destinations).enumerate() {
            if target_file.is_file() {
                let backup = temp.with_extension("backup");
                fs::rename(target_file, &backup)?;
                backups[index] = Some(backup);
            }
            installed += 1;
            finish_write(temp, target_file)?;
        }
        append_decision(pending_dir, skill_ref, "installed", Some(&destinations))?;
        fs::remove_file(&skill_path)?;
        Ok(())
    })();
    if let Err(error) = install_result {
        let restore = restore_skill_files(&destinations, &backups, installed);
        let truncate = (|| -> Result<()> {
            if fs::metadata(&decision_path).is_ok_and(|meta| meta.is_file()) {
                fs::OpenOptions::new()
                    .write(true)
                    .open(&decision_path)?
                    .set_len(decision_len)?;
            }
            Ok(())
        })();
        for temp in &staged {
            let _ = fs::remove_file(temp);
        }
        if let Err(rollback_error) = restore.and(truncate) {
            return Err(error.context(format!(
                "Skill install rollback also failed; any .backup files beside the skill were kept: {rollback_error:#}"
            )));
        }
        return Err(error);
    }
    for backup in backups.iter().flatten() {
        let _ = fs::remove_file(backup);
    }

    // Clean up empty date directory
    let date_dir = pending_dir.join(&date);
    if fs::read_dir(&date_dir)?.next().is_none() {
        fs::remove_dir(&date_dir)?;
    }
    for target_file in destinations {
        println!("✓ Skill installed: {}", target_file.display());
    }
    println!("The skill is now available to Claude Code, Codex, and Pi.");
    Ok(())
}

fn restore_skill_files(
    destinations: &[PathBuf],
    backups: &[Option<PathBuf>],
    installed: usize,
) -> Result<()> {
    // Best effort: keep restoring later files even if one fails.
    let mut result = Ok(());
    for index in (0..installed).rev() {
        let target = &destinations[index];
        let step = (|| -> Result<()> {
            if target.is_file() {
                fs::remove_file(target)?;
            }
            if let Some(backup) = &backups[index] {
                fs::rename(backup, target)?;
            }
            Ok(())
        })();
        if result.is_ok() {
            result = step;
        }
    }
    result
}
/// Delete a pending skill
fn delete_skill(pending_dir: &Path, skill_ref: &str) -> Result<()> {
    let (date, name) = parse_skill_ref(skill_ref)?;
    let skill_path = pending_dir.join(&date).join(format!("{}.md", name));

    if !skill_path.exists() {
        anyhow::bail!("Skill not found: {}/{}", date, name);
    }

    append_decision(pending_dir, skill_ref, "deleted", None)?;
    fs::remove_file(&skill_path)?;

    // Clean up empty date directory
    let date_dir = pending_dir.join(&date);
    if fs::read_dir(&date_dir)?.next().is_none() {
        fs::remove_dir(&date_dir)?;
    }

    println!("✓ Skill deleted: {}/{}", date, name);

    Ok(())
}

/// Parse skill reference like "2026-01-18/skill-name"
fn parse_skill_ref(skill_ref: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = skill_ref.split('/').collect();
    let valid_date = parts.first().is_some_and(|date| {
        date.len() == 10
            && date.chars().enumerate().all(|(i, c)| {
                if i == 4 || i == 7 {
                    c == '-'
                } else {
                    c.is_ascii_digit()
                }
            })
    });
    let valid_name = parts.get(1).is_some_and(|name| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    });
    if parts.len() != 2 || !valid_date || !valid_name {
        anyhow::bail!("Invalid skill reference. Use format: YYYY-MM-DD/skill-name");
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[derive(Serialize)]
struct SkillDecision<'a> {
    decided_at: String,
    skill: &'a str,
    decision: &'a str,
    destinations: Option<&'a [PathBuf]>,
}

fn decision_log_path(pending_dir: &Path) -> PathBuf {
    pending_dir
        .parent()
        .unwrap_or(pending_dir)
        .join("skill-decisions.jsonl")
}

fn append_decision(
    pending_dir: &Path,
    skill_ref: &str,
    decision: &str,
    destinations: Option<&[PathBuf]>,
) -> Result<()> {
    let path = decision_log_path(pending_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let entry = SkillDecision {
        decided_at: chrono::Utc::now().to_rfc3339(),
        skill: skill_ref,
        decision,
        destinations,
    };
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", serde_json::to_string(&entry)?)?;
    Ok(())
}

fn list_decisions(pending_dir: &Path) -> Result<()> {
    let path = decision_log_path(pending_dir);
    let Ok(content) = fs::read_to_string(&path) else {
        return Ok(());
    };
    let decisions: Vec<serde_json::Value> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if !decisions.is_empty() {
        println!("\nRecent review decisions ({}):", path.display());
        for decision in decisions.iter().rev().take(10) {
            println!(
                "  {} {} {}",
                decision["decided_at"].as_str().unwrap_or("unknown"),
                decision["decision"].as_str().unwrap_or("unknown"),
                decision["skill"].as_str().unwrap_or("unknown")
            );
        }
    }
    Ok(())
}

/// Extract description from YAML frontmatter
fn extract_description(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("description:") {
            let desc = line.trim_start_matches("description:").trim();
            let desc = desc.trim_matches('"').trim_matches('\'');
            if !desc.is_empty() {
                return Some(desc.to_string());
            }
        }
    }
    None
}

/// Extract a section from markdown content
fn extract_section(content: &str, header: &str) -> Option<String> {
    if let Some(start) = content.find(header) {
        let after_header = &content[start + header.len()..];
        // Find next section or end
        let end = after_header
            .find("\n## ")
            .unwrap_or(after_header.len().min(500));
        return Some(after_header[..end].trim().to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_records_decision_and_uses_all_runtime_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().join("storage");
        let pending = storage.join("pending-skills");
        let date_dir = pending.join("2026-11-14");
        fs::create_dir_all(&date_dir).unwrap();
        fs::write(date_dir.join("verify-work.md"), "skill body").unwrap();
        let home = dir.path().join("home");

        install_skill(&pending, "2026-11-14/verify-work", &home).unwrap();

        assert_eq!(
            fs::read_to_string(home.join(".agents/skills/verify-work/SKILL.md")).unwrap(),
            "skill body"
        );
        assert_eq!(
            fs::read_to_string(home.join(".claude/skills/verify-work/SKILL.md")).unwrap(),
            "skill body"
        );
        assert!(!date_dir.join("verify-work.md").exists());
        let decision: serde_json::Value = serde_json::from_str(
            fs::read_to_string(storage.join("skill-decisions.jsonl"))
                .unwrap()
                .trim(),
        )
        .unwrap();
        assert_eq!(decision["decision"], "installed");
        assert_eq!(decision["destinations"].as_array().unwrap().len(), 2);
        assert!(parse_skill_ref("../escape").is_err());
        assert!(parse_skill_ref("2026-11-14/../escape").is_err());
    }

    #[test]
    fn install_preserves_edited_skill_and_pending_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let pending = dir.path().join("storage/pending-skills");
        let date_dir = pending.join("2026-11-14");
        fs::create_dir_all(&date_dir).unwrap();
        fs::write(date_dir.join("verify-work.md"), "candidate").unwrap();
        let home = dir.path().join("home");
        let existing = home.join(".agents/skills/verify-work/SKILL.md");
        fs::create_dir_all(existing.parent().unwrap()).unwrap();
        fs::write(&existing, "local edits").unwrap();

        assert!(install_skill(&pending, "2026-11-14/verify-work", &home).is_err());
        assert_eq!(fs::read_to_string(existing).unwrap(), "local edits");
        assert!(date_dir.join("verify-work.md").exists());
        assert!(!pending
            .parent()
            .unwrap()
            .join("skill-decisions.jsonl")
            .exists());
    }

    #[cfg(unix)]
    #[test]
    fn install_preflights_both_destinations_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let pending = dir.path().join("storage/pending-skills");
        let date_dir = pending.join("2026-11-14");
        fs::create_dir_all(&date_dir).unwrap();
        fs::write(date_dir.join("verify-work.md"), "candidate").unwrap();
        let home = dir.path().join("home");
        let claude_skill = home.join(".claude/skills/verify-work");
        fs::create_dir_all(&claude_skill).unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, "keep").unwrap();
        std::os::unix::fs::symlink(&victim, claude_skill.join("SKILL.md")).unwrap();

        assert!(install_skill(&pending, "2026-11-14/verify-work", &home).is_err());
        assert!(!home.join(".agents/skills/verify-work/SKILL.md").exists());
        assert_eq!(fs::read_to_string(victim).unwrap(), "keep");
        assert!(date_dir.join("verify-work.md").exists());
    }

    #[test]
    fn install_rolls_back_both_files_when_decision_log_fails() {
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().join("storage");
        let pending = storage.join("pending-skills");
        let date_dir = pending.join("2026-11-14");
        fs::create_dir_all(&date_dir).unwrap();
        fs::write(date_dir.join("verify-work.md"), "candidate").unwrap();
        fs::create_dir_all(storage.join("skill-decisions.jsonl")).unwrap();
        let home = dir.path().join("home");
        let same = home.join(".claude/skills/verify-work/SKILL.md");
        fs::create_dir_all(same.parent().unwrap()).unwrap();
        fs::write(&same, "candidate").unwrap();

        assert!(install_skill(&pending, "2026-11-14/verify-work", &home).is_err());
        assert!(!home.join(".agents/skills/verify-work/SKILL.md").exists());
        assert_eq!(fs::read_to_string(&same).unwrap(), "candidate");
        assert!(date_dir.join("verify-work.md").exists());
        let leftovers: Vec<_> = fs::read_dir(same.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("SKILL.md")]);
    }

    #[test]
    fn install_rolls_back_first_file_when_second_write_fails() {
        let dir = tempfile::tempdir().unwrap();
        let pending = dir.path().join("storage/pending-skills");
        let date_dir = pending.join("2026-11-14");
        fs::create_dir_all(&date_dir).unwrap();
        fs::write(date_dir.join("verify-work.md"), "candidate").unwrap();
        let home = dir.path().join("home");
        let first = home.join(".agents/skills/verify-work/SKILL.md");
        fs::create_dir_all(first.parent().unwrap()).unwrap();
        fs::write(&first, "candidate").unwrap();
        // A directory at the second path makes its final rename fail.
        let blocked = home.join(".claude/skills/verify-work/SKILL.md");
        fs::create_dir_all(&blocked).unwrap();

        assert!(install_skill(&pending, "2026-11-14/verify-work", &home).is_err());
        assert_eq!(fs::read_to_string(&first).unwrap(), "candidate");
        assert!(blocked.is_dir());
        assert!(date_dir.join("verify-work.md").exists());
        for parent in [first.parent().unwrap(), blocked.parent().unwrap()] {
            assert_eq!(fs::read_dir(parent).unwrap().count(), 1);
        }
    }
    #[test]
    fn delete_records_decision() {
        let dir = tempfile::tempdir().unwrap();
        let pending = dir.path().join("storage/pending-skills");
        let date_dir = pending.join("2026-11-14");
        fs::create_dir_all(&date_dir).unwrap();
        fs::write(date_dir.join("obsolete.md"), "candidate").unwrap();

        delete_skill(&pending, "2026-11-14/obsolete").unwrap();
        let decision: serde_json::Value = serde_json::from_str(
            fs::read_to_string(dir.path().join("storage/skill-decisions.jsonl"))
                .unwrap()
                .trim(),
        )
        .unwrap();
        assert_eq!(decision["decision"], "deleted");
        assert!(!date_dir.join("obsolete.md").exists());
    }
}
