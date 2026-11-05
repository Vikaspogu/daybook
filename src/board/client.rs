use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::time::Duration;

use crate::config::Config;
use crate::ledger::{
    BoardAgentRow, BoardCaller, BoardCheckOutcome, BoardClaimLocksOutcome, BoardClaimLocksReq,
    BoardClaimTaskOutcome, BoardClaimTaskReq, BoardHeartbeatReq, BoardMessagesPage,
    BoardMessagesQuery, BoardPostMessageReq, BoardReleaseLocksReq, BoardStatusSnapshot,
    BoardTaskRow, BoardTouchLockReq, BoardTouchOutcome, BoardUpdateTaskReq, Ledger,
};
use crate::server::dto::{ApiResponse, BoardReleaseResponseDto};

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
