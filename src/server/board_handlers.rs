use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use std::sync::Arc;

use crate::ledger::{
    BoardCaller, BoardClaimLocksReq, BoardClaimTaskReq, BoardHeartbeatReq, BoardMessagesQuery,
    BoardPostMessageReq, BoardReleaseLocksReq, BoardTouchLockReq, BoardUpdateTaskReq,
};

use super::dto::{ApiResponse, BoardCheckQuery, BoardReleaseResponseDto, BoardStatusQuery};
use super::handlers::{AppState, Authed};
fn caller_from_authed(authed: &Authed) -> BoardCaller {
    BoardCaller {
        machine: authed.label.clone(),
        admin: authed.admin,
    }
}

fn board_err_response(err: anyhow::Error) -> Response {
    let msg = err.to_string();
    let status = if msg.contains("is owned by") {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, Json(ApiResponse::<()>::error(&msg))).into_response()
}

/// GET /api/board/status
pub async fn get_board_status(
    State(state): State<Arc<AppState>>,
    Query(q): Query<BoardStatusQuery>,
) -> Response {
    match state
        .ledger
        .board_status(q.project.as_deref(), q.agent.as_deref(), q.since_id)
    {
        Ok(snapshot) => (StatusCode::OK, Json(ApiResponse::success(snapshot))).into_response(),
        Err(e) => board_err_response(e),
    }
}

/// GET /api/board/messages
pub async fn get_board_messages(
    State(state): State<Arc<AppState>>,
    Query(q): Query<BoardMessagesQuery>,
) -> Response {
    match state.ledger.board_list_messages(&q) {
        Ok(page) => (StatusCode::OK, Json(ApiResponse::success(page))).into_response(),
        Err(e) => board_err_response(e),
    }
}

/// GET /api/board/check
pub async fn get_board_check(
    State(state): State<Arc<AppState>>,
    Query(q): Query<BoardCheckQuery>,
) -> Response {
    match state
        .ledger
        .board_check_path(&q.project, &q.path, q.agent.as_deref())
    {
        Ok(res) => (StatusCode::OK, Json(ApiResponse::success(res))).into_response(),
        Err(e) => board_err_response(e),
    }
}

/// GET /api/board/agents.md
pub async fn get_board_agents_md() -> Response {
    const GUIDE: &str = r#"# Daybook Multi-Agent Coordination Board (`daybook board`)

Coordinate across parallel worktrees and coding agents (T3 Code, Claude Code, Codex, Pi) on the same project.

## Core Commands

```bash
# 1. Check active agents, file locks, tasks, and unread @mentions
daybook board status

# 2. Claim file globs before editing shared areas
daybook board claim "src/server/**" "web/src/pages/Board.tsx" -m "Adding board API"

# 3. Claim or update a task
daybook board task claim BOARD-1 --title "Add board API" --priority high
daybook board task done BOARD-1 --pr https://github.com/org/repo/pull/42 --release-locks

# 4. Post handoffs, blockers, or @mentions across worktrees
daybook board post --channel handoff "@pi-t3-fb61e20a Router changes committed on branch t3/auth"

# 5. Release locks when finished
daybook board release --all
```

## MCP Server

Register `daybook board mcp` as an MCP stdio server to expose:
- `daybook_board_status`
- `daybook_board_claim`
- `daybook_board_release`
- `daybook_board_check_path`
- `daybook_board_post`
- `daybook_board_claim_task`
- `daybook_board_complete_task`
"#;
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        GUIDE,
    )
        .into_response()
}

/// POST /api/board/heartbeat
pub async fn post_board_heartbeat(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<Authed>,
    Json(req): Json<BoardHeartbeatReq>,
) -> Response {
    let caller = caller_from_authed(&authed);
    match state.ledger.board_heartbeat(&caller, &req) {
        Ok(row) => (StatusCode::OK, Json(ApiResponse::success(row))).into_response(),
        Err(e) => board_err_response(e),
    }
}

/// POST /api/board/locks/claim
pub async fn post_board_locks_claim(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<Authed>,
    Json(req): Json<BoardClaimLocksReq>,
) -> Response {
    let caller = caller_from_authed(&authed);
    match state.ledger.board_claim_locks(&caller, &req) {
        Ok(res) => (StatusCode::OK, Json(ApiResponse::success(res))).into_response(),
        Err(e) => board_err_response(e),
    }
}

