use super::board::*;
use super::Ledger;
use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

const MAX_NOTICE_ATTEMPTS: i64 = 5;

#[derive(Debug, thiserror::Error)]
#[error("invalid session credential")]
pub struct InvalidSessionCredential;

#[derive(Debug, thiserror::Error)]
pub enum SessionAuthorizationError {
    #[error("session credential required for sender")]
    Required,
    #[error("sender does not match session credential")]
    SenderMismatch,
}

impl InvalidSessionCredential {
    pub const CODE: &'static str = "invalid_session_credential";
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BoardSession {
    pub id: String,
    pub project: String,
    pub handle: String,
    pub credential: String,
}

#[derive(Serialize, Deserialize)]
pub struct BoardSessionRegistration {
    #[serde(flatten)]
    pub presence: BoardHeartbeatReq,
    pub machine: String,
}

#[derive(Serialize, Deserialize)]
pub struct BoardDelivery {
    pub message_id: i64,
    pub sender: String,
    pub preview: String,
    pub delivered_at: Option<String>,
    pub acked_at: Option<String>,
    pub attempts: i64,
}

#[derive(Serialize, Deserialize)]
pub struct BoardFullMessage {
    #[serde(flatten)]
    pub message: BoardMessageRow,
    pub sender_session_id: Option<String>,
    pub legacy: bool,
}

fn session_handle(prefix: &str, id: &str) -> String {
    let suffix: String = id
        .strip_prefix("ses_")
        .unwrap_or(id)
        .chars()
        .take(16)
        .collect();
    format!("{}-{suffix}", prefix.chars().take(45).collect::<String>())
}

impl Ledger {
    pub fn board_register_session(&self, req: &BoardSessionRegistration) -> Result<BoardSession> {
        self.board_register_session_with_ids(req, || {
            Ok(super::generate_secret()?.replace("dbk_", "ses_"))
        })
    }

