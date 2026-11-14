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

fn redact_facets(facets: &str, config: &Config) -> String {
    fn visit(value: &mut serde_json::Value, config: &Config, key: Option<&str>) {
        match value {
            serde_json::Value::String(text) => {
                *text = crate::redaction::redact_named_value(key, text, config)
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    visit(item, config, key);
                }
            }
            serde_json::Value::Object(map) => {
                for (key, item) in map {
                    visit(item, config, Some(key));
                }
            }
            serde_json::Value::Number(_)
                if key.is_some_and(crate::redaction::sensitive_numeric_key) =>
            {
                *value = serde_json::Value::String(crate::redaction::REDACTED.to_string());
            }
            _ => {}
        }
    }

    let Ok(mut value) = serde_json::from_str(facets) else {
        return crate::redaction::redact_for_config(facets, config);
    };
    visit(&mut value, config, None);
    serde_json::to_string(&value)
        .unwrap_or_else(|_| crate::redaction::redact_for_config(facets, config))
}

fn ingest_payload(config: &Config, record: &SessionRecord) -> Result<serde_json::Value> {
    let mut record = record.clone();
    record.title = record
        .title
        .map(|title| crate::redaction::redact_for_config(&title, config));
    record.summary_md = crate::redaction::redact_for_config(&record.summary_md, config);
    record.content_hash = crate::ledger::sha256_hex(record.summary_md.as_bytes());
    record.facets_json = record
        .facets_json
        .map(|facets| redact_facets(&facets, config));
    Ok(serde_json::to_value(record)?)
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
    let payload = ingest_payload(config, record)?;
    let client = client();
    let resp = client
        .post(format!("{}/api/ingest", url.trim_end_matches('/')))
        .bearer_auth(token)
        .json(&payload)
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

/// Fetch one day's summary from the ledger server (GET /api/dates/:date).
/// Returns the raw DTO as JSON so the CLI never drifts from server structs.
pub async fn fetch_daily(config: &Config, date: &str) -> Result<serde_json::Value> {
    let url = config.push.url.clone().context(
        "push.url is not configured — `daybook serve` on some machine, then set push.url",
    )?;
    let token = config
        .push
        .token
        .clone()
        .context("push.token is not configured — mint one with `daybook token`")?;
    let resp = client()
        .get(format!("{}/api/dates/{}", url.trim_end_matches('/'), date))
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("GET {}/api/dates/{}", url, date))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "daily summary returned {}: {}",
            status,
            truncate(&body, 200)
        );
    }
    let env: serde_json::Value = resp.json().await.context("malformed daily response")?;
    let ok = env["success"].as_bool().unwrap_or(false);
    let data = env["data"].clone();
    if ok && !data.is_null() {
        Ok(data)
    } else {
        anyhow::bail!(
            "{}",
            env["error"]
                .as_str()
                .unwrap_or("daily summary request failed")
        )
    }
}
/// Fetch project-scoped recall results from the ledger server.
pub async fn fetch_recall(
    config: &Config,
    project: Option<&str>,
    search: Option<&str>,
    limit: usize,
) -> Result<Vec<crate::ledger::RecallRow>> {
    let url = config.push.url.clone().context(
        "push.url is not configured — `daybook serve` on some machine, then set push.url",
    )?;
    let token = config
        .push
        .token
        .clone()
        .context("push.token is not configured — mint one with `daybook token`")?;
    let mut request = client()
        .get(format!("{}/api/recall", url.trim_end_matches('/')))
        .bearer_auth(token)
        .query(&[("limit", limit.clamp(1, 20).to_string())]);
    if let Some(project) = project {
        request = request.query(&[("project", project)]);
    }
    if let Some(search) = search {
        request = request.query(&[("search", search)]);
    }
    let resp = request.send().await.context("GET /api/recall")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("recall returned {}: {}", status, truncate(&body, 200));
    }
    let env: serde_json::Value = resp.json().await.context("malformed recall response")?;
    if !env["success"].as_bool().unwrap_or(false) {
        anyhow::bail!(env["error"]
            .as_str()
            .unwrap_or("recall request failed")
            .to_string());
    }
    serde_json::from_value(env["data"].clone()).context("malformed recall results")
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