/// POST /api/board/locks/touch
pub async fn post_board_locks_touch(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<Authed>,
    Json(req): Json<BoardTouchLockReq>,
) -> Response {
    let caller = caller_from_authed(&authed);
    match state.ledger.board_touch_lock(&caller, &req) {
        Ok(res) => (StatusCode::OK, Json(ApiResponse::success(res))).into_response(),
        Err(e) => board_err_response(e),
    }
}

/// POST /api/board/locks/release
pub async fn post_board_locks_release(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<Authed>,
    Json(req): Json<BoardReleaseLocksReq>,
) -> Response {
    let caller = caller_from_authed(&authed);
    match state.ledger.board_release_locks(&caller, &req) {
        Ok(released) => (
            StatusCode::OK,
            Json(ApiResponse::success(BoardReleaseResponseDto { released })),
        )
            .into_response(),
        Err(e) => board_err_response(e),
    }
}

/// POST /api/board/tasks/claim
pub async fn post_board_tasks_claim(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<Authed>,
    Json(req): Json<BoardClaimTaskReq>,
) -> Response {
    let caller = caller_from_authed(&authed);
    match state.ledger.board_claim_task(&caller, &req) {
        Ok(res) => (StatusCode::OK, Json(ApiResponse::success(res))).into_response(),
        Err(e) => board_err_response(e),
    }
}

/// POST /api/board/tasks/update
pub async fn post_board_tasks_update(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<Authed>,
    Json(req): Json<BoardUpdateTaskReq>,
) -> Response {
    let caller = caller_from_authed(&authed);
    match state.ledger.board_update_task(&caller, &req) {
        Ok(res) => (StatusCode::OK, Json(ApiResponse::success(res))).into_response(),
        Err(e) => board_err_response(e),
    }
}

/// POST /api/board/messages
pub async fn post_board_messages(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<Authed>,
    Json(req): Json<BoardPostMessageReq>,
) -> Response {
    let caller = caller_from_authed(&authed);
    match state.ledger.board_post_message(&caller, &req) {
        Ok(res) => (StatusCode::OK, Json(ApiResponse::success(res))).into_response(),
        Err(e) => board_err_response(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::ledger::Ledger;
    use crate::server::create_router;
    use crate::usage::pricing::PricingData;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::RwLock;
    use tower::ServiceExt;

    fn test_app() -> (axum::Router, String, String, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("board.db")).unwrap();
        let vm1_token = "dbk_vm1_secret".to_string();
        let vm2_token = "dbk_vm2_secret".to_string();
        ledger
            .mint_token("vm1", &crate::ledger::sha256_hex(vm1_token.as_bytes()))
            .unwrap();
        ledger
            .mint_token("vm2", &crate::ledger::sha256_hex(vm2_token.as_bytes()))
            .unwrap();
        let mut cfg = Config::default();
        cfg.server.admin_token = Some("adm_secret".into());
        let state = Arc::new(AppState {
            config: RwLock::new(cfg),
            pricing: PricingData::from_map(std::collections::HashMap::new()),
            ledger,
        });
        (create_router(state), vm1_token, vm2_token, dir)
    }

    #[tokio::test]
    async fn board_routes_require_auth_and_enforce_machine_ownership() {
        let (app, vm1_token, vm2_token, _dir) = test_app();

        // Unauthenticated GET /api/board/status -> 401
        let unauth = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/board/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

        // Claim lock as vm1 / agent-a
        let claim_body = serde_json::json!({
            "project": "daybook",
            "agent": "agent-a",
            "paths": ["src/server/**"],
            "note": "API work"
        });
        let claim_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/board/locks/claim")
                    .header("authorization", format!("Bearer {vm1_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(claim_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(claim_resp.status(), StatusCode::OK);

        // vm2 attempting to impersonate agent-a -> 403 FORBIDDEN
        let spoof_body = serde_json::json!({
            "project": "daybook",
            "agent": "agent-a",
            "body": "spoof"
        });
        let spoof_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/board/messages")
                    .header("authorization", format!("Bearer {vm2_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(spoof_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(spoof_resp.status(), StatusCode::FORBIDDEN);

        // Check path from vm2 / agent-b -> blocked == true
        let check_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/board/check?project=daybook&path=src/server/router.rs&agent=agent-b")
                    .header("authorization", format!("Bearer {vm2_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(check_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(check_resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["data"]["blocked"], serde_json::Value::Bool(true));
    }
}
