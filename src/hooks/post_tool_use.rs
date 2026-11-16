use crate::board::{
    client::BoardClient,
    globs::resolve_hook_file_path,
    identity::resolve_board_identity_for_hook,
    session::{peer_preview, session_for},
};
use crate::config::load_config;
use crate::hooks::{board_notice, read_hook_input};
use crate::ledger::BoardTouchLockReq;
use anyhow::Result;

pub async fn handle() -> Result<()> {
    let Ok(input) = read_hook_input() else {
        return Ok(());
    };
    let Ok(config) = load_config() else {
        return Ok(());
    };
    let cwd = if input.cwd.as_os_str().is_empty() {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    } else {
        input.cwd.clone()
    };
    if input.board_delivered.is_some() || input.board_legacy_cursor.is_some() {
        if let Some(ids) = input.board_delivered.filter(|ids| !ids.is_empty()) {
            board_notice::accept_ids(&config, &cwd, &input.session_id, ids);
        }
        if let Some(id) = input.board_legacy_cursor.filter(|id| *id > 0) {
            board_notice::advance_legacy_cursor(&legacy_cursor_file(&cwd, &input.session_id), id)?;
        }
        return Ok(());
    }
    let event = if input.hook_event_name == "PostToolUseFailure" {
        "PostToolUseFailure"
    } else {
        "PostToolUse"
    };
    if input.board_idle {
        let notice = board_notice::prepare_idle(&config, &cwd, &input.session_id);
        return board_notice::emit(
            &config,
            notice,
            &board_notice::LegacyNotice::default(),
            event,
        );
    }
    let (notice, legacy) = board_notice::prepare_tool_context(
        &config,
        &cwd,
        &input.session_id,
        input
            .edited_file_path()
            .filter(|_| event != "PostToolUseFailure"),
    );
    board_notice::emit(&config, notice, &legacy, event)
}

pub(super) async fn legacy_edit_notice(
    config: &crate::config::Config,
    cwd: &std::path::Path,
    runtime_id: &str,
    raw_path: &str,
    known_session: Option<crate::ledger::BoardSession>,
) -> board_notice::LegacyNotice {
    let identity = resolve_board_identity_for_hook(cwd, runtime_id);
    let Ok(Some(path)) = resolve_hook_file_path(cwd, &identity.worktree_root, raw_path) else {
        return board_notice::LegacyNotice::default();
    };
    let Ok(client) = BoardClient::from_config(config, false) else {
        return board_notice::LegacyNotice::default();
    };
    let cursor_file = legacy_cursor_file(&identity.worktree_root, runtime_id);
    // ponytail: new per-session cursors start from the old shared worktree cursor.
    let since_id = std::fs::read_to_string(&cursor_file)
        .or_else(|_| {
            std::fs::read_to_string(shared_legacy_cursor_file(
                &identity.worktree_root,
                runtime_id,
            ))
        })
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok());
    let Ok(outcome) = client
        .touch_lock(&BoardTouchLockReq {
            project: identity.project.clone(),
            agent: identity.agent.clone(),
            path,
            ttl_minutes: Some(30),
            since_id,
        })
        .await
    else {
        return board_notice::LegacyNotice::default();
    };
    let session = if known_session.is_some() || outcome.unread_mentions.is_empty() {
        known_session
    } else {
        session_for(&client, config, &identity, runtime_id)
            .await
            .ok()
    };
    let mut text = String::new();
    for m in outcome.unread_mentions {
        // Session-addressed messages use the inbox, including earlier or leased notices.
        if session
            .as_ref()
            .is_some_and(|s| m.mentions.contains(&s.handle))
        {
            continue;
        }
        if text.is_empty() {
            text.push_str(board_notice::PEER_NOTICE_HEADER);
        }
        text.push_str(&format!(
            "#{} from @{}: {}\n",
            m.id,
            m.agent,
            peer_preview(&m.body)
        ));
    }
    let cursor = (outcome.next_since_id > 0).then_some((cursor_file, outcome.next_since_id));
    if text.is_empty() {
        // Nothing to show, so nothing awaits acceptance: advance now to avoid re-scanning.
        if let Some((file, id)) = &cursor {
            let _ = board_notice::advance_legacy_cursor(file, *id);
        }
        return board_notice::LegacyNotice::default();
    }
    board_notice::LegacyNotice { text, cursor }
}

