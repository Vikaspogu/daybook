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
}

/// Resolve complete board identity (`project`, `agent`, `runtime`, `branch`, `worktree_root`) for `cwd`.
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
    BoardIdentity {
        project,
        agent,
        runtime,
        branch,
        worktree_root,
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
    }
}
