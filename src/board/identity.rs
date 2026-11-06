use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use crate::transcript::adapters::resolve_git_project;

pub const MAX_IDENT_LEN: usize = 64;
pub const MAX_NOTE_LEN: usize = 512;
pub const MAX_BODY_LEN: usize = 4096;

/// Walk up from `cwd` to find the nearest enclosing directory containing `.git` (dir or file).
pub fn resolve_worktree_root(cwd: &Path) -> PathBuf {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let git = d.join(".git");
        if git.exists() {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    cwd.to_path_buf()
}

/// Validate a board identifier (`project`, `task_id`, `lock_id`, `channel`) against `[a-zA-Z0-9._-]{1,64}`.
pub fn validate_identifier(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_IDENT_LEN {
        bail!("{field} must be 1..={MAX_IDENT_LEN} characters");
    }
    if trimmed.contains("..") {
        bail!("{field} must not contain '..'");
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        bail!("{field} may only contain ASCII alphanumeric characters, '.', '_', or '-'");
    }
    if field == "project"
        && (trimmed.eq_ignore_ascii_case("all") || trimmed.eq_ignore_ascii_case("agent"))
    {
        bail!("project '{}' is reserved", trimmed);
    }
    Ok(trimmed.to_string())
}

/// Sanitize a raw string into a valid lowercase agent handle `[a-z0-9][a-z0-9_-]{0,63}`.
pub fn sanitize_agent_handle(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_IDENT_LEN));
    for c in raw.trim().chars() {
        let lc = c.to_ascii_lowercase();
        if lc.is_ascii_alphanumeric() || lc == '_' || lc == '-' {
            out.push(lc);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    if trimmed.is_empty() {
        return "agent".to_string();
    }
    trimmed.chars().take(MAX_IDENT_LEN).collect()
}

/// Validate an explicit agent handle against `[a-z0-9][a-z0-9_-]{0,63}`.
pub fn validate_agent_handle(raw: &str) -> Result<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() || trimmed.len() > MAX_IDENT_LEN {
        bail!("agent handle must be 1..={MAX_IDENT_LEN} characters");
    }
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        bail!("agent handle must not be empty");
    };
    if !first.is_ascii_alphanumeric() {
        bail!("agent handle must start with an ASCII alphanumeric character");
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        bail!("agent handle may only contain lowercase alphanumeric characters, '_', or '-'");
    }
    Ok(trimmed)
}

/// Resolve canonical project slug from explicit override, `DAYBOOK_BOARD_PROJECT`, `.t3/worktrees/<project>/...`, or git root.
pub fn resolve_board_project(cwd: &Path, explicit: Option<&str>) -> String {
    if let Some(exp) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(valid) = validate_identifier(exp, "project") {
            return valid;
        }
    }
    if let Ok(env_proj) = std::env::var("DAYBOOK_BOARD_PROJECT") {
        if let Ok(valid) = validate_identifier(&env_proj, "project") {
            return valid;
        }
    }
    resolve_board_project_from_path(cwd)
}

/// Resolve canonical project slug purely from filesystem path without consulting process env vars.
pub fn resolve_board_project_from_path(cwd: &Path) -> String {
    let cwd_str = cwd.to_string_lossy().replace('\\', "/");
    if let Some(rest) = cwd_str.split("/.t3/worktrees/").nth(1) {
        if let Some(seg) = rest
            .split('/')
            .next()
            .filter(|s| !s.is_empty() && !s.contains(".."))
        {
            if let Ok(valid) = validate_identifier(seg, "project") {
                return valid;
            }
        }
    }
    if let Some(proj) = resolve_git_project(cwd) {
        if let Ok(valid) = validate_identifier(&proj, "project") {
            return valid;
        }
    }
    cwd.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .and_then(|s| validate_identifier(&s, "project").ok())
        .unwrap_or_else(|| "default".to_string())
}

