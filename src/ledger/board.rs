use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use super::Ledger;
use crate::board::globs::{
    glob_matches_path, globs_overlap, normalize_repo_path, validate_glob_pattern,
};
use crate::board::identity::{
    extract_mentions, validate_agent_handle, validate_identifier, MAX_BODY_LEN, MAX_NOTE_LEN,
};

pub const MAX_BOARD_PROJECTS: i64 = 100;
pub const MAX_LOCKS_PER_PROJECT: i64 = 200;
pub const MAX_LOCKS_PER_MACHINE_PROJECT: i64 = 50;
pub const MAX_AUTO_LOCKS_PER_AGENT: i64 = 20;
pub const MAX_AGENTS_PER_PROJECT: i64 = 100;
pub const MAX_AGENTS_PER_MACHINE_PROJECT: i64 = 25;
pub const MAX_TASKS_PER_PROJECT: i64 = 500;
pub const MAX_BOARD_MESSAGES: i64 = 500;

pub const DEFAULT_LOCK_TTL_MINUTES: u32 = 30;
pub const DEFAULT_AUTO_LOCK_TTL_MINUTES: u32 = 15;
pub const MAX_LOCK_TTL_MINUTES: u32 = 240;

#[derive(Debug, Clone)]
pub struct BoardCaller {
    pub machine: String,
    pub admin: bool,
}

