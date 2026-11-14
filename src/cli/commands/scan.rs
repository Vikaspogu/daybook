use anyhow::{Context, Result};

use crate::archive::session::frontmatter_value;
use crate::transcript::adapters;

/// `daybook scan --no-llm --local --day <d>`: discovery + parse only.
/// Without --no-llm, also runs the LLM summarization pass over every
/// listed session, skipping ones already archived (incremental).
/// --local: run the LLM pass but don't push to the ledger server.
pub async fn run(day: Option<String>, no_llm: bool, local: bool) -> Result<()> {
    let home = dirs::home_dir().context("Failed to get home directory")?;
    let sessions = adapters::discover_sessions(&home);

    let targets = resolve_days(day.as_deref())?;

    // Cheap prefilter: Pi/Codex transcript names embed the UTC start date;
    // sessions near UTC midnight can start on an adjacent calendar date, so
    // keep a ±1 day window. Claude Code names carry no date and skip this.
    let window: Option<Vec<String>> = targets.as_ref().map(|ts| {
        let mut w = Vec::new();
        for t in ts {
            w.push(t.clone());
            if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d") {
                if let Some(p) = d.pred_opt() {
                    w.push(p.to_string());
                }
                if let Some(s) = d.succ_opt() {
                    w.push(s.to_string());
                }
            }
        }
        w.sort();
        w.dedup();
        w
    });
    eprintln!(
        "[daybook] Discovered {} sessions{}",
        sessions.len(),
        targets
            .as_ref()
            .map(|ds| format!(" (filtering to {})", ds.join(", ")))
            .unwrap_or_default()
    );

    let mut rows = Vec::new();
    let mut internal = 0;
    let mut skipped = 0;
    for meta in &sessions {
        if let Some(window) = &window {
            if !path_matches_scan_window(&meta.path, window, meta.agent) {
                continue;
            }
        }
        let transcript_modified_ns = match adapters::transcript_modified_ns(&meta.path) {
            Ok(modified_ns) => modified_ns,
            Err(error) => {
                eprintln!(
                    "[daybook] Cannot timestamp {}, retrying next scan: {}",
                    clean(&meta.session_id),
                    clean(&error.to_string())
                );
                continue;
            }
        };
        let parsed = match adapters::parse_session(meta) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "[daybook] Skipping {}: {}",
                    clean(&meta.path.display().to_string()),
                    clean(&e.to_string())
                );
                skipped += 1;
                continue;
            }
        };
        if parsed.internal {
            internal += 1;
            continue;
        }

        // The journal date is the transcript start date. Explicit --day uses
        // only that date; default scans also include recently resumed sessions.
        if let Some(targets) = &targets {
            if !session_matches_dates(
                &meta.path,
                parsed.start_date().as_deref(),
                targets,
                day.is_none(),
            ) {
                continue;
            }
        }

        rows.push((meta, parsed, transcript_modified_ns));
    }

    println!(
        "{:<12} {:<18} {:<8} {:<13} {:<16} {:>7} {:>7}  session",
        "runtime", "project", "start", "duration", "model", "in", "out"
    );
    for (meta, parsed, _) in &rows {
        println!(
            "{:<12} {:<18} {:<8} {:<13} {:<16} {:>7} {:>7}  {}",
            meta.agent.as_str(),
            clean(&meta.project().unwrap_or_else(|| "-".to_string())),
            start_time(parsed),
            parsed.duration().unwrap_or_else(|| "-".to_string()),
            clean(&parsed.model.clone().unwrap_or_else(|| "-".to_string())),
            parsed.usage.map(|u| u.input).unwrap_or(0),
            parsed.usage.map(|u| u.output).unwrap_or(0),
            clean(
                &parsed
                    .title
                    .clone()
                    .unwrap_or_else(|| meta.session_id.clone())
            ),
        );
    }

    eprintln!(
        "[daybook] Scan complete: {} sessions listed, {} skipped, {} daybook-internal",
        rows.len(),
        skipped,
        internal
    );
    // S3: with --no-llm absent, run the LLM pass over every session
    // listed above (D4: codex fallback makes it work without claude).
    // Sessions already archived for their journal date are skipped, so
    // repeated scans are incremental, not duplicate-creating.
    if !no_llm {
        let config = crate::config::load_config()?;
        let mut ok = 0;
        let pricing = if config.push.url.is_some() {
            Some(crate::usage::pricing::PricingData::load().await)
        } else {
            None
        };
        if config.push.url.is_some() {
            let retried = crate::push::retry_pending(&config).await;
            if retried > 0 {
                eprintln!(
                    "[daybook] Delivered {} parked record(s) to the ledger server",
                    retried
                );
            }
        }
        let total = rows.len();
        for (i, (meta, parsed, transcript_modified_ns)) in rows.iter().enumerate() {
            if parsed.data.is_empty() {
                continue;
            }
            if !is_stale(&meta.path) {
                eprintln!(
                    "[daybook] Session looks live or just finished, skipping for now: {}",
                    clean(&meta.session_id)
                );
                continue;
            }
            let sid = clean(&meta.session_id);
            if sid.is_empty() || sid == "unknown" {
                // F5: two id-less sessions can't be deduped against each
                // other; skip rather than risk permanently blocking one.
                eprintln!("[daybook] Session has no usable id, skipping LLM pass");
                continue;
            }
            let journal_date = parsed
                .start_date()
                .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
            let transcript_revision = match adapters::transcript_revision(&meta.path) {
                Ok(revision) => revision,
                Err(e) => {
                    eprintln!(
                        "[daybook] Cannot hash {}: {}",
                        clean(&meta.session_id),
                        clean(&e.to_string())
                    );
                    if !local {
                        push_after_archive(
                            &config,
                            meta,
                            parsed,
                            &journal_date,
                            None,
                            pricing.as_ref(),
                        )
                        .await;
                    }
                    continue;
                }
            };
            let unchanged = adapters::transcript_modified_ns(&meta.path)
                .map(|modified_ns| modified_ns == *transcript_modified_ns)
                .unwrap_or(false);
            if !unchanged {
                eprintln!(
                    "[daybook] Transcript changed during scan, retrying next scan: {}",
                    clean(&meta.session_id)
                );
                continue;
            }
            let parsed = &parsed;
            if archive_is_current(
                &config,
                &journal_date,
                &meta.session_id,
                &transcript_revision,
            ) {
                eprintln!(
                    "[daybook] Already archived, skipping summary: {}",
                    clean(&meta.session_id)
                );
                if !local {
                    // Already summarized, but the ledger server may not have
                    // it (e.g. push.url configured after the session archived).
                    push_after_archive(
                        &config,
                        meta,
                        parsed,
                        &journal_date,
                        Some(&transcript_revision),
                        pricing.as_ref(),
                    )
                    .await;
                }
                continue;
            }
            eprintln!(
                "[daybook] Summarizing {}/{}: {}",
                i + 1,
                total,
                clean(parsed.title.as_deref().unwrap_or(&meta.session_id))
            );
            let cwd = meta
                .cwd
                .clone()
                .unwrap_or_else(|| home.display().to_string());
            match crate::cli::commands::summarize::run_summarization(
                &config,
                &meta.path,
                &cwd,
                Some((transcript_revision.clone(), *transcript_modified_ns)),
                Some((meta, parsed)),
            )
            .await
            {
                Ok(_) => {
                    ok += 1;
                    if !local {
                        push_after_archive(
                            &config,
                            meta,
                            parsed,
                            &journal_date,
                            Some(&transcript_revision),
                            pricing.as_ref(),
                        )
                        .await;
                    }
                }
                Err(e) => eprintln!(
                    "[daybook] Summarize failed for {}: {}",
                    clean(&meta.session_id),
                    clean(&e.to_string())
                ),
            }
        }
        eprintln!("[daybook] Summarized {} session(s) into the archive", ok);
        if let Ok(client) = crate::board::client::BoardClient::from_config(&config, local) {
            client.sync_active_sessions().await;
        }
    }
    Ok(())
}

