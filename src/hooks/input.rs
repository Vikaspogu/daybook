use anyhow::{Context, Result};
use serde::Deserialize;
use std::io::{self, Read};
use std::path::PathBuf;

/// Input data received from Claude Code hooks via stdin
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct HookInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub transcript_path: PathBuf,
    #[serde(default)]
    pub cwd: PathBuf,
    #[serde(default)]
    pub hook_event_name: String,
    #[serde(default)]
    pub reason: Option<String>, // Only for SessionEnd: prompt_input_exit, logout, clear, other
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<serde_json::Value>,
}

impl HookInput {
    /// Extract target file path if this hook event represents a file-editing tool call.
    pub fn edited_file_path(&self) -> Option<String> {
        if let Some(ref name) = self.tool_name {
            let lower = name.to_ascii_lowercase();
            if !matches!(
                lower.as_str(),
                "edit"
                    | "write"
                    | "multiedit"
                    | "notebookedit"
                    | "replace"
                    | "replace_match"
                    | "insert"
            ) {
                return None;
            }
        }
        let obj = self.tool_input.as_ref()?.as_object()?;
        for key in ["file_path", "path", "notebook_path"] {
            if let Some(val) = obj.get(key).and_then(|v| v.as_str()) {
                let trimmed = val.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
        None
    }
}
/// Read hook input JSON from stdin
pub fn read_hook_input() -> Result<HookInput> {
    let mut buffer = String::new();
    io::stdin()
        .read_to_string(&mut buffer)
        .context("Failed to read from stdin")?;

    // Handle empty input gracefully
    if buffer.trim().is_empty() {
        anyhow::bail!("No input received from stdin");
    }

    let input: HookInput =
        serde_json::from_str(&buffer).context("Failed to parse hook input JSON")?;

    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_session_start_input() {
        let json = r#"{
            "session_id": "abc123",
            "transcript_path": "/home/user/.claude/projects/xyz/session.jsonl",
            "cwd": "/home/user/project",
            "hook_event_name": "SessionStart"
        }"#;

        let input: HookInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.session_id, "abc123");
        assert_eq!(input.hook_event_name, "SessionStart");
        assert!(input.reason.is_none());
    }

    #[test]
    fn test_parse_session_end_input() {
        let json = r#"{
            "session_id": "abc123",
            "transcript_path": "/home/user/.claude/projects/xyz/session.jsonl",
            "cwd": "/home/user/project",
            "hook_event_name": "SessionEnd",
            "reason": "user_exit"
        }"#;

        let input: HookInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.reason, Some("user_exit".to_string()));
    }

    #[test]
    fn test_parse_tool_use_input() {
        let json = r#"{
            "session_id": "abc123",
            "cwd": "/home/user/project",
            "hook_event_name": "PreToolUse",
            "tool_name": "Edit",
            "tool_input": {
                "file_path": "/home/user/project/src/main.rs"
            }
        }"#;
        let input: HookInput = serde_json::from_str(json).unwrap();
        assert_eq!(
            input.edited_file_path().as_deref(),
            Some("/home/user/project/src/main.rs")
        );
    }
}
