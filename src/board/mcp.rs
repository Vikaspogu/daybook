use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Arc;

use crate::board::client::BoardClient;
use crate::board::globs::{normalize_repo_path, validate_glob_pattern};
use crate::board::identity::{resolve_board_identity, BoardIdentity};
use crate::ledger::{
    BoardClaimLocksReq, BoardClaimTaskReq, BoardPostMessageReq, BoardReleaseLocksReq,
    BoardUpdateTaskReq,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpFraming {
    Newline,
    ContentLength,
}

/// Detect framing from the first non-whitespace byte in `reader`, then process requests until EOF.
pub async fn run_mcp_stdio(client: Arc<BoardClient>, cwd: &Path) -> Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    run_mcp_loop(&mut reader, &mut writer, &client, cwd).await
}

pub async fn run_mcp_loop<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    client: &Arc<BoardClient>,
    cwd: &Path,
) -> Result<()> {
    let identity = resolve_board_identity(cwd, None, None);
    // Self-register on session start (first tick fires immediately) and keep presence
    // fresh every 60s, so pi agents appear on the board without the model volunteering a
    // write tool call. Runs off the request path: a down board only delays registration
    // to the next tick, and never blocks the MCP handshake. Invalid workspaces are
    // skipped by ensure_registered itself. Agents go stale (~2h) after the session ends
    // because nothing releases on exit.
    let hb_client = Arc::clone(client);
    let hb_identity = identity.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            hb_client.ensure_registered(&hb_identity).await;
        }
    });
    let mut framing: Option<McpFraming> = None;

    while let Some((msg_str, detected)) = read_next_message(reader, framing)? {
        framing = Some(detected);
        let parsed: Value = match serde_json::from_str(&msg_str) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("Parse error: {e}") }
                });
                write_message(writer, &err_resp, detected)?;
                continue;
            }
        };

        if let Some(resp) = handle_rpc_message(&parsed, client, &identity).await {
            write_message(writer, &resp, detected)?;
        }
    }
    Ok(())
}

fn read_next_message<R: BufRead>(
    reader: &mut R,
    known_framing: Option<McpFraming>,
) -> Result<Option<(String, McpFraming)>> {
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(None);
        }
        // Skip leading whitespace between frames
        let ws_len = buf
            .iter()
            .take_while(|&&b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\n')
            .count();
        if ws_len > 0 {
            reader.consume(ws_len);
            continue;
        }
        let first_byte = buf[0];
        let framing = known_framing.unwrap_or(if first_byte == b'{' {
            McpFraming::Newline
        } else {
            McpFraming::ContentLength
        });

        return match framing {
            McpFraming::Newline => {
                let mut line = String::new();
                let n = reader.read_line(&mut line)?;
                if n == 0 {
                    Ok(None)
                } else {
                    Ok(Some((line.trim().to_string(), McpFraming::Newline)))
                }
            }
            McpFraming::ContentLength => {
                let mut content_length: Option<usize> = None;
                loop {
                    let mut header_line = String::new();
                    let n = reader.read_line(&mut header_line)?;
                    if n == 0 {
                        return Ok(None);
                    }
                    let trimmed = header_line.trim();
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = trimmed.split_once(':') {
                        if k.trim().eq_ignore_ascii_case("content-length") {
                            content_length = Some(
                                v.trim()
                                    .parse::<usize>()
                                    .context("invalid Content-Length value")?,
                            );
                        }
                    }
                }
                let len = content_length.context("missing Content-Length header")?;
                if len > 4 * 1024 * 1024 {
                    bail!("MCP message exceeds 4 MiB limit");
                }
                let mut body = vec![0u8; len];
                reader.read_exact(&mut body)?;
                let s = String::from_utf8(body).context("MCP body is not valid UTF-8")?;
                Ok(Some((s, McpFraming::ContentLength)))
            }
        };
    }
}

fn write_message<W: Write>(writer: &mut W, msg: &Value, framing: McpFraming) -> Result<()> {
    let payload = serde_json::to_string(msg)?;
    match framing {
        McpFraming::Newline => {
            writeln!(writer, "{payload}")?;
        }
        McpFraming::ContentLength => {
            write!(
                writer,
                "Content-Length: {}\r\n\r\n{}",
                payload.len(),
                payload
            )?;
        }
    }
    writer.flush()?;
    Ok(())
}

async fn handle_rpc_message(
    req: &Value,
    client: &BoardClient,
    identity: &BoardIdentity,
) -> Option<Value> {
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let id = req.get("id").cloned();

    // JSON-RPC notifications have no `id` and expect no reply
    if id.is_none() || method.starts_with("notifications/") {
        return None;
    }
    let id = id.unwrap_or(Value::Null);

    match method {
        "initialize" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "daybook-board",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        })),
        "ping" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {}
        })),
        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": mcp_tool_definitions()
            }
        })),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
            let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            match dispatch_tool_call(tool_name, &args, client, identity).await {
                Ok(text) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": text }],
                        "isError": false
                    }
                })),
                Err(e) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": format!("Error: {e:#}") }],
                        "isError": true
                    }
                })),
            }
        }
        _ => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {method}")
            }
        })),
    }
}