/// Read current git branch from `.git/HEAD` or worktree `gitdir` pointer without spawning `git`.
pub fn resolve_git_branch(cwd: &Path) -> String {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let dot_git = d.join(".git");
        let head_path = if dot_git.is_dir() {
            Some(dot_git.join("HEAD"))
        } else if dot_git.is_file() {
            std::fs::read_to_string(&dot_git)
                .ok()
                .and_then(|txt| txt.strip_prefix("gitdir: ").map(|s| s.trim().to_string()))
                .map(|gd| {
                    let p = PathBuf::from(&gd);
                    if p.is_absolute() {
                        p.join("HEAD")
                    } else {
                        d.join(p).join("HEAD")
                    }
                })
        } else {
            None
        };

        if let Some(hp) = head_path {
            if let Ok(head) = std::fs::read_to_string(hp) {
                let trimmed = head.trim();
                if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
                    return branch.trim().to_string();
                }
                if trimmed.len() >= 7 {
                    return trimmed[..7].to_string();
                }
            }
            break;
        }
        dir = d.parent();
    }
    "unknown".to_string()
}

/// Detect active coding agent runtime from explicit flag or environment variables.
pub fn resolve_agent_runtime(explicit: Option<&str>) -> String {
    if let Some(exp) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return sanitize_agent_handle(exp);
    }
    if std::env::var_os("PI_SESSION_ID").is_some() || std::env::var_os("PI_AGENT").is_some() {
        return "pi".to_string();
    }
    if std::env::var_os("CODEX_SANDBOX").is_some() || std::env::var_os("CODEX_THREAD_ID").is_some()
    {
        return "codex".to_string();
    }
    if std::env::var_os("CLAUDE_PROJECT_DIR").is_some() || std::env::var_os("CLAUDECODE").is_some()
    {
        return "claude-code".to_string();
    }
    "cli".to_string()
}

/// Resolve deterministic agent handle for the current session/worktree.
pub fn resolve_agent_handle(cwd: &Path, explicit: Option<&str>, runtime: &str) -> String {
    if let Some(exp) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return sanitize_agent_handle(exp);
    }
    if let Ok(env_agent) = std::env::var("DAYBOOK_AGENT") {
        let trimmed = env_agent.trim();
        if !trimmed.is_empty() {
            return sanitize_agent_handle(trimmed);
        }
    }
    resolve_agent_handle_from_path(cwd, runtime)
}

/// Resolve deterministic agent handle purely from filesystem path without consulting process env vars.
pub fn resolve_agent_handle_from_path(cwd: &Path, runtime: &str) -> String {
    let cwd_str = cwd.to_string_lossy().replace('\\', "/");
    if let Some(rest) = cwd_str.split("/.t3/worktrees/").nth(1) {
        let mut parts = rest.split('/');
        let _repo = parts.next();
        if let Some(wt_id) = parts.next().filter(|s| !s.is_empty()) {
            return sanitize_agent_handle(wt_id);
        }
    }
    let branch = resolve_git_branch(cwd);
    sanitize_agent_handle(&format!("{runtime}-{branch}"))
}

#[derive(Debug, Clone)]
pub struct BoardIdentity {
    pub project: String,
    pub agent: String,
    pub runtime: String,
    pub branch: String,
    pub worktree_root: PathBuf,
    pub session_title: Option<String>,
}

/// Returns true when `cwd` belongs to an existing git repository or worktree (excluding `$HOME`, `/tmp`, and `.t3/scratch/`).
pub fn is_valid_board_workspace(cwd: &Path) -> bool {
    if !cwd.is_dir() {
        return false;
    }
    let cwd_str = cwd.to_string_lossy().replace('\\', "/");
    if cwd_str == "/tmp" || cwd_str.contains("/.t3/scratch/") || cwd_str.ends_with("/.t3/scratch") {
        return false;
    }
    let root = resolve_worktree_root(cwd);
    if let Some(home) = dirs::home_dir() {
        if root == home {
            return false;
        }
    }
    cwd.join(".git").exists() || root.join(".git").exists()
}

