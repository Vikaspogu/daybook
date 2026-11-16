use crate::board::{client::BoardClient, identity::BoardIdentity};
use crate::config::Config;
use crate::ledger::{BoardHeartbeatReq, BoardSession, BoardSessionRegistration};
use anyhow::Result;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

pub fn is_codex_hook() -> bool {
    std::env::var("DAYBOOK_RUNTIME").ok().as_deref() == Some("codex")
}

/// Pi env vars leak into child runtimes, so a Codex hook launched from Pi is not Pi.
pub fn is_pi() -> bool {
    !is_codex_hook() && std::env::var_os("PI_SESSION_ID").is_some()
}

pub fn runtime_session_id() -> String {
    session_id_from(is_codex_hook(), |name| std::env::var(name).ok())
}

fn session_id_from(codex_hook: bool, get: impl Fn(&str) -> Option<String>) -> String {
    let names: &[&str] = if codex_hook {
        // Never bind a parent Pi/Claude/override session from a nested Codex hook.
        &["CODEX_THREAD_ID"]
    } else {
        &[
            "DAYBOOK_SESSION_ID",
            "PI_SESSION_ID",
            "CLAUDE_SESSION_ID",
            "CODEX_THREAD_ID",
        ]
    };
    names
        .iter()
        .find_map(|name| get(name).filter(|s| !s.is_empty()))
        .unwrap_or_default()
}

pub async fn session_for(
    client: &BoardClient,
    config: &Config,
    identity: &BoardIdentity,
    runtime_id: &str,
) -> Result<BoardSession> {
    session_in(
        client,
        config,
        identity,
        runtime_id,
        &config.storage_path().join("board_sessions"),
    )
    .await
}

pub async fn session_in(
    client: &BoardClient,
    config: &Config,
    identity: &BoardIdentity,
    runtime_id: &str,
    dir: &Path,
) -> Result<BoardSession> {
    anyhow::ensure!(!runtime_id.is_empty(), "runtime session_id is required");
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    let file = cache_file(client, config, identity, runtime_id, dir);
    if let Some(session) = cached_session(&file)? {
        return Ok(session);
    }
    let lock = file.with_extension("lock");
    // A killed hook must not leave registration locked forever.
    if fs::metadata(&lock)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|d| d.as_secs() >= 15)
    {
        let _ = fs::remove_file(&lock);
    }
    let _guard = RegistrationLock::new(lock)?;
    if let Some(session) = cached_session(&file)? {
        return Ok(session);
    }
    let session = client
        .register_session(&BoardSessionRegistration {
            presence: BoardHeartbeatReq {
                project: identity.project.clone(),
                agent: identity.agent.clone(),
                runtime: identity.runtime.clone(),
                branch: identity.branch.clone(),
                cwd: identity.worktree_root.to_string_lossy().into_owned(),
                status: None,
                current_task: identity.session_title.clone(),
            },
            machine: crate::push::machine_label(config),
        })
        .await?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let tmp = file.with_extension(format!("{}.tmp", std::process::id()));
    let mut out = options.open(&tmp)?;
    use std::io::Write;
    out.write_all(&serde_json::to_vec(&session)?)?;
    fs::rename(tmp, file)?;
    Ok(session)
}

fn cache_file(
    client: &BoardClient,
    config: &Config,
    identity: &BoardIdentity,
    runtime_id: &str,
    dir: &Path,
) -> PathBuf {
    let endpoint = match client {
        BoardClient::Remote {
            base_url, token, ..
        } => format!("{base_url}:{}", crate::ledger::sha256_hex(token.as_bytes())),
        BoardClient::Local { .. } => config.server.db_path.to_string_lossy().into_owned(),
    };
    let key = crate::ledger::sha256_hex(
        format!("{endpoint}:{}:{runtime_id}", identity.project).as_bytes(),
    );
    dir.join(key)
}

fn cached_session(file: &Path) -> Result<Option<BoardSession>> {
    let Ok(text) = fs::read_to_string(file) else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_str(&text)?))
}