async fn dispatch_tool_call(
    tool_name: &str,
    args: &Value,
    client: &BoardClient,
    identity: &BoardIdentity,
) -> Result<String> {
    let project = args
        .get("project")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&identity.project)
        .to_string();
    let agent = args
        .get("agent")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&identity.agent)
        .to_string();
    let is_write_tool = matches!(
        tool_name,
        "daybook_board_claim"
            | "daybook_board_release"
            | "daybook_board_post"
            | "daybook_board_claim_task"
            | "daybook_board_update_task"
    );
    if is_write_tool && project == identity.project && agent == identity.agent {
        client.ensure_registered_debounced(identity).await;
    }

    match tool_name {
        "daybook_board_status" => {
            let since_id = args
                .get("since_id")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)));
            let snap = client
                .status(Some(&project), Some(&agent), since_id)
                .await?;
            Ok(serde_json::to_string_pretty(&snap)?)
        }
        "daybook_board_claim" => {
            let paths: Vec<String> = args
                .get("paths")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(|p| validate_glob_pattern(p, Some(&identity.worktree_root)))
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            let note = args
                .get("note")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let ttl_minutes = args
                .get("ttl_minutes")
                .and_then(Value::as_u64)
                .map(|v| v as u32);
            let res = client
                .claim_locks(&BoardClaimLocksReq {
                    project,
                    agent,
                    paths,
                    note,
                    ttl_minutes,
                })
                .await?;
            Ok(serde_json::to_string_pretty(&res)?)
        }
        "daybook_board_release" => {
            let lock_id = args
                .get("lock_id")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let all = args
                .get("all")
                .and_then(Value::as_bool)
                .unwrap_or(lock_id.is_none());
            let auto_only = args
                .get("auto_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
            let released = client
                .release_locks(&BoardReleaseLocksReq {
                    project,
                    agent,
                    lock_id,
                    all,
                    auto_only,
                    force,
                })
                .await?;
            Ok(serde_json::to_string_pretty(
                &json!({ "released": released }),
            )?)
        }
        "daybook_board_post" => {
            let channel = args
                .get("channel")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let body = args
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let parent_id = args.get("parent_id").and_then(Value::as_i64);
            let msg = client
                .post_message(&BoardPostMessageReq {
                    project,
                    channel,
                    agent,
                    body,
                    parent_id,
                })
                .await?;
            Ok(serde_json::to_string_pretty(&msg)?)
        }
        "daybook_board_claim_task" => {
            let task_id = args
                .get("task_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let priority = args
                .get("priority")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let note = args
                .get("note")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
            let branch = if identity.branch.is_empty() {
                None
            } else {
                Some(identity.branch.clone())
            };
            let res = client
                .claim_task(&BoardClaimTaskReq {
                    project,
                    task_id,
                    title,
                    priority,
                    agent,
                    branch,
                    note,
                    force,
                })
                .await?;
            Ok(serde_json::to_string_pretty(&res)?)
        }
        "daybook_board_update_task" => {
            let task_id = args
                .get("task_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let status = args
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("done")
                .to_string();
            let note = args
                .get("note")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let pr_url = args
                .get("pr_url")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let release_locks = args
                .get("release_locks")
                .and_then(Value::as_bool)
                .unwrap_or(status == "done");
            let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
            let res = client
                .update_task(&BoardUpdateTaskReq {
                    project,
                    task_id,
                    agent,
                    status,
                    note,
                    pr_url,
                    release_locks,
                    force,
                })
                .await?;
            Ok(serde_json::to_string_pretty(&res)?)
        }
        "daybook_board_check" => {
            let raw_path = args.get("path").and_then(Value::as_str).unwrap_or("");
            let norm_path = normalize_repo_path(raw_path, Some(&identity.worktree_root))?;
            let res = client
                .check_path(&project, &norm_path, Some(&agent))
                .await?;
            Ok(serde_json::to_string_pretty(&res)?)
        }
        _ => bail!("unknown tool '{tool_name}'"),
    }
}

fn mcp_tool_definitions() -> Value {
    json!([
        {
            "name": "daybook_board_status",
            "description": "Read active agents, file locks, tasks, messages, and unread @mentions across worktrees on this project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Optional project override (defaults to current repository)." },
                    "agent": { "type": "string", "description": "Optional agent handle override." },
                    "since_id": { "type": "integer", "description": "Optional message id cursor." }
                }
            }
        },
        {
            "name": "daybook_board_claim",
            "description": "Claim file path or glob locks before editing shared files so parallel worktrees detect conflicts early.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paths": { "type": "array", "items": { "type": "string" }, "description": "Repo-relative file paths or globs (e.g. ['src/server/**'])." },
                    "note": { "type": "string", "description": "Brief description of planned changes." },
                    "ttl_minutes": { "type": "integer", "description": "Lock TTL in minutes (default 30, max 240)." }
                },
                "required": ["paths"]
            }
        },
        {
            "name": "daybook_board_release",
            "description": "Release locks held by this agent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "lock_id": { "type": "string", "description": "Specific lock id to release." },
                    "all": { "type": "boolean", "description": "Release all locks held by this agent (default true if lock_id omitted)." },
                    "auto_only": { "type": "boolean", "description": "Release only auto-claimed edit locks." }
                }
            }
        },
        {
            "name": "daybook_board_post",
            "description": "Post a coordination message, handoff, or blocker to other agents/worktrees (supports @agent-handle mentions).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel": { "type": "string", "enum": ["general", "handoff", "blocker", "review"], "description": "Message channel (default general)." },
                    "body": { "type": "string", "description": "Message text (use @agent-handle to notify specific agents)." },
                    "parent_id": { "type": "integer", "description": "Optional parent message id for threaded replies." }
                },
                "required": ["body"]
            }
        },
        {
            "name": "daybook_board_claim_task",
            "description": "Create or claim a shared task on the project coordination board.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Short task identifier (e.g. 'BOARD-1')." },
                    "title": { "type": "string", "description": "Task title." },
                    "priority": { "type": "string", "enum": ["low", "medium", "high", "critical"] },
                    "note": { "type": "string", "description": "Status note." },
                    "force": { "type": "boolean", "description": "Force takeover if already claimed on this machine." }
                },
                "required": ["task_id"]
            }
        },
        {
            "name": "daybook_board_update_task",
            "description": "Update task status (open, claimed, review, blocked, done), PR link, and optionally release locks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task identifier." },
                    "status": { "type": "string", "enum": ["open", "claimed", "review", "blocked", "done"] },
                    "note": { "type": "string", "description": "Updated status note." },
                    "pr_url": { "type": "string", "description": "Optional pull request URL." },
                    "release_locks": { "type": "boolean", "description": "Release agent locks (default true when status == done)." }
                },
                "required": ["task_id", "status"]
            }
        },
        {
            "name": "daybook_board_check",
            "description": "Check whether a file path is locked by another agent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to check." }
                },
                "required": ["path"]
            }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{BoardCaller, Ledger};
    use std::io::Cursor;

    #[tokio::test]
    async fn mcp_registers_agent_on_session_start() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap();
        assert!(status.success());
        let ledger = Ledger::open(&dir.path().join("board.db")).unwrap();
        let client = Arc::new(BoardClient::Local {
            ledger,
            caller: BoardCaller::local(),
        });
        let mut reader = Cursor::new(Vec::new()); // EOF immediately
        let mut out = Vec::new();
        run_mcp_loop(&mut reader, &mut out, &client, &repo)
            .await
            .unwrap();
        // Registration happens in a spawned task; poll until it lands.
        let mut registered = false;
        for _ in 0..50 {
            let snap = client.status(Some("all"), None, None).await.unwrap();
            if snap.agents.iter().any(|a| a.project == "repo") {
                registered = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(registered);
    }

    #[tokio::test]
    async fn mcp_handles_newline_and_content_length_framing() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("board.db")).unwrap();
        let client = Arc::new(BoardClient::Local {
            ledger,
            caller: BoardCaller::local(),
        });

        // 1. Newline framing: initialize + tools/list + tools/call(claim)
        let input_nl = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"daybook_board_claim","arguments":{"project":"daybook","agent":"agent-a","paths":["src/board/**"],"note":"mcp test"}}}"#,
            "\n"
        );
        let mut reader = Cursor::new(input_nl.as_bytes());
        let mut out = Vec::new();
        run_mcp_loop(&mut reader, &mut out, &client, dir.path())
            .await
            .unwrap();
        let out_str = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = out_str.lines().collect();
        assert_eq!(lines.len(), 3); // notification produced no response
        let r1: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(r1["result"]["protocolVersion"], "2024-11-05");
        let r2: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(r2["result"]["tools"].as_array().unwrap().len(), 7);
        let r3: Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(r3["result"]["isError"], false);

        // 2. Content-Length framing: tools/call(daybook_board_check) from agent-b
        let req_body = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"daybook_board_check","arguments":{"project":"daybook","agent":"agent-b","path":"src/board/mcp.rs"}}}"#;
        let framed = format!("Content-Length: {}\r\n\r\n{}", req_body.len(), req_body);
        let mut reader2 = Cursor::new(framed.into_bytes());
        let mut out2 = Vec::new();
        run_mcp_loop(&mut reader2, &mut out2, &client, dir.path())
            .await
            .unwrap();
        let out2_str = String::from_utf8(out2).unwrap();
        assert!(out2_str.starts_with("Content-Length: "));
        let (_, json_part) = out2_str.split_once("\r\n\r\n").unwrap();
        let r4: Value = serde_json::from_str(json_part).unwrap();
        let text = r4["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"blocked\": true"));
    }
}