fn shared_legacy_cursor_file(cwd: &std::path::Path, runtime_id: &str) -> std::path::PathBuf {
    let identity = resolve_board_identity_for_hook(cwd, runtime_id);
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".daybook/board_cursors")
        .join(format!("{}__{}.cursor", identity.project, identity.agent))
}

/// Sibling sessions in one worktree share an agent name, so each keeps its own cursor.
fn legacy_cursor_file(cwd: &std::path::Path, runtime_id: &str) -> std::path::PathBuf {
    let shared = shared_legacy_cursor_file(cwd, runtime_id);
    let key = crate::ledger::sha256_hex(runtime_id.as_bytes());
    shared.with_extension(format!("{}.cursor", &key[..12]))
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn legacy_cursor_is_per_session_with_shared_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let a = super::legacy_cursor_file(tmp.path(), "session-a");
        let b = super::legacy_cursor_file(tmp.path(), "session-b");
        let shared = super::shared_legacy_cursor_file(tmp.path(), "session-a");
        assert_ne!(a, b);
        assert_ne!(a, shared);
        assert_eq!(a.parent(), shared.parent());
        assert!(a.to_string_lossy().ends_with(".cursor"));
    }

    #[test]
    fn post_tool_prepare_and_lock_share_one_deadline() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        use std::time::{Duration, Instant};
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let touched = Arc::new(AtomicBool::new(false));
        let called = touched.clone();
        let server = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let app = axum::Router::new()
                    .route("/api/board/sessions", axum::routing::post(|| async {
                        axum::Json(serde_json::json!({"success":true,"data":{"id":"session","project":"repo","handle":"agent-new","credential":"credential"}}))
                    }))
                    .route("/api/board/inbox", axum::routing::get(|| async {
                        tokio::time::sleep(Duration::from_millis(450)).await;
                        axum::Json(serde_json::json!({"success":true,"data":[]}))
                    }))
                    .route("/api/board/locks/touch", axum::routing::post(move || {
                        let called = called.clone();
                        async move {
                            called.store(true, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(600)).await;
                            axum::Json(serde_json::json!({"success":false,"error":"slow lock"}))
                        }
                    }));
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let _ = tokio::time::timeout(Duration::from_secs(3), async { axum::serve(listener, app).await }).await;
            });
        });
        let mut config = crate::config::Config::default();
        config.storage.path = tmp.path().join("storage");
        config.push.url = Some(format!("http://{address}"));
        config.push.token = Some("owner".into());
        let started = Instant::now();
        let _ = crate::hooks::board_notice::prepare_tool_context(
            &config,
            &repo,
            "runtime",
            Some("file.rs".into()),
        );
        let elapsed = started.elapsed();
        assert!(
            touched.load(Ordering::SeqCst),
            "test must reach the lock request"
        );
        assert!(elapsed < Duration::from_millis(950), "elapsed: {elapsed:?}");
        server.join().unwrap();
    }

    #[test]
    fn post_tool_hook_tolerates_missing_cwd() {
        const CHILD_DIR: &str = "DAYBOOK_MISSING_CWD_TEST_DIR";
        if let Some(dir) = std::env::var_os(CHILD_DIR) {
            let dir = std::path::PathBuf::from(dir);
            crate::config::set_config_path(Some(dir.join("config.toml")));
            std::fs::remove_dir(std::env::current_dir().unwrap()).unwrap();
            assert!(std::env::current_dir().is_err());
            crate::config::load_config().unwrap();
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(super::handle())
                .unwrap();
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), "").unwrap();
        let cwd = tmp.path().join("vanished");
        std::fs::create_dir(&cwd).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hooks::post_tool_use::tests::post_tool_hook_tolerates_missing_cwd",
                "--nocapture",
            ])
            .env(CHILD_DIR, tmp.path())
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"cwd":"","tool_name":"Read"}"#)
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