/// After a session archive lands on disk, ship it to the ledger server.
/// Push failures park the record in ~/.daybook/pending/ and the next
/// scan retries it. No-op when push.url isn't configured.
async fn push_after_archive(
    config: &crate::config::Config,
    meta: &adapters::SessionMeta,
    parsed: &adapters::ParsedSession,
    journal_date: &str,
    expected_revision: Option<&str>,
    pricing: Option<&crate::usage::pricing::PricingData>,
) {
    if config.push.url.is_none() {
        return;
    }
    // Archives are written under their HH_MM-topic title, not the scan
    // task name — locate the file by its session_id frontmatter line.
    let Some((_, summary_md, version)) =
        find_session_archive(config, journal_date, &meta.session_id)
    else {
        eprintln!(
            "[daybook] Archive missing, not pushing: {}",
            clean(&meta.session_id)
        );
        return;
    };
    if expected_revision.is_some_and(|revision| {
        frontmatter_value(&summary_md, "transcript_revision") != Some(revision)
    }) {
        return;
    }
    let cwd = meta.cwd.clone().unwrap_or_default();
    let usage = parsed
        .usage
        .as_ref()
        .map(|u| {
            let calculated = pricing.zip(parsed.model.as_deref()).map(|(p, m)| {
                p.calculate_cost(m, u.input, u.output, u.cache_creation, u.cache_read)
            });
            usage_record(u, calculated)
        })
        .unwrap_or_default();
    // The ledger hash describes the exact redacted content sent and stored.
    let summary_md = crate::redaction::redact_for_config(&summary_md, config);
    let record = crate::push::build_record(
        config,
        &meta.session_id,
        meta.agent.as_str(),
        &crate::ledger::normalize_project(meta.project().as_deref().unwrap_or(&cwd)),
        journal_date,
        parsed.title.clone(),
        parsed.start.clone(),
        parsed.end.clone(),
        crate::archive::session::get_git_branch(&cwd),
        version,
        crate::ledger::sha256_hex(summary_md.as_bytes()),
        summary_md,
        read_facets_json(&meta.session_id),
        usage,
        compute_aggregates(parsed),
    );
    match crate::push::push_record(config, &record).await {
        Ok(true) => {
            eprintln!(
                "[daybook] Pushed {} to ledger server",
                clean(&meta.session_id)
            )
        }
        Ok(false) => {}
        Err(e) => match crate::push::park_record(config, &record) {
            Ok(_) => {
                eprintln!(
                    "[daybook] Push failed, parked for retry: {}",
                    clean(&e.to_string())
                )
            }
            Err(pe) => eprintln!(
                "[daybook] Push failed and parking failed for {}: {} / {}",
                clean(&meta.session_id),
                clean(&e.to_string()),
                clean(&pe.to_string())
            ),
        },
    }
}

