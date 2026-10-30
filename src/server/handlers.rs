use axum::{
    extract::{Extension, Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::{Arc, RwLock};

use crate::config::Config;
use crate::insights::collector::InsightsData;
use crate::insights::daily::DateInsights;
use crate::ledger::{Ledger, SessionRecord, UpsertOutcome};
use crate::summarizer::Prompts;
use crate::usage::pricing::PricingData;

use super::dto::*;

/// Shared application state
pub struct AppState {
    pub config: RwLock<Config>,
    pub pricing: PricingData,
    pub ledger: Ledger,
}

/// Bearer-token identity attached by `auth_middleware`.
#[derive(Clone)]
pub struct Authed {
    pub label: String,
    pub admin: bool,
}

/// Rejects requests without a valid bearer token before axum buffers and
/// parses the JSON body. Machine tokens are checked against the ledger
/// (last_used_at stamped); the admin token from server config is checked
/// by equality (read from a local config file, not a DB row).
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let secret = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    // Empty configured admin token means "no admin configured", never a
    // match for an empty bearer header. Compare SHA-256 digests so response
    // timing cannot leak the operator-chosen secret's length or prefix.
    let admin_digest = state
        .config
        .read()
        .ok()
        .and_then(|c| c.server.admin_token.clone())
        .filter(|t| !t.is_empty())
        .map(|t| crate::ledger::sha256_hex(t.as_bytes()));
    let authed = match secret {
        Some(s)
            if admin_digest
                .as_ref()
                .is_some_and(|d| *d == crate::ledger::sha256_hex(s.as_bytes())) =>
        {
            Some(Authed {
                label: "admin".into(),
                admin: true,
            })
        }
        Some(s) => match state.ledger.check_token(s) {
            // DB errors propagate: a broken ledger must not masquerade as 401.
            Ok(Some(label)) => Some(Authed {
                label,
                admin: false,
            }),
            Ok(None) => None,
            Err(e) => {
                eprintln!("[daybook] token check failed: {e:#}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse::<()>::error("internal server error")),
                )
                    .into_response();
            }
        },
        None => None,
    };
    match authed {
        Some(a) => {
            req.extensions_mut().insert(a);
            next.run(req).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<()>::error("invalid or missing token")),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct StatsQuery {
    pub days: Option<usize>,
}

/// GET /api/stats?days=N — aggregate report over the ledger window.
/// Token-gated like ingest: query APIs need the same auth (S4 gate, S5
/// extends it to the browser).
pub async fn get_stats(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<StatsQuery>,
) -> Response {
    // ponytail: hard cap at 90 days; paginated queries if the ledger grows
    // past what one response should carry.
    let days = q.days.unwrap_or(7).clamp(1, 90);
    match state.ledger.stat_rows(days) {
        Ok(rows) => (
            StatusCode::OK,
            Json(ApiResponse::success(crate::ledger::aggregate_stats(&rows))),
        )
            .into_response(),
        Err(e) => {
            eprintln!("[daybook] stats query failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::error("internal server error")),
            )
                .into_response()
        }
    }
}

/// POST /api/ingest — append or refresh a session record pushed by a
/// client machine. Runs behind `auth_middleware` and the 1 MiB body cap
/// set in the router.
pub async fn ingest(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<Authed>,
    Json(record): Json<SessionRecord>,
) -> impl IntoResponse {
    if let Err(msg) = validate_record(&record) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<&str>::error(msg)),
        );
    }
    // Containment: a token minted for one machine must not write rows for another.
    if !authed.admin && authed.label != record.machine {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<&str>::error(
                "token label does not match record.machine",
            )),
        );
    }
    match state.ledger.upsert_session(&record) {
        Ok(UpsertOutcome::RejectedStale) => (
            StatusCode::CONFLICT,
            Json(ApiResponse::<&str>::error(
                "stale version: a newer record already exists",
            )),
        ),
        Ok(outcome) => (
            StatusCode::OK,
            // Callers can log whether this was a fresh insert, an update, or a repeat.
            Json(ApiResponse::success(outcome.as_str())),
        ),
        Err(e) => {
            // Internal detail stays in the server log; the client sees a generic 500.
            eprintln!("[daybook] ingest failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<&str>::error("internal server error")),
            )
        }
    }
}

// ---- Token management (admin only) ----

#[derive(serde::Serialize)]
pub struct TokenRow {
    pub label: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked: bool,
}

#[derive(serde::Deserialize)]
pub struct MintTokenBody {
    pub label: String,
}

#[derive(serde::Deserialize)]
pub struct RevokeTokenBody {
    pub label: String,
}

fn admin_forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ApiResponse::<&str>::error("admin token required")),
    )
        .into_response()
}

