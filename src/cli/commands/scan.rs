use anyhow::{Context, Result};

use crate::transcript::adapters;

/// `daybook scan --no-llm --local --day <d>`: discovery + parse only.
/// Without --no-llm, also runs the LLM summarization pass over every
/// listed session, skipping ones already archived (incremental).
/// --local is accepted for CLI compatibility and currently unused.
pub async fn run(day: Option<String>, no_llm: bool, _local: bool) -> Result<()> {
    let home = dirs::home_dir().context("Failed to get home directory")?;
    let sessions = adapters::discover_sessions(&home);

    let target = resolve_day(day.as_deref().unwrap_or("today"))?;

    // Cheap prefilter: Pi/Codex transcript names embed the UTC start date;
    // sessions near UTC midnight can start on an adjacent calendar date, so
    // keep a ±1 day window. Claude Code names carry no date and skip this.
    let window: Option<Vec<String>> = target.as_ref().map(|t| {
        let mut w = vec![t.clone()];
        if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d") {
            if let Some(p) = d.pred_opt() {
                w.push(p.to_string());
            }
            if let Some(s) = d.succ_opt() {
                w.push(s.to_string());
            }
        }
        w
    });
    eprintln!(
        "[daybook] Discovered {} sessions{}",
        sessions.len(),
        target
            .as_deref()
            .map(|d| format!(" (filtering to {})", d))
            .unwrap_or_default()
    );

    let mut rows = Vec::new();
    let mut internal = 0;
    let mut skipped = 0;
    for meta in &sessions {
        if let Some(window) = &window {
            if let Some(fd) = adapters::filename_date(&meta.path) {
                if !window.contains(&fd) {
                    continue;
                }
            }
        }
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

        // D6: the session's journal date is its transcript start date,
        // never the scan time.
        if let Some(target) = &target {
            if parsed.start_date().as_deref() != Some(target.as_str()) {
                continue;
            }
        }

        rows.push((meta, parsed));
    }

    println!(
        "{:<12} {:<18} {:<8} {:<13} {:<16} {:>7} {:>7}  {}",
        "runtime", "project", "start", "duration", "model", "in", "out", "session"
    );
    for (meta, parsed) in &rows {
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
    // S3: with --no-llm absent, run the LLM pass over every session
    // listed above (D4: codex fallback makes it work without claude).
    // Sessions already archived for their journal date are skipped, so
    // repeated scans are incremental, not duplicate-creating.
    if !no_llm {
        let config = crate::config::load_config()?;
        let mut ok = 0;
        let total = rows.len();
        for (i, (meta, parsed)) in rows.iter().enumerate() {
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
            let journal_date = parsed
                .start_date()
                .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
            if already_archived(&config, &journal_date, &meta.session_id) {
                eprintln!(
                    "[daybook] Already archived, skipping: {}",
                    clean(&meta.session_id)
                );
                continue;
            }
            eprintln!(
                "[daybook] Summarizing {}/{}: {}",
                i + 1,
                total,
                clean(parsed.title.as_deref().unwrap_or(&meta.session_id))
            );
            let task_name = format!("session-{}", meta.session_id);
            let cwd = meta
                .cwd
                .clone()
                .unwrap_or_else(|| home.display().to_string());
            match crate::cli::commands::summarize::run_summarization(
                &config, &meta.path, &task_name, &cwd,
            )
            .await
            {
                Ok(_) => ok += 1,
                Err(e) => eprintln!(
                    "[daybook] Summarize failed for {}: {}",
                    clean(&meta.session_id),
                    clean(&e.to_string())
                ),
            }
        }
        eprintln!("[daybook] Summarized {} session(s) into the archive", ok);
    }

    Ok(())
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

/// True if an archive for this session already exists for the journal date.
fn already_archived(config: &crate::config::Config, date: &str, session_id: &str) -> bool {
    let dir = config.storage.path.join(date);
    std::fs::read_dir(&dir)
        .map(|entries| {
            entries.flatten().any(|e| {
                std::fs::read_to_string(e.path())
                    .map(|c| c.contains(&format!("session_id: {}", session_id)))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
