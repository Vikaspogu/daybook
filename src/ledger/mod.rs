//! SQLite ledger: the server-side store for pushed session records.
//!
//! Upsert rule (D5): identical (session_id, machine) + identical
//! content_hash -> no-op; changed hash with a HIGHER transcript version
//! -> update; changed hash with a lower-or-equal version -> rejected
//! (an older queued revision must never overwrite a newer one).

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct UsageRecord {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_creation: u64,
    /// Session cost in USD, computed client-side during scan (the server
    /// has no transcript to price). None when no model pricing is known.
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AggregatesRecord {
    #[serde(default)]
    pub test_runs: u64,
    #[serde(default)]
    pub failed_tool_calls: u64,
    #[serde(default)]
    pub interrupted: u64,
    #[serde(default)]
    pub cache_hit_rate: Option<f64>,
}

/// Record pushed by a client machine. `version` is the transcript's last
/// mtime in epoch seconds: strictly increasing as a session grows.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionRecord {
    pub session_id: String,
    pub machine: String,
    pub runtime: String,
    pub project: String,
    pub journal_date: String,
    #[serde(default)]
    pub started_utc: Option<String>,
    #[serde(default)]
    pub ended_utc: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub git_branch: Option<String>,
    /// Unix seconds of the transcript's last write; drives stale rejection.
    pub version: i64,
    pub content_hash: String,
    /// Full markdown archive (frontmatter + summary sections).
    pub summary_md: String,
    #[serde(default)]
    pub facets_json: Option<String>,
    #[serde(default)]
    pub usage: UsageRecord,
    #[serde(default)]
    pub aggregates: AggregatesRecord,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpsertOutcome {
    Inserted,
    Updated,
    Noop,
    RejectedStale,
}

impl UpsertOutcome {
    /// Wire string for the ingest response payload.
    pub fn as_str(&self) -> &'static str {
        match self {
            UpsertOutcome::Inserted => "inserted",
            UpsertOutcome::Updated => "updated",
            UpsertOutcome::Noop => "noop",
            UpsertOutcome::RejectedStale => "stale",
        }
    }
}

/// Lightweight row shape for query endpoints.
/// Per-date rollup for the dashboard date list.
#[derive(Debug, Serialize, Clone)]
pub struct DateInfoRow {
    pub date: String,
    pub session_count: i64,
    /// Sessions with non-empty summary_md; drives the has_digest badge.
    pub md_count: i64,
}

/// Per-session row for the dashboard session list and day view.
#[derive(Debug, Serialize, Clone)]
pub struct SessionBriefRow {
    pub machine: String,
    pub session_id: String,
    pub runtime: String,
    pub project: String,
    pub title: String,
    pub git_branch: Option<String>,
    pub started_utc: Option<String>,
    pub ended_utc: Option<String>,
    pub summary_md: String,
    /// Parsed from usage_json at read time; None when absent or malformed.
    pub cost_usd: Option<f64>,
    pub usage: UsageRecord,
    pub aggregates: AggregatesRecord,
}

/// One session row shaped for the stats report (S4), with the JSON columns
/// decoded. Malformed JSON degrades to defaults, not a failed report.
/// Per-day rollup over the stats window.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct DailyStats {
    pub date: String,
    pub sessions: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
    pub cost_usd: f64,
    pub test_runs: u64,
    pub failed_tool_calls: u64,
    pub interrupted: u64,
    /// Pooled cache_read / (input + cache_read); None when no tokens seen.
    pub cache_hit_rate: Option<f64>,
}

/// The /api/stats payload: daily series, window totals, friction counts,
/// and the session-per-day trend over the requested window.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct StatsReport {
    /// Newest first.
    pub days: Vec<DailyStats>,
    /// Totals across the window; `date` is empty.
    pub totals: DailyStats,
    /// Friction label -> total count, summed from session facets.
    pub friction_distribution: std::collections::BTreeMap<String, u64>,
    /// Sessions per day, oldest first — a ready-to-print trend line.
    pub trend: Vec<TrendPoint>,
}

