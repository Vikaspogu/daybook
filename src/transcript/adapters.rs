use anyhow::{Context, Result};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::parser::{ToolCall, TranscriptData};

/// Which agent runtime a transcript belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    ClaudeCode,
    Pi,
    Codex,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Pi => "pi",
            Self::Codex => "codex",
        }
    }
}

/// A discovered session transcript with metadata from its header line.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub path: PathBuf,
    pub session_id: String,
    pub agent: AgentKind,
    pub cwd: Option<String>,
}

impl SessionMeta {
    /// Project attribution derived from the session cwd.
    ///
    /// T3 worktrees (`~/.t3/worktrees/<project>/<branch>`) attribute to the
    /// parent project; git worktrees (`.git` file pointer) to the main repo;
    /// otherwise the nearest enclosing git root's directory name, falling
    /// back to the last path component.
    /// ponytail: T3 scratch dirs carry no project marker, so they stay
    /// unattributed (shown as "-") until T3 adds one.
    pub fn project(&self) -> Option<String> {
        let cwd = self.cwd.as_deref()?;
        // Only absolute paths attribute; a relative cwd says nothing safe.
        if !cwd.starts_with('/') {
            return None;
        }
        if let Some(rest) = cwd.split("/.t3/worktrees/").nth(1) {
            return rest
                .split('/')
                .next()
                .filter(|seg| !seg.is_empty() && !seg.contains(".."))
                .map(String::from);
        }
        if cwd.contains("/.t3/scratch/") {
            return None;
        }
        if let Some(name) = resolve_git_project(Path::new(cwd)) {
            return Some(name);
        }
        Path::new(cwd)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
    }
}

/// Walk up from `start` to the nearest directory holding `.git`.
/// A `.git` file (linked worktree) resolves through its `gitdir:` pointer.
fn resolve_git_project(start: &Path) -> Option<String> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let git = d.join(".git");
        if git.is_dir() {
            return d.file_name().map(|n| n.to_string_lossy().to_string());
        }
        if git.is_file() {
            if let Ok(txt) = std::fs::read_to_string(&git) {
                if let Some(gitdir) = txt.strip_prefix("gitdir: ") {
                    // <repo>/.git/worktrees/<name> -> <repo>
                    let repo = Path::new(gitdir.trim()).ancestors().nth(3);
                    return repo
                        .and_then(|r| r.file_name())
                        .map(|n| n.to_string_lossy().to_string());
                }
            }
            return d.file_name().map(|n| n.to_string_lossy().to_string());
        }
        dir = d.parent();
    }
    None
}

/// Token usage for one session (uncached input/output tokens; per-turn
/// inputs are summed, so totals are an upper bound — good enough for journal stats).
#[derive(Debug, Default, Clone, Copy)]
pub struct SessionUsage {
    pub input: u64,
    pub output: u64,
}

/// Parsed session: transcript data plus metadata extracted during parse.
#[derive(Debug)]
pub struct ParsedSession {
    // ponytail: unused until S3 summaries; parsers fill it for every session
    #[allow(dead_code)]
    pub data: TranscriptData,
    pub title: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub model: Option<String>,
    pub usage: Option<SessionUsage>,
    /// True when this session is a daybook summarizer run itself (claude -p
    /// or codex exec spawned by daybook). Discovered via the first user
    /// message; without this, every scan feeds its own LLM calls back into
    /// the next scan (self-amplifying transcript growth).
    pub internal: bool,
}

impl ParsedSession {
    /// Marker detection: daybook prompts always start with one of these
    /// fixed template lines (EN + ZH defaults; a fully custom prompt
    /// template would need its own first line here).
    fn from_daybook_prompt(data: &TranscriptData) -> bool {
        const MARKERS: [&str; 2] = [
            "You are analyzing a Claude Code session transcript",
            "你正在分析一个 Claude Code 会话记录",
        ];
        // codex exec prepends AGENTS.md as a user message, so the prompt is
        // not always first — check the first few user messages.
        data.user_messages
            .iter()
            .take(3)
            .any(|m| MARKERS.iter().any(|mk| m.trim_start().starts_with(mk)))
    }
}

