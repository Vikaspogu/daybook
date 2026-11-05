use anyhow::Result;
use std::path::PathBuf;

use crate::board::client::BoardClient;
use crate::board::globs::resolve_hook_file_path;
use crate::board::identity::resolve_board_identity;
use crate::config::load_config;
use crate::hooks::read_hook_input;

/// Handle PreToolUse hook: check if the file about to be edited is locked by another agent.
/// Advisory by default (prints warning to stderr and exits 0).
/// Set DAYBOOK_STRICT_LOCKS=1 (or DAYBOOK_LOCK_ENFORCE=strict) to exit 2 and block conflicting edits
/// (and fail closed if a configured remote board server is unreachable).
pub async fn handle() -> Result<()> {
    let strict = std::env::var("DAYBOOK_STRICT_LOCKS")
        .ok()
        .is_some_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
        || std::env::var("DAYBOOK_LOCK_ENFORCE")
            .ok()
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("strict"));

    let input = match read_hook_input() {
        Ok(i) => i,
        Err(err) => {
            if strict {
                eprintln!("[daybook board] BLOCKED: invalid hook input in strict mode ({err})");
                std::process::exit(2);
            }
            return Ok(());
        }
    };
    let Some(raw_path) = input.edited_file_path() else {
        return Ok(());
    };

    let cwd = if input.cwd.as_os_str().is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        input.cwd.clone()
    };
    let identity = resolve_board_identity(&cwd, None, None);
    let norm_path = match resolve_hook_file_path(&cwd, &identity.worktree_root, &raw_path) {
        Ok(Some(p)) => p,
        Ok(None) => return Ok(()),
        Err(err) => {
            if strict {
                eprintln!(
                    "[daybook board] BLOCKED: invalid repository path '{}' in strict mode ({err})",
                    raw_path
                );
                std::process::exit(2);
            }
            return Ok(());
        }
    };

    let config = match load_config() {
        Ok(c) => c,
        Err(err) => {
            if strict {
                eprintln!(
                    "[daybook board] BLOCKED: failed to load daybook config in strict mode ({err})"
                );
                std::process::exit(2);
            }
            return Ok(());
        }
    };
    let has_remote = config
        .push
        .url
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());

    let client = match BoardClient::from_config(&config, false) {
        Ok(c) => c,
        Err(err) => {
            if strict {
                eprintln!(
                    "[daybook board] BLOCKED: board client configuration error in strict mode ({err})"
                );
                std::process::exit(2);
            }
            if has_remote {
                eprintln!("[daybook board] warning: board client unavailable: {err}");
            }
            return Ok(());
        }
    };

    match client
        .check_path(&identity.project, &norm_path, Some(&identity.agent))
        .await
    {
        Ok(check) => {
            if check.blocked {
                for lock in &check.blocking_locks {
                    let note = if lock.note.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", lock.note)
                    };
                    eprintln!(
                        "[daybook board] WARNING: '{}' is locked by @{} [{}]{}",
                        norm_path,
                        lock.agent,
                        lock.paths.join(", "),
                        note
                    );
                }
                if strict {
                    std::process::exit(2);
                }
            }
        }
        Err(err) => {
            if strict {
                eprintln!(
                    "[daybook board] BLOCKED: remote board server unreachable in strict mode ({err})"
                );
                std::process::exit(2);
            }
            if has_remote {
                eprintln!(
                    "[daybook board] warning: could not check remote board lock for '{}': {}",
                    norm_path, err
                );
            }
        }
    }

    Ok(())
}
