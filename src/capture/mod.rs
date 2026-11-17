//! Hourly structured capture: facts no session summary carries, posted to the
//! ledger's /api/captures endpoint. Runs at the end of `daybook scan`; every
//! collector is best-effort so a missing tool never fails the scan.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;
use crate::ledger::CaptureRecord;

const KIND_PR: &str = "merged_pr";
const KIND_WORKTREE: &str = "worktree";
const KIND_CORRECTION: &str = "correction";
const KIND_TOOL_ERROR: &str = "tool_error";
const KIND_HANDOFF: &str = "handoff";

/// Collect everything and push one batch. Returns the number of records sent.
pub async fn run(config: &Config) -> Result<usize> {
    if config.push.url.is_none() {
        return Ok(0);
    }
    let machine = crate::push::machine_label(config);
    let now = chrono::Utc::now().to_rfc3339();
    let mut records = Vec::new();
    let home = dirs::home_dir().context("no home dir")?;

    let handoffs = collect_handoffs(&home.join(".pi/agent/session-handoff"), &machine, &now);
    let worktrees = collect_worktrees(&handoffs, &machine, &now);
    records.extend(handoffs);
    records.extend(worktrees);
    if let Err(e) = collect_merged_prs(&machine, &now, &mut records) {
        eprintln!("[daybook] capture: merged PRs failed: {e:#}");
    }
    collect_session_facts(
        &home.join(".pi/agent/sessions"),
        &machine,
        &now,
        &mut records,
    );

    records.truncate(500);
    for r in &mut records {
        // Last line of defense: nothing leaves the machine unredacted.
        r.payload_json = crate::redaction::redact_for_config(&r.payload_json, config);
        r.content_hash = hash(&r.payload_json);
    }
    if records.is_empty() {
        return Ok(0);
    }
    let sent = records.len();
    let url = config.push.url.clone().unwrap();
    let token = config.push.token.clone().context("push.token missing")?;
    let client = crate::push::client();
    let resp = client
        .post(format!("{}/api/captures", url.trim_end_matches('/')))
        .bearer_auth(token)
        .json(&records)
        .send()
        .await
        .with_context(|| format!("POST {}/api/captures", url))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "captures returned {}: {}",
            status,
            &body[..body.len().min(200)]
        );
    }
    Ok(sent)
}

fn hash(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

fn record(machine: &str, kind: &str, key: String, payload: Value, now: &str) -> CaptureRecord {
    let payload_json = payload.to_string();
    let project = payload
        .get("project")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    CaptureRecord {
        machine: machine.to_string(),
        project,
        kind: kind.to_string(),
        key,
        content_hash: hash(&payload_json),
        payload_json,
        captured_utc: now.to_string(),
    }
}

fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ---- O3: per-turn session handoff entries written by the Pi extension ----

fn collect_handoffs(root: &Path, machine: &str, now: &str) -> Vec<CaptureRecord> {
    let mut out = Vec::new();
    let Ok(repos) = std::fs::read_dir(root) else {
        return out;
    };
    for repo in repos.flatten() {
        let Ok(files) = std::fs::read_dir(repo.path()) else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(file.path()) else {
                continue;
            };
            let Ok(entry) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            let Some(session) = entry.get("session").and_then(Value::as_str) else {
                continue;
            };
            let worktree = entry.get("worktree").and_then(Value::as_str).unwrap_or("");
            let mut payload = entry.clone();
            payload["project"] = json!(worktree);
            out.push(record(
                machine,
                KIND_HANDOFF,
                session.to_string(),
                payload,
                now,
            ));
            if out.len() >= 100 {
                return out;
            }
        }
    }
    out
}

// ---- A2: live git facts for every worktree seen in a handoff entry ----

fn collect_worktrees(handoffs: &[CaptureRecord], machine: &str, now: &str) -> Vec<CaptureRecord> {
    let mut paths: Vec<String> = handoffs
        .iter()
        .filter_map(|r| {
            serde_json::from_str::<Value>(&r.payload_json)
                .ok()?
                .get("worktree")?
                .as_str()
                .map(str::to_string)
        })
        .collect();
    paths.sort();
    paths.dedup();
    let mut out = Vec::new();
    for p in paths {
        let path = PathBuf::from(&p);
        if !path.is_dir() {
            continue;
        }
        let Some(branch) = git(&path, &["branch", "--show-current"]) else {
            continue;
        };
        let head = git(&path, &["rev-parse", "HEAD"]).unwrap_or_default();
        let dirty = git(&path, &["status", "--porcelain"])
            .map(|s| s.lines().filter(|l| !l.is_empty()).count())
            .unwrap_or(0);
        let payload = json!({ "project": p, "worktree": p, "branch": branch, "head": head, "dirty_files": dirty });
        out.push(record(machine, KIND_WORKTREE, p.clone(), payload, now));
    }
    out
}