pub async fn with_session<T, F, Fut>(
    client: &BoardClient,
    config: &Config,
    identity: &BoardIdentity,
    runtime_id: &str,
    mut operation: F,
) -> Result<(BoardSession, T)>
where
    F: FnMut(BoardSession) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let session = session_for(client, config, identity, runtime_id).await?;
    match operation(session.clone()).await {
        Ok(value) => Ok((session, value)),
        Err(error) if error.is::<crate::ledger::InvalidSessionCredential>() => {
            let file = cache_file(
                client,
                config,
                identity,
                runtime_id,
                &config.storage_path().join("board_sessions"),
            );
            fs::remove_file(file)?;
            let session = session_for(client, config, identity, runtime_id).await?;
            let value = operation(session.clone()).await?;
            Ok((session, value))
        }
        Err(error) => Err(error),
    }
}

// Only hooks update this binding. MCP arguments cannot replace the inherited identity.
pub fn bind_runtime(
    config: &Config,
    identity: &BoardIdentity,
    inherited: &str,
    live: &str,
) -> Result<()> {
    if inherited.is_empty() {
        return Ok(());
    }
    let file = runtime_binding_file(config, identity, inherited);
    let dir = file.parent().unwrap();
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    let tmp = file.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&tmp, live)?;
    fs::rename(tmp, file)?;
    Ok(())
}

pub fn live_runtime_id(
    config: &Config,
    identity: &BoardIdentity,
    inherited: &str,
) -> Result<String> {
    match fs::read_to_string(runtime_binding_file(config, identity, inherited)) {
        Ok(live) => {
            anyhow::ensure!(!live.trim().is_empty(), "empty live runtime binding");
            Ok(live)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(inherited.into()),
        Err(error) => Err(error.into()),
    }
}

fn runtime_binding_file(config: &Config, identity: &BoardIdentity, inherited: &str) -> PathBuf {
    let key = crate::ledger::sha256_hex(
        format!(
            "{:?}:{}:{}:{inherited}",
            config.push.url,
            config.server.db_path.display(),
            identity.project
        )
        .as_bytes(),
    );
    config
        .storage_path()
        .join("board_runtime_bindings")
        .join(key)
}

struct RegistrationLock(PathBuf);
impl RegistrationLock {
    fn new(path: PathBuf) -> Result<Self> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self(path))
    }
}
impl Drop for RegistrationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