/// GET /api/tokens — list machine tokens with last-used and revoked state.
pub async fn list_tokens_api(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<Authed>,
) -> Response {
    if !authed.admin {
        return admin_forbidden();
    }
    match state.ledger.list_tokens() {
        Ok(rows) => (
            StatusCode::OK,
            Json(ApiResponse::success(
                rows.into_iter()
                    .map(|(label, created_at, last_used_at, revoked)| TokenRow {
                        label,
                        created_at,
                        last_used_at: if last_used_at.is_empty() {
                            None
                        } else {
                            Some(last_used_at)
                        },
                        revoked,
                    })
                    .collect::<Vec<_>>(),
            )),
        )
            .into_response(),
        Err(e) => {
            eprintln!("[daybook] token list failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::error("internal server error")),
            )
                .into_response()
        }
    }
}

/// POST /api/tokens {label} — mint a machine token. The secret is
/// returned exactly once; only its hash is stored.
pub async fn mint_token_api(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<Authed>,
    Json(body): Json<MintTokenBody>,
) -> Response {
    if !authed.admin {
        return admin_forbidden();
    }
    let label = body.label.trim().to_string();
    if label.is_empty() || label.len() > 64 || label.contains(':') {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<&str>::error(
                "label must be 1-64 chars and not contain ':'",
            )),
        )
            .into_response();
    }
    let secret = match crate::ledger::generate_secret() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[daybook] secret generation failed: {e:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::error("internal server error")),
            )
                .into_response();
        }
    };
    match state
        .ledger
        .mint_token(&label, &crate::ledger::sha256_hex(secret.as_bytes()))
    {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse::success(serde_json::json!({
                "label": label,
                "token": secret,
            }))),
        )
            .into_response(),
        Err(e) => {
            eprintln!("[daybook] token mint failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::error("internal server error")),
            )
                .into_response()
        }
    }
}

/// POST /api/tokens/revoke {label} — soft-revoke a machine token.
pub async fn revoke_token_api(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<Authed>,
    Json(body): Json<RevokeTokenBody>,
) -> Response {
    if !authed.admin {
        return admin_forbidden();
    }
    match state.ledger.revoke_token(body.label.trim()) {
        Ok(true) => (StatusCode::OK, Json(ApiResponse::success(()))).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<&str>::error(
                "no active token with that label",
            )),
        )
            .into_response(),
        Err(e) => {
            eprintln!("[daybook] token revoke failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::error("internal server error")),
            )
                .into_response()
        }
    }
}

/// Cheap sanity checks before the record reaches SQLite: garbage keys
/// would otherwise live in the ledger forever.
fn validate_record(record: &SessionRecord) -> Result<(), String> {
    if record.session_id.trim().is_empty() {
        return Err("session_id must not be empty".into());
    }
    if record.session_id.len() > 128 {
        return Err("session_id too long (max 128 bytes)".into());
    }
    // Bound the other key fields: unbounded strings would live in the
    // ledger forever (the 1 MiB body cap is too generous for keys).
    for (field, val, max) in [
        ("machine", record.machine.as_str(), 128),
        ("runtime", record.runtime.as_str(), 64),
        ("project", record.project.as_str(), 256),
        ("content_hash", record.content_hash.as_str(), 128),
    ] {
        if val.len() > max {
            return Err(format!("{field} too long (max {max} bytes)"));
        }
    }
    if record.machine.trim().is_empty() {
        return Err("machine must not be empty".into());
    }
    // Session names embed "machine:session_id" and machine labels are
    // minted without ':' — records arriving here (e.g. via the admin
    // token, which skips the label check) must not break that split.
    if record.machine.contains(':') {
        return Err("machine must not contain ':'".into());
    }
    if chrono::NaiveDate::parse_from_str(&record.journal_date, "%Y-%m-%d").is_err() {
        return Err("journal_date must be a valid YYYY-MM-DD date".into());
    }
    if record.version < 1 {
        return Err("version must be >= 1".into());
    }
    Ok(())
}

/// List all available dates (ledger-backed; the cluster has no fs archives)
pub async fn list_dates(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.ledger.date_infos() {
        Ok(infos) => {
            let date_infos: Vec<DateInfo> = infos
                .into_iter()
                .map(|r| DateInfo {
                    date: r.date,
                    session_count: r.session_count as usize,
                    has_digest: r.md_count > 0,
                })
                .collect();
            Json(ApiResponse::success(date_infos))
        }
        Err(e) => {
            eprintln!("[daybook] list_dates failed: {e:#}");
            Json(ApiResponse::<Vec<DateInfo>>::error("internal server error"))
        }
    }
}

