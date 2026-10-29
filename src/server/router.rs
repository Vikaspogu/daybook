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
    // Read-only routes for the web dashboard
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
        .route("/health", get(handlers::health_check))
        .route("/insights", get(handlers::get_insights));

    // Ingest: reject unauthorized requests before the body is parsed, and
    // cap the body below axum's 2 MiB default so floods can't buffer much.
    let ingest_routes = Router::new()
        .route("/ingest", post(handlers::ingest))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            handlers::auth_middleware,
        ));

    Router::new()
        .nest("/api", api_routes.merge(ingest_routes))
        .fallback_service(serve_static())
        .with_state(state)
}