/// Roll stat rows up into the report. Pure function over rows so tests can
/// assert exact numbers without a server or database.
pub fn aggregate_stats(rows: &[StatRow]) -> StatsReport {
    use std::collections::BTreeMap;

    let mut by_date: BTreeMap<String, DailyStats> = BTreeMap::new();
    let mut friction: BTreeMap<String, u64> = BTreeMap::new();
    let mut totals = DailyStats::default();

    for row in rows {
        let day = by_date.entry(row.journal_date.clone()).or_default();
        day.date = row.journal_date.clone();
        day.sessions += 1;
        day.input += row.usage.input;
        day.output += row.usage.output;
        day.cache_read += row.usage.cache_read;
        day.cache_creation += row.usage.cache_creation;
        day.cost_usd += row.usage.cost_usd.unwrap_or(0.0);
        day.test_runs += row.aggregates.test_runs;
        day.failed_tool_calls += row.aggregates.failed_tool_calls;
        day.interrupted += row.aggregates.interrupted;

        totals.sessions += 1;
        totals.input += row.usage.input;
        totals.output += row.usage.output;
        totals.cache_read += row.usage.cache_read;
        totals.cache_creation += row.usage.cache_creation;
        totals.cost_usd += row.usage.cost_usd.unwrap_or(0.0);
        totals.test_runs += row.aggregates.test_runs;
        totals.failed_tool_calls += row.aggregates.failed_tool_calls;
        totals.interrupted += row.aggregates.interrupted;

        if let Some(fj) = &row.facets_json {
            if let Ok(facet) = serde_json::from_str::<crate::insights::facets::SessionFacet>(fj) {
                for (k, v) in facet.friction_counts {
                    *friction.entry(k).or_default() += v as u64;
                }
            }
        }
    }

    let mut days: Vec<DailyStats> = by_date.into_values().collect();
    // Pooled cache-hit rate from sums, per day and for totals.
    for d in &mut days {
        d.cache_hit_rate = pooled_hit_rate(d.input, d.cache_read);
    }
    totals.cache_hit_rate = pooled_hit_rate(totals.input, totals.cache_read);
    // BTreeMap order is oldest first; report newest first for the series,
    // oldest first for the trend line.
    days.reverse();
    let trend = days
        .iter()
        .rev()
        .map(|d| TrendPoint {
            date: d.date.clone(),
            sessions: d.sessions,
        })
        .collect();

    StatsReport {
        days,
        totals,
        friction_distribution: friction,
        trend,
    }
}

pub fn pooled_hit_rate(input: u64, cache_read: u64) -> Option<f64> {
    let denom = input + cache_read;
    if denom > 0 {
        Some(cache_read as f64 / denom as f64)
    } else {
        None
    }
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct StatRow {
    pub journal_date: String,
    #[serde(flatten)]
    pub usage: UsageRecord,
    #[serde(flatten)]
    pub aggregates: AggregatesRecord,
    #[serde(default)]
    pub facets_json: Option<String>,
}

/// One point on the session-per-day trend line.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct TrendPoint {
    pub date: String,
    pub sessions: u64,
}

pub struct Ledger {
    conn: Mutex<Connection>,
}