impl ParsedSession {
    /// Local calendar date of the session start (D6: journal date comes from
    /// the transcript start timestamp, never from scan time).
    pub fn start_date(&self) -> Option<String> {
        self.start
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string()
            })
    }

    /// Human-readable session duration from start to end timestamps.
    pub fn duration(&self) -> Option<String> {
        let start = chrono::DateTime::parse_from_rfc3339(self.start.as_deref()?).ok()?;
        let end = chrono::DateTime::parse_from_rfc3339(self.end.as_deref()?).ok()?;
        let secs = (end - start).num_seconds().max(0);
        let (h, rem) = (secs / 3600, secs % 3600);
        let (m, s) = (rem / 60, rem % 60);
        Some(if h > 0 {
            format!("{}h {:02}m {:02}s", h, m, s)
        } else if m > 0 {
            format!("{}m {:02}s", m, s)
        } else {
            format!("{}s", s)
        })
    }
}

/// Discover session transcripts for all supported agent runtimes under `home`.
pub fn discover_sessions(home: &Path) -> Vec<SessionMeta> {
    let mut sessions = discover_claude(home);
    sessions.extend(discover_pi(home));
    sessions.extend(discover_codex(home));
    sessions
}

/// Build a SessionMeta for an explicit transcript path, inferring the
/// runtime from its location under the home directory. Returns None for
/// paths outside the known session trees.
pub fn meta_for_path(path: &Path) -> Option<SessionMeta> {
    let s = path.to_string_lossy();
    if s.contains("/.claude/projects/") {
        let cwd = first_line_json(path)
            .and_then(|h| h.get("cwd").and_then(|v| v.as_str()).map(String::from));
        return Some(SessionMeta {
            session_id: path
                .file_stem()
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or_default(),
            path: path.to_path_buf(),
            agent: AgentKind::ClaudeCode,
            cwd,
        });
    }
    if s.contains("/.pi/agent/sessions/") {
        return pi_meta(path);
    }
    if s.contains("/.codex/sessions/") {
        return codex_meta(path);
    }
    None
}

/// Claude Code: ~/.claude/projects/<project-dir>/<session-id>.jsonl
fn discover_claude(home: &Path) -> Vec<SessionMeta> {
    let mut out = Vec::new();
    let projects = home.join(".claude").join("projects");
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            // Skip agent transcripts (temporary agent sessions)
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("agent-") {
                continue;
            }
            let cwd = first_line_json(&path)
                .and_then(|h| h.get("cwd").and_then(|v| v.as_str()).map(String::from));
            out.push(SessionMeta {
                session_id: path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path,
                agent: AgentKind::ClaudeCode,
                cwd,
            });
        }
    }
    out
}

/// Pi: ~/.pi/agent/sessions/<sanitized-cwd>/<timestamp>_<uuid>.jsonl
/// First line: {"type":"session","id":...,"cwd":...}
fn discover_pi(home: &Path) -> Vec<SessionMeta> {
    let mut out = Vec::new();
    let sessions_dir = home.join(".pi").join("agent").join("sessions");
    let Ok(dirs) = std::fs::read_dir(&sessions_dir) else {
        return out;
    };
    for entry in dirs.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(meta) = pi_meta(&path) {
                out.push(meta);
            }
        }
    }
    out
}

fn pi_meta(path: &Path) -> Option<SessionMeta> {
    let header = first_line_json(path)?;
    if header.get("type")?.as_str()? != "session" {
        return None;
    }
    Some(SessionMeta {
        path: path.to_path_buf(),
        session_id: header
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        agent: AgentKind::Pi,
        cwd: header.get("cwd").and_then(|v| v.as_str()).map(String::from),
    })
}

/// Codex: ~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl
/// First line: {"type":"session_meta","payload":{"id":...,"cwd":...}}
fn discover_codex(home: &Path) -> Vec<SessionMeta> {
    let mut out = Vec::new();
    let sessions_dir = home.join(".codex").join("sessions");
    // Walk up to 3 levels of date directories (YYYY/MM/DD)
    let Ok(years) = std::fs::read_dir(&sessions_dir) else {
        return out;
    };
    for year in years.flatten() {
        let Ok(months) = std::fs::read_dir(year.path()) else {
            continue;
        };
        for month in months.flatten() {
            let Ok(days) = std::fs::read_dir(month.path()) else {
                continue;
            };
            for day in days.flatten() {
                let Ok(files) = std::fs::read_dir(day.path()) else {
                    continue;
                };
                for file in files.flatten() {
                    let path = file.path();
                    if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("jsonl")
                    {
                        continue;
                    }
                    if let Some(meta) = codex_meta(&path) {
                        out.push(meta);
                    }
                }
            }
        }
    }
    out
}

