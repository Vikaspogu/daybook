use anyhow::{Context, Result};
use serde::Deserialize;
use std::io::{self, Read};
use std::path::PathBuf;

// Shared Claude, Codex, and Pi hook stdin.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct HookInput {
    #[serde(default)]
    pub board_idle: bool,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub board_delivered: Option<Vec<i64>>,
    #[serde(default)]
    pub board_legacy_cursor: Option<i64>,
    #[serde(default, deserialize_with = "nullable_path")]
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

fn nullable_path<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
    Ok(Option::<PathBuf>::deserialize(deserializer)?.unwrap_or_default())
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
    fn parse_codex_post_tool_input_with_null_transcript() {
        let input: HookInput = serde_json::from_value(serde_json::json!({
            "session_id": "019a-codex-thread", "turn_id": "turn-1",
            "cwd": "/repo", "hook_event_name": "PostToolUse",
            "transcript_path": null, "model": "gpt-6.1-sol",
            "permission_mode": "default", "tool_name": "Bash",
            "tool_use_id": "call-1", "tool_input": {"command": "cat README.md"},
            "tool_response": {"exit_code": 0}
        }))
        .unwrap();
        assert_eq!(input.session_id, "019a-codex-thread");
        assert!(input.transcript_path.as_os_str().is_empty());
        assert_eq!(input.edited_file_path(), None);
    }

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
