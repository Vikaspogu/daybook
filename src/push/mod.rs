//! Push client: send session records to the daybook server after the
//! local LLM pass. Failures park the record in `~/.daybook/pending/`
//! and are retried (and removed) on the next scan — ingest is
//! idempotent, so retries are always safe.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::ledger::{AggregatesRecord, SessionRecord, UsageRecord};

/// Build a record from scan-known data. `summary_md` is the archive
/// markdown written to disk by the local LLM pass.
#[allow(clippy::too_many_arguments)]
pub fn build_record(
    config: &Config,
    session_id: &str,
    runtime: &str,
    project: &str,
    journal_date: &str,
    title: Option<String>,
    started_utc: Option<String>,
    ended_utc: Option<String>,
    git_branch: Option<String>,
    version: i64,
    content_hash: String,
    summary_md: String,
    facets_json: Option<String>,
    usage: UsageRecord,
    aggregates: AggregatesRecord,
) -> SessionRecord {
    SessionRecord {
        session_id: session_id.to_string(),
        machine: machine_label(config),
        runtime: runtime.to_string(),
        project: project.to_string(),
        journal_date: journal_date.to_string(),
        started_utc,
        ended_utc,
        title,
        git_branch,
        version,
        content_hash,
        summary_md,
        facets_json,
        usage,
        aggregates,
    }
}

pub fn machine_label(config: &Config) -> String {
    if let Some(m) = &config.push.machine {
        return m.clone();
    }
    // HOSTNAME is not exported by default in many shells; /etc/hostname is
    // the stable fallback so two VMs never collide on "unknown-host".
    if let Ok(h) = std::env::var("HOSTNAME") {
        if !h.trim().is_empty() {
            return h;
        }
    }
    if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
        let h = h.trim();
        if !h.is_empty() {
            return h.into();
        }
    }
    "unknown-host".into()
}

/// Shared push client: one connection pool for the whole process, with a
/// timeout so a dead server cannot hang `scan`.
fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest client init")
    })
}

pub fn pending_dir(config: &Config) -> PathBuf {
    config.storage.path.join("pending")
}

/// POST one record; Ok(true) = delivered, Ok(false) = push not configured.
pub async fn push_record(config: &Config, record: &SessionRecord) -> Result<bool> {
    let Some(url) = config.push.url.clone() else {
        return Ok(false);
    };
    let Some(token) = config.push.token.clone() else {
        anyhow::bail!("push.url is set but push.token is missing");
    };
    if url.starts_with("http://") {
        // Exact host match: "localhost.evil.com" must not silence the warning.
        let host = url
            .trim_start_matches("http://")
            .split(['/', ':'])
            .next()
            .unwrap_or_default();
        let local = host == "localhost" || host.starts_with("127.");
        if !local {
            eprintln!(
                "[daybook] warning: push.url is cleartext http:// — the token travels unencrypted"
            );
        }
    }
    let client = client();
    let resp = client
        .post(format!("{}/api/ingest", url.trim_end_matches('/')))
        .bearer_auth(token)
        .json(record)
        .send()
        .await
        .with_context(|| format!("POST {}/api/ingest", url))?;
    let status = resp.status();
    if status.as_u16() == 409 {
        // Server holds a newer revision of this session; that is a
        // delivered-terminal outcome, not an error — never park it.
        return Ok(true);
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("ingest returned {}: {}", status, truncate(&body, 200));
    }
    Ok(true)
}

/// Wire shape of the server's ApiResponse for GET /api/stats.
// ponytail: duplicated envelope, not shared with server::dto — that type is
// Serialize-only and lives behind server feature assumptions.
#[derive(serde::Deserialize)]
struct StatsEnvelope {
    success: bool,
    data: Option<crate::ledger::StatsReport>,
    error: Option<String>,
}

/// GET /api/stats?days=N with bearer auth. The CLI stats command renders
/// the report; everything else stays server-side.
pub async fn fetch_stats(config: &Config, days: usize) -> Result<crate::ledger::StatsReport> {
    let url = config.push.url.clone().context(
        "push.url is not configured — `daybook serve` on some machine, then set push.url",
    )?;
    let token = config
        .push
        .token
        .clone()
        .context("push.token is not configured — mint one with `daybook token`")?;
    let resp = client()
        .get(format!(
            "{}/api/stats?days={}",
            url.trim_end_matches('/'),
            days.clamp(1, 90)
        ))
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("GET {}/api/stats", url))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("stats returned {}: {}", status, truncate(&body, 200));
    }
    let env: StatsEnvelope = resp.json().await.context("malformed stats response")?;
    match (env.success, env.data) {
        (true, Some(report)) => Ok(report),
        _ => anyhow::bail!(env.error.unwrap_or_else(|| "stats failed".into())),
    }
}