fn usage_record(
    usage: &adapters::SessionUsage,
    calculated_cost: Option<f64>,
) -> crate::ledger::UsageRecord {
    let cost_usd = if let Some(cost) = usage.cost_usd {
        Some(cost)
    } else if usage.multiple_models {
        None
    } else {
        calculated_cost.filter(|cost| *cost > 0.0)
    };
    crate::ledger::UsageRecord {
        input: usage.input,
        output: usage.output,
        cache_read: usage.cache_read,
        cache_creation: usage.cache_creation,
        cost_usd,
    }
}

/// True when the transcript looks finished: last modified more than
/// STALE_SECS ago. Freshly modified files are likely still-live sessions;
/// summarizing them produces partial archives that dedup then protects.
fn is_stale(path: &std::path::Path) -> bool {
    const STALE_SECS: u64 = 600;
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| match t.elapsed() {
            Ok(e) => e.as_secs() > STALE_SECS,
            // mtime in the future (clock skew or live write): treat as live
            Err(_) => false,
        })
        .unwrap_or(false)
}

/// Find this session's archive file for the journal date. Archives are
/// named by HH_MM-topic title (plus id-suffix variants), so the only
/// stable handle is the `session_id:` frontmatter line.
fn find_session_archive(
    config: &crate::config::Config,
    date: &str,
    session_id: &str,
) -> Option<(std::path::PathBuf, String, i64)> {
    crate::archive::manager::ArchiveManager::new(config.clone())
        .read_session_snapshot(date, session_id)
        .ok()
        .flatten()
}

