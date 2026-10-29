use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use super::templates::Templates;
use crate::config::Config;

/// Manages archive directory structure and file operations
pub struct ArchiveManager {
    config: Config,
}

impl ArchiveManager {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Ensure the storage directory exists
    pub fn ensure_storage_dir(&self) -> Result<PathBuf> {
        let path = self.config.storage_path();
        if !path.exists() {
            fs::create_dir_all(&path).context("Failed to create storage directory")?;
        }
        Ok(path)
    }

    /// Ensure today's directory exists and is initialized
    pub fn ensure_today_dir(&self) -> Result<PathBuf> {
        let today_dir = self.config.today_dir();

        if !today_dir.exists() {
            fs::create_dir_all(&today_dir).context("Failed to create today's directory")?;

            // Initialize daily.md
            let daily_md = today_dir.join("daily.md");
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let content = Templates::daily_init(&today);
            fs::write(&daily_md, content).context("Failed to write daily.md")?;
        }

        Ok(today_dir)
    }

    /// Ensure a specific date's directory exists
    pub fn ensure_date_dir(&self, date: &str) -> Result<PathBuf> {
        let date_dir = self.config.date_dir(date);

        if !date_dir.exists() {
            fs::create_dir_all(&date_dir).context("Failed to create date directory")?;

            // Initialize daily.md
            let daily_md = date_dir.join("daily.md");
            let content = Templates::daily_init(date);
            fs::write(&daily_md, content).context("Failed to write daily.md")?;
        }

        Ok(date_dir)
    }

    /// Get path for a session archive file
    pub fn session_archive_path(&self, date: &str, task_name: &str) -> PathBuf {
        self.config.date_dir(date).join(format!("{}.md", task_name))
    }

    /// Get path for the daily summary file
    pub fn daily_summary_path(&self, date: &str) -> PathBuf {
        self.config.date_dir(date).join("daily.md")
    }

    /// List all session archives for a date
    pub fn list_sessions(&self, date: &str) -> Result<Vec<String>> {
        let date_dir = self.config.date_dir(date);

        if !date_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in fs::read_dir(&date_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                if let Some(name) = path.file_stem() {
                    let name_str = name.to_string_lossy().to_string();
                    // Skip daily.md
                    if name_str != "daily" {
                        sessions.push(name_str);
                    }
                }
            }
        }

        sessions.sort();
        Ok(sessions)
    }

    /// List all available dates in the archive
    pub fn list_dates(&self) -> Result<Vec<String>> {
        let storage_path = self.config.storage_path();

        if !storage_path.exists() {
            return Ok(Vec::new());
        }

        let mut dates = Vec::new();
        for entry in fs::read_dir(&storage_path)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                if let Some(name) = path.file_name() {
                    let name_str = name.to_string_lossy().to_string();
                    // Check if it looks like a date (yyyy-mm-dd)
                    if name_str.len() == 10 && name_str.chars().nth(4) == Some('-') {
                        dates.push(name_str);
                    }
                }
            }
        }

        dates.sort();
        dates.reverse(); // Most recent first
        Ok(dates)
    }

    /// Read a session archive file
    pub fn read_session(&self, date: &str, task_name: &str) -> Result<String> {
        let path = self.session_archive_path(date, task_name);
        fs::read_to_string(&path).context(format!(
            "Failed to read session archive: {}",
            path.display()
        ))
    }

    /// Read the daily summary file
    pub fn read_daily_summary(&self, date: &str) -> Result<String> {
        let path = self.daily_summary_path(date);
        fs::read_to_string(&path)
            .context(format!("Failed to read daily summary: {}", path.display()))
    }

    /// Write a session archive file
    pub fn write_session(&self, date: &str, task_name: &str, content: &str) -> Result<PathBuf> {
        self.ensure_date_dir(date)?;
        let mut path = self.session_archive_path(date, task_name);
        // Two sessions that start in the same minute can share an
        // HH_MM-topic title; never clobber a different session's archive —
        // disambiguate with a short session-id suffix instead.
        if path.exists() {
            let existing = fs::read_to_string(&path).unwrap_or_default();
            if let Some(id) = frontmatter_id(content) {
                if !existing.contains(&format!("session_id: {}", id)) {
                    let short: String = id.chars().take(8).collect();
                    path = self.session_archive_path(date, &format!("{}-{}", task_name, short));
                }
            }
        }
        fs::write(&path, content).context(format!(
            "Failed to write session archive: {}",
            path.display()
        ))?;
        Ok(path)
    }

    /// Write the daily summary file
    pub fn write_daily_summary(&self, date: &str, content: &str) -> Result<PathBuf> {
        self.ensure_date_dir(date)?;
        let path = self.daily_summary_path(date);
        fs::write(&path, content)
            .context(format!("Failed to write daily summary: {}", path.display()))?;
        Ok(path)
    }

    /// Check if a date has session files (un-digested sessions)
    pub fn has_sessions(&self, date: &str) -> bool {
        match self.list_sessions(date) {
            Ok(sessions) => !sessions.is_empty(),
            Err(_) => false,
        }
    }
}

/// Pull `session_id: <id>` out of archive frontmatter.
fn frontmatter_id(content: &str) -> Option<&str> {
    content
        .lines()
        .find_map(|l| l.strip_prefix("session_id: "))
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(temp_dir: &TempDir) -> Config {
        let mut config = Config::default();
        config.storage.path = temp_dir.path().to_path_buf();
        config
    }

    #[test]
    fn test_write_session_collision_keeps_both() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let manager = ArchiveManager::new(config);

        let first = "---\nsession_id: aaaa1111\n---\nfirst";
        let second = "---\nsession_id: bbbb2222\n---\nsecond";
        manager
            .write_session("2026-10-29", "01_22-topic", first)
            .unwrap();
        let p2 = manager
            .write_session("2026-10-29", "01_22-topic", second)
            .unwrap();

        // Different session with the same title must not clobber the first.
        assert!(p2.to_string_lossy().contains("01_22-topic-bbbb2222"));
        let dir = tmp.path().join("2026-10-29");
        // daily.md (seeded) + two session archives, first not clobbered.
        assert_eq!(std::fs::read_dir(&dir).unwrap().flatten().count(), 3);

        // Same session written again stays idempotent (same name).
        let again = manager
            .write_session("2026-10-29", "01_22-topic", second)
            .unwrap();
        assert_eq!(again, p2);
    }

    #[test]
    fn test_ensure_today_dir() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let manager = ArchiveManager::new(config);

        let today_dir = manager.ensure_today_dir().unwrap();
        assert!(today_dir.exists());
        assert!(today_dir.join("daily.md").exists());
    }

    #[test]
    fn test_list_sessions_empty() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let manager = ArchiveManager::new(config);

        let sessions = manager.list_sessions("2026-01-16").unwrap();
        assert!(sessions.is_empty());
    }
}