// JSON quoting also escapes newlines. Escape markup and Unicode controls before quoting.
pub fn peer_preview(text: &str) -> String {
    let escaped: String = text.chars().take(160).flat_map(|c| {
        if c.is_control() || matches!(c, '<' | '>' | '&' | '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
            c.escape_unicode().collect::<Vec<_>>()
        } else { vec![c] }
    }).collect();
    serde_json::to_string(&escaped).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_codex_hook_ignores_parent_session_ids() {
        let env = |name: &str| {
            Some(
                match name {
                    "DAYBOOK_SESSION_ID" => "override",
                    "PI_SESSION_ID" => "pi",
                    "CLAUDE_SESSION_ID" => "claude",
                    "CODEX_THREAD_ID" => "codex",
                    _ => return None,
                }
                .to_string(),
            )
        };
        assert_eq!(session_id_from(true, env), "codex");
        assert_eq!(session_id_from(false, env), "override");
        assert_eq!(
            session_id_from(true, |n| (n == "PI_SESSION_ID").then(|| "pi".into())),
            ""
        );
    }

    #[tokio::test]
    async fn board_session_cache_reregisters_only_for_invalid_session_signal() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        for (status, code, invalid) in [
            (403, Some("invalid_session_credential"), true),
            (401, None, false),
            (403, None, false),
            (403, Some("proxy_denied"), false),
            (503, None, false),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let registrations = Arc::new(AtomicUsize::new(0));
            let count = registrations.clone();
            let app = axum::Router::new()
                .route("/api/board/sessions", axum::routing::post(move || {
                    let id = count.fetch_add(1, Ordering::SeqCst);
                    async move {
                        axum::Json(crate::server::dto::ApiResponse::success(BoardSession {
                            id: format!("session-{id}"), project: "repo".into(),
                            handle: format!("agent-{id}"), credential: format!("credential-{id}"),
                        }))
                    }
                }))
                .route("/api/board/inbox", axum::routing::get(move || async move {
                    (axum::http::StatusCode::from_u16(status).unwrap(), axum::Json(serde_json::json!({"success":false,"error":"server rejected request","code":code})))
                }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut config = Config::default();
            config.storage.path = tmp.path().join("storage");
            config.push.url = Some(format!("http://{}", listener.local_addr().unwrap()));
            config.push.token = Some("owner".into());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let client = BoardClient::from_config(&config, false).unwrap();
            let identity =
                crate::board::identity::resolve_board_identity_for_hook(tmp.path(), "runtime");
            let _first = session_for(&client, &config, &identity, "runtime")
                .await
                .unwrap();
            let second = with_session(&client, &config, &identity, "runtime", |session| {
                let client = &client;
                async move { client.inbox(&session, true).await }
            })
            .await;
            if !invalid {
                assert!(second.is_err());
                assert_eq!(registrations.load(Ordering::SeqCst), 1);
            } else {
                assert!(second.is_err());
                assert_eq!(registrations.load(Ordering::SeqCst), 2);
            }
            let files = std::fs::read_dir(config.storage_path().join("board_sessions"))
                .unwrap()
                .collect::<std::io::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(files.len(), 1);
            let cached: BoardSession =
                serde_json::from_slice(&std::fs::read(files[0].path()).unwrap()).unwrap();
            assert_eq!(cached.id, if !invalid { "session-0" } else { "session-1" });
            server.abort();
        }
    }

    #[tokio::test]
    async fn board_session_cache_recovers_after_server_session_disappears() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.storage.path = tmp.path().join("storage");
        config.server.db_path = tmp.path().join("board.db");
        let identity =
            crate::board::identity::resolve_board_identity_for_hook(tmp.path(), "runtime");
        let client = BoardClient::from_config(&config, true).unwrap();
        let first = session_for(&client, &config, &identity, "runtime")
            .await
            .unwrap();
        let again = session_for(&client, &config, &identity, "runtime")
            .await
            .unwrap();
        assert_eq!(first.id, again.id);
        rusqlite::Connection::open(&config.server.db_path)
            .unwrap()
            .execute("DELETE FROM board_sessions WHERE id=?1", [&first.id])
            .unwrap();
        let (recovered, _) = with_session(&client, &config, &identity, "runtime", |session| {
            let client = &client;
            async move { client.inbox(&session, true).await }
        })
        .await
        .unwrap();
        assert_ne!(first.id, recovered.id);
        let cached = session_for(&client, &config, &identity, "runtime")
            .await
            .unwrap();
        assert_eq!(cached.id, recovered.id);
        assert_eq!(
            std::fs::read_dir(config.storage_path().join("board_sessions"))
                .unwrap()
                .count(),
            1
        );
        client.inbox(&cached, false).await.unwrap();
        let connection = rusqlite::Connection::open(&config.server.db_path).unwrap();
        connection
            .execute(
                "ALTER TABLE board_deliveries RENAME TO unavailable_deliveries",
                [],
            )
            .unwrap();
        assert!(
            with_session(&client, &config, &identity, "runtime", |session| {
                let client = &client;
                async move { client.inbox(&session, true).await }
            })
            .await
            .is_err()
        );
        connection
            .execute(
                "ALTER TABLE unavailable_deliveries RENAME TO board_deliveries",
                [],
            )
            .unwrap();
        let preserved = session_for(&client, &config, &identity, "runtime")
            .await
            .unwrap();
        assert_eq!(preserved.id, cached.id);
    }
}
