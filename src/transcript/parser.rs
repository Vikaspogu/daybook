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
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TranscriptData {
    pub entries: Vec<TranscriptEntry>,
    pub user_messages: Vec<String>,
    pub assistant_messages: Vec<String>,
    pub tool_calls: Vec<ToolCall>,
    pub files_modified: Vec<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone)]
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
}
