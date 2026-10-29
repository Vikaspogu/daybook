use axum::{
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use super::handlers::{self, AppState};
use super::static_files::serve_static;

/// Create the main router with all routes
pub fn create_router(state: Arc<AppState>) -> Router {
    // Every /api route sits behind auth_middleware (S5): queries need the
    // same bearer token as ingest, including from the browser.
    let api_routes = Router::new()
        .route("/dates", get(handlers::list_dates))
        .route("/dates/:date", get(handlers::get_daily_summary))
        .route("/dates/:date/insights", get(handlers::get_date_insights))
        .route("/dates/:date/sessions", get(handlers::list_sessions))
        .route("/dates/:date/sessions/:name", get(handlers::get_session))
        .route("/config", get(handlers::get_config))
        .route(
            "/config/templates/defaults",
            get(handlers::get_default_templates),
        )
        .route("/insights", get(handlers::get_insights))
        .route("/stats", get(handlers::get_stats))
        // Token management: admin bearer only (machine tokens get 403).
        .route(
            "/tokens",
            get(handlers::list_tokens_api).post(handlers::mint_token_api),
        )
        .route("/tokens/revoke", post(handlers::revoke_token_api))
        .route("/ingest", post(handlers::ingest))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            handlers::auth_middleware,
        ));

    // Liveness probe stays open: it names the server, not the data.
    let health_routes = Router::new().route("/health", get(handlers::health_check));

    Router::new()
        .nest("/api", api_routes.merge(health_routes))
        .fallback_service(serve_static())
        .with_state(state)
}