    fn board_register_session_with_ids(
        &self,
        req: &BoardSessionRegistration,
        mut next_id: impl FnMut() -> Result<String>,
    ) -> Result<BoardSession> {
        use crate::board::identity::{validate_agent_handle, validate_identifier};
        let project = validate_identifier(&req.presence.project, "project")?;
        let prefix = validate_agent_handle(&req.presence.agent)?;
        let runtime = validate_identifier(&req.presence.runtime, "runtime")?;
        let cwd = super::board::validate_note(&req.presence.cwd)?;
        let branch = super::board::validate_note(&req.presence.branch)?;
        let machine = super::board::validate_note(&req.machine)?;
        let title = req
            .presence
            .current_task
            .as_deref()
            .map(super::board::validate_note)
            .transpose()?;
        let credential = super::generate_secret()?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut id = next_id()?;
        let mut handle = session_handle(&prefix, &id);
        for attempt in 0..2 {
            let occupied: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM board_agents WHERE project=?1 AND agent=?2 UNION ALL SELECT 1 FROM board_session_handles WHERE project=?1 AND handle=?2)",
                params![project, handle], |r| r.get(0),
            )?;
            if !occupied {
                break;
            }
            anyhow::ensure!(attempt == 0, "session handle collision after retry");
            id = next_id()?;
            handle = session_handle(&prefix, &id);
        }
        let now = chrono::Utc::now();
        super::board::prune_board_tx(&tx, &project, now.timestamp())?;
        super::board::ensure_and_touch_agent_tx(
            &tx,
            &BoardCaller {
                machine: machine.clone(),
                admin: false,
            },
            &project,
            &handle,
            &now.to_rfc3339(),
            now.timestamp(),
        )?;
        tx.execute("INSERT INTO board_sessions (id, project, handle, credential_hash, machine) VALUES (?1,?2,?3,?4,?5)", params![id, project, handle, super::sha256_hex(credential.as_bytes()), machine])?;
        tx.execute(
            "INSERT INTO board_session_handles (project,handle) VALUES (?1,?2)",
            params![project, handle],
        )?;
        tx.execute("UPDATE board_agents SET runtime=?1,cwd=?2,branch=?3,current_task=?4 WHERE project=?5 AND agent=?6", params![runtime, cwd, branch, title, project, handle])?;
        tx.commit()?;
        Ok(BoardSession {
            id,
            project,
            handle,
            credential,
        })
    }

    pub fn board_session(&self, credential: &str) -> Result<BoardSession> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT id, project, handle FROM board_sessions WHERE credential_hash=?1",
                [super::sha256_hex(credential.as_bytes())],
                |r| {
                    Ok(BoardSession {
                        id: r.get(0)?,
                        project: r.get(1)?,
                        handle: r.get(2)?,
                        credential: credential.into(),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| InvalidSessionCredential.into())
    }

    pub fn board_session_status(&self, credential: &str, status: &str) -> Result<()> {
        anyhow::ensure!(
            matches!(status, "active" | "idle"),
            "invalid session status"
        );
        let session = self.board_session(credential)?;
        let now = chrono::Utc::now();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if status == "active" {
            let machine: String = tx.query_row(
                "SELECT machine FROM board_sessions WHERE id=?1",
                [&session.id],
                |r| r.get(0),
            )?;
            check_agent_capacity(
                &tx,
                &session.project,
                &machine,
                &session.handle,
                now.timestamp(),
            )?;
        }
        // Upsert: pruning may have removed the presence row of a retained session.
        tx.execute(
            "INSERT INTO board_agents (project,agent,machine,runtime,cwd,branch,status,current_task,last_seen_utc,last_seen_epoch)
             SELECT project,handle,machine,'cli','','',?1,NULL,?3,?2 FROM board_sessions WHERE id=?4
             ON CONFLICT(project,agent) DO UPDATE SET status=?1,last_seen_epoch=?2,last_seen_utc=?3",
            params![status, now.timestamp(), now.to_rfc3339(), session.id],
        )?;
        tx.execute(
            "UPDATE board_sessions SET ended_epoch=?1 WHERE id=?2",
            params![
                if status == "idle" {
                    Some(now.timestamp())
                } else {
                    None
                },
                session.id
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn board_session_post(
        &self,
        credential: &str,
        req: &BoardPostMessageReq,
    ) -> Result<BoardMessageRow> {
        let session = self.board_session(credential)?;
        if req.project != session.project || (!req.agent.is_empty() && req.agent != session.handle)
        {
            return Err(SessionAuthorizationError::SenderMismatch.into());
        }
        let body = req.body.trim();
        if body.is_empty()
            || body.len() > crate::board::identity::MAX_BODY_LEN * 4
            || body.chars().count() > crate::board::identity::MAX_BODY_LEN
        {
            bail!("invalid message length");
        }
        let channel = super::board::validate_channel(req.channel.as_deref())?;
        let now = chrono::Utc::now();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        super::board::prune_board_tx(&tx, &session.project, now.timestamp())?;
        let machine: String = tx.query_row(
            "SELECT machine FROM board_sessions WHERE id=?1",
            [&session.id],
            |r| r.get(0),
        )?;
        let msg = super::board::post_message_in_tx(
            &tx,
            &session.project,
            &channel,
            &session.handle,
            &machine,
            body,
            req.parent_id,
            &now.to_rfc3339(),
            now.timestamp(),
        )?;
        tx.execute(
            "INSERT INTO board_message_auth VALUES (?1,?2)",
            params![msg.id, session.id],
        )?;
        tx.commit()?;
        Ok(msg)
    }

    // A crashed adapter can retry after 30 seconds, up to five notice attempts.
    pub fn board_inbox(&self, credential: &str, notices: bool) -> Result<Vec<BoardDelivery>> {
        let session = self.board_session(credential)?;
        let mut conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now();
        let stale: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM board_agents WHERE project=?1 AND agent=?2 AND last_seen_epoch < ?3)",
            params![session.project, session.handle, now.timestamp() - 60],
            |r| r.get(0),
        )?;
        if stale {
            let tx = conn.transaction()?;
            let (machine, status): (String, String) = tx.query_row(
                "SELECT machine,status FROM board_agents WHERE project=?1 AND agent=?2",
                params![session.project, session.handle],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if status != "idle" && status != "done" {
                check_agent_capacity(
                    &tx,
                    &session.project,
                    &machine,
                    &session.handle,
                    now.timestamp(),
                )?;
            }
            tx.execute("UPDATE board_agents SET last_seen_epoch=?1, last_seen_utc=?2 WHERE project=?3 AND agent=?4 AND last_seen_epoch < ?5", params![now.timestamp(), now.to_rfc3339(), session.project, session.handle, now.timestamp() - 60])?;
            tx.commit()?;
        }
        let read_deliveries = |conn: &rusqlite::Connection| -> Result<Vec<BoardDelivery>> {
            let mut stmt = conn.prepare("SELECT d.message_id, d.delivered_at, d.acked_at, d.attempts, m.agent, m.body FROM board_deliveries d JOIN board_messages m ON m.id=d.message_id WHERE d.recipient_session_id=?1 AND d.acked_at IS NULL AND (?2=0 OR (d.delivered_at IS NULL AND d.attempts < ?4 AND (d.attempted_epoch IS NULL OR d.attempted_epoch <= ?3))) ORDER BY d.message_id LIMIT CASE WHEN ?2 THEN 5 ELSE -1 END")?;
            let deliveries = stmt
                .query_map(
                    params![
                        session.id,
                        notices,
                        now.timestamp() - 30,
                        MAX_NOTICE_ATTEMPTS
                    ],
                    |r| {
                        Ok(BoardDelivery {
                            message_id: r.get(0)?,
                            delivered_at: r.get(1)?,
                            acked_at: r.get(2)?,
                            attempts: r.get(3)?,
                            sender: r.get(4)?,
                            preview: crate::board::session::peer_preview(&r.get::<_, String>(5)?),
                        })
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(deliveries)
        };
        let deliveries = read_deliveries(&conn)?;
        if !notices || deliveries.is_empty() {
            return Ok(deliveries);
        }
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Another boundary may have claimed these notices since the plain read.
        let mut deliveries = read_deliveries(&tx)?;
        for d in &mut deliveries {
            tx.execute("UPDATE board_deliveries SET attempts=attempts+1, attempted_epoch=?1 WHERE message_id=?2 AND recipient_session_id=?3", params![now.timestamp(), d.message_id, session.id])?;
            d.attempts += 1;
        }
        tx.commit()?;
        Ok(deliveries)
    }

    pub fn board_receipt(&self, credential: &str, message_id: i64, ack: bool) -> Result<()> {
        let session = self.board_session(credential)?;
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(if ack {
            "UPDATE board_deliveries SET acked_at=COALESCE(acked_at,?1) WHERE message_id=?2 AND recipient_session_id=?3"
        } else {
            "UPDATE board_deliveries SET delivered_at=COALESCE(delivered_at,?1) WHERE message_id=?2 AND recipient_session_id=?3"
        }, params![chrono::Utc::now().to_rfc3339(), message_id, session.id])?;
        if changed == 0 && !ack {
            bail!("message is not addressed to this session");
        }
        Ok(())
    }

    pub fn board_message(&self, project: &str, id: i64) -> Result<BoardFullMessage> {
        let conn = self.conn.lock().unwrap();
        let message = conn.query_row("SELECT id,project,channel,agent,machine,body,mentions_json,parent_id,created_utc,created_epoch FROM board_messages WHERE project=?1 AND id=?2", params![project,id], |r| Ok(BoardMessageRow { id:r.get(0)?, project:r.get(1)?, channel:r.get(2)?, agent:r.get(3)?, machine:r.get(4)?, body:r.get(5)?, mentions:serde_json::from_str(&r.get::<_,String>(6)?).unwrap_or_default(), parent_id:r.get(7)?, created_utc:r.get(8)?, created_epoch:r.get(9)? }))?;
        let sender_session_id: Option<String> = conn
            .query_row(
                "SELECT sender_session_id FROM board_message_auth WHERE message_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(BoardFullMessage {
            legacy: sender_session_id.is_none(),
            sender_session_id,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_handle_tolerates_changed_id_formats() {
        assert_eq!(
            session_handle("agent", "ses_0123456789abcdefmore"),
            "agent-0123456789abcdef"
        );
        assert_eq!(session_handle("agent", "x"), "agent-x");
        assert_eq!(session_handle("agent", ""), "agent-");
        assert_eq!(session_handle("agent", "ses_é🙂"), "agent-é🙂");
    }

    #[test]
    fn board_sessions_registration_retries_handle_collision_once() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        let req = registration("/same");
        let first = ledger
            .board_register_session_with_ids(&req, || Ok("ses_first".into()))
            .unwrap();
        let mut ids = ["ses_first", "ses_second"].into_iter();
        let second = ledger
            .board_register_session_with_ids(&req, || Ok(ids.next().unwrap().into()))
            .unwrap();
        assert_ne!(first.handle, second.handle);
        assert_eq!(second.handle, "claude-second");
        assert!(ids.next().is_none());
        let mut attempts = 0;
        let error = ledger
            .board_register_session_with_ids(&req, || {
                attempts += 1;
                Ok("ses_first".into())
            })
            .err()
            .unwrap();
        assert!(error.to_string().contains("collision after retry"));
        assert_eq!(attempts, 2);
        ledger
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM board_agents WHERE agent=?1", [&first.handle])
            .unwrap();
        let mut ids = ["ses_first", "ses_third"].into_iter();
        let third = ledger
            .board_register_session_with_ids(&req, || Ok(ids.next().unwrap().into()))
            .unwrap();
        assert_eq!(third.handle, "claude-third");
    }

    fn registration(cwd: &str) -> BoardSessionRegistration {
        BoardSessionRegistration {
            machine: "machine-one".into(),
            presence: BoardHeartbeatReq {
                project: "repo".into(),
                agent: "claude".into(),
                runtime: "claude".into(),
                cwd: cwd.into(),
                branch: "main".into(),
                status: None,
                current_task: None,
            },
        }
    }
    fn post(a: &BoardSession, b: &BoardSession) -> BoardPostMessageReq {
        BoardPostMessageReq {
            project: a.project.clone(),
            agent: a.handle.clone(),
            channel: None,
            body: format!("@{} full message\nwith details", b.handle),
            parent_id: None,
        }
    }

    #[test]
    fn board_sessions_prune_preserves_live_presence() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        let live = ledger
            .board_register_session(&registration("/live"))
            .unwrap();
        let ended = ledger
            .board_register_session(&registration("/ended"))
            .unwrap();
        ledger
            .board_session_status(&ended.credential, "idle")
            .unwrap();
        {
            let mut conn = ledger.conn.lock().unwrap();
            conn.execute("UPDATE board_agents SET last_seen_epoch=0", [])
                .unwrap();
            let tx = conn.transaction().unwrap();
            prune_board_tx(&tx, "repo", chrono::Utc::now().timestamp()).unwrap();
            tx.commit().unwrap();
        }
        let snap = ledger.board_status(Some("repo"), None, None).unwrap();
        assert!(snap.agents.iter().any(|a| a.agent == live.handle));
        assert!(!snap.agents.iter().any(|a| a.agent == ended.handle));
        ledger.board_inbox(&live.credential, false).unwrap();
        ledger
            .board_session_status(&live.credential, "active")
            .unwrap();
        let snap = ledger.board_status(Some("repo"), None, None).unwrap();
        assert!(snap
            .agents
            .iter()
            .any(|a| a.agent == live.handle && a.last_seen_epoch > 0));
        // Reactivating a retained session whose presence row was pruned restores it.
        ledger
            .board_session_status(&ended.credential, "active")
            .unwrap();
        let snap = ledger.board_status(Some("repo"), None, None).unwrap();
        assert!(snap
            .agents
            .iter()
            .any(|a| a.agent == ended.handle && a.status == "active"));
    }

    #[test]
    fn board_sessions_reactivation_enforces_machine_and_project_caps() {
        for project_cap in [false, true] {
            for stale in [false, true] {
                let tmp = tempfile::tempdir().unwrap();
                let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
                let session = ledger
                    .board_register_session(&registration("/idle"))
                    .unwrap();
                if stale {
                    ledger
                        .conn
                        .lock()
                        .unwrap()
                        .execute("UPDATE board_agents SET last_seen_epoch=0", [])
                        .unwrap();
                } else {
                    ledger
                        .board_session_status(&session.credential, "idle")
                        .unwrap();
                }
                let cap = if project_cap {
                    MAX_AGENTS_PER_PROJECT
                } else {
                    MAX_AGENTS_PER_MACHINE_PROJECT
                };
                {
                    let mut conn = ledger.conn.lock().unwrap();
                    let tx = conn.transaction().unwrap();
                    for n in 0..cap {
                        let machine = if project_cap {
                            format!("machine-{n}")
                        } else {
                            "machine-one".into()
                        };
                        tx.execute("INSERT INTO board_agents (project,agent,machine,runtime,last_seen_utc,last_seen_epoch) VALUES ('repo',?1,?2,'cli','now',?3)", params![format!("legacy-{n}"), machine, chrono::Utc::now().timestamp()]).unwrap();
                    }
                    tx.commit().unwrap();
                }
                let error = ledger
                    .board_session_status(&session.credential, "active")
                    .unwrap_err();
                assert!(error
                    .to_string()
                    .contains(&format!("agent handle cap ({cap})")));
                if stale {
                    assert!(ledger.board_inbox(&session.credential, false).is_err());
                }
                let conn = ledger.conn.lock().unwrap();
                let (status, seen): (String, i64) = conn
                    .query_row(
                        "SELECT status,last_seen_epoch FROM board_agents WHERE agent=?1",
                        [&session.handle],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap();
                assert!(if stale { seen == 0 } else { status == "idle" });
                conn.execute("DELETE FROM board_agents WHERE agent='legacy-0'", [])
                    .unwrap();
                drop(conn);
                ledger
                    .board_session_status(&session.credential, "active")
                    .unwrap();
            }
        }
    }

    #[test]
    fn board_sessions_plain_inbox_returns_more_than_five_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        let a = ledger.board_register_session(&registration("/a")).unwrap();
        let b = ledger.board_register_session(&registration("/b")).unwrap();
        for _ in 0..7 {
            ledger
                .board_session_post(&a.credential, &post(&a, &b))
                .unwrap();
        }
        assert_eq!(ledger.board_inbox(&b.credential, false).unwrap().len(), 7);
        assert_eq!(ledger.board_inbox(&b.credential, true).unwrap().len(), 5);
        assert_eq!(ledger.board_inbox(&b.credential, false).unwrap().len(), 7);
    }

    #[test]
    fn board_sessions_deliver_fetch_ack_and_isolate_same_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        let a = ledger
            .board_register_session(&registration("/worktrees/a"))
            .unwrap();
        let b = ledger
            .board_register_session(&registration("/worktrees/b"))
            .unwrap();
        let c = ledger
            .board_register_session(&registration("/worktrees/b"))
            .unwrap();
        assert_ne!(a.id, b.id);
        assert_ne!(b.handle, c.handle);
        assert_ne!(b.credential, c.credential);
        let m = ledger
            .board_session_post(&a.credential, &post(&a, &b))
            .unwrap();
        assert_eq!(m.agent, a.handle);
        assert!(ledger.board_inbox(&a.credential, true).unwrap().is_empty());
        assert!(ledger.board_inbox(&c.credential, true).unwrap().is_empty());
        let inbox = ledger.board_inbox(&b.credential, true).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].message_id, m.id);
        assert_eq!(inbox[0].attempts, 1);
        let inbox =
            serde_json::to_value(ledger.board_inbox(&b.credential, false).unwrap()).unwrap();
        assert_eq!(inbox[0]["sender"], a.handle);
        assert_eq!(
            inbox[0]["preview"],
            crate::board::session::peer_preview(&post(&a, &b).body)
        );
        assert!(ledger.board_inbox(&b.credential, true).unwrap().is_empty());
        let full = ledger.board_message("repo", m.id).unwrap();
        assert_eq!(full.message, m);
        assert_eq!(full.sender_session_id, Some(a.id.clone()));
        assert!(!full.legacy);
        assert_eq!(ledger.board_message("repo", m.id).unwrap().message, m);
        assert!(ledger.board_message("other", m.id).is_err());
        let pending = ledger.board_inbox(&b.credential, false).unwrap();
        assert!(pending[0].acked_at.is_none());
        assert!(pending[0].delivered_at.is_none());
        ledger.board_receipt(&b.credential, m.id, false).unwrap();
        assert!(ledger.board_inbox(&b.credential, true).unwrap().is_empty());
        assert!(ledger.board_inbox(&b.credential, false).unwrap()[0]
            .delivered_at
            .is_some());
        // A missing receipt is harmless and cannot acknowledge another recipient's row.
        ledger.board_receipt(&c.credential, m.id, true).unwrap();
        assert!(ledger.board_inbox(&b.credential, false).unwrap()[0]
            .acked_at
            .is_none());
        ledger.board_receipt(&b.credential, m.id, true).unwrap();
        ledger.board_receipt(&b.credential, m.id, true).unwrap();
        assert!(ledger.board_inbox(&b.credential, false).unwrap().is_empty());
        let count: i64 = ledger
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM board_deliveries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn board_sessions_failed_adapter_retries_without_inventing_ack() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        let a = ledger.board_register_session(&registration("/a")).unwrap();
        let b = ledger.board_register_session(&registration("/b")).unwrap();
        let m = ledger
            .board_session_post(&a.credential, &post(&a, &b))
            .unwrap();
        ledger.board_inbox(&b.credential, true).unwrap();
        // Simulate a crashed adapter before acceptance and expiration of its lease.
        ledger
            .conn
            .lock()
            .unwrap()
            .execute("UPDATE board_deliveries SET attempted_epoch=0", [])
            .unwrap();
        let retry = ledger.board_inbox(&b.credential, true).unwrap();
        assert_eq!(retry[0].message_id, m.id);
        assert_eq!(retry[0].attempts, 2);
        assert!(retry[0].acked_at.is_none());
        // A crash after acceptance must not repeat the notice or fabricate an acknowledgment.
        ledger.board_receipt(&b.credential, m.id, false).unwrap();
        ledger
            .conn
            .lock()
            .unwrap()
            .execute("UPDATE board_deliveries SET attempted_epoch=0", [])
            .unwrap();
        assert!(ledger.board_inbox(&b.credential, true).unwrap().is_empty());
        assert_eq!(ledger.board_inbox(&b.credential, false).unwrap().len(), 1);
    }

    #[test]
    fn board_sessions_notice_attempts_are_capped_without_removing_pending_message() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        let a = ledger.board_register_session(&registration("/a")).unwrap();
        let b = ledger.board_register_session(&registration("/b")).unwrap();
        let m = ledger
            .board_session_post(&a.credential, &post(&a, &b))
            .unwrap();
        for attempt in 1..=5 {
            let notice = ledger.board_inbox(&b.credential, true).unwrap();
            assert_eq!(notice.len(), 1);
            assert_eq!(notice[0].attempts, attempt);
            ledger
                .conn
                .lock()
                .unwrap()
                .execute("UPDATE board_deliveries SET attempted_epoch=0", [])
                .unwrap();
        }
        assert!(ledger.board_inbox(&b.credential, true).unwrap().is_empty());
        let pending = ledger.board_inbox(&b.credential, false).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].message_id, m.id);
        assert_eq!(pending[0].attempts, 5);
        assert!(pending[0].acked_at.is_none());
        assert!(pending[0].delivered_at.is_none());
        assert_eq!(ledger.board_message("repo", m.id).unwrap().message, m);
    }

    #[test]
    fn board_sessions_fresh_inbox_does_not_take_write_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("board.db");
        let ledger = Ledger::open(&path).unwrap();
        let a = ledger.board_register_session(&registration("/a")).unwrap();
        let b = ledger.board_register_session(&registration("/b")).unwrap();
        let blocker = rusqlite::Connection::open(&path).unwrap();
        ledger
            .conn
            .lock()
            .unwrap()
            .execute_batch("PRAGMA busy_timeout=0;")
            .unwrap();
        for state in ["empty", "leased", "exhausted", "delivered"] {
            if state == "leased" {
                ledger
                    .board_session_post(&a.credential, &post(&a, &b))
                    .unwrap();
                assert_eq!(ledger.board_inbox(&b.credential, true).unwrap().len(), 1);
            } else if state == "exhausted" {
                blocker
                    .execute(
                        "UPDATE board_deliveries SET attempted_epoch=0,attempts=5",
                        [],
                    )
                    .unwrap();
            } else if state == "delivered" {
                blocker
                    .execute(
                        "UPDATE board_deliveries SET attempts=1,delivered_at='accepted'",
                        [],
                    )
                    .unwrap();
            }
            blocker.execute_batch("BEGIN IMMEDIATE;").unwrap();
            assert!(ledger.board_inbox(&b.credential, true).unwrap().is_empty());
            assert_eq!(
                ledger.board_inbox(&b.credential, false).unwrap().len(),
                usize::from(state != "empty")
            );
            blocker.execute_batch("ROLLBACK;").unwrap();
        }
    }

    #[test]
    fn board_sessions_inbox_refreshes_only_stale_presence_without_activating() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        let b = ledger.board_register_session(&registration("/b")).unwrap();
        ledger.board_session_status(&b.credential, "idle").unwrap();
        let now = chrono::Utc::now().timestamp();
        for (epoch, stale) in [(now - 30, false), (now - 61, true)] {
            ledger
                .conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE board_agents SET last_seen_epoch=?1,last_seen_utc='unchanged' WHERE agent=?2",
                    params![epoch, b.handle],
                )
                .unwrap();
            assert!(ledger.board_inbox(&b.credential, true).unwrap().is_empty());
            let snapshot = ledger.board_status(Some("repo"), None, None).unwrap();
            let agent = snapshot
                .agents
                .iter()
                .find(|a| a.agent == b.handle)
                .unwrap();
            assert_eq!(agent.status, "idle");
            if stale {
                assert!(agent.last_seen_epoch >= now);
                assert_ne!(agent.last_seen_utc, "unchanged");
            } else {
                assert_eq!(agent.last_seen_epoch, epoch);
                assert_eq!(agent.last_seen_utc, "unchanged");
            }
        }
    }

    #[test]
    fn board_sessions_reject_spoofing_and_protect_pending_from_pruning() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        let a = ledger
            .board_register_session(&registration("/same"))
            .unwrap();
        let b = ledger
            .board_register_session(&registration("/same"))
            .unwrap();
        let req = post(&a, &b);
        assert!(ledger.board_session_post("wrong", &req).is_err());
        assert!(ledger.board_session_post(&b.credential, &req).is_err());
        assert!(ledger
            .board_post_message(&BoardCaller::local(), &req)
            .is_err());
        let mut req = req;
        req.agent.clear();
        let m = ledger.board_session_post(&a.credential, &req).unwrap();
        assert_eq!(m.agent, a.handle);
        for _ in 0..505 {
            let mut req = post(&a, &a);
            req.body = "ordinary message".into();
            ledger.board_session_post(&a.credential, &req).unwrap();
        }
        assert!(ledger.board_message("repo", m.id).is_ok());
        ledger.board_receipt(&b.credential, m.id, true).unwrap();
        ledger
            .board_session_post(&a.credential, &post(&a, &a))
            .unwrap();
        assert!(ledger.board_message("repo", m.id).is_err());
    }

    #[test]
    fn board_sessions_ack_after_pruning_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        let a = ledger.board_register_session(&registration("/a")).unwrap();
        let b = ledger.board_register_session(&registration("/b")).unwrap();
        let m = ledger
            .board_session_post(&a.credential, &post(&a, &b))
            .unwrap();
        ledger.board_receipt(&b.credential, m.id, true).unwrap();
        ledger
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM board_deliveries WHERE message_id=?1", [m.id])
            .unwrap();
        ledger.board_receipt(&b.credential, m.id, true).unwrap();
        ledger
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM board_messages WHERE id=?1", [m.id])
            .unwrap();
        ledger.board_receipt(&b.credential, m.id, true).unwrap();
        ledger.board_receipt(&b.credential, m.id, true).unwrap();
        assert!(ledger.board_receipt("wrong", m.id, true).is_err());
    }

    #[test]
    fn board_sessions_pending_protection_survives_age_and_session_end() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        let a = ledger.board_register_session(&registration("/a")).unwrap();
        let b = ledger.board_register_session(&registration("/b")).unwrap();
        let c = ledger.board_register_session(&registration("/c")).unwrap();
        let mut legacy_req = post(&a, &b);
        legacy_req.agent = "legacy".into();
        legacy_req.body = "legacy without deliveries".into();
        let legacy = ledger
            .board_post_message(&BoardCaller::local(), &legacy_req)
            .unwrap();
        let old = ledger
            .board_session_post(&a.credential, &post(&a, &b))
            .unwrap();
        let ended = ledger
            .board_session_post(&a.credential, &post(&a, &c))
            .unwrap();
        let recent = ledger
            .board_session_post(&a.credential, &post(&a, &b))
            .unwrap();
        ledger.board_session_status(&c.credential, "idle").unwrap();
        let now = chrono::Utc::now().timestamp();
        ledger
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE board_messages SET created_epoch=?1 WHERE id=?2",
                params![now - 14 * 86_400, old.id],
            )
            .unwrap();
        for _ in 0..505 {
            let mut req = post(&a, &a);
            req.body = "ordinary message".into();
            ledger.board_session_post(&a.credential, &req).unwrap();
        }
        assert!(ledger.board_message("repo", old.id).is_ok());
        assert!(ledger.board_message("repo", ended.id).is_ok());
        assert!(ledger.board_message("repo", recent.id).is_ok());
        assert!(ledger.board_message("repo", legacy.id).is_err());
        ledger.board_receipt(&b.credential, old.id, true).unwrap();
        ledger.board_receipt(&c.credential, ended.id, true).unwrap();
        ledger
            .board_session_post(&a.credential, &post(&a, &a))
            .unwrap();
        assert!(ledger.board_message("repo", old.id).is_err());
        assert!(ledger.board_message("repo", ended.id).is_err());
        assert!(ledger.board_message("repo", recent.id).is_ok());
    }

    #[test]
    fn board_sessions_prune_old_ended_without_pending_deliveries() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        let a = ledger.board_register_session(&registration("/a")).unwrap();
        let b = ledger.board_register_session(&registration("/b")).unwrap();
        let c = ledger.board_register_session(&registration("/c")).unwrap();
        let d = ledger.board_register_session(&registration("/d")).unwrap();
        let authored = ledger
            .board_session_post(&b.credential, &post(&b, &b))
            .unwrap();
        let pending = ledger
            .board_session_post(&a.credential, &post(&a, &c))
            .unwrap();
        let acked = ledger
            .board_session_post(&a.credential, &post(&a, &b))
            .unwrap();
        ledger.board_receipt(&b.credential, acked.id, true).unwrap();
        ledger.board_session_status(&b.credential, "idle").unwrap();
        ledger.board_session_status(&c.credential, "idle").unwrap();
        ledger.board_session_status(&d.credential, "idle").unwrap();
        // Age ended sessions without adding a sleep to the test.
        let now = chrono::Utc::now().timestamp();
        {
            let conn = ledger.conn.lock().unwrap();
            conn.execute(
                "UPDATE board_sessions SET ended_epoch=?1 WHERE id IN (?2,?3)",
                params![now - 14 * 86_400 - 1, b.id, c.id],
            )
            .unwrap();
            conn.execute(
                "UPDATE board_agents SET last_seen_epoch=?1 WHERE agent IN (?2,?3)",
                params![now - 14 * 86_400 - 1, b.handle, c.handle],
            )
            .unwrap();
        }
        ledger
            .board_session_post(&a.credential, &post(&a, &a))
            .unwrap();
        // Ended session C still has pending delivery. B has only an acknowledged one.
        assert!(ledger.board_session(&b.credential).is_err());
        let mut spoof = post(&b, &a);
        spoof.body = "impersonate pruned session".into();
        for admin in [false, true] {
            assert!(
                ledger
                    .board_post_message(
                        &BoardCaller {
                            machine: "machine-one".into(),
                            admin,
                        },
                        &spoof
                    )
                    .is_err(),
                "pruned session handle accepted without credential"
            );
        }
        assert!(ledger.board_session(&c.credential).is_ok());
        assert!(ledger.board_session(&d.credential).is_ok());
        assert!(ledger.board_session(&a.credential).is_ok());
        assert_eq!(
            ledger
                .board_message("repo", authored.id)
                .unwrap()
                .sender_session_id,
            Some(b.id.clone())
        );
        assert!(!ledger.board_message("repo", authored.id).unwrap().legacy);
        ledger
            .board_receipt(&c.credential, pending.id, true)
            .unwrap();
        ledger
            .board_session_post(&a.credential, &post(&a, &a))
            .unwrap();
        assert!(ledger.board_session(&c.credential).is_err());
    }

    #[test]
    fn board_sessions_migrate_legacy_without_delivery_or_ack() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("board.db");
        let ledger = Ledger::open(&path).unwrap();
        let req = BoardPostMessageReq {
            project: "repo".into(),
            agent: "old-agent".into(),
            channel: None,
            body: "@future legacy".into(),
            parent_id: None,
        };
        let m = ledger
            .board_post_message(&BoardCaller::local(), &req)
            .unwrap();
        ledger.conn.lock().unwrap().execute_batch("DROP TABLE board_deliveries; DROP TABLE board_message_auth; DROP TABLE board_sessions;").unwrap();
        drop(ledger);
        let ledger = Ledger::open(&path).unwrap();
        let legacy = ledger.board_message("repo", m.id).unwrap();
        assert!(legacy.legacy);
        assert!(legacy.sender_session_id.is_none());
        assert_eq!(legacy.message.id, m.id);
        let b = ledger.board_register_session(&registration("/b")).unwrap();
        assert!(ledger.board_inbox(&b.credential, false).unwrap().is_empty());
        assert_eq!(
            ledger
                .board_list_messages(&BoardMessagesQuery {
                    project: "repo".into(),
                    ..Default::default()
                })
                .unwrap()
                .messages[0]
                .id,
            m.id
        );
        ledger
            .board_post_message(&BoardCaller::local(), &req)
            .unwrap();
    }
    #[test]
    fn board_sessions_parallel_boundaries_claim_only_one_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("board.db");
        let ledger = Ledger::open(&path).unwrap();
        let a = ledger.board_register_session(&registration("/a")).unwrap();
        let b = ledger.board_register_session(&registration("/b")).unwrap();
        ledger
            .board_session_post(&a.credential, &post(&a, &b))
            .unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let worker = Ledger::open(&path).unwrap();
            let barrier = barrier.clone();
            let key = b.credential.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                worker.board_inbox(&key, true).unwrap().len()
            }));
        }
        let count: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
        assert_eq!(count, 1);
        assert_eq!(
            ledger.board_inbox(&b.credential, false).unwrap()[0].attempts,
            1
        );
    }
    #[test]
    fn board_sessions_ended_sessions_do_not_exhaust_live_presence_capacity() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&tmp.path().join("board.db")).unwrap();
        for _ in 0..30 {
            let session = ledger
                .board_register_session(&registration("/same"))
                .unwrap();
            ledger
                .board_session_status(&session.credential, "idle")
                .unwrap();
        }
        let session = ledger
            .board_register_session(&registration("/same"))
            .unwrap();
        ledger
            .board_post_message(
                &BoardCaller {
                    machine: "machine-one".into(),
                    admin: false,
                },
                &BoardPostMessageReq {
                    project: "repo".into(),
                    agent: "legacy-cli".into(),
                    channel: None,
                    body: format!("@{} legacy CLI still works", session.handle),
                    parent_id: None,
                },
            )
            .unwrap();
        assert_eq!(
            ledger
                .board_inbox(&session.credential, false)
                .unwrap()
                .len(),
            1
        );
    }
}