/// Recall from the central server, falling back to a local ledger when present.
pub async fn recall(
    config: &Config,
    project: Option<&str>,
    search: Option<&str>,
    limit: usize,
) -> Result<Vec<crate::ledger::RecallRow>> {
    if config.push.url.is_some() {
        match fetch_recall(config, project, search, limit).await {
            Ok(rows) if !rows.is_empty() => return Ok(rows),
            Ok(_) => {}
            Err(error) if !config.server.db_path.exists() => return Err(error),
            Err(_) => {}
        }
    }
    if config.server.db_path.exists() {
        return crate::ledger::Ledger::open(&config.server.db_path)?.recall(project, search, limit);
    }
    Ok(Vec::new())
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

    #[tokio::test]
    async fn recall_uses_local_ledger_without_push_configuration() {
        let mut config = test_config();
        config.server.db_path = config.storage.path.join("ledger.db3");
        let ledger = crate::ledger::Ledger::open(&config.server.db_path).unwrap();
        ledger.upsert_session(&rec()).unwrap();
        let rows = recall(&config, Some("/p"), Some("md"), 5).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source(), "ledger://2026-10-29/m/abc/123");
        std::fs::remove_dir_all(&config.storage.path).unwrap();
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
    #[test]
    fn ingest_payload_redacts_content_and_preserves_identity() {
        let mut config = test_config();
        config.push.token = Some("configured-token-12345".into());
        let mut record = rec();
        record.machine = "dev@example.com".into();
        record.content_hash = "AIza12345678901234567890123456789012345".into();
        record.title = Some("used configured-token-12345".into());
        record.summary_md = "DATABASE_PASSWORD=correct-horse-secret".into();
        record.facets_json = Some(
            r#"{"input_tokens":1234,"password":12345678,"command":"export API_KEY=sk-ant-abcdefghijklmnopqrstuvwx","nested":"export PASSWORD=\"hunter2\"","password_attempts":3,"userPassword":12345678,"accessToken":987654,"db-password":1234}"#.into(),
        );

        let payload = ingest_payload(&config, &record).unwrap();
        let serialized = serde_json::to_string(&payload).unwrap();
        assert!(!serialized.contains("configured-token-12345"));
        assert!(!serialized.contains("correct-horse-secret"));
        assert!(!serialized.contains("sk-ant-abcdefghijklmnopqrstuvwx"));
        assert!(!serialized.contains("hunter2"));
        let facets: serde_json::Value =
            serde_json::from_str(payload["facets_json"].as_str().unwrap()).unwrap();
        assert_eq!(facets["input_tokens"], 1234);
        assert_eq!(facets["password"], crate::redaction::REDACTED);
        assert_eq!(facets["password_attempts"], 3);
        assert_eq!(facets["userPassword"], crate::redaction::REDACTED);
        assert_eq!(facets["accessToken"], crate::redaction::REDACTED);
        assert_eq!(facets["db-password"], crate::redaction::REDACTED);
        let named: serde_json::Value = serde_json::from_str(
            &redact_facets(
                r#"{"password":"prefix-person@company.internal-secret-suffix","tokenizer":"bert-base"}"#,
                &config,
            ),
        )
        .unwrap();
        assert_eq!(named["password"], crate::redaction::REDACTED);
        assert_eq!(named["tokenizer"], "bert-base");
        assert_eq!(payload["machine"], "dev@example.com");
        assert_eq!(
            payload["content_hash"],
            crate::ledger::sha256_hex(payload["summary_md"].as_str().unwrap().as_bytes())
        );
        assert_eq!(payload["session_id"], "abc/123");
        assert_eq!(payload["version"], 1);
    }
    #[test]
    fn ingest_hash_matches_direct_and_replayed_summary() {
        let config = test_config();
        let mut record = rec();
        record.summary_md = "PASSWORD=hunter2".into();
        record.content_hash = crate::ledger::sha256_hex(record.summary_md.as_bytes());

        let direct = ingest_payload(&config, &record).unwrap();
        let replay = ingest_payload(&config, &record).unwrap();
        for payload in [direct, replay] {
            assert_eq!(
                payload["content_hash"],
                crate::ledger::sha256_hex(payload["summary_md"].as_str().unwrap().as_bytes())
            );
        }
    }
    #[test]
    fn redacts_invalid_facets_as_text() {
        let config = test_config();
        let mut record = rec();
        record.facets_json = Some("not-json DATABASE_PASSWORD=correct-horse-secret".into());
        let payload = ingest_payload(&config, &record).unwrap();
        assert_eq!(
            payload["facets_json"],
            "not-json DATABASE_PASSWORD=[REDACTED]"
        );
    }
}