/// Mint a fresh bearer secret: `dbk_` + 64 hex chars from the OS RNG.
pub fn generate_secret() -> Result<String> {
    use std::io::Read;
    let mut buf = vec![0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .context("read /dev/urandom")?;
    Ok(format!(
        "dbk_{}",
        buf.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    machine TEXT NOT NULL,
    runtime TEXT NOT NULL,
    project TEXT NOT NULL,
    journal_date TEXT NOT NULL,
    started_utc TEXT,
    ended_utc TEXT,
    title TEXT,
    git_branch TEXT,
    version INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    summary_md TEXT NOT NULL,
    facets_json TEXT,
    usage_json TEXT,
    aggregates_json TEXT,
    revision INTEGER NOT NULL DEFAULT 1,
    pushed_at TEXT NOT NULL,
    UNIQUE(session_id, machine)
);
CREATE INDEX IF NOT EXISTS idx_sessions_date ON sessions(journal_date);
CREATE TABLE IF NOT EXISTS tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    label TEXT NOT NULL UNIQUE,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    last_used_at TEXT,
    revoked INTEGER NOT NULL DEFAULT 0
);
";

/// US Eastern (America/New_York) offset in seconds at `utc`:
/// EDT (UTC-4 = -14400) from 2nd Sunday of March 07:00 UTC to 1st Sunday of Nov 06:00 UTC,
/// EST (UTC-5 = -18000) otherwise.
fn us_eastern_offset_secs(utc: chrono::DateTime<chrono::Utc>) -> i32 {
    use chrono::{Datelike, NaiveDate, NaiveTime};
    let year = utc.year();
    let march1 = NaiveDate::from_ymd_opt(year, 3, 1).unwrap();
    let days_to_first_sun_mar = (7 - march1.weekday().num_days_from_sunday()) % 7;
    let second_sun_mar = march1 + chrono::Days::new((days_to_first_sun_mar + 7) as u64);
    let dst_start = second_sun_mar
        .and_time(NaiveTime::from_hms_opt(7, 0, 0).unwrap())
        .and_utc();

    let nov1 = NaiveDate::from_ymd_opt(year, 11, 1).unwrap();
    let days_to_first_sun_nov = (7 - nov1.weekday().num_days_from_sunday()) % 7;
    let first_sun_nov = nov1 + chrono::Days::new(days_to_first_sun_nov as u64);
    let dst_end = first_sun_nov
        .and_time(NaiveTime::from_hms_opt(6, 0, 0).unwrap())
        .and_utc();

    if utc >= dst_start && utc < dst_end {
        -4 * 3600
    } else {
        -5 * 3600
    }
}

/// Derive the local journal calendar date (YYYY-MM-DD) from `started_utc` when present.
/// Preserves explicit non-UTC RFC3339 offsets from the client; when `started_utc` is in UTC
/// (`Z` / `+00:00`), honors `DAYBOOK_TZ_OFFSET_HOURS` or defaults to America/New_York so
/// evening work sessions on UTC VMs don't spill into tomorrow's UTC date.
pub fn normalize_journal_date(fallback_date: &str, started_utc: Option<&str>) -> String {
    let Some(raw) = started_utc.map(str::trim).filter(|s| !s.is_empty()) else {
        return fallback_date.to_string();
    };
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(raw) else {
        return fallback_date.to_string();
    };
    if parsed.offset().local_minus_utc() != 0 {
        return parsed.format("%Y-%m-%d").to_string();
    }
    let utc = parsed.with_timezone(&chrono::Utc);
    if let Ok(hours_str) = std::env::var("DAYBOOK_TZ_OFFSET_HOURS") {
        if let Ok(hours) = hours_str.trim().parse::<i32>() {
            if let Some(offset) = chrono::FixedOffset::east_opt(hours.saturating_mul(3600)) {
                return utc.with_timezone(&offset).format("%Y-%m-%d").to_string();
            }
        }
    }
    let offset = chrono::FixedOffset::east_opt(us_eastern_offset_secs(utc)).unwrap();
    utc.with_timezone(&offset).format("%Y-%m-%d").to_string()
}

/// Fixed prompt prefixes for automated Hindsight background repository surveys.
pub const SYNTHETIC_SURVEY_PREFIXES: &[&str] = &[
    "You are performing a one-time structural survey of THIS repository to seed its Hindsight",
    "You are running a one-shot background codebase survey for Hindsight",
];

/// Exact prompt strings for automated single-turn `PONG` connectivity probes.
pub const SYNTHETIC_PONG_PROMPTS: &[&str] = &[
    "Reply with the single word PONG and nothing else.",
    "Reply with EXACTLY: PONG",
    "Say PONG",
    "Reply with PONG",
];

/// Normalize T3 scratch directory paths to `"scratch"`.
pub fn normalize_project(project: &str) -> String {
    let trimmed = project.trim();
    if trimmed.contains("/.t3/scratch/") {
        return "scratch".to_string();
    }
    trimmed.to_string()
}

/// Extract the slug from YAML frontmatter `title: "HH_MM-slug"` (stripping optional `HH_MM-`).
fn frontmatter_title_slug(summary_md: &str) -> Option<&str> {
    let mut lines = summary_md.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("title:") {
            let val = rest.trim().trim_matches('"').trim_matches('\'');
            let b = val.as_bytes();
            if b.len() > 6
                && b[0..2].iter().all(|c| c.is_ascii_digit())
                && b[2] == b'_'
                && b[3..5].iter().all(|c| c.is_ascii_digit())
                && b[5] == b'-'
            {
                return Some(&val[6..]);
            }
            return Some(val);
        }
    }
    None
}

/// True when a session record is an automated Hindsight background survey or
/// a single-turn `PONG` readiness probe rather than user work.
/// Matches known raw survey prefixes or exact PONG strings on `title`, or
/// exact YAML frontmatter `title:` slugs on unmodified `codex` archives.
pub fn is_synthetic_noise_record(title: Option<&str>, summary_md: &str) -> bool {
    if let Some(t) = title.map(str::trim) {
        let matches_title = SYNTHETIC_SURVEY_PREFIXES.iter().any(|p| t.starts_with(p))
            || SYNTHETIC_PONG_PROMPTS
                .iter()
                .any(|p| t.eq_ignore_ascii_case(p));
        if matches_title && (summary_md.is_empty() || summary_md.contains("_No files modified._")) {
            return true;
        }
    }
    if let Some(slug) = frontmatter_title_slug(summary_md) {
        let is_survey_or_pong_slug = matches!(
            slug,
            "blocked-repository-survey"
                | "repository-survey-blocked"
                | "blocked-hindsight-survey"
                | "hindsight-survey-blocked"
                | "blocked-memory-survey"
                | "memory-survey-blocked"
                | "exact-pong-response"
                | "stdin-pong-response"
        );
        is_survey_or_pong_slug
            && summary_md.contains("tags: [codex, session-archive]")
            && summary_md.contains("_No files modified._")
    } else {
        false
    }
}

/// Schema data migrations (PRAGMA user_version):
/// - v1: rebucket pre-existing UTC-dated rows using `normalize_journal_date`
/// - v2: purge automated Hindsight survey / PONG probe rows and normalize
///   `/.t3/scratch/...` project names to `"scratch"`
fn reconcile_journal_dates(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version >= 2 {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    if version < 1 {
        let mut stmt = tx.prepare(
            "SELECT id, journal_date, started_utc FROM sessions WHERE started_utc IS NOT NULL AND started_utc != ''",
        )?;
        let rows: Vec<(i64, String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (id, current_date, started_utc) in rows {
            let normalized = normalize_journal_date(&current_date, Some(&started_utc));
            if normalized != current_date {
                tx.execute(
                    "UPDATE sessions SET journal_date = ?1 WHERE id = ?2",
                    rusqlite::params![normalized, id],
                )?;
            }
        }
    }
    if version < 2 {
        let mut stmt = tx.prepare(
            "SELECT id, project, COALESCE(title, ''), COALESCE(summary_md, '') FROM sessions",
        )?;
        let rows: Vec<(i64, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut purged = 0usize;
        for (id, project, title, summary_md) in rows {
            if is_synthetic_noise_record(Some(&title), &summary_md) {
                tx.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])?;
                purged += 1;
                continue;
            }
            let norm_project = normalize_project(&project);
            if norm_project != project {
                tx.execute(
                    "UPDATE sessions SET project = ?1 WHERE id = ?2",
                    rusqlite::params![norm_project, id],
                )?;
            }
        }
        if purged > 0 {
            eprintln!("[daybook] v2 migration purged {purged} synthetic survey/probe rows");
        }
    }
    tx.execute_batch("PRAGMA user_version = 2;")?;
    tx.commit()?;
    Ok(())
}
impl Ledger {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let existed = dir.exists();
            std::fs::create_dir_all(dir)
                .with_context(|| format!("create ledger dir {}", dir.display()))?;
            // Only chmod a dir we just created — a pre-existing parent
            // (config may point db_path anywhere, e.g. $HOME) keeps its
            // own permissions.
            #[cfg(unix)]
            if !existed {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }
        let conn =
            Connection::open(path).with_context(|| format!("open ledger {}", path.display()))?;
        // WAL: readers don't block the ingest writer.
        let _: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .with_context(|| "set WAL")?;
        conn.execute_batch("PRAGMA busy_timeout=5000;")?;
        conn.execute_batch(SCHEMA)?;
        reconcile_journal_dates(&conn)?;
        // Token hashes and all summaries live here: owner-only on unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::Permissions::from_mode(0o600);
            let _ = std::fs::set_permissions(path, mode.clone());
            // WAL sidecars hold the same data; best-effort same perms.
            for side in ["-wal", "-shm"] {
                let _ = std::fs::set_permissions(
                    std::path::PathBuf::from(format!("{}{}", path.display(), side)),
                    mode.clone(),
                );
            }
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// D5 upsert. Callers must send the full record every time.
    pub fn upsert_session(&self, r: &SessionRecord) -> Result<UpsertOutcome> {
        if is_synthetic_noise_record(r.title.as_deref(), &r.summary_md) {
            return Ok(UpsertOutcome::Noop);
        }
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let existing: Option<(i64, String)> = conn
            .query_row(
                "SELECT version, content_hash FROM sessions
                 WHERE session_id = ?1 AND machine = ?2",
                [&r.session_id, &r.machine],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })?;

        let outcome = match existing {
            None => UpsertOutcome::Inserted,
            Some((_, hash)) if hash == r.content_hash => UpsertOutcome::Noop,
            Some((version, _)) if r.version > version => UpsertOutcome::Updated,
            Some(_) => UpsertOutcome::RejectedStale,
        };
        if outcome == UpsertOutcome::RejectedStale {
            return Ok(outcome);
        }
        if outcome == UpsertOutcome::Noop {
            return Ok(outcome);
        }

        let journal_date = normalize_journal_date(&r.journal_date, r.started_utc.as_deref());
        let project = normalize_project(&r.project);
        let usage = serde_json::to_string(&r.usage)?;
        let aggs = serde_json::to_string(&r.aggregates)?;
        conn.execute(
            "INSERT INTO sessions (session_id, machine, runtime, project, journal_date,
                started_utc, ended_utc, title, git_branch, version, content_hash,
                summary_md, facets_json, usage_json, aggregates_json, revision, pushed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,1,?16)
             ON CONFLICT(session_id, machine) DO UPDATE SET
                runtime=excluded.runtime, project=excluded.project,
                journal_date=excluded.journal_date,
                started_utc=excluded.started_utc, ended_utc=excluded.ended_utc,
                title=excluded.title, git_branch=excluded.git_branch,
                version=excluded.version, content_hash=excluded.content_hash,
                summary_md=excluded.summary_md, facets_json=excluded.facets_json,
                usage_json=excluded.usage_json, aggregates_json=excluded.aggregates_json,
                revision=sessions.revision+1, pushed_at=excluded.pushed_at",
            rusqlite::params![
                r.session_id,
                r.machine,
                r.runtime,
                project,
                journal_date,
                r.started_utc,
                r.ended_utc,
                r.title,
                r.git_branch,
                r.version,
                r.content_hash,
                r.summary_md,
                r.facets_json,
                usage,
                aggs,
                now,
            ],
        )?;
        Ok(outcome)
    }

    /// Date list for the dashboard: one GROUP BY, no N+1.
    pub fn date_infos(&self) -> Result<Vec<DateInfoRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT journal_date, COUNT(*),
                    SUM(CASE WHEN summary_md IS NOT NULL AND summary_md != '' THEN 1 ELSE 0 END)
             FROM sessions GROUP BY journal_date ORDER BY journal_date DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(DateInfoRow {
                    date: row.get(0)?,
                    session_count: row.get(1)?,
                    md_count: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Sessions of one journal date with their summary markdown,
    /// ordered oldest-start first (stable day view).
    /// (project, title) pairs for a date — the minimum list_dates needs for
    /// previews, without loading full summary markdown or usage blobs.
    pub fn session_project_titles(&self, date: &str) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT project, COALESCE(title, '') FROM sessions
             WHERE journal_date = ?1
             ORDER BY COALESCE(started_utc, pushed_at), machine, session_id",
        )?;
        let pairs = stmt.query_map(rusqlite::params![date], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for p in pairs {
            out.push(p?);
        }
        Ok(out)
    }

    pub fn session_briefs(&self, date: &str) -> Result<Vec<SessionBriefRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT machine, session_id, runtime, project, COALESCE(title, ''),
                git_branch, started_utc, ended_utc, COALESCE(summary_md, ''),
                COALESCE(usage_json, ''), COALESCE(aggregates_json, '')
                FROM sessions
                WHERE journal_date = ?1
                ORDER BY COALESCE(started_utc, pushed_at), machine, session_id",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![date], |row| {
                let usage: UsageRecord =
                    serde_json::from_str(row.get::<_, String>(9)?.as_str()).unwrap_or_default();
                let aggregates: AggregatesRecord =
                    serde_json::from_str(row.get::<_, String>(10)?.as_str()).unwrap_or_default();
                Ok(SessionBriefRow {
                    machine: row.get(0)?,
                    session_id: row.get(1)?,
                    runtime: row.get(2)?,
                    project: row.get(3)?,
                    title: row.get(4)?,
                    git_branch: row.get(5)?,
                    started_utc: row.get(6)?,
                    ended_utc: row.get(7)?,
                    summary_md: row.get(8)?,
                    cost_usd: usage.cost_usd,
                    usage,
                    aggregates,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Point lookup for the session-detail endpoint: one row by
    /// (journal_date, machine, session_id). `Ok(None)` = no such row;
    /// `Err` = the query itself failed (caller must not fold it into a miss).
    pub fn session_brief(
        &self,
        date: &str,
        machine: &str,
        session_id: &str,
    ) -> Result<Option<SessionBriefRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT machine, session_id, runtime, project, COALESCE(title, ''),
                git_branch, started_utc, ended_utc, COALESCE(summary_md, ''),
                COALESCE(usage_json, ''), COALESCE(aggregates_json, '')
                FROM sessions
                WHERE journal_date = ?1 AND machine = ?2 AND session_id = ?3",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![date, machine, session_id], |row| {
            let usage: UsageRecord =
                serde_json::from_str(row.get::<_, String>(9)?.as_str()).unwrap_or_default();
            let aggregates: AggregatesRecord =
                serde_json::from_str(row.get::<_, String>(10)?.as_str()).unwrap_or_default();
            Ok(SessionBriefRow {
                machine: row.get(0)?,
                session_id: row.get(1)?,
                runtime: row.get(2)?,
                project: row.get(3)?,
                title: row.get(4)?,
                git_branch: row.get(5)?,
                started_utc: row.get(6)?,
                ended_utc: row.get(7)?,
                summary_md: row.get(8)?,
                cost_usd: usage.cost_usd,
                usage,
                aggregates,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Rows for the stats report (S4): the `days` most recent journal
    /// dates, newest first. Malformed JSON columns degrade to defaults
    /// rather than failing the whole report.
    pub fn stat_rows(&self, days: usize) -> Result<Vec<StatRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT journal_date, usage_json, aggregates_json, facets_json
             FROM sessions WHERE journal_date IN
               (SELECT DISTINCT journal_date FROM sessions
                ORDER BY journal_date DESC LIMIT ?1)
             ORDER BY journal_date DESC",
        )?;
        let rows = stmt
            .query_map([days as i64], |row| {
                let usage_s: Option<String> = row.get(1)?;
                let aggs_s: Option<String> = row.get(2)?;
                Ok(StatRow {
                    journal_date: row.get(0)?,
                    usage: usage_s
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    aggregates: aggs_s
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    facets_json: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    // ---- machine tokens (D7) ----

    pub fn mint_token(&self, label: &str, token_hash: &str) -> Result<()> {
        // A token value identifies one credential. When the GitOps label for
        // the same stored value changes (e.g. vm-push -> vm), re-home the hash
        // instead of tripping UNIQUE(token_hash) against the old row.
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM tokens WHERE token_hash = ?2 AND label <> ?1",
            rusqlite::params![label, token_hash],
        )?;
        tx.execute(
            "INSERT INTO tokens (label, token_hash, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(label) DO UPDATE SET
                 token_hash = excluded.token_hash,
                 created_at = excluded.created_at,
                 revoked = 0,
                 last_used_at = NULL",
            rusqlite::params![label, token_hash, chrono::Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// None when the token is unknown or revoked; stamps last_used_at.
    /// DB errors propagate (→ 500): only a clean miss is a 401.
    pub fn check_token(&self, token: &str) -> Result<Option<String>> {
        let hash = crate::ledger::sha256_hex(token.as_bytes());
        let conn = self.conn.lock().unwrap();
        let label = conn
            .query_row(
                "SELECT label FROM tokens WHERE token_hash = ?1 AND revoked = 0",
                [hash.clone()],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })?;
        // ponytail: touch-write per authenticated request serializes reads
        // behind the connection mutex. Fine for a single-operator dashboard;
        // batch or throttle last_used_at updates if reads ever contend.
        if label.is_some() {
            conn.execute(
                "UPDATE tokens SET last_used_at = ?1
                 WHERE token_hash = ?2 AND revoked = 0",
                rusqlite::params![chrono::Utc::now().to_rfc3339(), hash],
            )?;
        }
        Ok(label)
    }

    pub fn list_tokens(&self) -> Result<Vec<(String, String, String, bool)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT label, created_at, COALESCE(last_used_at, ''), revoked = 1
             FROM tokens ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn revoke_token(&self, label: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute(
            "UPDATE tokens SET revoked = 1 WHERE label = ?1 AND revoked = 0",
            [label],
        )? > 0)
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(session_id: &str, version: i64, hash: &str) -> SessionRecord {
        record_md(session_id, version, hash, "# md")
    }

    fn record_md(session_id: &str, version: i64, hash: &str, md: &str) -> SessionRecord {
        SessionRecord {
            session_id: session_id.into(),
            machine: "vm1".into(),
            runtime: "pi".into(),
            project: "/tmp/p".into(),
            journal_date: "2026-10-29".into(),
            started_utc: None,
            ended_utc: None,
            title: Some("t".into()),
            git_branch: None,
            version,
            content_hash: hash.into(),
            summary_md: md.into(),
            facets_json: None,
            usage: UsageRecord::default(),
            aggregates: AggregatesRecord::default(),
        }
    }

    #[test]
    fn test_upsert_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("t.db3")).unwrap();
        // insert
        assert_eq!(
            ledger
                .upsert_session(&record_md("s1", 100, "h1", "# v1"))
                .unwrap(),
            UpsertOutcome::Inserted
        );
        // identical push -> noop
        assert_eq!(
            ledger
                .upsert_session(&record_md("s1", 100, "h1", "# v1"))
                .unwrap(),
            UpsertOutcome::Noop
        );
        // grown transcript -> update
        assert_eq!(
            ledger
                .upsert_session(&record_md("s1", 200, "h2", "# v2"))
                .unwrap(),
            UpsertOutcome::Updated
        );
        // older queued revision after newer -> rejected
        assert_eq!(
            ledger
                .upsert_session(&record_md("s1", 150, "h3", "# v3"))
                .unwrap(),
            UpsertOutcome::RejectedStale
        );
        // latest content still the newer revision (session_briefs serves it)
        let rows = ledger.session_briefs("2026-10-29").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].summary_md, "# v2");
        // second machine is a separate row
        let mut other = record("s1", 100, "h1");
        other.machine = "vm2".into();
        assert_eq!(
            ledger.upsert_session(&other).unwrap(),
            UpsertOutcome::Inserted
        );
        let dates: Vec<String> = ledger
            .date_infos()
            .unwrap()
            .iter()
            .map(|d| d.date.clone())
            .collect();
        assert_eq!(dates, vec!["2026-10-29"]);
    }

    /// S4 gate fixture: known values in, every promised metric asserted out.
    #[test]
    fn stats_report_from_fixture_ledger_matches_expected_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("t.db3")).unwrap();

        let mut a1 = record_md("sA1", 100, "h1", "# a1");
        a1.journal_date = "2026-10-28".into();
        a1.usage = UsageRecord {
            input: 1_000,
            output: 200,
            cache_read: 3_000,
            cache_creation: 100,
            cost_usd: Some(0.10),
        };
        a1.aggregates = AggregatesRecord {
            test_runs: 3,
            failed_tool_calls: 1,
            interrupted: 0,
            cache_hit_rate: None,
        };
        a1.facets_json = Some(r#"{"friction_counts": {"misunderstood_request": 2}}"#.into());
        ledger.upsert_session(&a1).unwrap();

        let mut a2 = record_md("sA2", 100, "h2", "# a2");
        a2.journal_date = "2026-10-28".into();
        a2.usage = UsageRecord {
            input: 500,
            output: 100,
            cache_read: 500,
            cache_creation: 0,
            cost_usd: Some(0.05),
        };
        a2.aggregates = AggregatesRecord {
            test_runs: 2,
            failed_tool_calls: 0,
            interrupted: 1,
            cache_hit_rate: None,
        };
        // malformed facets JSON must degrade, not break the report
        a2.facets_json = Some("{{not json".into());
        ledger.upsert_session(&a2).unwrap();

        let mut b1 = record_md("sB1", 100, "h3", "# b1");
        b1.journal_date = "2026-10-29".into();
        b1.usage = UsageRecord {
            input: 2_000,
            output: 400,
            cache_read: 0,
            cache_creation: 0,
            cost_usd: Some(0.20),
        };
        b1.aggregates = AggregatesRecord {
            test_runs: 5,
            failed_tool_calls: 2,
            interrupted: 0,
            cache_hit_rate: None,
        };
        ledger.upsert_session(&b1).unwrap();

        // older date, outside a 2-day window
        let mut c1 = record_md("sC1", 100, "h4", "# c1");
        c1.journal_date = "2026-10-15".into();
        ledger.upsert_session(&c1).unwrap();
        let rows = ledger.stat_rows(2).unwrap();
        let r = aggregate_stats(&rows);

        // Totals
        assert_eq!(r.totals.sessions, 3);
        assert_eq!(r.totals.input, 3_500);
        assert_eq!(r.totals.output, 700);
        assert_eq!(r.totals.cache_read, 3_500);
        assert_eq!(r.totals.cache_creation, 100);
        assert!((r.totals.cost_usd - 0.35).abs() < 1e-9);
        assert_eq!(r.totals.test_runs, 10);
        assert_eq!(r.totals.failed_tool_calls, 3);
        assert_eq!(r.totals.interrupted, 1);
        assert_eq!(r.totals.cache_hit_rate, Some(0.5));

        // Per-day series, newest first
        assert_eq!(r.days.len(), 2);
        assert_eq!(r.days[0].date, "2026-10-29");
        assert_eq!(r.days[0].sessions, 1);
        assert_eq!(r.days[0].input, 2_000);
        assert!((r.days[0].cost_usd - 0.20).abs() < 1e-9);
        assert_eq!(r.days[0].test_runs, 5);
        assert_eq!(r.days[0].cache_hit_rate, Some(0.0));
        assert_eq!(r.days[1].date, "2026-10-28");
        assert_eq!(r.days[1].sessions, 2);
        assert_eq!(r.days[1].cache_hit_rate, Some(0.7)); // 3500/5000
        assert!((r.days[1].cost_usd - 0.15).abs() < 1e-9);

        // Friction distribution from facets
        assert_eq!(
            r.friction_distribution,
            std::collections::BTreeMap::from([("misunderstood_request".to_string(), 2)])
        );

        // Trend: oldest first, sessions per day
        assert_eq!(
            r.trend,
            vec![
                TrendPoint {
                    date: "2026-10-28".into(),
                    sessions: 2
                },
                TrendPoint {
                    date: "2026-10-29".into(),
                    sessions: 1
                },
            ]
        );

        // Window cutoff: a 3-day window also pulls in the older date
        let older = ledger.stat_rows(3).unwrap();
        let older_report = aggregate_stats(&older);
        assert_eq!(older_report.days.len(), 3);
        assert_eq!(older_report.days[2].date, "2026-10-15");
    }

    #[test]
    fn test_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("t.db3")).unwrap();
        ledger.mint_token("vm1", &sha256_hex(b"secret1")).unwrap();
        assert_eq!(
            ledger.check_token("secret1").unwrap().as_deref(),
            Some("vm1")
        );
        assert_eq!(ledger.check_token("wrong").unwrap(), None);
        assert!(ledger.revoke_token("vm1").unwrap());
        assert_eq!(ledger.check_token("secret1").unwrap(), None);
        assert!(!ledger.revoke_token("vm1").unwrap());
        // Rotation: minting the same label again revokes the old secret.
        ledger.mint_token("vm1", &sha256_hex(b"secret2")).unwrap();
        assert_eq!(ledger.check_token("secret1").unwrap(), None);
        assert_eq!(
            ledger.check_token("secret2").unwrap().as_deref(),
            Some("vm1")
        );
    }

    #[test]
    fn test_token_rehome_on_label_change() {
        // GitOps ensure with a renamed label (vm-push -> vm) and the same
        // stored value: the hash re-homes instead of hitting UNIQUE.
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("t.db3")).unwrap();
        ledger
            .mint_token("vm-push", &sha256_hex(b"secret1"))
            .unwrap();
        ledger.mint_token("vm", &sha256_hex(b"secret1")).unwrap();
        assert_eq!(
            ledger.check_token("secret1").unwrap().as_deref(),
            Some("vm")
        );
        // The old row is gone; only one active token remains.
        let rows = ledger.list_tokens().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "vm");
    }

    #[test]
    fn test_check_token_miss_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("t.db3")).unwrap();
        ledger.mint_token("vm1", &sha256_hex(b"secret1")).unwrap();
        // A miss must neither authenticate nor stamp last_used_at.
        assert_eq!(ledger.check_token("wrong").unwrap(), None);
        let stamped = ledger
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM tokens WHERE last_used_at IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(stamped, 0);
        // The hit still works afterwards.
        assert_eq!(
            ledger.check_token("secret1").unwrap().as_deref(),
            Some("vm1")
        );
    }

    #[test]
    fn test_reconcile_journal_dates_migrates_once_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("mig.db3");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            conn.execute(
                "INSERT INTO sessions (
                    session_id, machine, runtime, project, journal_date,
                    started_utc, version, summary_md, content_hash, pushed_at
                ) VALUES (
                    's1', 'vm1', 'codex', '/home/agent/.t3/scratch/2026-10-28-i-want-to-create-a-02113651', '2026-10-31',
                    '2026-10-31T00:15:00Z', 1, '---\ntitle: \"20_15-fix-survey-filter\"\n---\n## Summary\nDiscussed blocked-repository-survey and exact-pong-response in prose.', 'h1', '2026-10-31T00:20:00Z'
                ), (
                    's2', 'vm1', 'codex', 'durable-sre-agent', '2026-10-30',
                    '2026-10-30T12:00:00Z', 1, '---\ntitle: \"00_27-blocked-repository-survey\"\ntags: [codex, session-archive]\n---\n# 00_27-blocked-repository-survey\n\n## Code Changes\n\n_No files modified._\n', 'h2', '2026-10-30T12:05:00Z'
                )",
                [],
            )
            .unwrap();
        }
        let ledger = Ledger::open(&db_path).unwrap();
        let dates = ledger.date_infos().unwrap();
        assert_eq!(dates.len(), 1);
        assert_eq!(dates[0].date, "2026-10-30");
        assert_eq!(dates[0].session_count, 1);
        let briefs = ledger.session_briefs("2026-10-30").unwrap();
        assert_eq!(briefs.len(), 1);
        assert_eq!(briefs[0].project, "scratch");

        let mut noise = record_md(
            "s3",
            1,
            "h3",
            "---\r\ntitle: '00_58-exact-pong-response'\r\ntags: [codex, session-archive]\r\n---\r\n## Code Changes\r\n\r\n_No files modified._\r\n",
        );
        noise.project = "/home/agent/.t3/scratch/2026-10-28-i-want-to-create-a-02113651".into();
        assert_eq!(ledger.upsert_session(&noise).unwrap(), UpsertOutcome::Noop);
        assert!(is_synthetic_noise_record(
            Some("You are performing a one-time structural survey of THIS repository to seed its Hindsight memory bank"),
            ""
        ));
        assert!(is_synthetic_noise_record(
            Some("Reply with EXACTLY: PONG"),
            ""
        ));
        assert!(!is_synthetic_noise_record(
            Some("Reply with EXACTLY: PONG"),
            "## Code Changes\n\nModified src/main.rs\n"
        ));
        assert!(!is_synthetic_noise_record(
            Some("Real user session"),
            "---\ntitle: \"00_58-exact-pong-response\"\ntags: [pi, session-archive]\n---\n## Code Changes\n\n_No files modified._\n"
        ));
        assert!(!is_synthetic_noise_record(
            Some("Real user session"),
            "---\ntitle: \"00_58-exact-pong-response\"\ntags: [codex, session-archive]\n---\n## Code Changes\n\nModified src/main.rs\n"
        ));

        assert_eq!(
            normalize_project("/home/agent/.t3/scratch/2026-10-28-i-want-to-create-a-02113651"),
            "scratch"
        );
        assert_eq!(normalize_project("-"), "-");
        assert_eq!(normalize_project("daybook"), "daybook");

        let conn = ledger.conn.lock().unwrap();
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(user_version, 2);
        reconcile_journal_dates(&conn).unwrap();
    }
}