// ---- A1: PRs merged by the user in the last 26 hours ----

fn collect_merged_prs(machine: &str, now: &str, out: &mut Vec<CaptureRecord>) -> Result<()> {
    let since = (chrono::Utc::now() - chrono::Duration::hours(26))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let proc = Command::new("gh")
        .args([
            "search",
            "prs",
            "--author",
            "@me",
            "--merged-at",
            &format!(">={since}"),
            "--limit",
            "50",
            "--json",
            "url,title,repository,updatedAt,state",
        ])
        .output()
        .context("run gh search prs")?;
    if !proc.status.success() {
        anyhow::bail!(
            "gh search prs: {}",
            String::from_utf8_lossy(&proc.stderr).trim()
        );
    }
    let prs: Vec<Value> = serde_json::from_slice(&proc.stdout).context("parse gh search prs")?;
    for pr in prs {
        let Some(url) = pr.get("url").and_then(Value::as_str) else {
            continue;
        };
        let project = pr
            .get("repository")
            .and_then(|r| r.get("nameWithOwner"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let payload = json!({
            "project": project, "url": url,
            "title": pr.get("title").and_then(Value::as_str).unwrap_or(""),
            "updatedAt": pr.get("updatedAt").and_then(Value::as_str).unwrap_or(""),
        });
        out.push(record(machine, KIND_PR, url.to_string(), payload, now));
    }
    Ok(())
}

// ---- A4/A5: corrections and grouped tool errors from recent Pi sessions ----

const CORRECTION_MARKERS: &[&str] = &[
    "no,",
    "no ",
    "nope",
    "wrong",
    "not what",
    "stop",
    "don't",
    "do not ",
    "undo",
    "revert",
    "that's not",
    "thats not",
    "not that",
];

fn is_correction(text: &str) -> bool {
    if text.len() > 300 || text.contains('\n') {
        return false;
    }
    let lower = text.trim_start().to_lowercase();
    CORRECTION_MARKERS.iter().any(|m| lower.starts_with(m))
}

fn text_of(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn collect_session_facts(root: &Path, machine: &str, now: &str, out: &mut Vec<CaptureRecord>) {
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(26 * 3600);
    let Ok(dirs) = std::fs::read_dir(root) else {
        return;
    };
    // (tool, first error line) -> count, grouped across sessions of this run.
    let mut errors: BTreeMap<(String, String), u64> = BTreeMap::new();
    let mut corrections = 0;
    for dir in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(dir.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = file.metadata() else {
                continue;
            };
            if meta.modified().map(|m| m < cutoff).unwrap_or(true) {
                continue;
            }
            let session = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (idx, line) in raw.lines().enumerate() {
                let Ok(entry) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if entry.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                let Some(msg) = entry.get("message") else {
                    continue;
                };
                match msg.get("role").and_then(Value::as_str) {
                    Some("user") if corrections < 50 => {
                        let text = text_of(msg.get("content").unwrap_or(&Value::Null));
                        if is_correction(&text) {
                            let payload = json!({
                                "project": session, "session": session, "text": text.trim(),
                            });
                            out.push(record(
                                machine,
                                KIND_CORRECTION,
                                format!("{session}:{idx}"),
                                payload,
                                now,
                            ));
                            corrections += 1;
                        }
                    }
                    Some("toolResult") => {
                        let failed = msg
                            .get("is_error")
                            .or_else(|| msg.get("isError"))
                            .and_then(Value::as_bool)
                            == Some(true);
                        if !failed {
                            continue;
                        }
                        let tool = msg
                            .get("tool_name")
                            .or_else(|| msg.get("toolName"))
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        let first = text_of(msg.get("content").unwrap_or(&Value::Null))
                            .lines()
                            .next()
                            .unwrap_or("")
                            .chars()
                            .take(160)
                            .collect::<String>();
                        *errors.entry((tool, first)).or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    for ((tool, error), count) in errors.into_iter().take(100) {
        let key = format!("{}:{}", tool, &hash(&error)[..16]);
        let payload = json!({ "tool": tool, "error": error, "count": count });
        out.push(record(machine, KIND_TOOL_ERROR, key, payload, now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correction_markers_are_conservative() {
        assert!(is_correction("no, the other file"));
        assert!(is_correction("Wrong branch"));
        assert!(is_correction("don't do that"));
        assert!(!is_correction("Looks good, continue"));
        assert!(!is_correction("Now add the tests"));
        assert!(!is_correction(&"no ".repeat(200)));
        assert!(!is_correction("no\nmultiline"));
    }

    #[test]
    fn handoffs_and_worktrees_are_collected_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store/deadbeef");
        std::fs::create_dir_all(&store).unwrap();
        let wt = dir.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let entry = json!({
            "session": "s1", "worktree": wt.to_string_lossy(), "branch": "feat",
            "head": "abc", "dirty": [], "task": "fix", "status": "EFF x: done",
            "updated": 1,
        });
        std::fs::write(store.join("s1.json"), entry.to_string()).unwrap();
        std::fs::write(store.join("orphan.tmp"), "junk").unwrap();
        std::fs::write(store.join("broken.json"), "{").unwrap();

        let handoffs = collect_handoffs(&dir.path().join("store"), "m", "now");
        assert_eq!(handoffs.len(), 1);
        assert_eq!(handoffs[0].kind, KIND_HANDOFF);
        assert_eq!(handoffs[0].key, "s1");
        // Not a git repo, so no worktree record; nothing panics.
        let worktrees = collect_worktrees(&handoffs, "m", "now");
        assert!(worktrees.is_empty());
    }

    #[test]
    fn session_facts_groups_errors_and_bounds_corrections() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("proj");
        std::fs::create_dir_all(&sessions).unwrap();
        let mut lines = Vec::new();
        for i in 0..3 {
            lines.push(
                json!({ "type": "message", "message": {
                "role": "toolResult", "is_error": true, "tool_name": "bash",
                "content": [{"type": "text", "text": "exit code 1\nstack"}],
                "n": i,
            } })
                .to_string(),
            );
        }
        lines.push(
            json!({ "type": "message", "message": {
            "role": "user", "content": "no, use the other flag",
        } })
            .to_string(),
        );
        lines.push(
            json!({ "type": "message", "message": {
            "role": "user", "content": "please add tests",
        } })
            .to_string(),
        );
        std::fs::write(sessions.join("s1.jsonl"), lines.join("\n")).unwrap();

        let mut out = Vec::new();
        collect_session_facts(dir.path(), "m", "now", &mut out);
        let errors: Vec<_> = out.iter().filter(|r| r.kind == KIND_TOOL_ERROR).collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].payload_json.contains("\"count\":3"));
        assert!(errors[0].payload_json.contains("exit code 1"));
        let corrections: Vec<_> = out.iter().filter(|r| r.kind == KIND_CORRECTION).collect();
        assert_eq!(corrections.len(), 1);
        assert!(corrections[0].payload_json.contains("other flag"));
    }

    #[test]
    fn upsert_skips_unchanged_captures() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = crate::ledger::Ledger::open(&dir.path().join("t.db3")).unwrap();
        let r = record(
            "m",
            KIND_HANDOFF,
            "k".into(),
            json!({ "a": 1 }),
            "2026-01-01T00:00:00Z",
        );
        assert_eq!(ledger.upsert_captures(&[r.clone()]).unwrap(), 1);
        assert_eq!(ledger.upsert_captures(&[r.clone()]).unwrap(), 0);
        let mut changed = r.clone();
        changed.payload_json = json!({ "a": 2 }).to_string();
        changed.content_hash = hash(&changed.payload_json);
        assert_eq!(ledger.upsert_captures(&[changed]).unwrap(), 1);
        let rows = ledger
            .captures_since(KIND_HANDOFF, "2025-01-01T00:00:00Z", 10)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].payload_json.contains("\"a\":2"));
        // Unchanged payload: skipped in the written count but the observation
        // time moves forward, so it stays visible in recent windows.
        let mut later = record(
            "m",
            KIND_HANDOFF,
            "k".into(),
            json!({ "a": 2 }),
            "2026-06-01T00:00:00Z",
        );
        later.content_hash = hash(&later.payload_json);
        assert_eq!(ledger.upsert_captures(&[later]).unwrap(), 0);
        let rows = ledger
            .captures_since(KIND_HANDOFF, "2026-05-01T00:00:00Z", 10)
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "unchanged capture must stay in recent windows"
        );
    }
}