pub fn sanitize_session_title(raw: &str) -> String {
    let mut out = String::new();
    let mut char_count = 0usize;
    let mut last_sep = false;
    for ch in raw.lines().next().unwrap_or("").trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':' | '/' | '@' | '#') {
            out.push(ch);
            char_count += 1;
            last_sep = false;
        } else if !last_sep && !out.is_empty() {
            out.push(' ');
            char_count += 1;
            last_sep = true;
        }
        if char_count >= 120 {
            break;
        }
    }
    out.trim().to_string()
}

fn sanitize_git_branch(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
        .take(120)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Look up T3 Code thread metadata (`(title, provider, branch)`) for `worktree_root` from a specific `statev2.sqlite` path.
pub fn resolve_t3_worktree_meta_from_db(
    worktree_root: &Path,
    db_path: &Path,
) -> Option<(String, Option<String>, Option<String>)> {
    if !db_path.is_file() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(200));
    let root_str = worktree_root.to_string_lossy().to_string();
    let escaped_root = root_str
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let like_pat = format!("%{}%", escaped_root);
    let mut stmt = conn
        .prepare(
            "SELECT t.thread_id, t.title, t.default_provider, t.payload_json, p.workspace_root
             FROM orchestration_v2_projection_threads t
             LEFT JOIN projection_projects p ON t.project_id = p.project_id
             WHERE t.deleted_at IS NULL
               AND t.archived_at IS NULL
               AND (t.payload_json LIKE ?1 ESCAPE '\\' OR p.workspace_root = ?2)
             ORDER BY t.updated_at DESC
             LIMIT 20",
        )
        .ok()?;
    let rows = stmt
        .query_map(rusqlite::params![like_pat, root_str], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                r.get::<_, Option<String>>(4)?,
            ))
        })
        .ok()?;

    let mut fallback: Option<(String, Option<String>, Option<String>)> = None;
    for row in rows.flatten() {
        let (tid, title, provider, payload_raw, workspace_root) = row;
        let payload = serde_json::from_str::<serde_json::Value>(&payload_raw).unwrap_or_default();
        let wt_path = payload
            .get("worktreePath")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .or(workspace_root.as_deref());
        let Some(wt_path) = wt_path else {
            continue;
        };
        if Path::new(wt_path) != worktree_root {
            continue;
        }
        let clean_title = sanitize_session_title(&title);
        if clean_title.is_empty() {
            continue;
        }
        let prov = provider
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty());
        let br = payload
            .get("branch")
            .and_then(|v| v.as_str())
            .and_then(sanitize_git_branch);
        if !tid.starts_with("thread:delegated-task:") && !tid.starts_with("mcp:") {
            return Some((clean_title, prov, br));
        }
        if fallback.is_none() {
            fallback = Some((clean_title, prov, br));
        }
    }
    fallback
}

/// Discover active mid-conversation threads across `.t3/worktrees/*/*` and recent T3 Code threads in `statev2.sqlite`.
pub fn discover_active_board_identities() -> Vec<BoardIdentity> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let db_path = home.join(".t3").join("userdata").join("statev2.sqlite");
    let wt_base = home.join(".t3").join("worktrees");
    discover_active_board_identities_with_paths(Some(&db_path), Some(&wt_base))
}

