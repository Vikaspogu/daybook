use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::init::CLAUDE_EDIT_MATCHER;
use crate::config::{get_config_path, Config};
const RECALL_SKILL: &str = include_str!("../../../skills/daybook-recall/SKILL.md");
const PI_EXTENSION: &str = include_str!("../../../hooks/daybook-pi.ts");

pub async fn run() -> Result<()> {
    let config_path = get_config_path()?;
    let mut config_warning = None;
    let config = if config_path.exists() {
        match fs::read_to_string(&config_path)
            .map_err(anyhow::Error::from)
            .and_then(|raw| toml::from_str::<Config>(&raw).map_err(anyhow::Error::from))
        {
            Ok(config) => config,
            Err(error) => {
                config_warning = Some(error);
                Config::default()
            }
        }
    } else {
        Config::default()
    };
    let home = dirs::home_dir().context("Could not determine home directory for runtime checks")?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    println!("Daybook doctor (read-only)\n");
    if let Some(error) = config_warning {
        println!(
            "WARN Config: cannot read {}; using defaults for other checks: {error}",
            config_path.display()
        );
    }
    let storage = config.storage_path();
    println!("Storage: {}", storage.display());

    let archive_ids = archive_session_ids(&storage, None);
    match &archive_ids {
        Ok(ids) => println!(
            "OK   Local archives: {} sessions with session_id",
            ids.len()
        ),
        Err(error) => println!("WARN Local archives: cannot inspect: {error}"),
    }
    inspect_ledger(&config, archive_ids.as_ref().ok());

    let pending = count_files(&storage.join("pending"), None);
    if pending == 0 {
        println!("OK   Ingest queue: empty");
    } else {
        println!("WARN Ingest queue: {pending} files await push; run `daybook scan --day all`");
    }
    let pending_skills = count_files(&storage.join("pending-skills"), Some("md"));
    let decisions = fs::read_to_string(storage.join("skill-decisions.jsonl"))
        .map(|content| {
            content
                .lines()
                .filter(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
                .count()
        })
        .unwrap_or(0);
    if pending_skills == 0 {
        println!("OK   Skill review: no pending candidates; {decisions} recorded decisions");
    } else {
        println!("WARN Skill review: {pending_skills} pending candidates; {decisions} recorded decisions; run `daybook review-skills`");
    }

    println!("\nRuntime integrations");
    for (runtime, path) in [
        (
            "Pi/Codex",
            home.join(".agents/skills/daybook-recall/SKILL.md"),
        ),
        (
            "Claude",
            home.join(".claude/skills/daybook-recall/SKILL.md"),
        ),
    ] {
        println!(
            "{} Recall skill ({}): {}",
            if file_matches(&path, RECALL_SKILL) {
                "OK  "
            } else {
                "WARN"
            },
            runtime,
            installation_status(&path, RECALL_SKILL)
        );
    }
    let extension = home.join(".pi/agent/extensions/daybook-pi.ts");
    let extension_status = installation_status(&extension, PI_EXTENSION);
    println!(
        "{} Pi extension: {}{}",
        if extension_status == "current" {
            "OK  "
        } else {
            "WARN"
        },
        extension_status,
        if extension_status == "missing" {
            "; run `daybook init --board`"
        } else {
            ""
        }
    );
    inspect_claude_hooks(&home);
    let mcp = cwd.join(".mcp.json");
    let mcp_ok = fs::read_to_string(&mcp)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .is_some_and(|root| root["mcpServers"]["daybook-board"]["command"] == "daybook");
    println!(
        "{} Project board MCP: {}{}",
        if mcp_ok { "OK  " } else { "WARN" },
        mcp.display(),
        if mcp_ok {
            ""
        } else {
            "; run `daybook init --board` in this project"
        }
    );

    if let Some(url) = config.push.url.as_deref() {
        let health_url = format!("{}/api/health", url.trim_end_matches('/'));
        let display_url = redacted_url(&health_url);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()?;
        match client.get(&health_url).send().await {
            Ok(response) if response.status().is_success() => {
                println!("OK   Push server: {display_url}")
            }
            Ok(response) => println!(
                "WARN Push server: {} returned {}",
                display_url,
                response.status()
            ),
            Err(error) => println!(
                "WARN Push server: {display_url} unreachable: {}",
                error.without_url()
            ),
        }
        let remote_today = crate::ledger::journal_date_at(chrono::Utc::now());
        let remote_from = remote_today - chrono::Duration::days(89);
        let archived_count = archive_session_ids(&storage, Some((remote_from, remote_today)))
            .map_or(0, |ids| ids.len());
        inspect_remote_ledger(
            &client,
            url,
            config.push.token.as_deref(),
            archived_count,
            remote_today,
        )
        .await;
    } else {
        println!(
            "INFO Push server: not configured; ledger differences are expected in local-only mode"
        );
    }
    println!("\nNo Daybook configuration or records changed.");
    Ok(())
}

fn redacted_url(raw: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(raw) else {
        return "<invalid push.url>".into();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.to_string()
}

async fn inspect_remote_ledger(
    client: &reqwest::Client,
    base_url: &str,
    token: Option<&str>,
    archived_count: usize,
    today: chrono::NaiveDate,
) {
    let url = format!(
        "{}/api/stats?days=90&today={today}",
        base_url.trim_end_matches('/')
    );
    let mut request = client.get(&url);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    match request.send().await {
        Ok(response) if response.status().is_success() => {
            match response.json::<serde_json::Value>().await {
                Ok(stats) => match stats["data"]["totals"]["sessions"].as_u64() {
                    Some(remote_count) => {
                        let level = if (remote_count as usize) < archived_count {
                            "WARN"
                        } else {
                            "INFO"
                        };
                        println!(
                            "{level} Remote ledger (90 days): {remote_count} sessions; local archive has {archived_count} in the same window. Remote totals can include other machines."
                        );
                    }
                    None => println!("WARN Remote ledger: response omitted data.totals.sessions"),
                },
                Err(error) => println!("WARN Remote ledger: invalid stats response: {error}"),
            }
        }
        Ok(response) => println!("WARN Remote ledger: stats returned {}", response.status()),
        Err(error) => println!(
            "WARN Remote ledger: stats unreachable: {}",
            error.without_url()
        ),
    }
}
fn installation_status(path: &Path, expected: &str) -> &'static str {
    match fs::read_to_string(path) {
        Ok(actual) if actual == expected => "current",
        Ok(_) => "modified or outdated; review differences and replace manually",
        Err(_) => "missing",
    }
}

fn file_matches(path: &Path, expected: &str) -> bool {
    fs::read_to_string(path).is_ok_and(|actual| actual == expected)
}

fn inspect_claude_hooks(home: &Path) {
    let path = home.join(".claude/settings.json");
    let settings = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    for (event, name, matcher) in [
        ("SessionStart", "session-start", None),
        ("PreToolUse", "pre-tool-use", Some(CLAUDE_EDIT_MATCHER)),
        ("PostToolUse", "post-tool-use", Some(CLAUDE_EDIT_MATCHER)),
        ("SessionEnd", "session-end", None),
    ] {
        let command = format!("daybook hook {name}");
        let installed = settings
            .as_ref()
            .is_some_and(|root| claude_hook_installed(root, event, &command, matcher));
        println!(
            "{} Claude {event}: {}{}",
            if installed { "OK  " } else { "WARN" },
            if installed {
                "installed"
            } else {
                "missing or settings are invalid"
            },
            if installed {
                ""
            } else {
                "; run `daybook init --board`"
            }
        );
    }
}

fn claude_hook_installed(
    settings: &serde_json::Value,
    event: &str,
    command: &str,
    matcher: Option<&str>,
) -> bool {
    settings["hooks"][event].as_array().is_some_and(|groups| {
        groups.iter().any(|group| {
            group["matcher"].as_str() == matcher
                && group["hooks"]
                    .as_array()
                    .is_some_and(|entries| entries.iter().any(|entry| entry["command"] == command))
        })
    })
}

fn inspect_ledger(config: &Config, archive_ids: Option<&BTreeSet<String>>) {
    let path = &config.server.db_path;
    if !path.exists() {
        println!(
            "INFO Ledger: {} does not exist; start `daybook serve` or configure push",
            path.display()
        );
        return;
    }
    let conn = match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(conn) => {
            if let Err(error) = conn.busy_timeout(Duration::from_secs(3)) {
                println!("WARN Ledger: cannot set busy timeout: {error}");
            }
            conn
        }
        Err(error) => {
            println!(
                "WARN Ledger: {} cannot be opened read-only: {error}",
                path.display()
            );
            return;
        }
    };
    let mut stmt = match conn.prepare("SELECT session_id, runtime FROM sessions") {
        Ok(stmt) => stmt,
        Err(error) => {
            println!("WARN Ledger: sessions table is unavailable: {error}");
            return;
        }
    };
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    });
    let mut ledger_ids = BTreeSet::new();
    let mut runtimes = std::collections::BTreeMap::<String, usize>::new();
    match rows {
        Ok(rows) => {
            for row in rows.flatten() {
                ledger_ids.insert(row.0);
                *runtimes.entry(row.1).or_default() += 1;
            }
            println!(
                "OK   Ledger: {} sessions ({})",
                ledger_ids.len(),
                path.display()
            );
            for runtime in ["claude-code", "codex", "pi"] {
                let count = runtimes.get(runtime).copied().unwrap_or(0);
                println!(
                    "{} Ledger capture {runtime}: {count} sessions{}",
                    if count > 0 { "OK  " } else { "INFO" },
                    if count > 0 {
                        ""
                    } else {
                        "; run `daybook scan --day all` if transcripts exist"
                    }
                );
            }
        }
        Err(error) => println!("WARN Ledger: cannot read sessions: {error}"),
    }
    if config.push.url.is_some() {
        println!("INFO Archive/ledger: remote push is configured; the local SQLite ledger is not the ingestion destination");
    } else if let Some(archives) = archive_ids {
        let missing: Vec<_> = archives.difference(&ledger_ids).cloned().collect();
        if missing.is_empty() {
            println!("OK   Archive/ledger: every identified local archive has a ledger row");
        } else {
            println!(
                "INFO Archive/ledger: {} local-only archives have no central row",
                missing.len()
            );
        }
    }
    match conn.query_row(
        "SELECT last_seen_utc, status FROM board_agents WHERE runtime = 'pi' ORDER BY last_seen_epoch DESC LIMIT 1",
        [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ) {
        Ok((seen, status)) => println!("OK   Pi board lifecycle: last {status} event at {seen}"),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            println!("INFO Pi board lifecycle: no Pi presence recorded in this ledger")
        }
        Err(error) => println!("WARN Pi board lifecycle: cannot inspect presence: {error}"),
    }
}

