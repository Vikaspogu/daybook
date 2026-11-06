use crate::board::identity::{
    discover_active_board_identities, is_valid_board_workspace, BoardIdentity,
};
use crate::config::Config;
use crate::ledger::{
    BoardAgentRow, BoardCaller, BoardCheckOutcome, BoardClaimLocksOutcome, BoardClaimLocksReq,
    BoardClaimTaskOutcome, BoardClaimTaskReq, BoardHeartbeatReq, BoardMessagesPage,
    BoardMessagesQuery, BoardPostMessageReq, BoardReleaseLocksReq, BoardStatusSnapshot,
    BoardTaskRow, BoardTouchLockReq, BoardTouchOutcome, BoardUpdateTaskReq, Ledger,
};
use crate::server::dto::{ApiResponse, BoardReleaseResponseDto};
use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::time::Duration;
pub enum BoardClient {
    Remote {
        base_url: String,
        token: String,
        http: reqwest::Client,
    },
    Local {
        ledger: Ledger,
        caller: BoardCaller,
    },
}

impl BoardClient {
    pub fn from_config(config: &Config, force_local: bool) -> Result<Self> {
        if !force_local {
            let url = config
                .push
                .url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let token = std::env::var("DAYBOOK_TOKEN")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    config
                        .push
                        .token
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                });