/// Discover active mid-conversation threads from explicit `statev2.sqlite` and `.t3/worktrees` paths.
pub fn discover_active_board_identities_with_paths(
    db_path: Option<&Path>,
    wt_base: Option<&Path>,
) -> Vec<BoardIdentity> {
    let mut seen = std::collections::HashSet::new();
    let mut db_meta_by_path: std::collections::HashMap<
        PathBuf,
        (String, Option<String>, Option<String>),
    > = std::collections::HashMap::new();
    let mut out = Vec::new();

    // 1. Top-level threads in statev2.sqlite (single read serves both active thread discovery and Step 2 metadata lookup)
    if let Some(db_path) = db_path.filter(|p| p.is_file()) {
        if let Ok(conn) = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            let _ = conn.busy_timeout(std::time::Duration::from_millis(200));
            let cutoff_dt = chrono::Utc::now() - chrono::Duration::hours(2);
            let cutoff = cutoff_dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            if let Ok(mut stmt) = conn.prepare(
                "SELECT t.thread_id, t.title, t.default_provider, t.payload_json, COALESCE(p.workspace_root, ''), COALESCE(t.updated_at, '')
                 FROM orchestration_v2_projection_threads t
                 LEFT JOIN projection_projects p ON t.project_id = p.project_id
                 WHERE t.deleted_at IS NULL
                   AND t.archived_at IS NULL
                 ORDER BY t.updated_at DESC
                 LIMIT 200",
            ) {
                if let Ok(rows) = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                }) {
                    for row in rows.flatten() {
                        let (tid, title, provider, payload_str, workspace_root, updated_at) = row;
                        if tid.starts_with("thread:delegated-task:") || tid.starts_with("mcp:") {
                            continue;
                        }
                        let payload: serde_json::Value =
                            serde_json::from_str(&payload_str).unwrap_or_default();
                        let wt_str = payload
                            .get("worktreePath")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or(&workspace_root);
                        if wt_str.trim().is_empty() {
                            continue;
                        }
                        let wt_path = PathBuf::from(wt_str);
                        let clean_title = sanitize_session_title(&title);
                        let prov_clean = provider
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(sanitize_agent_handle);
                        let branch_clean = payload
                            .get("branch")
                            .and_then(|v| v.as_str())
                            .and_then(sanitize_git_branch);
                        db_meta_by_path
                            .entry(wt_path.clone())
                            .or_insert_with(|| {
                                (clean_title.clone(), prov_clean.clone(), branch_clean.clone())
                            });

                        let is_recent = chrono::DateTime::parse_from_rfc3339(&updated_at)
                            .map(|dt| dt.with_timezone(&chrono::Utc) >= cutoff_dt)
                            .unwrap_or_else(|_| updated_at >= cutoff);
                        if !is_recent || !is_valid_board_workspace(&wt_path) {
                            continue;
                        }
                        let mut id = resolve_worktree_board_identity(&wt_path);
                        if id.project.eq_ignore_ascii_case("agent")
                            || id.project.eq_ignore_ascii_case("all")
                        {
                            continue;
                        }
                        if !wt_path
                            .to_string_lossy()
                            .replace('\\', "/")
                            .contains("/.t3/worktrees/")
                        {
                            let short_tid: String = tid
                                .chars()
                                .filter(|c| c.is_ascii_alphanumeric())
                                .take(8)
                                .collect();
                            if !short_tid.is_empty() {
                                id.agent = format!("t3-{}", short_tid.to_ascii_lowercase());
                            }
                        }
                        if let Some(prov) = prov_clean {
                            id.runtime = prov;
                        }
                        if id.branch == "unknown" {
                            if let Some(br) = branch_clean {
                                id.branch = br;
                            }
                        }
                        if !clean_title.is_empty() {
                            id.session_title = Some(clean_title);
                        }
                        if seen.insert((id.project.clone(), id.agent.clone())) {
                            out.push(id);
                        }
                    }
                }
            }
        }
    }

    // 2. Any .t3/worktrees/<project>/<id> directory modified within the 2-hour board window (7200s)
    if let Some(wt_base) = wt_base {
        if let Ok(projects) = std::fs::read_dir(wt_base) {
            for proj_entry in projects.flatten() {
                if let Ok(worktrees) = std::fs::read_dir(proj_entry.path()) {
                    for wt_entry in worktrees.flatten() {
                        let path = wt_entry.path();
                        let dot_git = path.join(".git");
                        let mut candidates = vec![dot_git.clone(), path.clone()];
                        if let Ok(raw) = std::fs::read_to_string(&dot_git) {
                            if let Some(gd) = raw.trim().strip_prefix("gitdir:").map(str::trim) {
                                let gd_path = PathBuf::from(gd);
                                let resolved = if gd_path.is_absolute() {
                                    gd_path
                                } else {
                                    path.join(gd_path)
                                };
                                candidates.push(resolved.join("index"));
                                candidates.push(resolved.join("HEAD"));
                                candidates.push(resolved);
                            }
                        }
                        let recently_modified = candidates
                            .iter()
                            .filter_map(|p| std::fs::metadata(p).ok()?.modified().ok())
                            .filter_map(|m| m.elapsed().ok())
                            .any(|d| d.as_secs() <= 7200);
                        if recently_modified && is_valid_board_workspace(&path) {
                            let mut id = resolve_worktree_board_identity(&path);
                            if !id.project.eq_ignore_ascii_case("agent")
                                && !id.project.eq_ignore_ascii_case("all")
                                && seen.insert((id.project.clone(), id.agent.clone()))
                            {
                                if let Some((title, prov, br)) = db_meta_by_path.get(&path) {
                                    if id.session_title.is_none() && !title.is_empty() {
                                        id.session_title = Some(title.clone());
                                    }
                                    if id.runtime == "cli" {
                                        if let Some(p) = prov {
                                            id.runtime = p.clone();
                                        }
                                    }
                                    if id.branch == "unknown" {
                                        if let Some(b) = br {
                                            id.branch = b.clone();
                                        }
                                    }
                                }
                                out.push(id);
                            }
                        }
                    }
                }
            }
        }
    }

    out
}