/// Get daily summary for a specific date, synthesized from the ledger's
/// session records (no fs day digests and no LLM on the cluster pod).
pub async fn get_daily_summary(
    State(state): State<Arc<AppState>>,
    Path(date): Path<String>,
) -> impl IntoResponse {
    match state.ledger.session_briefs(&date) {
        Ok(rows) => {
            let session_count = rows.len();
            let sessions: Vec<String> = rows.iter().map(session_name).collect();
            let mut projects: std::collections::BTreeMap<&str, usize> =
                std::collections::BTreeMap::new();
            for r in &rows {
                *projects.entry(r.project.as_str()).or_default() += 1;
            }
            let overview = if session_count == 0 {
                "No sessions recorded yet.".to_string()
            } else {
                let tally: Vec<String> =
                    projects.iter().map(|(p, n)| format!("{p} ({n})")).collect();
                format!(
                    "{session_count} session(s) across {} project(s): {}",
                    projects.len(),
                    tally.join(", ")
                )
            };
            // Deterministic summary, no LLM on the pod: one insight card per
            // session plus its "Potential Skills/Commands" hints collected as
            // command cards. raw_content stays empty so the web UI renders the
            // structured cards instead of a wall of raw markdown.
            let mut insights = Vec::new();
            let mut commands = Vec::new();
            for r in &rows {
                let title = session_display_title(r);
                let preview = summary_preview(&r.summary_md);
                let preview = if preview.chars().count() >= 400 {
                    format!("{}...", preview.chars().take(400).collect::<String>())
                } else {
                    preview
                };
                if !preview.is_empty() {
                    insights.push(SummaryCardDto {
                        title: title.clone(),
                        content: preview,
                    });
                }
                if let Some(start) = r.summary_md.find("## Potential Skills/Commands\n") {
                    let rest = &r.summary_md[start..];
                    let end = rest.find("\n## ").map(|i| start + i).unwrap_or(r.summary_md.len());
                    for line in r.summary_md[start..end].lines().skip(1) {
                        let line = line.trim().trim_start_matches("- ").trim();
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        commands.push(SummaryCardDto {
                            title: format!("{} — {}", r.project, line),
                            content: line.to_string(),
                        });
                    }
                }
            }
            let summary = DailySummaryDto {
                date: date.clone(),
                overview,
                session_count,
                sessions,
                insights,
                skills: Vec::new(),
                commands,
                reflections: None,
                tomorrow_focus: Vec::new(),
                raw_content: String::new(),
                file_path: String::new(),
            };
            Json(ApiResponse::success(summary))
        }
        Err(e) => {
            eprintln!("[daybook] get_daily_summary failed: {e:#}");
            Json(ApiResponse::<DailySummaryDto>::error(
                "internal server error",
            ))
        }
    }
}

/// List sessions for a specific date (ledger-backed)
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Path(date): Path<String>,
) -> impl IntoResponse {
    match state.ledger.session_briefs(&date) {
        Ok(rows) => {
            let session_briefs: Vec<SessionBrief> = rows
                // ponytail: session_briefs loads full summary_md per row; fine at
                // tens of sessions/date — a SQL substr would break "## Summary"
                // section semantics. Revisit if dates grow past hundreds.
                .iter()
                .map(|r| SessionBrief {
                    name: session_name(r),
                    title: session_display_title(r),
                    summary_preview: summary_preview(&r.summary_md),
                })
                .collect();
            Json(ApiResponse::success(session_briefs))
        }
        Err(e) => {
            eprintln!("[daybook] list_sessions failed: {e:#}");
            Json(ApiResponse::<Vec<SessionBrief>>::error(
                "internal server error",
            ))
        }
    }
}

