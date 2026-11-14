use anyhow::Result;
use std::path::PathBuf;

use crate::board::client::BoardClient;
use crate::board::globs::resolve_hook_file_path;
use crate::board::identity::resolve_board_identity_for_hook;
use crate::config::load_config;
use crate::hooks::read_hook_input;
use crate::ledger::BoardTouchLockReq;

/// Handle PostToolUse hook: auto-claim/refresh a 30m file lock on the edited path
/// and surface any unread @mentions from other worktrees.
pub async fn handle() -> Result<()> {
    let input = match read_hook_input() {
        Ok(i) => i,
        Err(_) => return Ok(()),
    };
    let Some(raw_path) = input.edited_file_path() else {
        return Ok(());
    };

    let Ok(config) = load_config() else {
        return Ok(());
    };
    let Ok(client) = BoardClient::from_config(&config, false) else {
        return Ok(());
    };

    let cwd = if input.cwd.as_os_str().is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        input.cwd.clone()
    };
    let identity = resolve_board_identity_for_hook(&cwd, &input.session_id);
    let Ok(Some(norm_path)) = resolve_hook_file_path(&cwd, &identity.worktree_root, &raw_path)
    else {
        return Ok(());
    };

    let cursor_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".daybook")
        .join("board_cursors");
    let cursor_file = cursor_dir.join(format!("{}__{}.cursor", identity.project, identity.agent));
    let since_id = std::fs::read_to_string(&cursor_file)
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok());

    if let Ok(outcome) = client
        .touch_lock(&BoardTouchLockReq {
            project: identity.project,
            agent: identity.agent.clone(),
            path: norm_path,
            ttl_minutes: Some(30),
            since_id,
        })
        .await
    {
        for m in &outcome.unread_mentions {
            eprintln!(
                "[daybook board] @{} mentioned you in [{}] (#{}): {}",
                m.agent, m.channel, m.id, m.body
            );
        }
        if outcome.next_since_id > 0 {
            let _ = std::fs::create_dir_all(&cursor_dir);
            let _ = std::fs::write(&cursor_file, outcome.next_since_id.to_string());
        }
    }

    Ok(())
}