/// Enrich a `BoardIdentity` with T3 Code thread title/provider/branch from `~/.t3/userdata/statev2.sqlite` when missing.
pub fn enrich_board_identity_from_t3(id: &mut BoardIdentity) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let db_path = home.join(".t3").join("userdata").join("statev2.sqlite");
    enrich_board_identity_from_t3_db(id, &db_path);
}

/// Enrich a `BoardIdentity` from an explicit `statev2.sqlite` path when fields are missing.
pub fn enrich_board_identity_from_t3_db(id: &mut BoardIdentity, db_path: &Path) {
    if id.session_title.is_some() && id.runtime != "cli" && id.branch != "unknown" {
        return;
    }
    if let Some((title, t3_prov, t3_br)) =
        resolve_t3_worktree_meta_from_db(&id.worktree_root, db_path)
    {
        if id.session_title.is_none() && !title.is_empty() {
            id.session_title = Some(title);
        }
        if id.runtime == "cli" {
            if let Some(prov) = t3_prov {
                id.runtime = sanitize_agent_handle(&prov);
            }
        }
        if id.branch == "unknown" {
            if let Some(br) = t3_br {
                id.branch = br;
            }
        }
    }
}

/// Resolve complete board identity (project, agent, runtime, branch, worktree_root, session_title) for `cwd`.
pub fn resolve_board_identity(
    cwd: &Path,
    explicit_project: Option<&str>,
    explicit_agent: Option<&str>,
) -> BoardIdentity {
    let worktree_root = resolve_worktree_root(cwd);
    let project = resolve_board_project(&worktree_root, explicit_project);
    let runtime = resolve_agent_runtime(None);
    let agent = resolve_agent_handle(&worktree_root, explicit_agent, &runtime);
    let branch = resolve_git_branch(&worktree_root);
    let session_title = std::env::var("DAYBOOK_SESSION_TITLE")
        .ok()
        .map(|s| sanitize_session_title(&s))
        .filter(|s| !s.is_empty());
    BoardIdentity {
        project,
        agent,
        runtime,
        branch,
        worktree_root,
        session_title,
    }
}

/// Resolve board identity for an arbitrary worktree on disk without reading the caller process's env vars.
pub fn resolve_worktree_board_identity(cwd: &Path) -> BoardIdentity {
    let worktree_root = resolve_worktree_root(cwd);
    let project = resolve_board_project_from_path(&worktree_root);
    let runtime = "cli".to_string();
    let agent = resolve_agent_handle_from_path(&worktree_root, &runtime);
    let branch = resolve_git_branch(&worktree_root);
    BoardIdentity {
        project,
        agent,
        runtime,
        branch,
        worktree_root,
        session_title: None,
    }
}