            if let Some(base_url) = url {
                let Some(token) = token else {
                    bail!(
                        "remote push.url '{base_url}' is configured, but push.token / DAYBOOK_TOKEN is missing; configure a token or pass --local"
                    );
                };
                let http = reqwest::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .build()
                    .context("failed to build HTTP client")?;
                return Ok(Self::Remote {
                    base_url: base_url.trim_end_matches('/').to_string(),
                    token,
                    http,
                });
            }
        }

        let ledger = Ledger::open(&config.server.db_path)?;
        let caller = BoardCaller {
            machine: crate::push::machine_label(config),
            admin: false,
        };
        Ok(Self::Local { ledger, caller })
    }

    async fn get_json<T: Serialize + DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let Self::Remote {
            base_url,
            token,
            http,
        } = self
        else {
            unreachable!()
        };
        let url = format!("{base_url}{path}");
        let resp = http
            .get(&url)
            .bearer_auth(token)
            .query(query)
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?;
        let status = resp.status();
        let parsed: ApiResponse<T> = resp
            .json()
            .await
            .with_context(|| format!("invalid JSON from GET {url} ({status})"))?;
        if !parsed.success {
            bail!(
                "{}",
                parsed
                    .error
                    .unwrap_or_else(|| format!("HTTP {status} from {url}"))
            );
        }
        parsed.data.context("missing data in response")
    }

    async fn post_json<B: Serialize, T: Serialize + DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let Self::Remote {
            base_url,
            token,
            http,
        } = self
        else {
            unreachable!()
        };
        let url = format!("{base_url}{path}");
        let resp = http
            .post(&url)
            .bearer_auth(token)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url} failed"))?;
        let status = resp.status();
        let parsed: ApiResponse<T> = resp
            .json()
            .await
            .with_context(|| format!("invalid JSON from POST {url} ({status})"))?;
        if !parsed.success {
            bail!(
                "{}",
                parsed
                    .error
                    .unwrap_or_else(|| format!("HTTP {status} from {url}"))
            );
        }
        parsed.data.context("missing data in response")
    }

    pub async fn status(
        &self,
        project: Option<&str>,
        agent: Option<&str>,
        since_id: Option<i64>,
    ) -> Result<BoardStatusSnapshot> {
        match self {
            Self::Local { ledger, .. } => ledger.board_status(project, agent, since_id),
            Self::Remote { .. } => {
                let mut q = Vec::new();
                if let Some(p) = project {
                    q.push(("project", p.to_string()));
                }
                if let Some(a) = agent {
                    q.push(("agent", a.to_string()));
                }
                if let Some(s) = since_id {
                    q.push(("since_id", s.to_string()));
                }
                self.get_json("/api/board/status", &q).await
            }
        }
    }

    pub async fn check_path(
        &self,
        project: &str,
        path: &str,
        agent: Option<&str>,
    ) -> Result<BoardCheckOutcome> {
        match self {
            Self::Local { ledger, .. } => ledger.board_check_path(project, path, agent),
            Self::Remote { .. } => {
                let mut q = vec![("project", project.to_string()), ("path", path.to_string())];
                if let Some(a) = agent {
                    q.push(("agent", a.to_string()));
                }
                self.get_json("/api/board/check", &q).await
            }
        }
    }

    pub async fn list_messages(&self, q: &BoardMessagesQuery) -> Result<BoardMessagesPage> {
        match self {
            Self::Local { ledger, .. } => ledger.board_list_messages(q),
            Self::Remote { .. } => {
                let mut params = vec![("project", q.project.clone())];
                if let Some(ref c) = q.channel {
                    params.push(("channel", c.clone()));
                }
                if let Some(ref m) = q.mention {
                    params.push(("mention", m.clone()));
                }
                if let Some(s) = q.since_id {
                    params.push(("since_id", s.to_string()));
                }
                if let Some(l) = q.limit {
                    params.push(("limit", l.to_string()));
                }
                self.get_json("/api/board/messages", &params).await
            }
        }
    }

    pub async fn heartbeat(&self, req: &BoardHeartbeatReq) -> Result<BoardAgentRow> {
        match self {
            Self::Local { ledger, caller } => ledger.board_heartbeat(caller, req),
            Self::Remote { .. } => self.post_json("/api/board/heartbeat", req).await,
        }
    }

    /// Ensure a mid-conversation agent is registered on the board with its full identity and session title.
    pub async fn ensure_registered(&self, identity: &BoardIdentity) -> bool {
        if !is_valid_board_workspace(&identity.worktree_root)
            || identity.project.eq_ignore_ascii_case("agent")
            || identity.project.eq_ignore_ascii_case("all")
        {
            return false;
        }
        let mut enriched = identity.clone();
        crate::board::identity::enrich_board_identity_from_t3(&mut enriched);
        self.heartbeat(&BoardHeartbeatReq {
            project: enriched.project,
            agent: enriched.agent,
            runtime: enriched.runtime,
            branch: enriched.branch,
            cwd: enriched.worktree_root.to_string_lossy().to_string(),
            status: None,
            current_task: enriched.session_title,
        })
        .await
        .is_ok()
    }

    /// Debounced variant for hot-path tool hooks (`PreToolUse` / `PostToolUse`): sends a heartbeat at most once per 60s per worktree agent.
    pub async fn ensure_registered_debounced(&self, identity: &BoardIdentity) {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let cursor_dir = home.join(".daybook").join("board_cursors");
        self.ensure_registered_debounced_in(identity, &cursor_dir)
            .await;
    }

    pub async fn ensure_registered_debounced_in(
        &self,
        identity: &BoardIdentity,
        cursor_dir: &std::path::Path,
    ) {
        if identity.project.eq_ignore_ascii_case("agent")
            || identity.project.eq_ignore_ascii_case("all")
            || !is_valid_board_workspace(&identity.worktree_root)
        {
            return;
        }
        let mode = if matches!(self, Self::Remote { .. }) {
            "rem"
        } else {
            "loc"
        };
        let marker = cursor_dir.join(format!(
            "{}_{}_{}_{}.hb",
            mode,
            identity.project.len(),
            identity.project,
            identity.agent
        ));
        let fresh = std::fs::metadata(&marker)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|d| d.as_secs() < 60);
        if fresh {
            return;
        }
        let _ = std::fs::create_dir_all(cursor_dir);
        let _ = std::fs::write(&marker, b"1");
        let _ = self.ensure_registered(identity).await;
    }

    /// Reconcile active mid-conversation threads/worktrees on this machine with the board
    /// for projects already tracked on the board, without overwriting existing agent status or bumping already-complete rows.
    pub async fn sync_active_sessions(&self) {
        let Ok(snap) = self.status(Some("all"), None, None).await else {
            return;
        };
        if snap.projects.is_empty() {
            return;
        }
        self.sync_active_sessions_with_snapshot(&snap, discover_active_board_identities())
            .await;
    }

    #[cfg(test)]
    pub async fn sync_active_sessions_with_identities(&self, identities: Vec<BoardIdentity>) {
        let Ok(snap) = self.status(Some("all"), None, None).await else {
            return;
        };
        self.sync_active_sessions_with_snapshot(&snap, identities)
            .await;
    }

    async fn sync_active_sessions_with_snapshot(
        &self,
        snap: &BoardStatusSnapshot,
        identities: Vec<BoardIdentity>,
    ) {
        let known_projects: std::collections::HashSet<&str> =
            snap.projects.iter().map(String::as_str).collect();
        if known_projects.is_empty() {
            return;
        }
        for id in identities {
            if !known_projects.contains(id.project.as_str()) {
                continue;
            }
            let existing = snap
                .agents
                .iter()
                .find(|a| a.project == id.project && a.agent == id.agent);
            if existing.is_some_and(|a| a.current_task.is_some() && a.branch != "unknown") {
                continue;
            }
            let preserved_status = existing
                .map(|a| a.status.clone())
                .unwrap_or_else(|| "active".to_string());
            let _ = self
                .heartbeat(&BoardHeartbeatReq {
                    project: id.project,
                    agent: id.agent,
                    runtime: id.runtime,
                    branch: id.branch,
                    cwd: id.worktree_root.to_string_lossy().to_string(),
                    status: Some(preserved_status),
                    current_task: id.session_title,
                })
                .await;
        }
    }
    pub async fn claim_locks(&self, req: &BoardClaimLocksReq) -> Result<BoardClaimLocksOutcome> {
        match self {
            Self::Local { ledger, caller } => ledger.board_claim_locks(caller, req),
            Self::Remote { .. } => self.post_json("/api/board/locks/claim", req).await,
        }
    }

    pub async fn touch_lock(&self, req: &BoardTouchLockReq) -> Result<BoardTouchOutcome> {
        match self {
            Self::Local { ledger, caller } => ledger.board_touch_lock(caller, req),
            Self::Remote { .. } => self.post_json("/api/board/locks/touch", req).await,
        }
    }

    pub async fn release_locks(&self, req: &BoardReleaseLocksReq) -> Result<usize> {
        match self {
            Self::Local { ledger, caller } => ledger.board_release_locks(caller, req),
            Self::Remote { .. } => {
                let res: BoardReleaseResponseDto =
                    self.post_json("/api/board/locks/release", req).await?;
                Ok(res.released)
            }
        }
    }

    pub async fn claim_task(&self, req: &BoardClaimTaskReq) -> Result<BoardClaimTaskOutcome> {
        match self {
            Self::Local { ledger, caller } => ledger.board_claim_task(caller, req),
            Self::Remote { .. } => self.post_json("/api/board/tasks/claim", req).await,
        }
    }

    pub async fn update_task(&self, req: &BoardUpdateTaskReq) -> Result<BoardTaskRow> {
        match self {
            Self::Local { ledger, caller } => ledger.board_update_task(caller, req),
            Self::Remote { .. } => self.post_json("/api/board/tasks/update", req).await,
        }
    }

    pub async fn post_message(
        &self,
        req: &BoardPostMessageReq,
    ) -> Result<crate::ledger::BoardMessageRow> {
        match self {
            Self::Local { ledger, caller } => ledger.board_post_message(caller, req),
            Self::Remote { .. } => self.post_json("/api/board/messages", req).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sync_active_sessions_preserves_status_and_skips_complete_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("ledger.db");
        let ledger = Ledger::open(&db).unwrap();
        let client = BoardClient::Local {
            ledger,
            caller: BoardCaller {
                machine: "test-vm".into(),
                admin: false,
            },
        };
        let wt_repo = tmp.path().join("repo");
        std::fs::create_dir_all(wt_repo.join(".git")).unwrap();

        // Seed project 'daybook' with an idle agent lacking a current_task
        client
            .heartbeat(&BoardHeartbeatReq {
                project: "daybook".into(),
                agent: "t3-idle1".into(),
                runtime: "pi".into(),
                branch: "unknown".into(),
                cwd: wt_repo.to_string_lossy().to_string(),
                status: Some("idle".into()),
                current_task: None,
            })
            .await
            .unwrap();

        // Sync discovered identity for t3-idle1 + an untracked project 'other-repo'
        client
            .sync_active_sessions_with_identities(vec![
                BoardIdentity {
                    project: "daybook".into(),
                    agent: "t3-idle1".into(),
                    runtime: "pi".into(),
                    branch: "feat/sync".into(),
                    worktree_root: wt_repo.clone(),
                    session_title: Some("Investigate board sync".into()),
                },
                BoardIdentity {
                    project: "other-repo".into(),
                    agent: "t3-other".into(),
                    runtime: "pi".into(),
                    branch: "main".into(),
                    worktree_root: wt_repo.clone(),
                    session_title: Some("Ignored untracked project".into()),
                },
            ])
            .await;

        let snap = client.status(Some("all"), None, None).await.unwrap();
        assert_eq!(snap.projects, vec!["daybook".to_string()]);
        let row = snap.agents.iter().find(|a| a.agent == "t3-idle1").unwrap();
        assert_eq!(row.status, "idle");
        assert_eq!(row.branch, "feat/sync");
        assert_eq!(row.current_task.as_deref(), Some("Investigate board sync"));
        client
            .sync_active_sessions_with_identities(vec![BoardIdentity {
                project: "daybook".into(),
                agent: "t3-idle1".into(),
                runtime: "pi".into(),
                branch: "feat/should-skip".into(),
                worktree_root: wt_repo.clone(),
                session_title: Some("Should not overwrite complete row".into()),
            }])
            .await;
        let snap_after_skip = client.status(Some("all"), None, None).await.unwrap();
        let row_after_skip = snap_after_skip
            .agents
            .iter()
            .find(|a| a.agent == "t3-idle1")
            .unwrap();
        assert_eq!(row_after_skip.branch, "feat/sync");
        assert_eq!(
            row_after_skip.current_task.as_deref(),
            Some("Investigate board sync")
        );

        //Reserved project names are rejected by ensure_registered
        assert!(
            !client
                .ensure_registered(&BoardIdentity {
                    project: "ALL".into(),
                    agent: "t3-1".into(),
                    runtime: "pi".into(),
                    branch: "main".into(),
                    worktree_root: wt_repo.clone(),
                    session_title: None,
                })
                .await
        );

        // Debounced registration writes marker and suppresses repeat calls within 60s
        let cursor_dir = tmp.path().join("cursors");
        let valid_id = BoardIdentity {
            project: "daybook".into(),
            agent: "t3-debounced".into(),
            runtime: "pi".into(),
            branch: "feat/debounced".into(),
            worktree_root: wt_repo,
            session_title: Some("Debounced session".into()),
        };
        client
            .ensure_registered_debounced_in(&valid_id, &cursor_dir)
            .await;
        assert!(std::fs::read_dir(&cursor_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.path().extension().and_then(|s| s.to_str()) == Some("hb")));
        let mut second_id = valid_id.clone();
        second_id.branch = "feat/suppressed-within-60s".into();
        client
            .ensure_registered_debounced_in(&second_id, &cursor_dir)
            .await;
        let snap2 = client.status(Some("daybook"), None, None).await.unwrap();
        let debounced_row = snap2
            .agents
            .iter()
            .find(|a| a.agent == "t3-debounced")
            .unwrap();
        assert_eq!(debounced_row.branch, "feat/debounced");
    }
}