/// Park a failed record for the next scan. Filename is stable per session
/// so retries overwrite, not accumulate: `pending/<journal_date>-<session_id>.json`.
/// A newer parked revision may replace an older one, never the reverse.
pub fn park_record(config: &Config, record: &SessionRecord) -> Result<PathBuf> {
    let dir = pending_dir(config);
    std::fs::create_dir_all(&dir)?;
    // Parked records live in the 0600-config world: owner-only dir.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let safe_date = sanitize(&record.journal_date, 12);
    let safe_id = sanitize(&record.session_id, 64);
    let path = dir.join(format!("{safe_date}-{safe_id}.json"));
    // Never let an older revision clobber a newer parked one.
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(existing) = serde_json::from_slice::<SessionRecord>(&bytes) {
            if existing.version >= record.version {
                return Ok(path);
            }
        }
    }
    let bytes = serde_json::to_vec(record)?;
    // tmp+rename so a crash never leaves a half-written pending record.
    // Created 0600 from the start — no world-readable window.
    let tmp = dir.join(format!("{safe_date}-{safe_id}.json.tmp"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .and_then(|mut f| std::io::Write::write_all(&mut f, &bytes))
            .with_context(|| format!("write pending {}", tmp.display()))?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, bytes).with_context(|| format!("write pending {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("publish pending {}", path.display()))?;
    Ok(path)
}

fn sanitize(s: &str, max: usize) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(max)
        .collect()
}

/// Retry everything parked in pending/. Called at the top of every scan.
/// A record the server rejects as stale is dropped (a newer revision is
/// already there); other failures stay parked.
pub async fn retry_pending(config: &Config) -> usize {
    let dir = pending_dir(config);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut retried = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match retry_one(config, &path).await {
            Ok(true) => retried += 1,
            Ok(false) => {} // push unconfigured: stays parked
            Err(e) => eprintln!(
                "[daybook] Pending {} stays parked: {}",
                path.display(),
                truncate(&e.to_string(), 120)
            ),
        }
    }
    retried
}

async fn retry_one(config: &Config, path: &Path) -> Result<bool> {
    let bytes = std::fs::read(path)?;
    let record: SessionRecord = serde_json::from_slice(&bytes)?;
    if !push_record(config, &record).await? {
        return Ok(false); // push unconfigured: stays parked
    }
    std::fs::remove_file(path)?;
    Ok(true)
}

fn truncate(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::SessionRecord;

    fn test_config() -> Config {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut c = Config::default();
        c.storage.path =
            std::env::temp_dir().join(format!("daybook-test-{}-{n}", std::process::id()));
        c
    }

    fn rec() -> SessionRecord {
        SessionRecord {
            session_id: "abc/123".into(),
            machine: "m".into(),
            runtime: "pi".into(),
            project: "/p".into(),
            journal_date: "2026-10-29".into(),
            started_utc: None,
            ended_utc: None,
            title: None,
            git_branch: None,
            version: 1,
            content_hash: "h".into(),
            summary_md: "md".into(),
            facets_json: None,
            usage: UsageRecord::default(),
            aggregates: AggregatesRecord::default(),
        }
    }

    #[test]
    fn test_park_and_reload_roundtrip() {
        let config = test_config();
        let path = park_record(&config, &rec()).unwrap();
        assert!(path.to_string_lossy().ends_with("2026-10-29-abc_123.json"));
        let bytes = std::fs::read(&path).unwrap();
        let loaded: SessionRecord = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(loaded.session_id, "abc/123");
        assert_eq!(loaded.machine, "m");
        std::fs::remove_dir_all(pending_dir(&config)).unwrap();
    }

    #[test]
    fn test_park_record_keeps_newer_revision() {
        let config = test_config();
        let mut newer = rec();
        newer.version = 2;
        park_record(&config, &newer).unwrap();
        // An older revision arriving late must not clobber the parked newer one.
        let path = park_record(&config, &rec()).unwrap();
        let loaded: SessionRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded.version, 2);
        std::fs::remove_dir_all(pending_dir(&config)).unwrap();
    }
}
