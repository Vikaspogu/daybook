use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::manager::ArchiveManager;
use super::templates::Templates;
use crate::config::Config;
use crate::transcript::TranscriptData;

/// Bump when summary inputs or instructions change in a way that requires refresh.
pub const SUMMARIZER_VERSION: u32 = 2;

pub(crate) fn frontmatter_value<'a>(content: &'a str, key: &str) -> Option<&'a str> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let prefix = format!("{}: ", key);
    lines
        .take_while(|line| line.trim() != "---")
        .find_map(|line| line.trim().strip_prefix(&prefix).map(str::trim))
}
/// Represents a summarized session ready for archiving
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArchive {
    pub title: String,
    pub date: String,
    pub session_id: String,
    pub cwd: String,
    pub git_branch: Option<String>,
    pub transcript_path: Option<String>,
    pub transcript_revision: String,
    pub transcript_modified_ns: u128,
    pub summarizer_version: u32,
    pub summary: String,
    pub decisions: String,
    pub code_changes: String,
    pub learnings: String,
    pub skill_hints: String,
    /// Runtime that produced the transcript (e.g. "claude-code", "pi", "codex").
    /// None falls back to the upstream default tag.
    pub agent: Option<String>,
}

impl SessionArchive {
    /// Create a new session archive from raw data
    pub fn new(title: String, date: String, session_id: String, cwd: String) -> Self {
        Self {
            title,
            date,
            session_id,
            cwd,
            git_branch: None,
            transcript_path: None,
            transcript_revision: String::new(),
            transcript_modified_ns: 0,
            summarizer_version: SUMMARIZER_VERSION,
            summary: String::new(),
            decisions: String::new(),
            code_changes: String::new(),
            learnings: String::new(),
            skill_hints: String::new(),
            agent: None,
        }
    }

    /// Set the transcript path
    pub fn with_transcript_path(mut self, path: impl Into<String>) -> Self {
        self.transcript_path = Some(path.into());
        self
    }

    /// Fill in data from transcript
    pub fn with_transcript_data(mut self, data: &TranscriptData) -> Self {
        // Build code changes from files modified
        if !data.files_modified.is_empty() {
            self.code_changes = data
                .files_modified
                .iter()
                .map(|f| format!("- `{}`", f))
                .collect::<Vec<_>>()
                .join("\n");
        } else {
            self.code_changes = "_No files modified._".to_string();
        }

        self
    }

    /// Fill in summary data from AI analysis
    pub fn with_summary(
        mut self,
        summary: String,
        decisions: String,
        learnings: String,
        skill_hints: String,
    ) -> Self {
        self.summary = summary;
        self.decisions = decisions;
        self.learnings = learnings;
        self.skill_hints = skill_hints;
        self
    }

    /// Generate Markdown content for this archive
    pub fn to_markdown(&self) -> String {
        Templates::session_archive(
            &self.title,
            &self.date,
            &self.session_id,
            &self.cwd,
            self.git_branch.as_deref(),
            self.transcript_path.as_deref(),
            &self.transcript_revision,
            self.transcript_modified_ns,
            self.summarizer_version,
            &self.summary,
            &self.decisions,
            &self.code_changes,
            &self.learnings,
            &self.skill_hints,
            self.agent.as_deref(),
        )
    }

    /// Save this archive to disk
    pub fn save(&self, config: &Config) -> Result<std::path::PathBuf> {
        let manager = ArchiveManager::new(config.clone());
        let content = self.to_markdown();
        manager.write_session(&self.date, &self.title, &content)
    }
}

/// Get git branch from working directory
pub fn get_git_branch(cwd: &str) -> Option<String> {
    // cwd can originate from a transcript header, so neutralize repo-config
    // code-exec vectors before reading the branch.
    // ponytail: covers the known read-command vectors (fsmonitor); a fully
    // untrusted cwd would need a bare --git-dir invocation instead.
    std::process::Command::new("git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=",
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ])
        .current_dir(cwd)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_archive_new() {
        let archive = SessionArchive::new(
            "test-session".to_string(),
            "2026-01-16".to_string(),
            "abc123".to_string(),
            "/home/user/project".to_string(),
        );

        assert_eq!(archive.title, "test-session");
    }

    #[test]
    fn test_session_archive_to_markdown() {
        let archive = SessionArchive::new(
            "test-session".to_string(),
            "2026-01-16".to_string(),
            "abc123".to_string(),
            "/home/user/project".to_string(),
        );

        let md = archive.to_markdown();
        assert!(md.contains("title: \"test-session\""));
        assert!(md.contains("# test-session"));
        assert!(md.contains("transcript_revision:"));
        assert!(md.contains(&format!("summarizer_version: {}", SUMMARIZER_VERSION)));
    }

    #[test]
    fn test_changed_summary_replaces_existing_session_archive() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut config = Config::default();
        config.storage.path = temp.path().to_path_buf();
        let first = SessionArchive::new(
            "20_33-investigating".into(),
            "2026-11-11".into(),
            "same-session".into(),
            "/tmp/project".into(),
        )
        .with_summary(
            "Still investigating".into(),
            String::new(),
            String::new(),
            String::new(),
        );
        let first_path = first.save(&config).unwrap();

        let first_content = std::fs::read_to_string(&first_path).unwrap();
        let first_version = frontmatter_value(&first_content, "version")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap();
        let mut final_archive = SessionArchive::new(
            "20_33-fix-verified".into(),
            "2026-11-11".into(),
            "same-session".into(),
            "/tmp/project".into(),
        )
        .with_summary(
            "Fix deployed and verified".into(),
            String::new(),
            String::new(),
            String::new(),
        );
        final_archive.transcript_modified_ns = 2;
        let final_path = final_archive.save(&config).unwrap();

        assert_eq!(final_path, first_path);
        let saved = std::fs::read_to_string(final_path).unwrap();
        assert!(saved.contains("Fix deployed and verified"));
        let final_version = frontmatter_value(&saved, "version")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap();
        assert!(final_version > first_version);
        assert!(!saved.contains("Still investigating"));
        assert_eq!(
            std::fs::read_dir(temp.path().join("2026-11-11"))
                .unwrap()
                .flatten()
                .count(),
            2,
            "daily.md plus one session archive"
        );
    }

    #[test]
    fn test_session_id_in_daily_body_is_not_an_archive() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut config = Config::default();
        config.storage.path = temp.path().to_path_buf();
        let manager = ArchiveManager::new(config.clone());
        manager.ensure_date_dir("2026-11-11").unwrap();
        let daily_path = temp.path().join("2026-11-11/daily.md");
        std::fs::write(
            &daily_path,
            "---\ndate: 2026-11-11\n---\nsession_id: same-session\n",
        )
        .unwrap();
        let daily_before = std::fs::read_to_string(&daily_path).unwrap();

        let archive = SessionArchive::new(
            "20_33-fix-verified".into(),
            "2026-11-11".into(),
            "same-session".into(),
            "/tmp/project".into(),
        );
        let archive_path = archive.save(&config).unwrap();

        assert_ne!(archive_path, daily_path);
        assert_eq!(std::fs::read_to_string(daily_path).unwrap(), daily_before);
    }
}