/// Extract deduplicated `@agent` mentions (`@([a-z0-9][a-z0-9_-]{0,63})`) from message text.
pub fn extract_mentions(body: &str) -> Vec<String> {
    let mut mentions = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let preceded_by_word = i > 0 && {
                let prev = bytes[i - 1];
                prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'-' || prev == b'.'
            };
            if !preceded_by_word && i + 1 < bytes.len() && bytes[i + 1].is_ascii_alphanumeric() {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len()
                    && (end - start) < MAX_IDENT_LEN
                    && (bytes[end].is_ascii_alphanumeric()
                        || bytes[end] == b'_'
                        || bytes[end] == b'-')
                {
                    end += 1;
                }
                let handle = body[start..end].to_ascii_lowercase();
                if !mentions.contains(&handle) {
                    mentions.push(handle);
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    mentions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t3_worktree_resolves_project_and_handle() {
        let p = Path::new("/home/agent/.t3/worktrees/daybook/t3-fb61e20a/src");
        assert_eq!(resolve_board_project(p, None), "daybook");
        assert_eq!(resolve_agent_handle(p, None, "pi"), "t3-fb61e20a");
    }

    #[test]
    fn extract_mentions_ignores_emails_and_deduplicates() {
        let msg =
            "Hey @t3-fb61e20a and @Claude_UI, email me at dev@example.com or ping @t3-fb61e20a!";
        assert_eq!(
            extract_mentions(msg),
            vec!["t3-fb61e20a".to_string(), "claude_ui".to_string()]
        );
    }

    #[test]
    fn validate_identifier_rejects_traversal_and_invalid_chars() {
        assert!(validate_identifier("daybook", "project").is_ok());
        assert!(validate_identifier("BOARD-123_a.1", "task_id").is_ok());
        assert!(validate_identifier("../bad", "project").is_err());
        assert!(validate_identifier("bad/slash", "project").is_err());
        assert!(validate_identifier("", "project").is_err());
        assert!(validate_identifier("all", "project").is_err());
        assert!(validate_identifier("agent", "project").is_err());
        assert!(validate_identifier("all", "channel").is_ok());
    }

    #[test]
    fn sanitize_session_title_cleans_and_caps_length() {
        assert_eq!(
            sanitize_session_title("  Fix board_status — cross-project sync!\nsecond line"),
            "Fix board_status cross project sync"
        );
        let long = "a".repeat(200);
        assert_eq!(sanitize_session_title(&long).chars().count(), 120);
        assert_eq!(sanitize_session_title("   ---   "), "");
    }

    #[test]
    fn is_valid_board_workspace_checks_git_and_rejects_scratch_or_tmp() {
        assert!(!is_valid_board_workspace(Path::new("/tmp")));
        assert!(!is_valid_board_workspace(Path::new(
            "/home/agent/.t3/scratch/2026-10-28-foo"
        )));
        let tmp = tempfile::tempdir().unwrap();
        let non_git = tmp.path().join("plain");
        std::fs::create_dir_all(&non_git).unwrap();
        assert!(!is_valid_board_workspace(&non_git));

        let wt = tmp
            .path()
            .join(".t3")
            .join("worktrees")
            .join("daybook")
            .join("t3-abc12345");
        std::fs::create_dir_all(&wt).unwrap();
        assert!(!is_valid_board_workspace(&wt));
        std::fs::write(wt.join(".git"), "gitdir: /tmp/fake\n").unwrap();
        assert!(is_valid_board_workspace(&wt));
    }

    #[test]
    fn resolve_t3_worktree_meta_from_db_handles_wildcards_and_delegates() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("statev2.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE projection_projects (
                project_id TEXT PRIMARY KEY,
                workspace_root TEXT
            );
            CREATE TABLE orchestration_v2_projection_threads (
                thread_id TEXT PRIMARY KEY,
                project_id TEXT,
                title TEXT,
                default_provider TEXT,
                payload_json TEXT,
                updated_at TEXT,
                deleted_at TEXT,
                archived_at TEXT
            );",
        )
        .unwrap();
        let wt_target = tmp.path().join("work_tree_100%");
        let wt_other = tmp.path().join("workXtreeX100X");
        let payload_target = serde_json::json!({
            "worktreePath": wt_target.to_string_lossy(),
            "branch": "feat/board-sync"
        })
        .to_string();
        let payload_other = serde_json::json!({
            "worktreePath": wt_other.to_string_lossy(),
            "branch": "feat/other"
        })
        .to_string();
        conn.execute(
            "INSERT INTO orchestration_v2_projection_threads
             (thread_id, project_id, title, default_provider, payload_json, updated_at)
             VALUES (?1, 'p1', 'Delegated child task', 'claudeAgent', ?2, '2026-10-31T16:00:00Z')",
            rusqlite::params!["thread:delegated-task:1", payload_target],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO orchestration_v2_projection_threads
             (thread_id, project_id, title, default_provider, payload_json, updated_at)
             VALUES (?1, 'p1', 'Main worktree thread!', 'pi', ?2, '2026-10-31T15:00:00Z')",
            rusqlite::params!["t3-main-1", payload_target],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO orchestration_v2_projection_threads
             (thread_id, project_id, title, default_provider, payload_json, updated_at)
             VALUES (?1, 'p1', 'Unrelated wildcard path', 'codex', ?2, '2026-10-31T17:00:00Z')",
            rusqlite::params!["t3-other-1", payload_other],
        )
        .unwrap();

        let meta = resolve_t3_worktree_meta_from_db(&wt_target, &db_path).unwrap();
        assert_eq!(meta.0, "Main worktree thread");
        assert_eq!(meta.1.as_deref(), Some("pi"));
        assert_eq!(meta.2.as_deref(), Some("feat/board-sync"));
        let mut id = BoardIdentity {
            project: "daybook".into(),
            agent: "t3-abc12345".into(),
            runtime: "cli".into(),
            branch: "unknown".into(),
            worktree_root: wt_target.clone(),
            session_title: None,
        };
        enrich_board_identity_from_t3_db(&mut id, &db_path);
        assert_eq!(id.session_title.as_deref(), Some("Main worktree thread"));
        assert_eq!(id.runtime, "pi");
        assert_eq!(id.branch, "feat/board-sync");

        let wt_base = tmp.path().join(".t3").join("worktrees");
        let wt_dir = wt_base.join("daybook").join("t3-abc12345");
        std::fs::create_dir_all(&wt_dir).unwrap();
        std::fs::write(wt_dir.join(".git"), "gitdir: /tmp/fake\n").unwrap();
        let payload_wt = serde_json::json!({
            "worktreePath": wt_dir.to_string_lossy(),
            "branch": "feat/wt-discovery"
        })
        .to_string();
        let now_iso = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        conn.execute(
            "INSERT INTO orchestration_v2_projection_threads
             (thread_id, project_id, title, default_provider, payload_json, updated_at)
             VALUES ('t3-wt-1', 'p1', 'Discovered worktree thread', 'codex', ?1, ?2)",
            rusqlite::params![payload_wt, now_iso],
        )
        .unwrap();
        let discovered =
            discover_active_board_identities_with_paths(Some(&db_path), Some(&wt_base));
        let found = discovered
            .iter()
            .find(|d| d.project == "daybook" && d.agent == "t3-abc12345")
            .expect("discovered daybook/t3-abc12345");
        assert_eq!(
            found.session_title.as_deref(),
            Some("Discovered worktree thread")
        );
        assert_eq!(found.runtime, "codex");
        assert_eq!(found.branch, "feat/wt-discovery");
    }
}