impl BoardCaller {
    #[allow(dead_code)]
    pub fn local() -> Self {
        Self {
            machine: "local".to_string(),
            admin: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardAgentRow {
    pub project: String,
    pub agent: String,
    pub machine: String,
    pub runtime: String,
    pub cwd: String,
    pub branch: String,
    pub status: String,
    pub current_task: Option<String>,
    pub last_seen_utc: String,
    pub last_seen_epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardLockRow {
    pub id: String,
    pub project: String,
    pub agent: String,
    pub machine: String,
    pub paths: Vec<String>,
    pub note: String,
    pub auto: bool,
    pub claimed_utc: String,
    pub expires_utc: String,
    pub expires_epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardTaskRow {
    pub project: String,
    pub task_id: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub agent: Option<String>,
    pub machine: Option<String>,
    pub branch: Option<String>,
    pub note: String,
    pub pr_url: Option<String>,
    pub created_utc: String,
    pub updated_utc: String,
    pub updated_epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardMessageRow {
    pub id: i64,
    pub project: String,
    pub channel: String,
    pub agent: String,
    pub machine: String,
    pub body: String,
    pub mentions: Vec<String>,
    pub parent_id: Option<i64>,
    pub created_utc: String,
    pub created_epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardStatusSnapshot {
    pub projects: Vec<String>,
    pub project: Option<String>,
    pub agents: Vec<BoardAgentRow>,
    pub locks: Vec<BoardLockRow>,
    pub tasks: Vec<BoardTaskRow>,
    pub messages: Vec<BoardMessageRow>,
    pub unread_mentions: Vec<BoardMessageRow>,
    pub next_since_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardClaimLocksOutcome {
    pub lock: BoardLockRow,
    pub conflicts: Vec<BoardLockRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardTouchOutcome {
    pub lock: BoardLockRow,
    pub unread_mentions: Vec<BoardMessageRow>,
    pub next_since_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardCheckOutcome {
    pub blocked: bool,
    pub blocking_locks: Vec<BoardLockRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardClaimTaskOutcome {
    pub ok: bool,
    pub held_by: Option<String>,
    pub task: BoardTaskRow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardMessagesPage {
    pub messages: Vec<BoardMessageRow>,
    pub next_since_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardHeartbeatReq {
    pub project: String,
    pub agent: String,
    #[serde(default)]
    pub runtime: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub current_task: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardClaimLocksReq {
    pub project: String,
    pub agent: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub ttl_minutes: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardTouchLockReq {
    pub project: String,
    pub agent: String,
    pub path: String,
    #[serde(default)]
    pub ttl_minutes: Option<u32>,
    #[serde(default)]
    pub since_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardReleaseLocksReq {
    pub project: String,
    pub agent: String,
    #[serde(default)]
    pub lock_id: Option<String>,
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub auto_only: bool,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardClaimTaskReq {
    pub project: String,
    pub task_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    pub agent: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardUpdateTaskReq {
    pub project: String,
    pub task_id: String,
    pub agent: String,
    pub status: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub release_locks: bool,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardPostMessageReq {
    pub project: String,
    #[serde(default)]
    pub channel: Option<String>,
    pub agent: String,
    pub body: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoardMessagesQuery {
    pub project: String,
    pub channel: Option<String>,
    pub mention: Option<String>,
    pub since_id: Option<i64>,
    pub limit: Option<usize>,
}

fn validate_note(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.len() > MAX_NOTE_LEN * 4 || trimmed.chars().count() > MAX_NOTE_LEN {
        bail!("note exceeds maximum length of {MAX_NOTE_LEN} characters");
    }
    Ok(trimmed.to_string())
}

fn validate_channel(raw: Option<&str>) -> Result<String> {
    let ch = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("general");
    match ch {
        "general" | "handoff" | "blocker" | "review" => Ok(ch.to_string()),
        _ => bail!("invalid channel '{ch}' (expected general, handoff, blocker, or review)"),
    }
}

fn validate_task_status(raw: &str) -> Result<String> {
    let st = raw.trim();
    match st {
        "open" | "claimed" | "review" | "blocked" | "done" => Ok(st.to_string()),
        _ => bail!("invalid task status '{st}' (expected open, claimed, review, blocked, or done)"),
    }
}

fn validate_priority(raw: Option<&str>) -> Result<String> {
    let pr = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("medium");
    match pr {
        "low" | "medium" | "high" | "critical" => Ok(pr.to_string()),
        _ => bail!("invalid priority '{pr}' (expected low, medium, high, or critical)"),
    }
}

fn validate_agent_status(raw: Option<&str>) -> Result<String> {
    let st = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("active");
    match st {
        "active" | "idle" | "blocked" | "done" => Ok(st.to_string()),
        _ => bail!("invalid agent status '{st}' (expected active, idle, blocked, or done)"),
    }
}

fn validate_pr_url(raw: Option<&str>) -> Result<Option<String>> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if s.len() > MAX_NOTE_LEN {
        bail!("pr_url exceeds maximum length of {MAX_NOTE_LEN} characters");
    }
    if !(s.starts_with("https://") || s.starts_with("http://")) {
        bail!("pr_url must start with https:// or http://");
    }
    Ok(Some(s.to_string()))
}

fn clamp_ttl_minutes(ttl: Option<u32>, default_mins: u32) -> i64 {
    let mins = ttl.unwrap_or(default_mins).clamp(1, MAX_LOCK_TTL_MINUTES);
    i64::from(mins) * 60
}

fn mint_lock_id(project: &str, agent: &str, paths: &[String], now_nanos: i128) -> String {
    static LOCK_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let seq = LOCK_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id();
    let seed = format!(
        "{project}:{agent}:{}:{now_nanos}:{pid}:{seq}",
        paths.join(",")
    );
    let hex = super::sha256_hex(seed.as_bytes());
    format!("lck_{}", &hex[..12])
}

/// Write-path pruning and bounded table caps inside a single transaction.
fn prune_board_tx(tx: &Transaction<'_>, project: &str, now_epoch: i64) -> Result<()> {
    // 1. Expired locks across all projects
    tx.execute(
        "DELETE FROM board_locks WHERE expires_epoch <= ?1",
        params![now_epoch],
    )?;
    // 2. Check distinct project cap if `project` is brand new
    let known_in_board: bool = tx.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM board_agents WHERE project = ?1
            UNION ALL SELECT 1 FROM board_locks WHERE project = ?1
            UNION ALL SELECT 1 FROM board_tasks WHERE project = ?1
            UNION ALL SELECT 1 FROM board_messages WHERE project = ?1
            UNION ALL SELECT 1 FROM sessions WHERE project = ?1
        )",
        params![project],
        |r| r.get(0),
    )?;
    if !known_in_board {
        let distinct_projects: i64 = tx.query_row(
            "SELECT COUNT(*) FROM (
                SELECT project FROM board_agents
                UNION SELECT project FROM board_locks
                UNION SELECT project FROM board_tasks
                UNION SELECT project FROM board_messages
            )",
            [],
            |r| r.get(0),
        )?;
        if distinct_projects >= MAX_BOARD_PROJECTS {
            bail!("maximum number of board projects ({MAX_BOARD_PROJECTS}) reached");
        }
    }

    // 3. Prune stale inactive agents (>7 days old with no active locks or active tasks)
    let stale_cutoff = now_epoch - 7 * 86_400;
    tx.execute(
        "DELETE FROM board_agents
         WHERE project = ?1
           AND last_seen_epoch < ?2
           AND NOT EXISTS (
               SELECT 1 FROM board_locks l
               WHERE l.project = board_agents.project
                 AND l.agent = board_agents.agent
                 AND l.expires_epoch > ?3
           )
           AND NOT EXISTS (
               SELECT 1 FROM board_tasks t
               WHERE t.project = board_agents.project
                 AND t.agent = board_agents.agent
                 AND t.status IN ('claimed', 'review', 'blocked')
           )",
        params![project, stale_cutoff, now_epoch],
    )?;

    // 4. Prune old done tasks (>7 days) when task count in project exceeds 400
    let task_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM board_tasks WHERE project = ?1",
        params![project],
        |r| r.get(0),
    )?;
    if task_count > 400 {
        tx.execute(
            "DELETE FROM board_tasks
             WHERE project = ?1 AND status = 'done' AND updated_epoch < ?2",
            params![project, stale_cutoff],
        )?;
    }

    // 5. Retain only the newest MAX_BOARD_MESSAGES (500) messages per project
    tx.execute(
        "DELETE FROM board_messages
         WHERE project = ?1 AND id <= COALESCE(
             (SELECT id FROM board_messages WHERE project = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2),
             0
         )",
        params![project, MAX_BOARD_MESSAGES],
    )?;

    Ok(())
}

/// Verify that `(project, agent)` belongs to `caller.machine` (or is new and within caps) and refresh presence.
fn ensure_and_touch_agent_tx(
    tx: &Transaction<'_>,
    caller: &BoardCaller,
    project: &str,
    agent: &str,
    now_utc: &str,
    now_epoch: i64,
) -> Result<()> {
    let existing_machine: Option<String> = tx
        .query_row(
            "SELECT machine FROM board_agents WHERE project = ?1 AND agent = ?2",
            params![project, agent],
            |r| r.get(0),
        )
        .optional()?;

    match existing_machine {
        Some(owner) => {
            if owner != caller.machine && !caller.admin {
                bail!(
                    "agent handle '{agent}' in project '{project}' is owned by machine '{owner}'"
                );
            }
            tx.execute(
                "UPDATE board_agents
                 SET last_seen_utc = ?1, last_seen_epoch = ?2
                 WHERE project = ?3 AND agent = ?4",
                params![now_utc, now_epoch, project, agent],
            )?;
        }
        None => {
            let machine_agents: i64 = tx.query_row(
                "SELECT COUNT(*) FROM board_agents WHERE project = ?1 AND machine = ?2",
                params![project, caller.machine],
                |r| r.get(0),
            )?;
            if machine_agents >= MAX_AGENTS_PER_MACHINE_PROJECT {
                bail!(
                    "agent handle cap ({MAX_AGENTS_PER_MACHINE_PROJECT}) reached for machine '{}' in project '{project}'",
                    caller.machine
                );
            }
            let project_agents: i64 = tx.query_row(
                "SELECT COUNT(*) FROM board_agents WHERE project = ?1",
                params![project],
                |r| r.get(0),
            )?;
            if project_agents >= MAX_AGENTS_PER_PROJECT {
                bail!("agent handle cap ({MAX_AGENTS_PER_PROJECT}) reached in project '{project}'");
            }
            tx.execute(
                "INSERT INTO board_agents (
                    project, agent, machine, runtime, cwd, branch, status, current_task, last_seen_utc, last_seen_epoch
                ) VALUES (?1, ?2, ?3, 'cli', '', '', 'active', NULL, ?4, ?5)",
                params![project, agent, caller.machine, now_utc, now_epoch],
            )?;
        }
    }
    Ok(())
}

/// Enforce per-machine (`50`) and per-project (`200`) active lock caps before inserting a new lock,
/// evicting oldest `auto = 1` locks first so auto-locks can never starve manual claims.
fn enforce_lock_capacity_before_insert_tx(
    tx: &Transaction<'_>,
    project: &str,
    machine: &str,
    now_epoch: i64,
) -> Result<()> {
    let machine_locks: i64 = tx.query_row(
        "SELECT COUNT(*) FROM board_locks WHERE project = ?1 AND machine = ?2 AND expires_epoch > ?3",
        params![project, machine, now_epoch],
        |r| r.get(0),
    )?;
    if machine_locks >= MAX_LOCKS_PER_MACHINE_PROJECT {
        let evicted = tx.execute(
            "DELETE FROM board_locks
             WHERE id = (
                 SELECT id FROM board_locks
                 WHERE project = ?1 AND machine = ?2 AND auto = 1
                 ORDER BY expires_epoch ASC, id ASC LIMIT 1
             )",
            params![project, machine],
        )?;
        if evicted == 0 {
            bail!(
                "manual lock limit ({MAX_LOCKS_PER_MACHINE_PROJECT}) reached for machine '{machine}' in project '{project}'"
            );
        }
    }

    let project_locks: i64 = tx.query_row(
        "SELECT COUNT(*) FROM board_locks WHERE project = ?1 AND expires_epoch > ?2",
        params![project, now_epoch],
        |r| r.get(0),
    )?;
    if project_locks >= MAX_LOCKS_PER_PROJECT {
        let evicted = tx.execute(
            "DELETE FROM board_locks
             WHERE id = (
                 SELECT id FROM board_locks
                 WHERE project = ?1 AND auto = 1
                 ORDER BY expires_epoch ASC, id ASC LIMIT 1
             )",
            params![project],
        )?;
        if evicted == 0 {
            bail!("manual lock limit ({MAX_LOCKS_PER_PROJECT}) reached in project '{project}'");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn post_message_in_tx(
    tx: &Transaction<'_>,
    project: &str,
    channel: &str,
    agent: &str,
    machine: &str,
    body: &str,
    parent_id: Option<i64>,
    now_utc: &str,
    now_epoch: i64,
) -> Result<BoardMessageRow> {
    if let Some(pid) = parent_id {
        let parent_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM board_messages WHERE id = ?1 AND project = ?2)",
            params![pid, project],
            |r| r.get(0),
        )?;
        if !parent_exists {
            bail!("parent message #{pid} does not exist in project '{project}'");
        }
    }
    let mentions = extract_mentions(body);
    let mentions_json = serde_json::to_string(&mentions)?;
    tx.execute(
        "INSERT INTO board_messages (
            project, channel, agent, machine, body, mentions_json, parent_id, created_utc, created_epoch
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            project,
            channel,
            agent,
            machine,
            body,
            mentions_json,
            parent_id,
            now_utc,
            now_epoch
        ],
    )?;
    let id = tx.last_insert_rowid();
    Ok(BoardMessageRow {
        id,
        project: project.to_string(),
        channel: channel.to_string(),
        agent: agent.to_string(),
        machine: machine.to_string(),
        body: body.to_string(),
        mentions,
        parent_id,
        created_utc: now_utc.to_string(),
        created_epoch: now_epoch,
    })
}

fn query_active_locks_conn(
    conn: &rusqlite::Connection,
    project: &str,
    now_epoch: i64,
) -> Result<Vec<BoardLockRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, project, agent, machine, paths_json, note, auto, claimed_utc, expires_utc, expires_epoch
         FROM board_locks
         WHERE project = ?1 AND expires_epoch > ?2
         ORDER BY auto ASC, expires_epoch DESC, id ASC",
    )?;
    let rows = stmt
        .query_map(params![project, now_epoch], |r| {
            let paths_s: String = r.get(4)?;
            let paths: Vec<String> = serde_json::from_str(&paths_s).unwrap_or_default();
            let auto_i: i64 = r.get(6)?;
            Ok(BoardLockRow {
                id: r.get(0)?,
                project: r.get(1)?,
                agent: r.get(2)?,
                machine: r.get(3)?,
                paths,
                note: r.get(5)?,
                auto: auto_i != 0,
                claimed_utc: r.get(7)?,
                expires_utc: r.get(8)?,
                expires_epoch: r.get(9)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

impl Ledger {
    /// List distinct projects present across `board_*` tables (plus `sessions`), sorted alphabetically.
    pub fn board_projects(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT project FROM (
                SELECT project FROM board_agents
                UNION SELECT project FROM board_locks
                UNION SELECT project FROM board_tasks
                UNION SELECT project FROM board_messages
                UNION SELECT project FROM sessions WHERE project != '' AND project != '-' AND project != 'scratch'
            ) WHERE project != '' AND LOWER(project) NOT IN ('agent', 'all') ORDER BY project ASC",
        )?;
        let projects = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(projects)
    }

    /// Pure read-only status snapshot (no SQLite writes).
    /// Passing `Some("all")` or `Some("*")` for `project` returns active agents, locks, tasks, and messages across all projects.
    pub fn board_status(
        &self,
        project: Option<&str>,
        agent: Option<&str>,
        since_id: Option<i64>,
    ) -> Result<BoardStatusSnapshot> {
        let validated_agent = match agent.map(str::trim).filter(|s| !s.is_empty()) {
            Some(ag) => Some(validate_agent_handle(ag)?),
            None => None,
        };
        let projects = self.board_projects()?;
        let is_all_projects = matches!(
            project.map(str::trim),
            Some(p) if p.eq_ignore_ascii_case("all") || p == "*"
        );
        let target_project = if is_all_projects {
            Some("all".to_string())
        } else {
            match project.map(str::trim).filter(|s| !s.is_empty()) {
                Some(p) => Some(validate_identifier(p, "project")?),
                None => projects.first().cloned(),
            }
        };

        let Some(proj) = target_project else {
            return Ok(BoardStatusSnapshot {
                projects,
                project: None,
                agents: Vec::new(),
                locks: Vec::new(),
                tasks: Vec::new(),
                messages: Vec::new(),
                unread_mentions: Vec::new(),
                next_since_id: since_id.unwrap_or(0),
            });
        };
        let now_epoch = chrono::Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();

        let map_agent = |r: &rusqlite::Row<'_>| -> rusqlite::Result<BoardAgentRow> {
            Ok(BoardAgentRow {
                project: r.get(0)?,
                agent: r.get(1)?,
                machine: r.get(2)?,
                runtime: r.get(3)?,
                cwd: r.get(4)?,
                branch: r.get(5)?,
                status: r.get(6)?,
                current_task: r.get(7)?,
                last_seen_utc: r.get(8)?,
                last_seen_epoch: r.get(9)?,
            })
        };
        let agents = if is_all_projects {
            let mut stmt = conn.prepare(
                "SELECT project, agent, machine, runtime, cwd, branch, status, current_task, last_seen_utc, last_seen_epoch
                 FROM board_agents
                 WHERE LOWER(project) NOT IN ('agent', 'all')
                 ORDER BY last_seen_epoch DESC, project ASC, agent ASC",
            )?;
            let rows = stmt
                .query_map([], map_agent)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        } else {
            let mut stmt = conn.prepare(
                "SELECT project, agent, machine, runtime, cwd, branch, status, current_task, last_seen_utc, last_seen_epoch
                 FROM board_agents
                 WHERE project = ?1
                 ORDER BY last_seen_epoch DESC, agent ASC",
            )?;
            let rows = stmt
                .query_map(params![proj], map_agent)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };

        let locks = if !is_all_projects {
            query_active_locks_conn(&conn, &proj, now_epoch)?
        } else {
            let mut l_stmt = conn.prepare(
                "SELECT id, project, agent, machine, paths_json, note, auto, claimed_utc, expires_utc, expires_epoch
                 FROM board_locks
                 WHERE expires_epoch > ?1 AND LOWER(project) NOT IN ('agent', 'all')
                 ORDER BY auto ASC, expires_epoch DESC, id ASC",
            )?;
            let rows = l_stmt
                .query_map(params![now_epoch], |r| {
                    let paths_s: String = r.get(4)?;
                    let paths: Vec<String> = serde_json::from_str(&paths_s).unwrap_or_default();
                    let auto_i: i64 = r.get(6)?;
                    Ok(BoardLockRow {
                        id: r.get(0)?,
                        project: r.get(1)?,
                        agent: r.get(2)?,
                        machine: r.get(3)?,
                        paths,
                        note: r.get(5)?,
                        auto: auto_i != 0,
                        claimed_utc: r.get(7)?,
                        expires_utc: r.get(8)?,
                        expires_epoch: r.get(9)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };

        let map_task = |r: &rusqlite::Row<'_>| -> rusqlite::Result<BoardTaskRow> {
            Ok(BoardTaskRow {
                project: r.get(0)?,
                task_id: r.get(1)?,
                title: r.get(2)?,
                status: r.get(3)?,
                priority: r.get(4)?,
                agent: r.get(5)?,
                machine: r.get(6)?,
                branch: r.get(7)?,
                note: r.get(8)?,
                pr_url: r.get(9)?,
                created_utc: r.get(10)?,
                updated_utc: r.get(11)?,
                updated_epoch: r.get(12)?,
            })
        };
        let tasks = if is_all_projects {
            let mut stmt = conn.prepare(
                "SELECT project, task_id, title, status, priority, agent, machine, branch, note, pr_url,
                        created_utc, updated_utc, updated_epoch
                 FROM board_tasks
                 WHERE LOWER(project) NOT IN ('agent', 'all')
                 ORDER BY CASE status
                     WHEN 'blocked' THEN 0
                     WHEN 'claimed' THEN 1
                     WHEN 'review' THEN 2
                     WHEN 'open' THEN 3
                     ELSE 4
                 END, updated_epoch DESC, task_id ASC",
            )?;
            let rows = stmt
                .query_map([], map_task)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        } else {
            let mut stmt = conn.prepare(
                "SELECT project, task_id, title, status, priority, agent, machine, branch, note, pr_url,
                        created_utc, updated_utc, updated_epoch
                 FROM board_tasks
                 WHERE project = ?1
                 ORDER BY CASE status
                     WHEN 'blocked' THEN 0
                     WHEN 'claimed' THEN 1
                     WHEN 'review' THEN 2
                     WHEN 'open' THEN 3
                     ELSE 4
                 END, updated_epoch DESC, task_id ASC",
            )?;
            let rows = stmt
                .query_map(params![proj], map_task)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };

        let floor_id = since_id.unwrap_or(0);
        let map_msg = |r: &rusqlite::Row<'_>| -> rusqlite::Result<BoardMessageRow> {
            let mentions_s: String = r.get(6)?;
            let mentions: Vec<String> = serde_json::from_str(&mentions_s).unwrap_or_default();
            Ok(BoardMessageRow {
                id: r.get(0)?,
                project: r.get(1)?,
                channel: r.get(2)?,
                agent: r.get(3)?,
                machine: r.get(4)?,
                body: r.get(5)?,
                mentions,
                parent_id: r.get(7)?,
                created_utc: r.get(8)?,
                created_epoch: r.get(9)?,
            })
        };
        let mut messages = if is_all_projects {
            let sql = if floor_id > 0 {
                "SELECT id, project, channel, agent, machine, body, mentions_json, parent_id, created_utc, created_epoch
                 FROM board_messages
                 WHERE LOWER(project) NOT IN ('agent', 'all') AND id > ?1
                 ORDER BY id ASC
                 LIMIT 50"
            } else {
                "SELECT id, project, channel, agent, machine, body, mentions_json, parent_id, created_utc, created_epoch
                 FROM board_messages
                 WHERE LOWER(project) NOT IN ('agent', 'all') AND id > ?1
                 ORDER BY id DESC
                 LIMIT 50"
            };
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt
                .query_map(params![floor_id], map_msg)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        } else {
            let sql = if floor_id > 0 {
                "SELECT id, project, channel, agent, machine, body, mentions_json, parent_id, created_utc, created_epoch
                 FROM board_messages
                 WHERE project = ?1 AND id > ?2
                 ORDER BY id ASC
                 LIMIT 50"
            } else {
                "SELECT id, project, channel, agent, machine, body, mentions_json, parent_id, created_utc, created_epoch
                 FROM board_messages
                 WHERE project = ?1 AND id > ?2
                 ORDER BY id DESC
                 LIMIT 50"
            };
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt
                .query_map(params![proj, floor_id], map_msg)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        if floor_id == 0 {
            messages.reverse();
        }

        let max_id: i64 = if is_all_projects {
            conn.query_row(
                "SELECT COALESCE(MAX(id), 0) FROM board_messages WHERE LOWER(project) NOT IN ('agent', 'all')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0)
        } else {
            conn.query_row(
                "SELECT COALESCE(MAX(id), 0) FROM board_messages WHERE project = ?1",
                params![proj],
                |r| r.get(0),
            )
            .unwrap_or(0)
        };
        let next_since_id = if floor_id > 0 && messages.len() == 50 {
            messages.last().map(|m| m.id).unwrap_or(floor_id)
        } else {
            floor_id.max(max_id)
        };

        let unread_mentions = if let Some(needle) = validated_agent {
            let like_pat = format!("%\"{needle}\"%");
            let rows: Vec<BoardMessageRow> = if is_all_projects {
                let mut stmt = conn.prepare(
                    "SELECT id, project, channel, agent, machine, body, mentions_json, parent_id, created_utc, created_epoch
                     FROM board_messages
                     WHERE LOWER(project) NOT IN ('agent', 'all') AND id > ?1 AND agent != ?2 AND mentions_json LIKE ?3
                     ORDER BY id ASC",
                )?;
                let collected = stmt
                    .query_map(params![floor_id, needle, like_pat], map_msg)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                collected
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id, project, channel, agent, machine, body, mentions_json, parent_id, created_utc, created_epoch
                     FROM board_messages
                     WHERE project = ?1 AND id > ?2 AND agent != ?3 AND mentions_json LIKE ?4
                     ORDER BY id ASC",
                )?;
                let collected = stmt
                    .query_map(params![proj, floor_id, needle, like_pat], map_msg)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                collected
            };
            rows.into_iter()
                .filter(|m| m.mentions.iter().any(|x| x == &needle))
                .collect()
        } else {
            Vec::new()
        };

        Ok(BoardStatusSnapshot {
            projects,
            project: Some(proj),
            agents,
            locks,
            tasks,
            messages,
            unread_mentions,
            next_since_id,
        })
    }

    /// Pure read-only check of whether `path` matches any active lock in `project` held by another agent.
    pub fn board_check_path(
        &self,
        project: &str,
        path: &str,
        agent: Option<&str>,
    ) -> Result<BoardCheckOutcome> {
        let proj = validate_identifier(project, "project")?;
        let norm_path = normalize_repo_path(path, None)?;
        let caller_agent = match agent.map(str::trim).filter(|s| !s.is_empty()) {
            Some(a) => Some(validate_agent_handle(a)?),
            None => None,
        };
        let now_epoch = chrono::Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();
        let active_locks = query_active_locks_conn(&conn, &proj, now_epoch)?;

        let blocking_locks: Vec<BoardLockRow> = active_locks
            .into_iter()
            .filter(|l| {
                if let Some(ref ag) = caller_agent {
                    if &l.agent == ag {
                        return false;
                    }
                }
                l.paths.iter().any(|g| glob_matches_path(g, &norm_path))
            })
            .collect();

        Ok(BoardCheckOutcome {
            blocked: !blocking_locks.is_empty(),
            blocking_locks,
        })
    }

    /// Pure read-only message query with optional channel, mention, and `since_id` filters.
    pub fn board_list_messages(&self, q: &BoardMessagesQuery) -> Result<BoardMessagesPage> {
        let proj = validate_identifier(&q.project, "project")?;
        let ch = match q
            .channel
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(c) => Some(validate_channel(Some(c))?),
            None => None,
        };
        let mention = match q
            .mention
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(m) => Some(validate_agent_handle(m)?),
            None => None,
        };
        let floor_id = q.since_id.unwrap_or(0);
        let limit = q.limit.unwrap_or(100).clamp(1, 200);

        let conn = self.conn.lock().unwrap();
        let mention_like = mention.as_ref().map(|m| format!("%\"{m}\"%"));
        let sql = if floor_id > 0 {
            "SELECT id, project, channel, agent, machine, body, mentions_json, parent_id, created_utc, created_epoch
             FROM board_messages
             WHERE project = ?1
               AND id > ?2
               AND (?3 IS NULL OR channel = ?3)
               AND (?4 IS NULL OR mentions_json LIKE ?4)
             ORDER BY id ASC
             LIMIT ?5"
        } else {
            "SELECT id, project, channel, agent, machine, body, mentions_json, parent_id, created_utc, created_epoch
             FROM board_messages
             WHERE project = ?1
               AND id > ?2
               AND (?3 IS NULL OR channel = ?3)
               AND (?4 IS NULL OR mentions_json LIKE ?4)
             ORDER BY id DESC
             LIMIT ?5"
        };
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt
            .query_map(
                params![proj, floor_id, ch, mention_like, limit as i64],
                |r| {
                    let mentions_s: String = r.get(6)?;
                    let mentions: Vec<String> =
                        serde_json::from_str(&mentions_s).unwrap_or_default();
                    Ok(BoardMessageRow {
                        id: r.get(0)?,
                        project: r.get(1)?,
                        channel: r.get(2)?,
                        agent: r.get(3)?,
                        machine: r.get(4)?,
                        body: r.get(5)?,
                        mentions,
                        parent_id: r.get(7)?,
                        created_utc: r.get(8)?,
                        created_epoch: r.get(9)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if floor_id == 0 {
            rows.reverse();
        }
        let hit_limit = rows.len() == limit;
        let last_row_id = rows.last().map(|m| m.id);

        let filtered: Vec<BoardMessageRow> = if let Some(ref m) = mention {
            rows.into_iter()
                .filter(|msg| msg.mentions.iter().any(|x| x == m))
                .collect()
        } else {
            rows
        };

        let max_id: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(id), 0) FROM board_messages WHERE project = ?1",
                params![proj],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let next_since_id = if floor_id > 0 && hit_limit {
            last_row_id.unwrap_or(floor_id)
        } else {
            floor_id.max(max_id)
        };

        Ok(BoardMessagesPage {
            messages: filtered,
            next_since_id,
        })
    }

    /// Upsert agent presence row in `board_agents`.
    pub fn board_heartbeat(
        &self,
        caller: &BoardCaller,
        req: &BoardHeartbeatReq,
    ) -> Result<BoardAgentRow> {
        let proj = validate_identifier(&req.project, "project")?;
        let agent = validate_agent_handle(&req.agent)?;
        let runtime = if req.runtime.trim().is_empty() {
            "cli".to_string()
        } else {
            validate_identifier(&req.runtime, "runtime")?
        };
        let cwd = validate_note(&req.cwd)?;
        let branch = validate_note(&req.branch)?;
        let status = match req
            .status
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(s) => Some(validate_agent_status(Some(s))?),
            None => None,
        };
        let current_task = match req
            .current_task
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(t) => Some(validate_note(t)?),
            None => None,
        };

        let now = chrono::Utc::now();
        let now_utc = now.to_rfc3339();
        let now_epoch = now.timestamp();

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        prune_board_tx(&tx, &proj, now_epoch)?;
        ensure_and_touch_agent_tx(&tx, caller, &proj, &agent, &now_utc, now_epoch)?;

        tx.execute(
            "UPDATE board_agents
             SET runtime = CASE WHEN ?1 = 'cli' AND runtime != '' THEN runtime ELSE ?1 END,
                 cwd = CASE WHEN ?2 = '' THEN cwd ELSE ?2 END,
                 branch = CASE WHEN ?3 IN ('', 'unknown') AND branch != '' THEN branch ELSE ?3 END,
                 status = CASE WHEN ?4 IS NOT NULL THEN ?4 WHEN status = 'blocked' THEN 'blocked' ELSE 'active' END,
                 current_task = CASE
                     WHEN current_task IS NOT NULL AND EXISTS (
                         SELECT 1 FROM board_tasks t
                         WHERE t.project = board_agents.project
                           AND t.agent = board_agents.agent
                           AND t.task_id = board_agents.current_task
                           AND t.status IN ('claimed', 'review', 'blocked')
                     ) AND (?5 IS NULL OR NOT EXISTS (
                         SELECT 1 FROM board_tasks t2
                         WHERE t2.project = board_agents.project
                           AND t2.agent = board_agents.agent
                           AND t2.task_id = ?5
                           AND t2.status IN ('claimed', 'review', 'blocked')
                     )) THEN current_task
                     ELSE COALESCE(?5, current_task)
                 END,
                 last_seen_utc = ?6,
                 last_seen_epoch = ?7
             WHERE project = ?8 AND agent = ?9",
            params![
                runtime,
                cwd,
                branch,
                status,
                current_task,
                now_utc,
                now_epoch,
                proj,
                agent
            ],
        )?;

        let row = tx.query_row(
            "SELECT project, agent, machine, runtime, cwd, branch, status, current_task, last_seen_utc, last_seen_epoch
             FROM board_agents WHERE project = ?1 AND agent = ?2",
            params![proj, agent],
            |r| {
                Ok(BoardAgentRow {
                    project: r.get(0)?,
                    agent: r.get(1)?,
                    machine: r.get(2)?,
                    runtime: r.get(3)?,
                    cwd: r.get(4)?,
                    branch: r.get(5)?,
                    status: r.get(6)?,
                    current_task: r.get(7)?,
                    last_seen_utc: r.get(8)?,
                    last_seen_epoch: r.get(9)?,
                })
            },
        )?;
        tx.commit()?;
        Ok(row)
    }

    /// Claim explicit file globs in `board_locks` and return any overlapping locks held by other agents.
    pub fn board_claim_locks(
        &self,
        caller: &BoardCaller,
        req: &BoardClaimLocksReq,
    ) -> Result<BoardClaimLocksOutcome> {
        let proj = validate_identifier(&req.project, "project")?;
        let agent = validate_agent_handle(&req.agent)?;
        if req.paths.is_empty() {
            bail!("at least one path or glob pattern is required");
        }
        if req.paths.len() > 32 {
            bail!("at most 32 path patterns may be claimed in one lock");
        }
        let mut norm_paths = Vec::with_capacity(req.paths.len());
        for p in &req.paths {
            let np = validate_glob_pattern(p, None)?;
            if !norm_paths.contains(&np) {
                norm_paths.push(np);
            }
        }
        let note = validate_note(&req.note)?;
        let ttl_secs = clamp_ttl_minutes(req.ttl_minutes, DEFAULT_LOCK_TTL_MINUTES);

        let now = chrono::Utc::now();
        let now_utc = now.to_rfc3339();
        let now_epoch = now.timestamp();
        let exp_epoch = now_epoch + ttl_secs;
        let exp_utc = chrono::DateTime::<chrono::Utc>::from_timestamp(exp_epoch, 0)
            .unwrap_or(now)
            .to_rfc3339();

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        prune_board_tx(&tx, &proj, now_epoch)?;
        ensure_and_touch_agent_tx(&tx, caller, &proj, &agent, &now_utc, now_epoch)?;

        let existing = query_active_locks_conn(&tx, &proj, now_epoch)?;
        let conflicts: Vec<BoardLockRow> = existing
            .iter()
            .filter(|l| {
                l.agent != agent
                    && l.paths
                        .iter()
                        .any(|ep| norm_paths.iter().any(|np| globs_overlap(ep, np)))
            })
            .cloned()
            .collect();

        let paths_json = serde_json::to_string(&norm_paths)?;

        // Refresh existing identical manual lock by the same agent if present
        let existing_same: Option<String> = tx
            .query_row(
                "SELECT id FROM board_locks
                 WHERE project = ?1 AND agent = ?2 AND auto = 0 AND paths_json = ?3",
                params![proj, agent, paths_json],
                |r| r.get(0),
            )
            .optional()?;

        let lock_id = if let Some(id) = existing_same {
            tx.execute(
                "UPDATE board_locks
                 SET note = CASE WHEN ?1 = '' THEN note ELSE ?1 END,
                     claimed_utc = ?2,
                     expires_utc = ?3,
                     expires_epoch = ?4
                 WHERE id = ?5",
                params![note, now_utc, exp_utc, exp_epoch, id],
            )?;
            id
        } else {
            enforce_lock_capacity_before_insert_tx(&tx, &proj, &caller.machine, now_epoch)?;
            let id = mint_lock_id(
                &proj,
                &agent,
                &norm_paths,
                now.timestamp_nanos_opt().unwrap_or(0) as i128,
            );
            tx.execute(
                "INSERT INTO board_locks (
                    id, project, agent, machine, paths_json, note, auto, claimed_utc, expires_utc, expires_epoch
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?9)",
                params![
                    id,
                    proj,
                    agent,
                    caller.machine,
                    paths_json,
                    note,
                    now_utc,
                    exp_utc,
                    exp_epoch
                ],
            )?;
            id
        };

        tx.commit()?;
        Ok(BoardClaimLocksOutcome {
            lock: BoardLockRow {
                id: lock_id,
                project: proj,
                agent,
                machine: caller.machine.clone(),
                paths: norm_paths,
                note,
                auto: false,
                claimed_utc: now_utc,
                expires_utc: exp_utc,
                expires_epoch: exp_epoch,
            },
            conflicts,
        })
    }

    /// Create or refresh an `auto = 1` single-path lock (used by `PostToolUse` edit hooks),
    /// returning the lock and any unread `@mentions` for `agent`.
    pub fn board_touch_lock(
        &self,
        caller: &BoardCaller,
        req: &BoardTouchLockReq,
    ) -> Result<BoardTouchOutcome> {
        let proj = validate_identifier(&req.project, "project")?;
        let agent = validate_agent_handle(&req.agent)?;
        let norm_path = normalize_repo_path(&req.path, None)?;
        let paths = vec![norm_path.clone()];
        let paths_json = serde_json::to_string(&paths)?;
        let ttl_secs = clamp_ttl_minutes(req.ttl_minutes, DEFAULT_AUTO_LOCK_TTL_MINUTES);

        let now = chrono::Utc::now();
        let now_utc = now.to_rfc3339();
        let now_epoch = now.timestamp();
        let exp_epoch = now_epoch + ttl_secs;
        let exp_utc = chrono::DateTime::<chrono::Utc>::from_timestamp(exp_epoch, 0)
            .unwrap_or(now)
            .to_rfc3339();

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        ensure_and_touch_agent_tx(&tx, caller, &proj, &agent, &now_utc, now_epoch)?;

        // If already covered by an existing lock from this agent, refresh without running full prune
        let existing_id: Option<(String, i64, String)> = tx
            .query_row(
                "SELECT id, auto, note FROM board_locks
                 WHERE project = ?1 AND agent = ?2 AND paths_json = ?3 AND expires_epoch > ?4",
                params![proj, agent, paths_json, now_epoch],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if existing_id.is_none() {
            prune_board_tx(&tx, &proj, now_epoch)?;
        }
        let lock = if let Some((id, auto_i, note)) = existing_id {
            tx.execute(
                "UPDATE board_locks
                 SET expires_utc = ?1, expires_epoch = ?2
                 WHERE id = ?3",
                params![exp_utc, exp_epoch, id],
            )?;
            BoardLockRow {
                id,
                project: proj.clone(),
                agent: agent.clone(),
                machine: caller.machine.clone(),
                paths,
                note,
                auto: auto_i != 0,
                claimed_utc: now_utc.clone(),
                expires_utc: exp_utc,
                expires_epoch: exp_epoch,
            }
        } else {
            // Enforce per-agent auto-lock cap (20) by evicting oldest auto lock of this agent
            let agent_auto_count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM board_locks
                 WHERE project = ?1 AND agent = ?2 AND auto = 1 AND expires_epoch > ?3",
                params![proj, agent, now_epoch],
                |r| r.get(0),
            )?;
            if agent_auto_count >= MAX_AUTO_LOCKS_PER_AGENT {
                tx.execute(
                    "DELETE FROM board_locks
                     WHERE id = (
                         SELECT id FROM board_locks
                         WHERE project = ?1 AND agent = ?2 AND auto = 1
                         ORDER BY expires_epoch ASC, id ASC LIMIT 1
                     )",
                    params![proj, agent],
                )?;
            }
            enforce_lock_capacity_before_insert_tx(&tx, &proj, &caller.machine, now_epoch)?;
            let id = mint_lock_id(
                &proj,
                &agent,
                &paths,
                now.timestamp_nanos_opt().unwrap_or(0) as i128,
            );
            let note = "auto-lock on file edit".to_string();
            tx.execute(
                "INSERT INTO board_locks (
                    id, project, agent, machine, paths_json, note, auto, claimed_utc, expires_utc, expires_epoch
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9)",
                params![
                    id,
                    proj,
                    agent,
                    caller.machine,
                    paths_json,
                    note,
                    now_utc,
                    exp_utc,
                    exp_epoch
                ],
            )?;
            BoardLockRow {
                id,
                project: proj.clone(),
                agent: agent.clone(),
                machine: caller.machine.clone(),
                paths,
                note,
                auto: true,
                claimed_utc: now_utc,
                expires_utc: exp_utc,
                expires_epoch: exp_epoch,
            }
        };

        // Surface recent unread @mentions for `agent` so running worktree agents see messages on edit
        let floor_id = req.since_id.unwrap_or(0);
        let recent_cutoff = now_epoch - 900; // last 15 minutes when since_id is 0
        let like_pat = format!("%\"{agent}\"%");
        let mut msg_stmt = tx.prepare(
            "SELECT id, project, channel, agent, machine, body, mentions_json, parent_id, created_utc, created_epoch
             FROM board_messages
             WHERE project = ?1
               AND id > ?2
               AND (?2 > 0 OR created_epoch >= ?3)
               AND agent != ?4
               AND mentions_json LIKE ?5
             ORDER BY id ASC",
        )?;
        let unread_mentions = msg_stmt
            .query_map(
                params![proj, floor_id, recent_cutoff, agent, like_pat],
                |r| {
                    let mentions_s: String = r.get(6)?;
                    let mentions: Vec<String> =
                        serde_json::from_str(&mentions_s).unwrap_or_default();
                    Ok(BoardMessageRow {
                        id: r.get(0)?,
                        project: r.get(1)?,
                        channel: r.get(2)?,
                        agent: r.get(3)?,
                        machine: r.get(4)?,
                        body: r.get(5)?,
                        mentions,
                        parent_id: r.get(7)?,
                        created_utc: r.get(8)?,
                        created_epoch: r.get(9)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|m| m.mentions.iter().any(|x| x == &agent))
            .collect();

        drop(msg_stmt);
        let max_id: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(id), 0) FROM board_messages WHERE project = ?1",
                params![proj],
                |r| r.get(0),
            )
            .unwrap_or(0);

        tx.commit()?;
        Ok(BoardTouchOutcome {
            lock,
            unread_mentions,
            next_since_id: floor_id.max(max_id),
        })
    }

    /// Release locks by `lock_id`, `auto_only`, or `all` with machine/agent ownership verification.
    pub fn board_release_locks(
        &self,
        caller: &BoardCaller,
        req: &BoardReleaseLocksReq,
    ) -> Result<usize> {
        let proj = validate_identifier(&req.project, "project")?;
        let agent = validate_agent_handle(&req.agent)?;
        let now_epoch = chrono::Utc::now().timestamp();

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        prune_board_tx(&tx, &proj, now_epoch)?;

        let released = if let Some(lid) = req
            .lock_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let lock_id = validate_identifier(lid, "lock_id")?;
            let existing: Option<(String, String)> = tx
                .query_row(
                    "SELECT agent, machine FROM board_locks WHERE id = ?1 AND project = ?2",
                    params![lock_id, proj],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((owner_agent, owner_machine)) = existing else {
                tx.commit()?;
                return Ok(0);
            };
            let allowed = caller.admin
                || (owner_agent == agent && owner_machine == caller.machine)
                || (req.force && owner_machine == caller.machine);
            if !allowed {
                bail!("lock '{lock_id}' is owned by '{owner_agent}' on machine '{owner_machine}'");
            }
            tx.execute(
                "DELETE FROM board_locks WHERE id = ?1 AND project = ?2",
                params![lock_id, proj],
            )?
        } else if req.auto_only {
            if caller.admin && req.force {
                tx.execute(
                    "DELETE FROM board_locks WHERE project = ?1 AND agent = ?2 AND auto = 1",
                    params![proj, agent],
                )?
            } else {
                tx.execute(
                    "DELETE FROM board_locks
                     WHERE project = ?1 AND agent = ?2 AND machine = ?3 AND auto = 1",
                    params![proj, agent, caller.machine],
                )?
            }
        } else if req.all {
            if caller.admin && req.force {
                tx.execute(
                    "DELETE FROM board_locks WHERE project = ?1 AND agent = ?2",
                    params![proj, agent],
                )?
            } else {
                tx.execute(
                    "DELETE FROM board_locks
                     WHERE project = ?1 AND agent = ?2 AND machine = ?3",
                    params![proj, agent, caller.machine],
                )?
            }
        } else {
            bail!("specify lock_id, all: true, or auto_only: true to release locks");
        };

        tx.commit()?;
        Ok(released)
    }

    /// Create or claim a task in `board_tasks`.
    pub fn board_claim_task(
        &self,
        caller: &BoardCaller,
        req: &BoardClaimTaskReq,
    ) -> Result<BoardClaimTaskOutcome> {
        let proj = validate_identifier(&req.project, "project")?;
        let task_id = validate_identifier(&req.task_id, "task_id")?;
        let agent = validate_agent_handle(&req.agent)?;
        let title_opt = req
            .title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(t) = title_opt {
            validate_note(t)?;
        }
        let priority = validate_priority(req.priority.as_deref())?;
        let branch = match req
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(b) => Some(validate_note(b)?),
            None => None,
        };
        let note = match req.note.as_deref() {
            Some(n) => validate_note(n)?,
            None => String::new(),
        };

        let now = chrono::Utc::now();
        let now_utc = now.to_rfc3339();
        let now_epoch = now.timestamp();

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        prune_board_tx(&tx, &proj, now_epoch)?;
        ensure_and_touch_agent_tx(&tx, caller, &proj, &agent, &now_utc, now_epoch)?;

        let existing: Option<BoardTaskRow> = tx
            .query_row(
                "SELECT project, task_id, title, status, priority, agent, machine, branch, note, pr_url,
                        created_utc, updated_utc, updated_epoch
                 FROM board_tasks WHERE project = ?1 AND task_id = ?2",
                params![proj, task_id],
                |r| {
                    Ok(BoardTaskRow {
                        project: r.get(0)?,
                        task_id: r.get(1)?,
                        title: r.get(2)?,
                        status: r.get(3)?,
                        priority: r.get(4)?,
                        agent: r.get(5)?,
                        machine: r.get(6)?,
                        branch: r.get(7)?,
                        note: r.get(8)?,
                        pr_url: r.get(9)?,
                        created_utc: r.get(10)?,
                        updated_utc: r.get(11)?,
                        updated_epoch: r.get(12)?,
                    })
                },
            )
            .optional()?;

        if let Some(ex) = existing {
            if matches!(ex.status.as_str(), "claimed" | "review" | "blocked") {
                if let Some(ref holder) = ex.agent {
                    if holder != &agent {
                        let same_machine = ex.machine.as_deref() == Some(caller.machine.as_str());
                        if !(req.force && (same_machine || caller.admin)) {
                            tx.commit()?;
                            return Ok(BoardClaimTaskOutcome {
                                ok: false,
                                held_by: Some(holder.clone()),
                                task: ex,
                            });
                        }
                    }
                }
            }
            let new_title = title_opt.unwrap_or(&ex.title).to_string();
            let new_priority = if req.priority.is_some() {
                priority
            } else {
                ex.priority.clone()
            };
            let new_note = if note.is_empty() {
                ex.note.clone()
            } else {
                note
            };
            let new_branch = branch.or(ex.branch.clone());

            tx.execute(
                "UPDATE board_tasks
                 SET title = ?1, status = 'claimed', priority = ?2, agent = ?3, machine = ?4,
                     branch = ?5, note = ?6, updated_utc = ?7, updated_epoch = ?8
                 WHERE project = ?9 AND task_id = ?10",
                params![
                    new_title,
                    new_priority,
                    agent,
                    caller.machine,
                    new_branch,
                    new_note,
                    now_utc,
                    now_epoch,
                    proj,
                    task_id
                ],
            )?;
        } else {
            let active_tasks: i64 = tx.query_row(
                "SELECT COUNT(*) FROM board_tasks WHERE project = ?1",
                params![proj],
                |r| r.get(0),
            )?;
            if active_tasks >= MAX_TASKS_PER_PROJECT {
                bail!("task limit ({MAX_TASKS_PER_PROJECT}) reached in project '{proj}'");
            }
            let title = title_opt.unwrap_or(&task_id).to_string();
            tx.execute(
                "INSERT INTO board_tasks (
                    project, task_id, title, status, priority, agent, machine, branch, note, pr_url,
                    created_utc, updated_utc, updated_epoch
                ) VALUES (?1, ?2, ?3, 'claimed', ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?9, ?10)",
                params![
                    proj,
                    task_id,
                    title,
                    priority,
                    agent,
                    caller.machine,
                    branch,
                    note,
                    now_utc,
                    now_epoch
                ],
            )?;
        }

        tx.execute(
            "UPDATE board_agents SET current_task = ?1 WHERE project = ?2 AND agent = ?3",
            params![task_id, proj, agent],
        )?;

        let task = tx.query_row(
            "SELECT project, task_id, title, status, priority, agent, machine, branch, note, pr_url,
                    created_utc, updated_utc, updated_epoch
             FROM board_tasks WHERE project = ?1 AND task_id = ?2",
            params![proj, task_id],
            |r| {
                Ok(BoardTaskRow {
                    project: r.get(0)?,
                    task_id: r.get(1)?,
                    title: r.get(2)?,
                    status: r.get(3)?,
                    priority: r.get(4)?,
                    agent: r.get(5)?,
                    machine: r.get(6)?,
                    branch: r.get(7)?,
                    note: r.get(8)?,
                    pr_url: r.get(9)?,
                    created_utc: r.get(10)?,
                    updated_utc: r.get(11)?,
                    updated_epoch: r.get(12)?,
                })
            },
        )?;

        tx.commit()?;
        Ok(BoardClaimTaskOutcome {
            ok: true,
            held_by: None,
            task,
        })
    }

    /// Update task status (`open`, `claimed`, `review`, `blocked`, `done`), optionally releasing all locks held by `agent`.
    pub fn board_update_task(
        &self,
        caller: &BoardCaller,
        req: &BoardUpdateTaskReq,
    ) -> Result<BoardTaskRow> {
        let proj = validate_identifier(&req.project, "project")?;
        let task_id = validate_identifier(&req.task_id, "task_id")?;
        let agent = validate_agent_handle(&req.agent)?;
        let status = validate_task_status(&req.status)?;
        let note_opt = match req.note.as_deref() {
            Some(n) => Some(validate_note(n)?),
            None => None,
        };
        let pr_url = validate_pr_url(req.pr_url.as_deref())?;

        let now = chrono::Utc::now();
        let now_utc = now.to_rfc3339();
        let now_epoch = now.timestamp();

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        prune_board_tx(&tx, &proj, now_epoch)?;
        ensure_and_touch_agent_tx(&tx, caller, &proj, &agent, &now_utc, now_epoch)?;

        let existing: Option<(Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT agent, machine FROM board_tasks WHERE project = ?1 AND task_id = ?2",
                params![proj, task_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;

        let Some((owner_agent, owner_machine)) = existing else {
            bail!("task '{task_id}' does not exist in project '{proj}'");
        };

        let allowed = caller.admin
            || owner_agent.is_none()
            || (owner_agent.as_deref() == Some(agent.as_str())
                && owner_machine.as_deref() == Some(caller.machine.as_str()))
            || (req.force && owner_machine.as_deref() == Some(caller.machine.as_str()));

        if !allowed {
            bail!(
                "task '{task_id}' is owned by '{}' on machine '{}'",
                owner_agent.unwrap_or_default(),
                owner_machine.unwrap_or_default()
            );
        }

        let new_agent = if status == "open" {
            None
        } else {
            Some(agent.clone())
        };
        let new_machine = if status == "open" {
            None
        } else {
            Some(caller.machine.clone())
        };

        tx.execute(
            "UPDATE board_tasks
             SET status = ?1,
                 agent = ?2,
                 machine = ?3,
                 note = COALESCE(?4, note),
                 pr_url = COALESCE(?5, pr_url),
                 updated_utc = ?6,
                 updated_epoch = ?7
             WHERE project = ?8 AND task_id = ?9",
            params![
                status,
                new_agent,
                new_machine,
                note_opt,
                pr_url,
                now_utc,
                now_epoch,
                proj,
                task_id
            ],
        )?;

        if status == "done" || status == "open" {
            tx.execute(
                "UPDATE board_agents
                 SET current_task = NULL
                 WHERE project = ?1 AND agent = ?2 AND current_task = ?3",
                params![proj, agent, task_id],
            )?;
        }

        if req.release_locks {
            tx.execute(
                "DELETE FROM board_locks WHERE project = ?1 AND agent = ?2 AND machine = ?3",
                params![proj, agent, caller.machine],
            )?;
        }

        let task = tx.query_row(
            "SELECT project, task_id, title, status, priority, agent, machine, branch, note, pr_url,
                    created_utc, updated_utc, updated_epoch
             FROM board_tasks WHERE project = ?1 AND task_id = ?2",
            params![proj, task_id],
            |r| {
                Ok(BoardTaskRow {
                    project: r.get(0)?,
                    task_id: r.get(1)?,
                    title: r.get(2)?,
                    status: r.get(3)?,
                    priority: r.get(4)?,
                    agent: r.get(5)?,
                    machine: r.get(6)?,
                    branch: r.get(7)?,
                    note: r.get(8)?,
                    pr_url: r.get(9)?,
                    created_utc: r.get(10)?,
                    updated_utc: r.get(11)?,
                    updated_epoch: r.get(12)?,
                })
            },
        )?;

        tx.commit()?;
        Ok(task)
    }

    /// Post a message to `board_messages` with `@mention` extraction.
    pub fn board_post_message(
        &self,
        caller: &BoardCaller,
        req: &BoardPostMessageReq,
    ) -> Result<BoardMessageRow> {
        let proj = validate_identifier(&req.project, "project")?;
        let channel = validate_channel(req.channel.as_deref())?;
        let agent = validate_agent_handle(&req.agent)?;
        let body = req.body.trim();
        if body.is_empty() {
            bail!("message body must not be empty");
        }
        if body.len() > MAX_BODY_LEN * 4 || body.chars().count() > MAX_BODY_LEN {
            bail!("message body exceeds maximum length of {MAX_BODY_LEN} characters");
        }

        let now = chrono::Utc::now();
        let now_utc = now.to_rfc3339();
        let now_epoch = now.timestamp();

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        prune_board_tx(&tx, &proj, now_epoch)?;
        ensure_and_touch_agent_tx(&tx, caller, &proj, &agent, &now_utc, now_epoch)?;

        let msg = post_message_in_tx(
            &tx,
            &proj,
            &channel,
            &agent,
            &caller.machine,
            body,
            req.parent_id,
            &now_utc,
            now_epoch,
        )?;
        tx.commit()?;
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_locks_detect_conflicts_and_check_path() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("board.db")).unwrap();
        let vm1 = BoardCaller {
            machine: "vm1".into(),
            admin: false,
        };
        let vm2 = BoardCaller {
            machine: "vm2".into(),
            admin: false,
        };

        let c1 = ledger
            .board_claim_locks(
                &vm1,
                &BoardClaimLocksReq {
                    project: "daybook".into(),
                    agent: "t3-api".into(),
                    paths: vec!["src/server/**".into()],
                    note: "API routes".into(),
                    ttl_minutes: Some(30),
                },
            )
            .unwrap();
        assert!(c1.conflicts.is_empty());

        // Overlapping claim from another agent reports conflict
        let c2 = ledger
            .board_claim_locks(
                &vm2,
                &BoardClaimLocksReq {
                    project: "daybook".into(),
                    agent: "claude-ui".into(),
                    paths: vec!["src/**/*.rs".into()],
                    note: "Wide refactor".into(),
                    ttl_minutes: Some(30),
                },
            )
            .unwrap();
        assert_eq!(c2.conflicts.len(), 1);
        assert_eq!(c2.conflicts[0].agent, "t3-api");

        // Check path blocks claude-ui on src/server/router.rs, but not t3-api if c2 is released
        let released = ledger
            .board_release_locks(
                &vm2,
                &BoardReleaseLocksReq {
                    project: "daybook".into(),
                    agent: "claude-ui".into(),
                    lock_id: Some(c2.lock.id),
                    all: false,
                    auto_only: false,
                    force: false,
                },
            )
            .unwrap();
        assert_eq!(released, 1);

        let chk_other = ledger
            .board_check_path("daybook", "src/server/router.rs", Some("claude-ui"))
            .unwrap();
        assert!(chk_other.blocked);
        assert_eq!(chk_other.blocking_locks[0].agent, "t3-api");

        let chk_self = ledger
            .board_check_path("daybook", "src/server/router.rs", Some("t3-api"))
            .unwrap();
        assert!(!chk_self.blocked);
    }

    #[test]
    fn cross_machine_impersonation_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("board.db")).unwrap();
        let vm1 = BoardCaller {
            machine: "vm1".into(),
            admin: false,
        };
        let vm2 = BoardCaller {
            machine: "vm2".into(),
            admin: false,
        };

        ledger
            .board_heartbeat(
                &vm1,
                &BoardHeartbeatReq {
                    project: "daybook".into(),
                    agent: "t3-api".into(),
                    runtime: "pi".into(),
                    cwd: "/repo".into(),
                    branch: "main".into(),
                    status: None,
                    current_task: None,
                },
            )
            .unwrap();

        // vm2 cannot post or claim locks pretending to be t3-api
        let spoof = ledger.board_post_message(
            &vm2,
            &BoardPostMessageReq {
                project: "daybook".into(),
                channel: Some("general".into()),
                agent: "t3-api".into(),
                body: "spoofed".into(),
                parent_id: None,
            },
        );
        assert!(spoof.is_err());
    }

    #[test]
    fn tasks_and_mentions_round_trip_with_touch_lock_notifications() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("board.db")).unwrap();
        let vm1 = BoardCaller::local();

        let claim1 = ledger
            .board_claim_task(
                &vm1,
                &BoardClaimTaskReq {
                    project: "daybook".into(),
                    task_id: "BOARD-1".into(),
                    title: Some("Build board".into()),
                    priority: Some("high".into()),
                    agent: "agent-a".into(),
                    branch: Some("feat/board".into()),
                    note: Some("starting".into()),
                    force: false,
                },
            )
            .unwrap();
        assert!(claim1.ok);
        assert_eq!(claim1.task.title, "Build board");

        // Second agent cannot steal without force
        let claim2 = ledger
            .board_claim_task(
                &vm1,
                &BoardClaimTaskReq {
                    project: "daybook".into(),
                    task_id: "BOARD-1".into(),
                    title: None,
                    priority: None,
                    agent: "agent-b".into(),
                    branch: None,
                    note: None,
                    force: false,
                },
            )
            .unwrap();
        assert!(!claim2.ok);
        assert_eq!(claim2.held_by.as_deref(), Some("agent-a"));

        // agent-b posts a mention to @agent-a
        let msg = ledger
            .board_post_message(
                &vm1,
                &BoardPostMessageReq {
                    project: "daybook".into(),
                    channel: Some("handoff".into()),
                    agent: "agent-b".into(),
                    body: "Hey @agent-a please check src/server/dto.rs".into(),
                    parent_id: None,
                },
            )
            .unwrap();
        assert_eq!(msg.mentions, vec!["agent-a".to_string()]);

        // When agent-a touches a file via PostToolUse, it receives the unread @mention
        let touch = ledger
            .board_touch_lock(
                &vm1,
                &BoardTouchLockReq {
                    project: "daybook".into(),
                    agent: "agent-a".into(),
                    path: "src/server/dto.rs".into(),
                    ttl_minutes: Some(15),
                    since_id: Some(0),
                },
            )
            .unwrap();
        assert!(touch.lock.auto);
        assert_eq!(touch.unread_mentions.len(), 1);
        assert_eq!(touch.unread_mentions[0].id, msg.id);

        // Completing task with release_locks=true drops agent-a's lock
        let done = ledger
            .board_update_task(
                &vm1,
                &BoardUpdateTaskReq {
                    project: "daybook".into(),
                    task_id: "BOARD-1".into(),
                    agent: "agent-a".into(),
                    status: "done".into(),
                    note: Some("shipped".into()),
                    pr_url: Some("https://github.com/example/daybook/pull/1".into()),
                    release_locks: true,
                    force: false,
                },
            )
            .unwrap();
        assert_eq!(done.status, "done");

        let status = ledger
            .board_status(Some("daybook"), Some("agent-a"), Some(0))
            .unwrap();
        assert!(status.locks.is_empty());
        assert_eq!(status.unread_mentions.len(), 1);
    }

    #[test]
    fn task_review_and_blocked_protected_and_cursor_suppresses_repeat_mentions() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("board.db")).unwrap();
        let vm1 = BoardCaller {
            machine: "vm1".into(),
            admin: false,
        };
        let vm2 = BoardCaller {
            machine: "vm2".into(),
            admin: false,
        };

        // agent-a claims task and moves it to review
        ledger
            .board_claim_task(
                &vm1,
                &BoardClaimTaskReq {
                    project: "daybook".into(),
                    task_id: "T-1".into(),
                    title: Some("Review protection".into()),
                    priority: Some("high".into()),
                    agent: "agent-a".into(),
                    branch: None,
                    note: None,
                    force: false,
                },
            )
            .unwrap();
        ledger
            .board_update_task(
                &vm1,
                &BoardUpdateTaskReq {
                    project: "daybook".into(),
                    task_id: "T-1".into(),
                    agent: "agent-a".into(),
                    status: "review".into(),
                    note: Some("PR ready".into()),
                    pr_url: None,
                    release_locks: false,
                    force: false,
                },
            )
            .unwrap();

        // agent-b on vm2 cannot steal a task in 'review' even with force=true (cross-machine)
        let steal = ledger
            .board_claim_task(
                &vm2,
                &BoardClaimTaskReq {
                    project: "daybook".into(),
                    task_id: "T-1".into(),
                    title: None,
                    priority: None,
                    agent: "agent-b".into(),
                    branch: None,
                    note: None,
                    force: true,
                },
            )
            .unwrap();
        assert!(!steal.ok);
        assert_eq!(steal.held_by.as_deref(), Some("agent-a"));

        // agent-b posts a mention to @agent-a
        ledger
            .board_post_message(
                &vm2,
                &BoardPostMessageReq {
                    project: "daybook".into(),
                    channel: Some("general".into()),
                    agent: "agent-b".into(),
                    body: "Ping @agent-a once merged".into(),
                    parent_id: None,
                },
            )
            .unwrap();

        // First touch returns the mention and advances next_since_id
        let t1 = ledger
            .board_touch_lock(
                &vm1,
                &BoardTouchLockReq {
                    project: "daybook".into(),
                    agent: "agent-a".into(),
                    path: "src/lib.rs".into(),
                    ttl_minutes: Some(10),
                    since_id: None,
                },
            )
            .unwrap();
        assert_eq!(t1.unread_mentions.len(), 1);
        assert!(t1.next_since_id > 0);

        // Second touch passing since_id = Some(t1.next_since_id) suppresses repeat mention and extends persisted TTL
        let t2 = ledger
            .board_touch_lock(
                &vm1,
                &BoardTouchLockReq {
                    project: "daybook".into(),
                    agent: "agent-a".into(),
                    path: "src/lib.rs".into(),
                    ttl_minutes: Some(60),
                    since_id: Some(t1.next_since_id),
                },
            )
            .unwrap();
        assert!(t2.unread_mentions.is_empty());
        assert_eq!(t2.lock.id, t1.lock.id);
        let snap = ledger.board_status(Some("daybook"), None, None).unwrap();
        let persisted = snap.locks.iter().find(|l| l.id == t1.lock.id).unwrap();
        assert!(persisted.expires_epoch > t1.lock.expires_epoch);
        assert_eq!(persisted.expires_epoch, t2.lock.expires_epoch);

        // Heartbeat with a session title must not overwrite an active claimed/review task_id,
        // while a heartbeat naming a different active claimed task_id ('T-2') updates current_task.
        let hb = ledger
            .board_heartbeat(
                &vm1,
                &BoardHeartbeatReq {
                    project: "daybook".into(),
                    agent: "agent-a".into(),
                    runtime: "pi".into(),
                    branch: "main".into(),
                    cwd: "/tmp/daybook".into(),
                    status: None,
                    current_task: Some("Session title that is not a task_id".into()),
                },
            )
            .unwrap();
        assert_eq!(hb.current_task.as_deref(), Some("T-1"));
        assert_eq!(hb.status, "active");

        ledger
            .board_claim_task(
                &vm1,
                &BoardClaimTaskReq {
                    project: "daybook".into(),
                    task_id: "T-2".into(),
                    title: Some("Second active task".into()),
                    priority: Some("medium".into()),
                    agent: "agent-a".into(),
                    branch: None,
                    note: None,
                    force: false,
                },
            )
            .unwrap();
        let hb2 = ledger
            .board_heartbeat(
                &vm1,
                &BoardHeartbeatReq {
                    project: "daybook".into(),
                    agent: "agent-a".into(),
                    runtime: "pi".into(),
                    branch: "main".into(),
                    cwd: "/tmp/daybook".into(),
                    status: Some("idle".into()),
                    current_task: Some("T-2".into()),
                },
            )
            .unwrap();
        assert_eq!(hb2.current_task.as_deref(), Some("T-2"));
        assert_eq!(hb2.status, "idle");
        let hb_blocked = ledger
            .board_heartbeat(
                &vm1,
                &BoardHeartbeatReq {
                    project: "daybook".into(),
                    agent: "agent-a".into(),
                    runtime: "pi".into(),
                    branch: "main".into(),
                    cwd: "/tmp/daybook".into(),
                    status: Some("blocked".into()),
                    current_task: None,
                },
            )
            .unwrap();
        assert_eq!(hb_blocked.status, "blocked");
        let hb_preserve = ledger
            .board_heartbeat(
                &vm1,
                &BoardHeartbeatReq {
                    project: "daybook".into(),
                    agent: "agent-a".into(),
                    runtime: "cli".into(),
                    branch: "main".into(),
                    cwd: "/tmp/daybook".into(),
                    status: None,
                    current_task: None,
                },
            )
            .unwrap();
        assert_eq!(hb_preserve.status, "blocked");
        assert_eq!(hb_preserve.runtime, "pi");
        // Reserved project names ('all', 'agent') are rejected on write paths
        for reserved in ["all", "ALL", "agent", "Agent"] {
            assert!(ledger
                .board_heartbeat(
                    &vm1,
                    &BoardHeartbeatReq {
                        project: reserved.into(),
                        agent: "agent-a".into(),
                        runtime: "pi".into(),
                        branch: "main".into(),
                        cwd: "/tmp/x".into(),
                        status: None,
                        current_task: None,
                    },
                )
                .is_err());
            assert!(ledger
                .board_post_message(
                    &vm1,
                    &BoardPostMessageReq {
                        project: reserved.into(),
                        channel: Some("general".into()),
                        agent: "agent-a".into(),
                        body: "hi".into(),
                        parent_id: None,
                    },
                )
                .is_err());
        }

        // All-projects status ('all') aggregates agents and unread_mentions across multiple projects and advances next_since_id
        ledger
            .board_heartbeat(
                &vm2,
                &BoardHeartbeatReq {
                    project: "other-repo".into(),
                    agent: "agent-b".into(),
                    runtime: "codex".into(),
                    branch: "feat/x".into(),
                    cwd: "/tmp/other-repo".into(),
                    status: None,
                    current_task: Some("Cross-project test".into()),
                },
            )
            .unwrap();
        ledger
            .board_post_message(
                &vm1,
                &BoardPostMessageReq {
                    project: "other-repo".into(),
                    channel: Some("handoff".into()),
                    agent: "agent-a".into(),
                    body: "Hey @agent-b check other-repo".into(),
                    parent_id: None,
                },
            )
            .unwrap();
        let all_snap = ledger
            .board_status(Some("all"), Some("agent-b"), Some(0))
            .unwrap();
        assert_eq!(all_snap.project.as_deref(), Some("all"));
        assert!(all_snap.agents.iter().any(|a| a.project == "daybook"));
        assert!(all_snap.agents.iter().any(|a| a.project == "other-repo"));
        assert_eq!(all_snap.unread_mentions.len(), 1);
        assert_eq!(all_snap.unread_mentions[0].project, "other-repo");
        assert!(all_snap.next_since_id > t1.next_since_id);
        assert_eq!(all_snap.locks.len(), 1);
        assert_eq!(all_snap.tasks.len(), 2);
        let star_snap = ledger.board_status(Some("*"), None, None).unwrap();
        assert_eq!(star_snap.project.as_deref(), Some("all"));
        assert_eq!(star_snap.agents.len(), all_snap.agents.len());
        let projects = ledger.board_projects().unwrap();
        assert_eq!(
            projects,
            vec!["daybook".to_string(), "other-repo".to_string()]
        );
    }
}