/// True only when the archive matches both the transcript bytes and summarizer.
fn archive_is_current(
    config: &crate::config::Config,
    date: &str,
    session_id: &str,
    transcript_revision: &str,
) -> bool {
    let Some((_, content, _)) = find_session_archive(config, date, session_id) else {
        return false;
    };
    frontmatter_value(&content, "transcript_revision") == Some(transcript_revision)
        && frontmatter_value(&content, "summarizer_version").and_then(|v| v.parse::<u32>().ok())
            == Some(crate::archive::session::SUMMARIZER_VERSION)
}

fn modified_date(path: &std::path::Path) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        chrono::DateTime::<chrono::Local>::from(modified)
            .format("%Y-%m-%d")
            .to_string(),
    )
}

/// Claude filenames have no date, so parse them before applying the day filter.
fn path_matches_scan_window(
    path: &std::path::Path,
    dates: &[String],
    agent: adapters::AgentKind,
) -> bool {
    agent == adapters::AgentKind::ClaudeCode || path_matches_dates(path, dates)
}

/// Date prefilter includes recently modified transcripts whose start date is old.
fn path_matches_dates(path: &std::path::Path, dates: &[String]) -> bool {
    let Some(filename_date) = adapters::filename_date(path) else {
        // Dateless runtimes need the parsed start date before we can filter them.
        return true;
    };
    dates.contains(&filename_date) || modified_date(path).is_some_and(|date| dates.contains(&date))
}

fn session_matches_dates(
    path: &std::path::Path,
    start_date: Option<&str>,
    targets: &[String],
    include_modified: bool,
) -> bool {
    start_date.is_some_and(|date| targets.iter().any(|target| target == date))
        || (include_modified && modified_date(path).is_some_and(|date| targets.contains(&date)))
}

/// Resolve optional `--day`: when omitted, scan both `today` and `yesterday`
/// so sessions that finished within `STALE_SECS` of yesterday's last shutdown
/// are picked up automatically on the next day's scan.
fn resolve_days(day: Option<&str>) -> Result<Option<Vec<String>>> {
    match day {
        None => {
            let today = chrono::Local::now().date_naive();
            let mut days = vec![today.format("%Y-%m-%d").to_string()];
            if let Some(yesterday) = today.pred_opt() {
                days.push(yesterday.format("%Y-%m-%d").to_string());
            }
            Ok(Some(days))
        }
        Some(d) => Ok(resolve_day(d)?.map(|single| vec![single])),
    }
}

/// Resolve `--day`: "today", "yesterday", "all", or a literal YYYY-MM-DD.
fn resolve_day(day: &str) -> Result<Option<String>> {
    match day {
        "all" => Ok(None),
        "today" => Ok(Some(chrono::Local::now().format("%Y-%m-%d").to_string())),
        "yesterday" => Ok(Some(
            (chrono::Local::now() - chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string(),
        )),
        date => {
            chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").with_context(|| {
                format!(
                    "Invalid --day '{}': expected today|yesterday|all|YYYY-MM-DD",
                    date
                )
            })?;
            Ok(Some(date.to_string()))
        }
    }
}

