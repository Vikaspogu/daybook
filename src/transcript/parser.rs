use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// A single entry in the Claude Code transcript JSONL file
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TranscriptEntry {
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    pub role: Option<String>,
    pub content: Option<serde_json::Value>,
    pub timestamp: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub tool_response: Option<serde_json::Value>,
    pub summary: Option<String>,
    // Capture any additional fields
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

/// Parsed transcript data with extracted information
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct TranscriptData {
    pub entries: Vec<TranscriptEntry>,
    pub user_messages: Vec<String>,
    pub assistant_messages: Vec<String>,
    pub tool_calls: Vec<ToolCall>,
    pub files_modified: Vec<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct ToolCall {
    pub name: String,
    pub input: serde_json::Value,
    pub response: Option<serde_json::Value>,
}

impl TranscriptData {
    /// Check if the session is empty (no meaningful user interaction)
    pub fn is_empty(&self) -> bool {
        // A session is empty if there are no user messages
        // or all user messages are blank/whitespace only
        if self.user_messages.is_empty() {
            return true;
        }

        // Check if all user messages are just whitespace
        self.user_messages.iter().all(|msg| msg.trim().is_empty())
    }
}

/// Parser for Claude Code transcript JSONL files
pub struct TranscriptParser;

impl TranscriptParser {
    /// Extract message content from a transcript entry
    /// Handles both old format (content field) and new format (message.content in extra)
    fn extract_message_content(entry: &TranscriptEntry) -> Option<String> {
        // First try the old format: direct content field
        if let Some(content) = &entry.content {
            if let Some(text) = content.as_str() {
                return Some(text.to_string());
            }
        }

        // Try new format: message.content in extra fields
        if let Some(message) = entry.extra.get("message") {
            if let Some(content) = message.get("content") {
                // Content can be a string or an array of content blocks
                if let Some(text) = content.as_str() {
                    return Some(text.to_string());
                }

                // If it's an array, extract text from content blocks
                if let Some(arr) = content.as_array() {
                    let texts: Vec<String> = arr
                        .iter()
                        .filter_map(|block| {
                            // Extract "text" type content blocks
                            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                                block.get("text").and_then(|t| t.as_str()).map(String::from)
                            } else {
                                None
                            }
                        })
                        .collect();

                    if !texts.is_empty() {
                        return Some(texts.join("\n"));
                    }
                }
            }
        }

        None
    }

    /// Parse a transcript file and extract relevant information
    pub fn parse<P: AsRef<Path>>(path: P) -> Result<TranscriptData> {
        let file = File::open(path.as_ref()).context("Failed to open transcript file")?;
        let reader = BufReader::new(file);

        let mut entries = Vec::new();
        let mut user_messages = Vec::new();
        let mut assistant_messages = Vec::new();
        let mut tool_calls = Vec::new();
        let mut files_modified = Vec::new();
        let mut summary = None;
        let mut claude_call_ids: Vec<(usize, String)> = Vec::new();
        let mut claude_responses: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();

        for line in reader.lines() {
            let line = line.context("Failed to read line")?;
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<TranscriptEntry>(&line) {
                Ok(entry) => {
                    // Extract user messages
                    // Support both old format (role: "user") and new format (type: "user")
                    if entry.role.as_deref() == Some("user")
                        || entry.entry_type.as_deref() == Some("user")
                    {
                        if let Some(text) = Self::extract_message_content(&entry) {
                            user_messages.push(text);
                        }
                    }

                    // Extract assistant messages
                    // Support both old format (role: "assistant") and new format (type: "assistant")
                    if entry.role.as_deref() == Some("assistant")
                        || entry.entry_type.as_deref() == Some("assistant")
                    {
                        if let Some(text) = Self::extract_message_content(&entry) {
                            assistant_messages.push(text);
                        }
                    }

                    // Claude stores calls and results in message.content blocks.
                    if let Some(blocks) = entry
                        .extra
                        .get("message")
                        .and_then(|m| m.get("content"))
                        .and_then(serde_json::Value::as_array)
                    {
                        for block in blocks {
                            match block.get("type").and_then(|v| v.as_str()) {
                                Some("tool_use") => {
                                    let name = block
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown")
                                        .to_string();
                                    let input = block
                                        .get("input")
                                        .cloned()
                                        .unwrap_or(serde_json::Value::Null);
                                    if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
                                        claude_call_ids.push((tool_calls.len(), id.to_string()));
                                    }
                                    if matches!(name.as_str(), "Write" | "Edit") {
                                        if let Some(path) =
                                            input.get("file_path").and_then(|v| v.as_str())
                                        {
                                            if !files_modified.iter().any(|p| p == path) {
                                                files_modified.push(path.to_string());
                                            }
                                        }
                                    }
                                    tool_calls.push(ToolCall {
                                        name,
                                        input,
                                        response: None,
                                    });
                                }
                                Some("tool_result") => {
                                    if let Some(id) =
                                        block.get("tool_use_id").and_then(|v| v.as_str())
                                    {
                                        let text = match block.get("content") {
                                            Some(serde_json::Value::String(text)) => text.clone(),
                                            Some(serde_json::Value::Array(parts)) => parts
                                                .iter()
                                                .filter_map(|part| {
                                                    part.get("text").and_then(|v| v.as_str())
                                                })
                                                .collect::<Vec<_>>()
                                                .join("\n"),
                                            Some(value) => value.to_string(),
                                            None => String::new(),
                                        };
                                        let response =
                                            if block.get("is_error").and_then(|v| v.as_bool())
                                                == Some(true)
                                            {
                                                serde_json::json!({"is_error": true, "text": text})
                                            } else {
                                                serde_json::Value::String(text)
                                            };
                                        claude_responses.insert(id.to_string(), response);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    // Extract tool calls
                    if let Some(tool_name) = &entry.tool_name {
                        let tool_call = ToolCall {
                            name: tool_name.clone(),
                            input: entry.tool_input.clone().unwrap_or(serde_json::Value::Null),
                            response: entry.tool_response.clone(),
                        };

                        // Track file modifications
                        if tool_name == "Write" || tool_name == "Edit" {
                            if let Some(input) = &entry.tool_input {
                                if let Some(file_path) =
                                    input.get("file_path").and_then(|v| v.as_str())
                                {
                                    if !files_modified.contains(&file_path.to_string()) {
                                        files_modified.push(file_path.to_string());
                                    }
                                }
                            }
                        }

                        tool_calls.push(tool_call);
                    }

                    // Extract summary if present
                    if entry.entry_type.as_deref() == Some("TranscriptSummary") {
                        summary = entry.summary.clone();
                    }

                    entries.push(entry);
                }
                Err(e) => {
                    // Log but don't fail on parse errors for individual lines
                    eprintln!("[daybook] Warning: Failed to parse transcript line: {}", e);
                }
            }
        }
        for (index, id) in claude_call_ids {
            if let Some(response) = claude_responses.get(&id) {
                tool_calls[index].response = Some(response.clone());
            }
        }

        Ok(TranscriptData {
            entries,
            user_messages,
            assistant_messages,
            tool_calls,
            files_modified,
            summary,
        })
    }

    /// Get a condensed text representation of the transcript for summarization
    pub fn to_condensed_text(data: &TranscriptData) -> String {
        let mut text = String::new();

        // Add user messages
        if !data.user_messages.is_empty() {
            text.push_str("## User Requests\n\n");
            for (i, msg) in data.user_messages.iter().enumerate() {
                text.push_str(&format!("{}. {}\n\n", i + 1, truncate_text(msg, 500)));
            }
        }

        // Keep ordered head-and-tail evidence so final outcomes survive condensation.
        if !data.assistant_messages.is_empty() {
            text.push_str("## Assistant Messages\n\n");
            let mut previous = None;
            for index in bounded_indices(data.assistant_messages.len(), 20) {
                if let Some(previous) = previous {
                    if index > previous + 1 {
                        text.push_str(&format!(
                            "- ... {} assistant messages omitted ...\n",
                            index - previous - 1
                        ));
                    }
                }
                text.push_str(&format!(
                    "- [{}] {}\n",
                    index + 1,
                    truncate_text(&data.assistant_messages[index], 300)
                ));
                previous = Some(index);
            }
            text.push('\n');
        }

        // Add tool usage summary
        if !data.tool_calls.is_empty() {
            text.push_str("## Tools Used\n\n");
            let mut tool_counts: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            for call in &data.tool_calls {
                *tool_counts.entry(&call.name).or_insert(0) += 1;
            }
            for (tool, count) in tool_counts {
                text.push_str(&format!("- {}: {} calls\n", tool, count));
            }

            // Include early failures and late verification without dropping chronology.
            let outcomes: Vec<_> = data
                .tool_calls
                .iter()
                .enumerate()
                .filter(|(_, call)| {
                    call.response
                        .as_ref()
                        .is_some_and(tool_response_has_content)
                })
                .collect();
            let mut previous = None;
            for outcome_index in bounded_indices(outcomes.len(), 15) {
                if let Some(previous) = previous {
                    if outcome_index > previous + 1 {
                        text.push_str(&format!(
                            "- ... {} tool outcomes omitted ...\n",
                            outcome_index - previous - 1
                        ));
                    }
                }
                let (index, call) = outcomes[outcome_index];
                if let Some(response) = &call.response {
                    let resp = tool_response_text(response);
                    text.push_str(&format!(
                        "- [{}] {} -> {}\n",
                        index + 1,
                        call.name,
                        truncate_text(&resp, 200)
                    ));
                }
                previous = Some(outcome_index);
            }
            text.push('\n');
        }

        // Add files modified
        if !data.files_modified.is_empty() {
            text.push_str("## Files Modified\n\n");
            for file in &data.files_modified {
                text.push_str(&format!("- {}\n", file));
            }
            text.push('\n');
        }

        // Add existing summary if available
        if let Some(summary) = &data.summary {
            text.push_str("## Existing Summary\n\n");
            text.push_str(summary);
            text.push('\n');
        }

        text
    }
}

fn tool_response_has_content(response: &serde_json::Value) -> bool {
    use serde_json::Value;
    match response {
        Value::Null => false,
        Value::String(text) => !text.trim().trim_matches('"').trim().is_empty(),
        Value::Object(map) if map.get("is_error").and_then(Value::as_bool) == Some(true) => map
            .get("text")
            .and_then(Value::as_str)
            .is_none_or(|text| !text.trim().is_empty()),
        _ => true,
    }
}

fn tool_response_text(response: &serde_json::Value) -> String {
    // Pi error results are {"is_error": true, "text": ...}.
    let text = match response.get("text").and_then(|t| t.as_str()) {
        Some(text) if response.get("is_error").and_then(|v| v.as_bool()) == Some(true) => {
            format!("ERROR: {text}")
        }
        _ => response.to_string(),
    };
    text.trim().trim_matches('"').to_string()
}

fn bounded_indices(len: usize, limit: usize) -> Vec<usize> {
    if len <= limit {
        return (0..len).collect();
    }
    let head = limit / 2;
    (0..head).chain(len - (limit - head)..len).collect()
}

/// Truncate text to a maximum length, adding ellipsis if needed
/// Handles UTF-8 character boundaries correctly
fn truncate_text(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_len).collect();
        format!("{}...", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_text() {
        assert_eq!(truncate_text("short", 10), "short");
        assert_eq!(truncate_text("this is a longer text", 10), "this is a ...");
    }

    fn create_empty_transcript_data() -> TranscriptData {
        TranscriptData {
            entries: vec![],
            user_messages: vec![],
            assistant_messages: vec![],
            tool_calls: vec![],
            files_modified: vec![],
            summary: None,
        }
    }

    #[test]
    fn test_is_empty_no_messages() {
        let data = create_empty_transcript_data();
        assert!(data.is_empty());
    }

    #[test]
    fn test_is_empty_whitespace_only() {
        let mut data = create_empty_transcript_data();
        data.user_messages = vec!["   ".to_string(), "\n\t".to_string()];
        assert!(data.is_empty());
    }

    #[test]
    fn test_is_empty_with_content() {
        let mut data = create_empty_transcript_data();
        data.user_messages = vec!["Hello, world!".to_string()];
        assert!(!data.is_empty());
    }

    #[test]
    fn test_is_empty_mixed_messages() {
        let mut data = create_empty_transcript_data();
        data.user_messages = vec!["   ".to_string(), "actual content".to_string()];
        assert!(!data.is_empty());
    }
    #[test]
    fn condensed_text_keeps_later_completion_and_verification() {
        let mut data = create_empty_transcript_data();
        data.user_messages = vec![
            "Fix the duplicate chat".into(),
            "Fix the duplicate chat".into(),
        ];
        data.assistant_messages = (0..30).map(|i| format!("Investigation step {i}")).collect();
        data.assistant_messages.push(
            "Completed the duplicate chat fix. Verification: cargo test --quiet: 42 passed.".into(),
        );
        data.tool_calls = (0..20)
            .map(|i| ToolCall {
                name: "bash".into(),
                input: serde_json::Value::Null,
                response: Some(serde_json::json!(format!("check {i}"))),
            })
            .collect();
        data.tool_calls.push(ToolCall {
            name: "bash".into(),
            input: serde_json::Value::Null,
            response: Some(serde_json::json!("final verification: 42 passed")),
        });
        data.tool_calls.extend((0..2).map(|_| ToolCall {
            name: "read".into(),
            input: serde_json::Value::Null,
            response: None,
        }));
        data.tool_calls.extend((0..20).map(|i| ToolCall {
            name: "read".into(),
            input: serde_json::Value::Null,
            response: Some(match i % 3 {
                0 => serde_json::Value::Null,
                1 => serde_json::json!(""),
                _ => serde_json::json!({"is_error": true, "text": ""}),
            }),
        }));

        let text = TranscriptParser::to_condensed_text(&data);
        let completion =
            "Completed the duplicate chat fix. Verification: cargo test --quiet: 42 passed.";
        assert!(text.contains(completion));
        assert!(text.contains("final verification: 42 passed"));
        assert!(text.contains("- [21] bash -> final verification: 42 passed"));
        assert!(text.contains("11 assistant messages omitted"));
        assert!(text.contains("6 tool outcomes omitted"));
        assert!(text.find("Investigation step 0").unwrap() < text.find(completion).unwrap());
        assert!(!text.contains("Investigation step 15"));
    }

    #[test]
    fn parses_claude_nested_tool_calls_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"call-1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
                "\n",
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","is_error":true,"content":[{"type":"text","text":"exit 1"}]}]}}"#,
                "\n"
            ),
        )
        .unwrap();
        let data = TranscriptParser::parse(path).unwrap();
        assert_eq!(data.tool_calls.len(), 1);
        assert_eq!(data.tool_calls[0].input["command"], "cargo test");
        assert_eq!(
            data.tool_calls[0].response,
            Some(serde_json::json!({"is_error": true, "text": "exit 1"}))
        );
    }

    #[test]
    fn claude_file_activity_requires_a_write_tool() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"read-1","name":"Read","input":{"file_path":"src/read.rs"}}]}}"#,
                "\n",
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"edit-1","name":"Edit","input":{"file_path":"src/edit.rs"}}]}}"#,
                "\n"
            ),
        )
        .unwrap();
        let data = TranscriptParser::parse(path).unwrap();
        assert_eq!(data.files_modified, vec!["src/edit.rs"]);
    }
}