fn archive_session_ids(
    storage: &Path,
    window: Option<(chrono::NaiveDate, chrono::NaiveDate)>,
) -> std::io::Result<BTreeSet<String>> {
    let mut ids = BTreeSet::new();
    if !storage.exists() {
        return Ok(ids);
    }
    for date in fs::read_dir(storage)? {
        let date = date?;
        if !date.file_type()?.is_dir() {
            continue;
        }
        if let Some((from, to)) = window {
            let in_window = date
                .file_name()
                .to_str()
                .and_then(|name| chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d").ok())
                .is_some_and(|date| date >= from && date <= to);
            if !in_window {
                continue;
            }
        }
        for file in fs::read_dir(date.path())? {
            let file = file?;
            let path = file.path();
            if path.extension().is_none_or(|ext| ext != "md")
                || path.file_name().is_some_and(|n| n == "daily.md")
            {
                continue;
            }
            let Ok(content) = fs::read_to_string(path) else {
                continue;
            };
            if let Some(id) = content.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("session_id:")
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
            }) {
                ids.insert(id.to_string());
            }
        }
    }
    Ok(ids)
}

fn count_files(root: &Path, extension: Option<&str>) -> usize {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                count_files(&path, extension)
            } else {
                usize::from(
                    extension.is_none_or(|ext| path.extension().is_some_and(|value| value == ext)),
                )
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_recognizes_all_claude_lifecycle_hooks() {
        let settings = serde_json::json!({
            "hooks": {
                "SessionStart": [{"hooks": [{"command": "daybook hook session-start"}]}],
                "PreToolUse": [{"matcher": CLAUDE_EDIT_MATCHER, "hooks": [{"command": "daybook hook pre-tool-use"}]}],
                "PostToolUse": [{"matcher": CLAUDE_EDIT_MATCHER, "hooks": [{"command": "daybook hook post-tool-use"}]}],
                "SessionEnd": [{"hooks": [{"command": "daybook hook session-end"}]}]
            }
        });
        for (event, name, matcher) in [
            ("SessionStart", "session-start", None),
            ("PreToolUse", "pre-tool-use", Some(CLAUDE_EDIT_MATCHER)),
            ("PostToolUse", "post-tool-use", Some(CLAUDE_EDIT_MATCHER)),
            ("SessionEnd", "session-end", None),
        ] {
            assert!(claude_hook_installed(
                &settings,
                event,
                &format!("daybook hook {name}"),
                matcher
            ));
        }
        assert!(!claude_hook_installed(
            &settings,
            "PreToolUse",
            "daybook hook pre-tool-use",
            None
        ));
        assert!(!claude_hook_installed(
            &settings,
            "SessionEnd",
            "daybook hook session_end",
            None
        ));
    }

    #[test]
    fn doctor_redacts_push_url_credentials() {
        let redacted = redacted_url("https://user:secret@example.com/api/health");
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("user"));
        assert!(redacted.contains("example.com"));
    }
    #[test]
    fn archive_ids_ignore_daily_digest_and_compare_with_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("2026-11-14");
        fs::create_dir(&day).unwrap();
        fs::write(day.join("one.md"), "session_id: one\n---\nsummary").unwrap();
        fs::write(day.join("daily.md"), "session_id: ignored\n").unwrap();
        let ids = archive_session_ids(dir.path(), None).unwrap();
        let ledger = BTreeSet::from(["one".to_string(), "other-host".to_string()]);
        assert!(ids.difference(&ledger).next().is_none());
        assert_eq!(ids.len(), 1);
        let old = dir.path().join("2020-01-01");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("old.md"), "session_id: old\n---\nsummary").unwrap();
        let from = chrono::NaiveDate::from_ymd_opt(2026, 11, 13).unwrap();
        let to = chrono::NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
        let recent = archive_session_ids(dir.path(), Some((from, to))).unwrap();
        assert_eq!(recent, BTreeSet::from(["one".to_string()]));
    }
}