/// Get session details (ledger-backed). `name` is "machine:session_id" as
/// produced by `list_sessions` / the day view.
pub async fn get_session(
    State(state): State<Arc<AppState>>,
    Path((date, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let Some((machine, session_id)) = split_session_name(&name) else {
        return Json(ApiResponse::<SessionDetailDto>::error(
            "session name must be machine:session_id",
        ));
    };
    let row = match state.ledger.session_brief(&date, &machine, &session_id) {
        Ok(row) => row,
        // DB errors must not masquerade as "session not found".
        Err(e) => {
            eprintln!("[daybook] get_session failed: {e:#}");
            return Json(ApiResponse::<SessionDetailDto>::error(
                "internal server error",
            ));
        }
    };
    match row {
        Some(r) => {
            let metadata = SessionMetadata {
                title: session_display_title(&r),
                date: date.clone(),
                session_id: Some(r.session_id.clone()),
                cwd: None,
                git_branch: r.git_branch.clone(),
                duration: None,
            };
            let detail = SessionDetailDto {
                name,
                content: r.summary_md,
                metadata,
                file_path: format!("ledger://{date}/{machine}/{session_id}"),
            };
            Json(ApiResponse::success(detail))
        }
        None => Json(ApiResponse::<SessionDetailDto>::error("session not found")),
    }
}

/// Health check endpoint
pub async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Get current configuration
pub async fn get_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.read().unwrap();
    let config_dto = ConfigDto {
        storage_path: config.storage.path.to_string_lossy().to_string(),
        model: config.summarization.model.clone(),
        summary_language: config.summarization.summary_language.clone(),
        enable_daily_summary: config.summarization.enable_daily_summary,
        enable_extraction_hints: config.summarization.enable_extraction_hints,
        auto_digest_enabled: config.summarization.auto_digest_enabled,
        digest_time: config.summarization.digest_time.clone(),
        author: config.archive.author.clone(),
        prompt_templates: PromptTemplatesDto {
            session_summary: config.prompt_templates.session_summary.clone(),
            daily_summary: config.prompt_templates.daily_summary.clone(),
            skill_extract: config.prompt_templates.skill_extract.clone(),
            command_extract: config.prompt_templates.command_extract.clone(),
        },
        auto_summarize_enabled: config.summarization.auto_summarize_enabled,
        auto_summarize_on_show: config.summarization.auto_summarize_on_show,
        auto_summarize_inactive_minutes: config.summarization.auto_summarize_inactive_minutes,
    };
    Json(ApiResponse::success(config_dto))
}

/// Get default prompt templates
pub async fn get_default_templates() -> impl IntoResponse {
    let defaults = DefaultTemplatesDto {
        session_summary_en: Prompts::default_session_summary_template("en").to_string(),
        session_summary_zh: Prompts::default_session_summary_template("zh").to_string(),
        daily_summary_en: Prompts::default_daily_summary_template("en").to_string(),
        daily_summary_zh: Prompts::default_daily_summary_template("zh").to_string(),
        skill_extract_en: Prompts::default_skill_extract_template("en").to_string(),
        skill_extract_zh: Prompts::default_skill_extract_template("zh").to_string(),
        command_extract_en: Prompts::default_command_extract_template("en").to_string(),
        command_extract_zh: Prompts::default_command_extract_template("zh").to_string(),
    };
    Json(ApiResponse::success(defaults))
}

/// Get insights data
pub async fn get_insights(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let config = state.config.read().unwrap().clone();
    let days: usize = params
        .get("days")
        .and_then(|d| d.parse().ok())
        .unwrap_or(30);

    match InsightsData::collect(&config, Some(days), &state.pricing) {
        Ok(data) => {
            let dto = InsightsDto {
                total_days: data.total_days,
                total_sessions: data.total_sessions,
                daily_stats: data
                    .daily_stats
                    .into_iter()
                    .map(|s| DailyStatDto {
                        date: s.date,
                        session_count: s.session_count,
                        has_digest: s.has_digest,
                        total_tokens: s.total_tokens,
                        total_cost: s.total_cost,
                    })
                    .collect(),
                goal_distribution: data
                    .goal_distribution
                    .into_iter()
                    .map(|c| CategoryCountDto {
                        name: c.name,
                        count: c.count,
                    })
                    .collect(),
                friction_distribution: data
                    .friction_distribution
                    .into_iter()
                    .map(|c| CategoryCountDto {
                        name: c.name,
                        count: c.count,
                    })
                    .collect(),
                satisfaction_distribution: data
                    .satisfaction_distribution
                    .into_iter()
                    .map(|c| CategoryCountDto {
                        name: c.name,
                        count: c.count,
                    })
                    .collect(),
                language_distribution: data
                    .language_distribution
                    .into_iter()
                    .map(|c| CategoryCountDto {
                        name: c.name,
                        count: c.count,
                    })
                    .collect(),
                session_type_distribution: data
                    .session_type_distribution
                    .into_iter()
                    .map(|c| CategoryCountDto {
                        name: c.name,
                        count: c.count,
                    })
                    .collect(),
                session_details: data
                    .session_details
                    .into_iter()
                    .map(|s| SessionInsightDto {
                        session_id: s.session_id,
                        date: s.date,
                        session_name: s.session_name,
                        brief_summary: s.brief_summary,
                        outcome: s.outcome,
                        goal_categories: s.goal_categories,
                        friction_types: s.friction_types,
                        friction_detail: s.friction_detail,
                        satisfaction: s.satisfaction,
                        claude_helpfulness: s.claude_helpfulness,
                        session_type: s.session_type,
                        token_usage: s.token_usage.map(map_session_usage_dto),
                    })
                    .collect(),
                trends: data.trends.map(|t| TrendDto {
                    period_label: t.period_label,
                    comparison_label: t.comparison_label,
                    current_sessions: t.current_sessions,
                    previous_sessions: t.previous_sessions,
                    sessions_change_pct: t.sessions_change_pct,
                    current_friction_rate: t.current_friction_rate,
                    previous_friction_rate: t.previous_friction_rate,
                    friction_change_pct: t.friction_change_pct,
                    current_success_rate: t.current_success_rate,
                    previous_success_rate: t.previous_success_rate,
                    success_change_pct: t.success_change_pct,
                    current_satisfaction_score: t.current_satisfaction_score,
                    previous_satisfaction_score: t.previous_satisfaction_score,
                    satisfaction_change_pct: t.satisfaction_change_pct,
                    weekly_stats: t
                        .weekly_stats
                        .into_iter()
                        .map(|w| WeeklyStatDto {
                            week_label: w.week_label,
                            session_count: w.session_count,
                            friction_rate: w.friction_rate,
                            success_rate: w.success_rate,
                        })
                        .collect(),
                }),
                usage_summary: data.usage_summary.map(|u| UsageSummaryDto {
                    total_input_tokens: u.total_input_tokens,
                    total_output_tokens: u.total_output_tokens,
                    total_cache_creation_tokens: u.total_cache_creation_tokens,
                    total_cache_read_tokens: u.total_cache_read_tokens,
                    total_cost_usd: u.total_cost_usd,
                    total_sessions: u.total_sessions,
                    model_distribution: u
                        .model_distribution
                        .into_iter()
                        .map(|m| ModelUsageCountDto {
                            model: m.model,
                            count: m.count,
                        })
                        .collect(),
                    daily_usage: u
                        .daily_usage
                        .into_iter()
                        .map(|d| DailyUsageDto {
                            date: d.date,
                            input_tokens: d.input_tokens,
                            output_tokens: d.output_tokens,
                            cache_creation_tokens: d.cache_creation_tokens,
                            cache_read_tokens: d.cache_read_tokens,
                            total_cost_usd: d.total_cost_usd,
                            session_count: d.session_count,
                        })
                        .collect(),
                }),
            };
            Json(ApiResponse::success(dto))
        }
        Err(e) => Json(ApiResponse::<InsightsDto>::error(e.to_string())),
    }
}

/// Get per-day insights combining session facet data
pub async fn get_date_insights(
    State(state): State<Arc<AppState>>,
    Path(date): Path<String>,
) -> impl IntoResponse {
    let config = state.config.read().unwrap().clone();

    match DateInsights::collect(&date, &config, &state.pricing) {
        Ok(data) => {
            let dto = DateInsightsDto {
                sessions: data
                    .sessions
                    .into_iter()
                    .map(|s| DateSessionInsightDto {
                        name: s.name,
                        session_id: s.session_id,
                        brief_summary: s.brief_summary,
                        outcome: s.outcome,
                        goal_categories: s.goal_categories,
                        friction_types: s.friction_types,
                        friction_detail: s.friction_detail,
                        satisfaction: s.satisfaction,
                        claude_helpfulness: s.claude_helpfulness,
                        token_usage: s.token_usage.map(map_session_usage_dto),
                    })
                    .collect(),
                day_summary: DayInsightSummaryDto {
                    total_sessions: data.day_summary.total_sessions,
                    sessions_with_friction: data.day_summary.sessions_with_friction,
                    overall_satisfaction: data.day_summary.overall_satisfaction,
                    top_goals: data.day_summary.top_goals,
                    top_frictions: data.day_summary.top_frictions,
                    recommendations: data.day_summary.recommendations,
                    total_tokens: data.day_summary.total_tokens,
                    total_cost: data.day_summary.total_cost_usd,
                    model_distribution: data
                        .day_summary
                        .model_distribution
                        .into_iter()
                        .map(|(model, count)| ModelUsageCountDto { model, count })
                        .collect(),
                },
            };
            Json(ApiResponse::success(dto))
        }
        Err(e) => Json(ApiResponse::<DateInsightsDto>::error(e.to_string())),
    }
}

// Helper functions

/// Dashboard session name: "machine:session_id" (machine labels reject ':',
/// so the split is unambiguous).
fn session_name(r: &crate::ledger::SessionBriefRow) -> String {
    format!("{}:{}", r.machine, r.session_id)
}

/// Inverse of `session_name`; None when malformed.
fn split_session_name(name: &str) -> Option<(String, String)> {
    let (machine, session_id) = name.split_once(':')?;
    Some((machine.to_string(), session_id.to_string()))
}

/// Title from the transcript archive's frontmatter, if present.
fn frontmatter_title(summary_md: &str) -> Option<String> {
    if !summary_md.starts_with("---\n") {
        return None;
    }
    for line in summary_md[4..].lines() {
        if line.trim() == "---" {
            break;
        }
        if let Some(rest) = line.strip_prefix("title: ") {
            let t = rest.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn session_display_title(record: &crate::ledger::SessionBriefRow) -> String {
    if record.title.trim().is_empty() {
        frontmatter_title(&record.summary_md).unwrap_or_else(|| record.session_id.clone())
    } else {
        record.title.trim().to_string()
    }
}
fn summary_preview(summary_md: &str) -> String {
    let Some(start) = summary_md.find("## Summary\n").map(|i| i + 11) else {
        return String::new();
    };
    let end = summary_md[start..]
        .find("\n## ")
        .map(|i| start + i)
        .unwrap_or(summary_md.len());
    let text = summary_md[start..end].trim();
    if text.chars().count() > 200 {
        format!("{}...", text.chars().take(200).collect::<String>())
    } else {
        text.to_string()
    }
}

/// Map a SessionUsage domain model to SessionUsageDto
fn map_session_usage_dto(u: crate::usage::types::SessionUsage) -> SessionUsageDto {
    SessionUsageDto {
        session_id: u.session_id,
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        cache_creation_tokens: u.cache_creation_tokens,
        cache_read_tokens: u.cache_read_tokens,
        total_cost_usd: u.total_cost_usd,
        model_calls: u
            .model_calls
            .into_iter()
            .map(|(model, count)| ModelUsageCountDto { model, count })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> SessionRecord {
        SessionRecord {
            session_id: "s1".into(),
            machine: "vm1".into(),
            runtime: "pi".into(),
            project: "/tmp/p".into(),
            journal_date: "2026-10-29".into(),
            started_utc: None,
            ended_utc: None,
            title: None,
            git_branch: None,
            version: 1,
            content_hash: "h".into(),
            summary_md: "md".into(),
            facets_json: None,
            usage: crate::ledger::UsageRecord::default(),
            aggregates: crate::ledger::AggregatesRecord::default(),
        }
    }

    #[test]
    fn test_validate_record_rejects_garbage() {
        assert!(validate_record(&rec()).is_ok());
        let mut bad = rec();
        bad.session_id = "  ".into();
        assert!(validate_record(&bad).is_err());
        let mut bad = rec();
        bad.journal_date = "2026/10/29".into();
        assert!(validate_record(&bad).is_err());
        let mut bad = rec();
        bad.version = 0;
        assert!(validate_record(&bad).is_err());
        let mut bad = rec();
        bad.machine = String::new();
        assert!(validate_record(&bad).is_err());
        let mut bad = rec();
        bad.machine = "vm:1".into();
        assert!(validate_record(&bad).is_err());
    }

    // --- HTTP contract tests: exercise the real router (auth middleware,
    // body cap, JSON layer) with tower::ServiceExt::oneshot. ---

    use crate::server::router::create_router;
    async fn test_app(name: &str) -> (axum::Router, Arc<AppState>, &'static str) {
        let dir =
            std::env::temp_dir().join(format!("daybook-httptest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ledger = Ledger::open(&dir.join("t.db3")).unwrap();
        let secret = "dbk_testsecret";
        ledger
            .mint_token("vm1", &crate::ledger::sha256_hex(secret.as_bytes()))
            .unwrap();
        let mut config = Config::default();
        config.server.admin_token = Some("dbk_admin_secret".into());
        let state = Arc::new(AppState {
            config: RwLock::new(config),
            pricing: PricingData::from_map(std::collections::HashMap::new()),
            ledger,
        });
        (create_router(state.clone()), state, secret)
    }

    /// Admin bearer for test_app's fixed config.
    const ADMIN_SECRET: &str = "dbk_admin_secret";

    fn get_req(uri: &str, token: Option<&str>) -> axum::http::Request<axum::body::Body> {
        let mut builder = axum::http::Request::builder().method("GET").uri(uri);
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        builder.body(axum::body::Body::empty()).unwrap()
    }

    fn post_json_req(
        uri: &str,
        token: Option<&str>,
        body: serde_json::Value,
    ) -> axum::http::Request<axum::body::Body> {
        let mut builder = axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        builder
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    fn ingest_req(
        token: Option<&str>,
        body: &serde_json::Value,
    ) -> axum::http::Request<axum::body::Body> {
        let mut builder = axum::http::Request::builder()
            .method("POST")
            .uri("/api/ingest")
            .header("content-type", "application/json");
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        builder
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    async fn status_of(
        router: &axum::Router,
        req: axum::http::Request<axum::body::Body>,
    ) -> (axum::http::StatusCode, String) {
        use tower::ServiceExt;
        let resp = router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn test_ingest_http_contract() {
        let (app, _state, token) = test_app("contract").await;
        let good = serde_json::to_value(rec()).unwrap();

        // missing token -> 401
        let (s, _) = status_of(&app, ingest_req(None, &good)).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        // invalid token -> 401
        let (s, _) = status_of(&app, ingest_req(Some("dbk_wrong"), &good)).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        // token valid for vm1 but record.machine is other -> 403
        let mut wrong_machine = good.clone();
        wrong_machine["machine"] = serde_json::json!("other");
        let (s, b) = status_of(&app, ingest_req(Some(token), &wrong_machine)).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert!(b.contains("does not match"));
        // invalid date -> 400
        let mut bad_date = good.clone();
        bad_date["journal_date"] = serde_json::json!("2026-13-99");
        let (s, _) = status_of(&app, ingest_req(Some(token), &bad_date)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        // valid first push -> 200 inserted
        let (s, b) = status_of(&app, ingest_req(Some(token), &good)).await;
        assert_eq!(s, StatusCode::OK);
        assert!(b.contains("inserted"), "body: {b}");
        // identical re-push -> 200 noop
        let (s, b) = status_of(&app, ingest_req(Some(token), &good)).await;
        assert_eq!(s, StatusCode::OK);
        assert!(b.contains("noop"), "body: {b}");
        // newer revision -> 200 updated
        let mut v2 = good.clone();
        v2["version"] = serde_json::json!(2);
        v2["content_hash"] = serde_json::json!("h2");
        let (s, b) = status_of(&app, ingest_req(Some(token), &v2)).await;
        assert_eq!(s, StatusCode::OK);
        assert!(b.contains("updated"), "body: {b}");
        // older revision after newer -> 409
        let mut v1 = good;
        v1["content_hash"] = serde_json::json!("h-old");
        let (s, _) = status_of(&app, ingest_req(Some(token), &v1)).await;
        assert_eq!(s, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_ingest_revoked_token_is_401() {
        let (app, state, token) = test_app("revoked").await;
        let good = serde_json::to_value(rec()).unwrap();
        // Token works, then is revoked: the same secret must stop working.
        let (s, _) = status_of(&app, ingest_req(Some(token), &good)).await;
        assert_eq!(s, StatusCode::OK);
        assert!(state.ledger.revoke_token("vm1").unwrap());
        let (s, _) = status_of(&app, ingest_req(Some(token), &good)).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_stats_http_contract() {
        let (app, _state, token) = test_app("stats-contract").await;
        // seed one session so the report has data
        let good = serde_json::to_value(rec()).unwrap();
        let (s, _) = status_of(&app, ingest_req(Some(token), &good)).await;
        assert_eq!(s, StatusCode::OK);

        let stats_req = |token: Option<&str>, days: &str| {
            let mut builder = axum::http::Request::builder()
                .method("GET")
                .uri(format!("/api/stats?days={days}"));
            if let Some(t) = token {
                builder = builder.header("authorization", format!("Bearer {t}"));
            }
            builder.body(axum::body::Body::empty()).unwrap()
        };

        // missing token -> 401
        let (s, _) = status_of(&app, stats_req(None, "7")).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        // invalid token -> 401
        let (s, _) = status_of(&app, stats_req(Some("dbk_wrong"), "7")).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        // valid token -> 200 with the ApiResponse<StatsReport> envelope
        let (s, body) = status_of(&app, stats_req(Some(token), "7")).await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["success"], serde_json::json!(true));
        assert_eq!(v["data"]["totals"]["sessions"], serde_json::json!(1));
        assert_eq!(
            v["data"]["days"][0]["date"],
            serde_json::json!("2026-10-29")
        );
        assert!(v["data"]["trend"].is_array());
        // clamps: 0 -> 1, 999 -> 90 (both still 200)
        let (s, _) = status_of(&app, stats_req(Some(token), "0")).await;
        assert_eq!(s, StatusCode::OK);
        let (s, _) = status_of(&app, stats_req(Some(token), "999")).await;
        assert_eq!(s, StatusCode::OK);
        // days param absent -> default window
        let no_days = axum::http::Request::builder()
            .method("GET")
            .uri("/api/stats")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let (s, _) = status_of(&app, no_days).await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_s5_auth_gates_query_apis() {
        let (app, _state, token) = test_app("s5-auth-gate").await;
        // read API: missing / wrong token -> 401
        let (s, _) = status_of(&app, get_req("/api/dates", None)).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        let (s, _) = status_of(&app, get_req("/api/dates", Some("dbk_wrong"))).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        // machine token and admin token both read
        let (s, _) = status_of(&app, get_req("/api/dates", Some(token))).await;
        assert_eq!(s, StatusCode::OK);
        let (s, _) = status_of(&app, get_req("/api/dates", Some(ADMIN_SECRET))).await;
        assert_eq!(s, StatusCode::OK);
        // health stays open
        let (s, _) = status_of(&app, get_req("/api/health", None)).await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_s5_token_management_admin_only() {
        let (app, _state, token) = test_app("s5-tokens").await;
        // machine token cannot manage tokens
        let (s, _) = status_of(&app, get_req("/api/tokens", Some(token))).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, _) = status_of(
            &app,
            post_json_req(
                "/api/tokens",
                Some(token),
                serde_json::json!({"label": "vm2"}),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        // admin lists
        let (s, body) = status_of(&app, get_req("/api/tokens", Some(ADMIN_SECRET))).await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["data"][0]["label"], serde_json::json!("vm1"));
        // admin mints vm2; the secret comes back exactly once
        let (s, body) = status_of(
            &app,
            post_json_req(
                "/api/tokens",
                Some(ADMIN_SECRET),
                serde_json::json!({"label": "vm2"}),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let vm2_secret = v["data"]["token"].as_str().unwrap().to_string();
        assert!(vm2_secret.starts_with("dbk_"));
        // vm2's secret works on read APIs
        let (s, _) = status_of(&app, get_req("/api/dates", Some(&vm2_secret))).await;
        assert_eq!(s, StatusCode::OK);
        // bad label rejected
        let (s, _) = status_of(
            &app,
            post_json_req(
                "/api/tokens",
                Some(ADMIN_SECRET),
                serde_json::json!({"label": ""}),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_s5_revocation_isolates_machines() {
        let (app, _state, vm1_secret) = test_app("s5-revoke").await;
        // mint vm2 via admin API
        let (_, body) = status_of(
            &app,
            post_json_req(
                "/api/tokens",
                Some(ADMIN_SECRET),
                serde_json::json!({"label": "vm2"}),
            ),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let vm2_secret = v["data"]["token"].as_str().unwrap().to_string();

        // vm1 record pushed with vm1 token
        let mut r = rec();
        r.machine = "vm1".into();
        let (s, _) = status_of(
            &app,
            ingest_req(Some(vm1_secret), &serde_json::to_value(&r).unwrap()),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        // vm2 record pushed with vm2 token
        let mut r = rec();
        r.machine = "vm2".into();
        r.session_id = "s-vm2-1".into();
        let (s, _) = status_of(
            &app,
            ingest_req(Some(&vm2_secret), &serde_json::to_value(&r).unwrap()),
        )
        .await;
        assert_eq!(s, StatusCode::OK);

        // revoke vm1
        let (s, _) = status_of(
            &app,
            post_json_req(
                "/api/tokens/revoke",
                Some(ADMIN_SECRET),
                serde_json::json!({"label": "vm1"}),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::OK);

        // vm1 blocked everywhere, vm2 unaffected
        let (s, _) = status_of(&app, get_req("/api/dates", Some(vm1_secret))).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        // fresh vm1 record (previous `r` was mutated to vm2's identity above,
        // so reusing it would have tested vm1's token against vm2's record)
        let mut r = rec();
        r.machine = "vm1".into();
        let (s, _) = status_of(
            &app,
            ingest_req(Some(vm1_secret), &serde_json::to_value(&r).unwrap()),
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        // vm2 can still ingest after vm1's revocation
        let mut r = rec();
        r.machine = "vm2".into();
        r.session_id = "s-vm2-2".into();
        let (s, _) = status_of(
            &app,
            ingest_req(Some(&vm2_secret), &serde_json::to_value(&r).unwrap()),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let (s, _) = status_of(&app, get_req("/api/dates", Some(&vm2_secret))).await;
        assert_eq!(s, StatusCode::OK);
        // revoking again -> 404
        let (s, _) = status_of(
            &app,
            post_json_req(
                "/api/tokens/revoke",
                Some(ADMIN_SECRET),
                serde_json::json!({"label": "vm1"}),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        // admin token list reflects the revocation
        let (s, body) = status_of(&app, get_req("/api/tokens", Some(ADMIN_SECRET))).await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let row = |label: &str| {
            v["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["label"] == label)
                .unwrap_or_else(|| panic!("token list missing {label}"))
                .clone()
        };
        assert_eq!(row("vm1")["revoked"], serde_json::json!(true));
        assert_eq!(row("vm2")["revoked"], serde_json::json!(false));
    }
    #[tokio::test]
    async fn test_s5_machine_token_revoke_forbidden() {
        let (app, _state, vm1_secret) = test_app("s5-mt-revoke").await;
        let (s, _) = status_of(
            &app,
            post_json_req(
                "/api/tokens",
                Some(vm1_secret),
                serde_json::json!({"label": "vm2"}),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, _) = status_of(
            &app,
            post_json_req(
                "/api/tokens/revoke",
                Some(vm1_secret),
                serde_json::json!({"label": "vm1"}),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        // ':' in label is rejected (session names embed machine:session_id)
        let (s, _) = status_of(
            &app,
            post_json_req(
                "/api/tokens",
                Some(ADMIN_SECRET),
                serde_json::json!({"label": "bad:label"}),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        // revoking an unknown label -> 404, ledger untouched
        let (s, _) = status_of(
            &app,
            post_json_req(
                "/api/tokens/revoke",
                Some(ADMIN_SECRET),
                serde_json::json!({"label": "no-such-machine"}),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        // failed revoke attempts left vm1's token active
        let (s, _) = status_of(&app, get_req("/api/dates", Some(vm1_secret))).await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_s5_ingested_content_is_readable() {
        let (app, _state, vm1_secret) = test_app("s5-content").await;
        // ingest a record with real journal-shaped content
        let mut r = rec();
        r.title = Some("Fix ledger race".into());
        r.summary_md = "---\ntitle: Fix ledger race\n---\n## Summary\nFixed a write race in the ledger by adding a transaction. Tests added.\n".into();
        let (s, _) = status_of(
            &app,
            ingest_req(Some(vm1_secret), &serde_json::to_value(&r).unwrap()),
        )
        .await;
        assert_eq!(s, StatusCode::OK);

        // date list contains the journal date
        let (s, body) = status_of(&app, get_req("/api/dates", Some(vm1_secret))).await;
        assert_eq!(s, StatusCode::OK);
        assert!(
            body.contains("2026-10-29"),
            "dates list missing journal date"
        );

        // day summary lists the session with its preview
        let (s, body) = status_of(&app, get_req("/api/dates/2026-10-29", Some(vm1_secret))).await;
        assert_eq!(s, StatusCode::OK);
        assert!(
            body.contains("vm1:s1"),
            "day summary missing session name, body: {body}"
        );
        assert!(
            body.contains("Fixed a write race"),
            "day summary missing preview"
        );

        // session list shows the session; detail returns the full summary
        let (s, body) = status_of(
            &app,
            get_req("/api/dates/2026-10-29/sessions", Some(vm1_secret)),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert!(body.contains("vm1:s1"), "session list missing session");
        assert!(
            body.contains("Fix ledger race"),
            "session list missing title"
        );
        assert!(
            body.contains("Fixed a write race"),
            "session list missing summary preview"
        );

        let (s, body) = status_of(
            &app,
            get_req("/api/dates/2026-10-29/sessions/vm1:s1", Some(vm1_secret)),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert!(
            body.contains("## Summary"),
            "session detail missing summary"
        );
        assert!(body.contains("transaction"), "session detail missing body");
    }
}