fn codex_meta(path: &Path) -> Option<SessionMeta> {
    let header = first_line_json(path)?;
    if header.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let payload = header.get("payload")?;
    Some(SessionMeta {
        path: path.to_path_buf(),
        session_id: payload
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        agent: AgentKind::Codex,
        cwd: payload
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

/// First YYYY-MM-DD date embedded in the file name, if any (Pi and Codex
/// encode the UTC start time in transcript names). Cheap scan prefilter.
pub fn filename_date(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let b = name.as_bytes();
    for i in 0..b.len().saturating_sub(9) {
        if b[i..i + 4].iter().all(|c| c.is_ascii_digit())
            && b[i + 4] == b'-'
            && b[i + 5..i + 7].iter().all(|c| c.is_ascii_digit())
            && b[i + 7] == b'-'
            && b[i + 8..i + 10].iter().all(|c| c.is_ascii_digit())
        {
            return Some(name[i..i + 10].to_string());
        }
    }
    None
}

fn first_line_json(path: &Path) -> Option<Value> {
    let file = File::open(path).ok()?;
    let mut line = String::new();
    BufReader::new(file).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

/// Parse a discovered session with the right runtime parser.
pub fn parse_session(meta: &SessionMeta) -> Result<ParsedSession> {
    match meta.agent {
        AgentKind::ClaudeCode => {
            let data = super::TranscriptParser::parse(&meta.path)?;
            let mut start = None;
            let mut end = None;
            let mut model = None;
            let mut usage = SessionUsage::default();
            for entry in &data.entries {
                if let Some(ts) = &entry.timestamp {
                    if start.is_none() {
                        start = Some(ts.clone());
                    }
                    end = Some(ts.clone());
                }
                if let Some(message) = entry.extra.get("message") {
                    if model.is_none() {
                        if let Some(m) = message.get("model").and_then(|v| v.as_str()) {
                            model = Some(m.to_string());
                        }
                    }
                    if let Some(u) = message.get("usage") {
                        usage.input = usage.input.saturating_add(
                            u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        );
                        usage.output = usage.output.saturating_add(
                            u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        );
                    }
                }
            }
            let usage = (usage.input.saturating_add(usage.output) > 0).then_some(usage);
            let internal = ParsedSession::from_daybook_prompt(&data);
            Ok(ParsedSession {
                internal,
                data,
                title: None,
                start,
                end,
                model,
                usage,
            })
        }
        AgentKind::Pi => parse_pi(&meta.path),
        AgentKind::Codex => parse_codex(&meta.path),
    }
}

fn parse_pi(path: &Path) -> Result<ParsedSession> {
    // Lossy whole-file read: a truncated final line or stray non-UTF-8 byte
    // degrades that line only, never the whole session.
    let bytes = std::fs::read(path).context("Failed to read Pi transcript")?;
    let text = String::from_utf8_lossy(&bytes);

    let mut data = TranscriptData::default();
    let mut title = None;
    let mut start = None;
    let mut end = None;
    let mut model = None;
    let mut usage = SessionUsage::default();
    // toolCallId -> response text, matched back to calls after the loop
    let mut call_ids: Vec<(usize, String)> = Vec::new();
    let mut tool_responses: std::collections::HashMap<String, Value> =
        std::collections::HashMap::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(ts) = entry.get("timestamp").and_then(|t| t.as_str()) {
            if start.is_none() {
                start = Some(ts.to_string());
            }
            end = Some(ts.to_string());
        }

        match entry.get("type").and_then(|t| t.as_str()) {
            Some("model_change") => {
                if model.is_none() {
                    model = entry
                        .get("modelId")
                        .and_then(|m| m.as_str())
                        .map(String::from);
                }
            }
            Some("session_info") => {
                title = entry
                    .get("name")
                    .and_then(|n| n.as_str())
                    .filter(|n| !n.is_empty())
                    .map(String::from);
            }
            Some("message") => {
                let message = entry.get("message").unwrap_or(&Value::Null);
                let role = message.get("role").and_then(|r| r.as_str());
                if model.is_none() {
                    model = message
                        .get("model")
                        .and_then(|m| m.as_str())
                        .map(String::from);
                }
                if let Some(u) = message.get("usage") {
                    usage.input = usage
                        .input
                        .saturating_add(u.get("input").and_then(|v| v.as_u64()).unwrap_or(0));
                    usage.output = usage
                        .output
                        .saturating_add(u.get("output").and_then(|v| v.as_u64()).unwrap_or(0));
                }
                let Some(content) = message.get("content").and_then(|c| c.as_array()) else {
                    continue;
                };
                match role {
                    Some("user") => {
                        if let Some(text) = join_text_blocks(content, "text") {
                            data.user_messages.push(text);
                        }
                    }
                    Some("assistant") => {
                        for block in content {
                            match block.get("type").and_then(|t| t.as_str()) {
                                Some("text") => {
                                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                        data.assistant_messages.push(t.to_string());
                                    }
                                }
                                Some("toolCall") => {
                                    let name = block
                                        .get("name")
                                        .and_then(|n| n.as_str())
                                        .unwrap_or("unknown")
                                        .to_string();
                                    let input =
                                        block.get("arguments").cloned().unwrap_or(Value::Null);
                                    if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
                                        call_ids.push((data.tool_calls.len(), id.to_string()));
                                    }
                                    track_file_modified(&mut data, &name, &input);
                                    data.tool_calls.push(ToolCall {
                                        name,
                                        input,
                                        response: None,
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                    Some("toolResult") => {
                        if let (Some(id), Some(text)) = (
                            message.get("toolCallId").and_then(|v| v.as_str()),
                            message
                                .get("content")
                                .and_then(|c| c.as_array())
                                .and_then(|c| join_text_blocks(c, "text")),
                        ) {
                            tool_responses.insert(id.to_string(), Value::String(text));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    for (idx, id) in call_ids {
        if let Some(resp) = tool_responses.get(&id) {
            data.tool_calls[idx].response = Some(resp.clone());
        }
    }

    let usage = (usage.input.saturating_add(usage.output) > 0).then_some(usage);
    let internal = ParsedSession::from_daybook_prompt(&data);
    Ok(ParsedSession {
        internal,
        data,
        title,
        start,
        end,
        model,
        usage,
    })
}

fn parse_codex(path: &Path) -> Result<ParsedSession> {
    // Lossy whole-file read (see parse_pi).
    let bytes = std::fs::read(path).context("Failed to read Codex transcript")?;
    let text = String::from_utf8_lossy(&bytes);

    let mut data = TranscriptData::default();
    let mut start = None;
    let mut end = None;
    let mut model = None;
    let mut record_usage = SessionUsage::default();
    let mut count_usage: Option<SessionUsage> = None;
    let mut call_ids: Vec<(usize, String)> = Vec::new();
    let mut tool_responses: std::collections::HashMap<String, Value> =
        std::collections::HashMap::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(ts) = entry.get("timestamp").and_then(|t| t.as_str()) {
            if start.is_none() {
                start = Some(ts.to_string());
            }
            end = Some(ts.to_string());
        }
        // Token accounting: prefer token_usage_record (per-turn), fall back
        // to the last token_count.total_token_usage (cumulative).
        match entry.get("type").and_then(|t| t.as_str()) {
            Some("token_usage_record") => {
                if let Some(u) = entry.get("payload").and_then(|p| p.get("usage")) {
                    record_usage.input = record_usage.input.saturating_add(
                        u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                    );
                    record_usage.output = record_usage.output.saturating_add(
                        u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                    );
                }
            }
            Some("token_count") => {
                if let Some(total) = entry.get("info").and_then(|i| i.get("total_token_usage")) {
                    count_usage = Some(SessionUsage {
                        input: total
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        output: total
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                    });
                }
            }
            Some("event_msg") => {
                // Real Codex rollouts wrap token_count as an event_msg payload.
                if let Some(total) = entry
                    .get("payload")
                    .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("token_count"))
                    .and_then(|p| p.get("info"))
                    .and_then(|i| i.get("total_token_usage"))
                {
                    count_usage = Some(SessionUsage {
                        input: total
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        output: total
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                    });
                }
            }
            Some("turn_context") => {
                if model.is_none() {
                    model = entry
                        .get("payload")
                        .and_then(|p| p.get("model"))
                        .and_then(|m| m.as_str())
                        .map(String::from);
                }
            }
            _ => {}
        }
        if entry.get("type").and_then(|t| t.as_str()) != Some("response_item") {
            continue;
        }
        let Some(payload) = entry.get("payload") else {
            continue;
        };

        match payload.get("type").and_then(|t| t.as_str()) {
            Some("message") => {
                let role = payload.get("role").and_then(|r| r.as_str());
                let Some(content) = payload.get("content").and_then(|c| c.as_array()) else {
                    continue;
                };
                match role {
                    Some("user") => {
                        if let Some(text) = join_text_blocks(content, "input_text") {
                            if !is_injected_context(&text) {
                                data.user_messages.push(text);
                            }
                        }
                    }
                    Some("assistant") => {
                        if let Some(text) = join_text_blocks(content, "output_text") {
                            data.assistant_messages.push(text);
                        }
                    }
                    _ => {}
                }
            }
            Some("custom_tool_call") => {
                let name = payload
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let input = match payload.get("input") {
                    Some(Value::String(s)) => Value::String(s.clone()),
                    Some(other) => other.clone(),
                    None => Value::Null,
                };
                // Codex edits files via apply_patch inside an exec call; the
                // patch header lines name every touched file.
                if let Value::String(s) = &input {
                    for line in s.lines() {
                        for marker in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
                            if let Some(p) = line.trim_start().strip_prefix(marker) {
                                let p = p.trim().to_string();
                                if !data.files_modified.contains(&p) {
                                    data.files_modified.push(p);
                                }
                            }
                        }
                    }
                }
                if let Some(id) = payload.get("call_id").and_then(|v| v.as_str()) {
                    call_ids.push((data.tool_calls.len(), id.to_string()));
                }
                data.tool_calls.push(ToolCall {
                    name,
                    input,
                    response: None,
                });
            }
            Some("function_call") => {
                if let Some(id) = payload.get("call_id").and_then(|v| v.as_str()) {
                    call_ids.push((data.tool_calls.len(), id.to_string()));
                }
                data.tool_calls.push(ToolCall {
                    name: payload
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    input: Value::String(
                        payload
                            .get("arguments")
                            .and_then(|a| a.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
                    response: None,
                });
            }
            Some("function_call_output") => {
                if let Some(id) = payload.get("call_id").and_then(|v| v.as_str()) {
                    if let Some(out) = payload.get("output").and_then(|o| o.as_str()) {
                        tool_responses.insert(id.to_string(), Value::String(out.to_string()));
                    }
                }
            }
            Some("custom_tool_call_output") => {
                if let Some(id) = payload.get("call_id").and_then(|v| v.as_str()) {
                    // Output is a text-block array, but some rollout versions
                    // serialize it as a plain string.
                    let text = payload.get("output").and_then(|o| {
                        o.as_array()
                            .and_then(|a| join_text_blocks(a, "input_text"))
                            .or_else(|| o.as_str().map(str::to_string))
                    });
                    if let Some(text) = text {
                        tool_responses.insert(id.to_string(), Value::String(text));
                    }
                }
            }
            _ => {}
        }
    }

    for (idx, id) in call_ids {
        if let Some(resp) = tool_responses.get(&id) {
            data.tool_calls[idx].response = Some(resp.clone());
        }
    }

    let usage = if record_usage.input.saturating_add(record_usage.output) > 0 {
        Some(record_usage)
    } else {
        count_usage
    };
    let internal = ParsedSession::from_daybook_prompt(&data);
    Ok(ParsedSession {
        internal,
        data,
        title: None,
        start,
        end,
        model,
        usage,
    })
}

/// Join content blocks of the given type into a single string.
fn join_text_blocks(content: &[Value], block_type: &str) -> Option<String> {
    let texts: Vec<&str> = content
        .iter()
        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some(block_type))
        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
        .collect();
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

/// Codex injects boilerplate (AGENTS.md, environment context) as user-role
/// messages. Skip those so summaries reflect real user requests only.
fn is_injected_context(text: &str) -> bool {
    text.starts_with("# AGENTS.md")
        || text.starts_with("<environment_context>")
        || text.starts_with("<user_instructions>")
        || text.starts_with("<model_switch>")
        || text.starts_with("<turn_context>")
}

/// Track files modified by write-like tool calls across runtimes.
fn track_file_modified(data: &mut TranscriptData, tool_name: &str, input: &Value) {
    const WRITE_TOOLS: &[&str] = &["write", "replace", "edit"];
    if !WRITE_TOOLS.contains(&tool_name) {
        return;
    }
    let file_path = input
        .get("path")
        .or_else(|| input.get("file_path"))
        .and_then(|v| v.as_str());
    if let Some(file_path) = file_path {
        if !data.files_modified.contains(&file_path.to_string()) {
            data.files_modified.push(file_path.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_for_path_all_runtimes_and_unknown() {
        let base = std::env::temp_dir().join(format!("daybook-mfp-{}", std::process::id()));
        let mk = |rel: &str, line: &str| {
            let p = base.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, line).unwrap();
            p
        };

        let claude = mk(
            ".claude/projects/p/sess.jsonl",
            r#"{"cwd":"/home/agent/git-repos/daybook"}"#,
        );
        let m = meta_for_path(&claude).expect("claude meta");
        assert_eq!(m.agent, AgentKind::ClaudeCode);
        assert_eq!(m.session_id, "sess");
        assert_eq!(m.cwd.as_deref(), Some("/home/agent/git-repos/daybook"));

        let pi = mk(
            ".pi/agent/sessions/p/sess.jsonl",
            r#"{"type":"session","id":"01a-pi","cwd":"/home/agent/git-repos/daybook"}"#,
        );
        let m = meta_for_path(&pi).expect("pi meta");
        assert_eq!(m.agent, AgentKind::Pi);
        assert_eq!(m.session_id, "01a-pi");

        let codex = mk(
            ".codex/sessions/2026/10/28/sess.jsonl",
            r#"{"type":"session_meta","payload":{"id":"01a-codex","cwd":"/home/agent/git-repos/daybook"}}"#,
        );
        let m = meta_for_path(&codex).expect("codex meta");
        assert_eq!(m.agent, AgentKind::Codex);
        assert_eq!(m.session_id, "01a-codex");

        let outside = mk("plain/sess.jsonl", "{}");
        assert!(meta_for_path(&outside).is_none());

        let _ = std::fs::remove_dir_all(&base);
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn test_internal_marker_after_agents_md() {
        let mk = |first: Option<&str>, prompt: &str| ParsedSession {
            data: TranscriptData {
                user_messages: {
                    let mut v = Vec::new();
                    if let Some(f) = first {
                        v.push(f.to_string());
                    }
                    v.push(prompt.to_string());
                    v
                },
                ..Default::default()
            },
            title: None,
            model: None,
            start: None,
            end: None,
            usage: None,
            internal: false,
        };
        let prompt = "You are analyzing a Claude Code session transcript. Generate a summary.";
        // codex exec prepends AGENTS.md instructions as a user message
        let codex = mk(Some("# AGENTS.md instructions\n<INSTRUCTIONS>"), prompt);
        assert!(ParsedSession::from_daybook_prompt(&codex.data));
        // claude -p has the prompt as the first user message
        let claude = mk(None, prompt);
        assert!(ParsedSession::from_daybook_prompt(&claude.data));
        // a real interactive session does not match
        let real = mk(None, "Fix the login bug in daybook please");
        assert!(!ParsedSession::from_daybook_prompt(&real.data));
    }

    #[test]
    fn test_parse_pi_fixture() {
        let meta = pi_meta(&fixture("pi-session.jsonl")).expect("pi meta");
        assert_eq!(meta.session_id, "01abc-pi-session");
        assert_eq!(meta.cwd.as_deref(), Some("/home/agent/git-repos/daybook"));
        assert_eq!(meta.project().as_deref(), Some("daybook"));

        let parsed = parse_session(&meta).expect("pi parse");
        assert_eq!(parsed.title.as_deref(), Some("Pi fixture session"));
        assert_eq!(parsed.data.user_messages.len(), 1);
        assert!(parsed.data.user_messages[0].contains("Fix the login bug"));
        assert_eq!(
            parsed.data.assistant_messages,
            vec!["Done".to_string(), "final".to_string()]
        );
        assert_eq!(parsed.data.tool_calls.len(), 1);
        assert_eq!(parsed.data.tool_calls[0].name, "write");
        assert_eq!(
            parsed.data.tool_calls[0].response,
            Some(serde_json::Value::String("ok".to_string()))
        );
        assert_eq!(
            parsed.data.files_modified,
            vec!["/home/agent/git-repos/daybook/src/lib.rs".to_string()]
        );
    }

    #[test]
    fn test_parse_codex_fixture() {
        let meta = codex_meta(&fixture("codex-session.jsonl")).expect("codex meta");
        assert_eq!(meta.session_id, "01abc-codex-session");
        assert_eq!(
            meta.cwd.as_deref(),
            Some("/home/agent/git-repos/t3code-orbit")
        );
        assert_eq!(meta.project().as_deref(), Some("t3code-orbit"));

        let parsed = parse_session(&meta).expect("codex parse");
        assert_eq!(parsed.data.user_messages.len(), 1);
        // AGENTS.md injection must be filtered out
        assert!(parsed.data.user_messages[0].contains("Add a test"));
        assert_eq!(
            parsed.data.assistant_messages,
            vec!["Added the test".to_string()]
        );
        assert_eq!(parsed.data.tool_calls.len(), 2);
        assert_eq!(parsed.data.tool_calls[0].name, "exec");
        assert_eq!(
            parsed.data.tool_calls[0].response,
            Some(serde_json::Value::String("ok. 1 passed".to_string()))
        );
        assert_eq!(
            parsed.data.tool_calls[1].response,
            Some(serde_json::Value::String("Done! Applied patch".to_string()))
        );
        assert_eq!(
            parsed.data.files_modified,
            vec!["/home/agent/git-repos/t3code-orbit/src/auth.rs".to_string()]
        );
    }

    #[test]
    fn test_discover_sessions_from_temp_home() {
        let home = std::env::temp_dir().join("daybook-test-home");
        let _ = std::fs::remove_dir_all(&home);
        let pi_dir = home.join(".pi/agent/sessions/--home-agent-git-repos-daybook--");
        std::fs::create_dir_all(&pi_dir).unwrap();
        std::fs::copy(
            fixture("pi-session.jsonl"),
            pi_dir.join("2026-10-28T10-00-00-000Z_01abc.jsonl"),
        )
        .unwrap();

        let sessions = discover_sessions(&home);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent, AgentKind::Pi);
        assert_eq!(sessions[0].session_id, "01abc-pi-session");

        let _ = std::fs::remove_dir_all(&home);
    }

    fn expected_local_date(iso: &str) -> String {
        chrono::DateTime::parse_from_rfc3339(iso)
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string()
    }

    #[test]
    fn test_pi_metadata_extraction() {
        let meta = pi_meta(&fixture("pi-session.jsonl")).expect("pi meta");
        let parsed = parse_session(&meta).expect("pi parse");
        assert_eq!(parsed.start.as_deref(), Some("2026-10-28T10:00:00.000Z"));
        assert_eq!(parsed.end.as_deref(), Some("2026-10-28T10:00:06.000Z"));
        assert_eq!(parsed.model.as_deref(), Some("gpt-6.1-sol"));
        let usage = parsed.usage.expect("usage");
        assert_eq!(usage.input, 2720);
        assert_eq!(usage.output, 257);
        assert_eq!(
            parsed.start_date().as_deref(),
            Some(expected_local_date("2026-10-28T10:00:00.000Z").as_str())
        );
        assert!(parsed.duration().is_some());
    }

    #[test]
    fn test_codex_usage_prefers_token_usage_record() {
        let meta = codex_meta(&fixture("codex-session.jsonl")).expect("codex meta");
        let parsed = parse_session(&meta).expect("codex parse");
        assert_eq!(parsed.model.as_deref(), Some("gpt-6.1-sol"));
        // token_usage_record (1000/50) wins over token_count total (20000/90)
        let usage = parsed.usage.expect("usage");
        assert_eq!(usage.input, 1000);
        assert_eq!(usage.output, 50);
    }

    #[test]
    fn test_codex_string_output_and_dedup_files() {
        let meta = codex_meta(&fixture("codex-session-string-output.jsonl")).expect("codex meta");
        let parsed = parse_session(&meta).expect("codex parse");
        // Output serialized as a plain string is captured, not dropped.
        assert_eq!(
            parsed.data.tool_calls[0].response,
            Some(serde_json::Value::String("ok, applied".to_string()))
        );
        // Re-patching the same file lists it once.
        assert_eq!(
            parsed.data.files_modified,
            vec![
                "/home/agent/git-repos/t3code-orbit/src/auth.rs".to_string(),
                "/home/agent/git-repos/t3code-orbit/tests/auth.rs".to_string()
            ]
        );
    }

    #[test]
    fn test_codex_usage_falls_back_to_event_msg_token_count() {
        // Real Codex rollouts wrap token_count in event_msg; when no
        // token_usage_record exists, the wrapped total is the usage source.
        let meta = codex_meta(&fixture("codex-session-no-record.jsonl")).expect("codex meta");
        let parsed = parse_session(&meta).expect("codex parse");
        let usage = parsed.usage.expect("usage");
        assert_eq!(usage.input, 20000);
        assert_eq!(usage.output, 90);
    }

    #[test]
    fn test_unknown_event_types_skipped_not_crashed() {
        let meta = pi_meta(&fixture("pi-unknown-events.jsonl")).expect("pi meta");
        let parsed = parse_session(&meta).expect("pi parse");
        assert_eq!(parsed.data.user_messages.len(), 1);
        assert!(parsed.data.user_messages[0].contains("Real message after unknown event"));
    }

    #[test]
    fn test_truncated_jsonl_does_not_panic() {
        let meta = pi_meta(&fixture("pi-truncated.jsonl")).expect("pi meta");
        let parsed = parse_session(&meta).expect("pi parse");
        assert_eq!(parsed.data.user_messages.len(), 1);
        assert!(parsed.data.user_messages[0].contains("Message before the truncation"));
    }

    #[test]
    fn test_worktree_session_attributes_to_parent_project() {
        let meta = pi_meta(&fixture("pi-worktree-session.jsonl")).expect("pi meta");
        assert_eq!(meta.project().as_deref(), Some("t3code-orbit"));
    }

    #[test]
    fn test_scratch_session_stays_unattributed() {
        let meta = pi_meta(&fixture("pi-scratch-session.jsonl")).expect("pi meta");
        assert_eq!(meta.project(), None);
    }

    #[test]
    fn test_d6_journal_date_comes_from_start_timestamp() {
        let meta = pi_meta(&fixture("pi-yesterday-session.jsonl")).expect("pi meta");
        let parsed = parse_session(&meta).expect("pi parse");
        // Started 2026-10-27T23:00Z; journal date follows that instant in
        // local time, never the scan time.
        assert_eq!(
            parsed.start_date().as_deref(),
            Some(expected_local_date("2026-10-27T23:00:00.000Z").as_str())
        );
    }

    #[test]
    fn test_git_worktree_file_pointer_resolves_to_main_repo() {
        let base = std::env::temp_dir().join("daybook-worktree-test");
        let _ = std::fs::remove_dir_all(&base);
        let main = base.join("myproject");
        let wt = base.join("myproject-wt");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(main.join(".git/worktrees/feature")).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!(
                "gitdir: {}\n",
                main.join(".git/worktrees/feature").display()
            ),
        )
        .unwrap();

        let meta = SessionMeta {
            path: PathBuf::new(),
            session_id: "x".to_string(),
            agent: AgentKind::Pi,
            cwd: Some(wt.to_string_lossy().to_string()),
        };
        assert_eq!(meta.project().as_deref(), Some("myproject"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