/// Local HH:MM of the session start, if the transcript carries a timestamp.
fn start_time(parsed: &adapters::ParsedSession) -> String {
    parsed
        .start
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Strip control characters so transcript-derived text cannot inject
/// terminal control codes (ANSI escapes, OSC) into daybook output.
fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            // Cc controls, plus Unicode Cf format chars (bidi overrides,
            // zero-width, soft hyphen) that can visually spoof terminal output.
            let spoofing = matches!(c as u32,
                0xAD | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064
                | 0x2066..=0x206F | 0x061C | 0x2028 | 0x2029 | 0xFEFF | 0xFFF9..=0xFFFB);
            if !c.is_control() && !spoofing {
                out.push(c);
            }
            continue;
        }
        // CSI: ESC [ <params> <final 0x40-0x7e>. OSC: ESC ] ... BEL/ST.
        match chars.next() {
            Some('[') => {
                for c2 in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c2) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c2) = chars.next() {
                    // Terminators: BEL, ST (0x9c), or 7-bit ST (ESC \).
                    if c2 == '\u{7}' || c2 == '\u{9c}' {
                        break;
                    }
                    if c2 == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Claude Code's native usage-data writes per-session facets to
/// ~/.claude/usage-data/facets/<session_id>.json. Codex sessions have no
/// facet file — None is a normal result, facets are optional enrichment.
// ponytail: Claude-only facet source; when S5 wants runtime-agnostic facets,
// generate them in summarize and write next to the archive.
fn read_facets_json(session_id: &str) -> Option<String> {
    let dir = dirs::home_dir()?.join(".claude/usage-data/facets");
    std::fs::read_to_string(dir.join(format!("{}.json", session_id))).ok()
}

/// Client-side aggregate metrics (F17): computed once at scan, pushed with
/// the record, aggregated by the server. Heuristics on tool-call shapes —
/// good enough for "how is my week going", not an audit.
// ponytail: error/interrupt detection is lenient (None response, "interrupt"
// substring); tighten per-runtime if the friction report miscounts.
fn compute_aggregates(parsed: &adapters::ParsedSession) -> crate::ledger::AggregatesRecord {
    use crate::ledger::AggregatesRecord;
    let mut agg = AggregatesRecord::default();

    for tc in &parsed.data.tool_calls {
        // A test run = a shell tool call whose command mentions a test runner.
        let tool_name = tc.name.to_ascii_lowercase();
        if ["bash", "execute", "exec", "shell"]
            .iter()
            .any(|part| tool_name.contains(part))
        {
            let cmd = tc
                .input
                .get("command")
                .or_else(|| tc.input.get("cmd"))
                .and_then(|value| value.as_str())
                .or_else(|| tc.input.as_str());
            if let Some(cmd) = cmd {
                if cmd.contains("cargo test")
                    || cmd.contains("npm test")
                    || cmd.contains("pytest")
                    || cmd.contains("go test")
                    || cmd.contains("jest")
                    || cmd.contains("vitest")
                    || cmd.contains(" make test")
                {
                    agg.test_runs += 1;
                }
            }
        }
        let is_error = tc
            .response
            .as_ref()
            .and_then(|r| r.get("is_error"))
            .and_then(|v| v.as_bool())
            == Some(true);
        if is_error {
            agg.failed_tool_calls += 1;
        } else if tc.response.is_none() {
            agg.unfinished_tool_calls += 1;
        }
    }

    for msg in &parsed.data.user_messages {
        if msg.contains("interrupt") || msg.contains("[Request interrupted") {
            agg.interrupted += 1;
        }
    }

    if let Some(u) = &parsed.usage {
        let denom = u.input.saturating_add(u.cache_read);
        if denom > 0 {
            agg.cache_hit_rate = Some(u.cache_read as f64 / denom as f64);
        }
    }

    agg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_record_prefers_recorded_cost_and_never_estimates_mixed_models() {
        let recorded = adapters::SessionUsage {
            cost_usd: Some(0.125),
            ..Default::default()
        };
        assert_eq!(usage_record(&recorded, Some(9.0)).cost_usd, Some(0.125));
        let mixed = adapters::SessionUsage {
            multiple_models: true,
            ..Default::default()
        };
        assert_eq!(usage_record(&mixed, Some(9.0)).cost_usd, None);
        let single = adapters::SessionUsage::default();
        assert_eq!(usage_record(&single, Some(0.25)).cost_usd, Some(0.25));
    }

    #[test]
    fn test_compute_aggregates_counts_tool_errors() {
        use crate::transcript::{ToolCall, TranscriptData};
        let mut data = TranscriptData::default();
        for response in [
            None,
            Some(serde_json::json!({ "is_error": true, "text": "boom" })),
            Some(serde_json::Value::String("ok".into())),
        ] {
            data.tool_calls.push(ToolCall {
                name: "bash".into(),
                input: serde_json::Value::Null,
                response,
            });
        }
        data.tool_calls.push(ToolCall {
            name: "exec_command".into(),
            input: serde_json::json!({"command": "cargo test --quiet"}),
            response: Some(serde_json::Value::String("ok".into())),
        });
        data.tool_calls.push(ToolCall {
            name: "exec_command".into(),
            input: serde_json::json!({"cmd": "npm test"}),
            response: Some(serde_json::Value::String("ok".into())),
        });
        data.tool_calls.push(ToolCall {
            name: "exec_command".into(),
            input: serde_json::Value::String("pytest".into()),
            response: Some(serde_json::Value::String("ok".into())),
        });
        let parsed = adapters::ParsedSession {
            data,
            title: None,
            start: None,
            end: None,
            model: None,
            usage: None,
            internal: false,
        };
        let aggregates = compute_aggregates(&parsed);
        assert_eq!(aggregates.failed_tool_calls, 1);
        assert_eq!(aggregates.unfinished_tool_calls, 1);
        assert_eq!(aggregates.test_runs, 3);
    }

    #[test]
    fn test_resolve_day_accepts_known_forms() {
        assert!(resolve_day("all").unwrap().is_none());
        assert!(resolve_day("today").unwrap().is_some());
        assert!(resolve_day("yesterday").unwrap().is_some());
        assert_eq!(
            resolve_day("2026-10-28").unwrap().as_deref(),
            Some("2026-10-28")
        );
        assert!(resolve_day("not-a-date").is_err());
        let default_days = resolve_days(None).unwrap().unwrap();
        assert_eq!(default_days.len(), 2);
        assert!(default_days[0] > default_days[1]);
        assert_eq!(
            resolve_days(Some("2026-10-28")).unwrap().as_deref(),
            Some(&["2026-10-28".to_string()][..])
        );
        assert!(resolve_days(Some("all")).unwrap().is_none());
    }

    #[test]
    fn test_clean_strips_control_chars() {
        assert_eq!(clean("normal text"), "normal text");
        assert_eq!(clean("a\u{1b}[31mb"), "ab");
        assert_eq!(clean("line1\nline2"), "line1line2");
        // OSC with BEL and with 7-bit ST (ESC \) terminators.
        assert_eq!(clean("a\u{1b}]0;title\u{7}b"), "ab");
        assert_eq!(clean("a\u{1b}]8;;http://x\u{1b}\\b"), "ab");
        // Truncated escape at end of string is dropped, not kept.
        // Bidi overrides and zero-width chars are stripped too.
        assert_eq!(clean("a\u{202e}b\u{200b}c\u{061c}d\u{2028}e"), "abcde");
        assert_eq!(clean("a\u{1b}"), "a");
    }

    #[test]
    fn test_filename_date_extraction() {
        use adapters::filename_date;
        use std::path::Path;
        // Pi: <timestamp>_<uuid>.jsonl
        assert_eq!(
            filename_date(Path::new("2026-10-28T10-00-00-000Z_01abc.jsonl")).as_deref(),
            Some("2026-10-28")
        );
        // Codex: rollout-<timestamp>_<uuid>.jsonl (nested under YYYY/MM/DD)
        assert_eq!(
            filename_date(Path::new("rollout-2026-10-28T20-18-29-01abc.jsonl")).as_deref(),
            Some("2026-10-28")
        );
        // Claude Code: <uuid>.jsonl, no date
        assert_eq!(filename_date(Path::new("01a14c0a-b03b.jsonl")), None);
        // Date inside a longer name still found
        assert_eq!(
            filename_date(Path::new("x2026-01-02y.jsonl")).as_deref(),
            Some("2026-01-02")
        );
    }

    #[test]
    fn test_archive_snapshot_waits_for_writer() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut config = crate::config::Config::default();
        config.storage.path = temp.path().to_path_buf();
        let manager = crate::archive::manager::ArchiveManager::new(config.clone());
        manager.ensure_date_dir("2026-11-11").unwrap();
        let path = temp.path().join("2026-11-11/session.md");
        std::fs::write(&path, "---\nsession_id: same\n---\nold summary").unwrap();

        let (_, legacy, legacy_version) =
            find_session_archive(&config, "2026-11-11", "same").unwrap();
        assert!(legacy.ends_with("old summary"));
        assert!(legacy_version > 0);
        let lock = crate::archive::manager::archive_lock(&config).unwrap();
        let reader_config = config.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            tx.send(find_session_archive(&reader_config, "2026-11-11", "same"))
                .unwrap();
        });
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(50))
            .is_err());

        std::fs::write(&path, "---\nsession_id: same\nversion: 2\n---\nnew summary").unwrap();
        drop(lock);
        let (_, snapshot, version) = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap();
        assert_eq!(frontmatter_value(&snapshot, "version"), Some("2"));
        assert_eq!(version, 2);
        assert!(snapshot.ends_with("new summary"));
    }

    #[test]
    fn test_archive_freshness_requires_revision_and_summarizer_version() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut config = crate::config::Config::default();
        config.storage.path = temp.path().to_path_buf();
        let dir = temp.path().join("2026-11-11");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("20_33-duplicate-chat.md");
        std::fs::write(&path, "---\nsession_id: session-1\n---\nold").unwrap();
        assert!(!archive_is_current(
            &config,
            "2026-11-11",
            "session-1",
            "new-hash"
        ));

        std::fs::write(
            &path,
            format!(
                "---\nsession_id: session-1\ntranscript_revision: new-hash\nsummarizer_version: {}\n---\nfinal",
                crate::archive::session::SUMMARIZER_VERSION
            ),
        )
        .unwrap();
        assert!(archive_is_current(
            &config,
            "2026-11-11",
            "session-1",
            "new-hash"
        ));
        assert!(!archive_is_current(
            &config,
            "2026-11-11",
            "session-1",
            "later-hash"
        ));

        let outdated = std::fs::read_to_string(&path).unwrap().replace(
            &format!(
                "summarizer_version: {}",
                crate::archive::session::SUMMARIZER_VERSION
            ),
            "summarizer_version: 0",
        );
        std::fs::write(path, outdated).unwrap();
        assert!(!archive_is_current(
            &config,
            "2026-11-11",
            "session-1",
            "new-hash"
        ));
    }

    #[test]
    fn test_dateless_sessions_reach_parsed_date_filter() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("01a14c0a-b03b.jsonl");
        std::fs::write(&path, "old").unwrap();
        let targets = vec!["2000-01-01".to_string()];
        assert!(path_matches_dates(&path, &targets));
        assert!(path_matches_scan_window(
            &path,
            &targets,
            adapters::AgentKind::ClaudeCode
        ));
        assert!(path_matches_scan_window(
            &path,
            &targets,
            adapters::AgentKind::Pi
        ));
    }

    #[test]
    fn test_recently_modified_old_session_matches_default_scan() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("2020-01-01T00-00-00-000Z_old.jsonl");
        std::fs::write(&path, "resumed").unwrap();
        let today = modified_date(&path).unwrap();
        let targets = vec![today];
        assert!(path_matches_dates(&path, &targets));
        assert!(session_matches_dates(
            &path,
            Some("2020-01-01"),
            &targets,
            true
        ));
        assert!(!session_matches_dates(
            &path,
            Some("2020-01-01"),
            &targets,
            false
        ));
        assert!(!session_matches_dates(
            &path,
            Some("2020-01-01"),
            &["2000-01-01".to_string()],
            true
        ));
    }
}
