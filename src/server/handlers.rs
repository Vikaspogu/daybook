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
                .map(|r| {
                    // ponytail: one lean (project,title) query per date,
                    // SQLite-local; collapses the web client's N+1 summary
                    // fan-out into a single /api/dates call.
                    match state.ledger.session_project_titles(&r.date) {
                        Ok(pairs) => DateInfo {
                            preview: format_day_overview(
                                &pairs.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
                            ),
                            session_titles: {
                                let cleaned: Vec<String> = pairs
                                    .iter()
                                    .map(|(_, t)| clean_session_title(t))
                                    .filter(|t| !t.is_empty())
                                    .collect();
                                let signal: Vec<String> = cleaned
                                    .iter()
                                    .filter(|t| {
                                        let l = t.to_lowercase();
                                        !(l.starts_with("blocked ")
                                            || l.ends_with(" blocked")
                                            || l.contains("repository survey")
                                            || l.contains("pong response")
                                            || l.contains("pong-response")
                                            || l.contains("review sub-agent"))
                                    })
                                    .take(3)
                                    .cloned()
                                    .collect();
                                if signal.is_empty() {
                                    cleaned.into_iter().take(3).collect()
                                } else {
                                    signal
                                }
                            },
                            date: r.date,
                            session_count: r.session_count as usize,
                            has_digest: r.md_count > 0,
                        },
                        Err(e) => {
                            eprintln!("[daybook] list_dates preview failed for {}: {e:#}", r.date);
                            DateInfo {
                                preview: "Preview unavailable.".to_string(),
                                session_titles: Vec::new(),
                                date: r.date,
                                session_count: r.session_count as usize,
                                has_digest: r.md_count > 0,
                            }
                        }
                    }
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

/// Deterministic one-line day overview, shared by the dates list and the
/// day digest so both surfaces say the same thing.
fn format_day_overview(projects: &[&str]) -> String {
    if projects.is_empty() {
        return "No sessions recorded yet.".to_string();
    }
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for p in projects {
        *counts.entry(p).or_default() += 1;
    }
    let tally: Vec<String> = counts.iter().map(|(p, n)| format!("{p} ({n})")).collect();
    format!(
        "{} session(s) across {} project(s): {}",
        projects.len(),
        counts.len(),
        tally.join(", ")
    )
}

/// Decorative/empty extraction-hint lines must not become cards
/// (serves the "None identified in this session." noise visible in the Skills tab).
fn is_junk_hint(line: &str) -> bool {
    let t = line
        .trim()
        .trim_matches(|c: char| c == '*' || c == '_' || c == '`')
        .trim();
    if t.len() < 4 || t.starts_with('#') {
        return true;
    }
    if t.chars().all(|c| matches!(c, '-' | '=' | '*' | ' ')) {
        return true;
    }
    let lower = t.to_lowercase();
    lower.starts_with("none identified")
        || lower.starts_with("(none")
        || lower == "none"
        || lower == "none."
        || lower.starts_with("none found")
        || lower.starts_with("n/a")
        || lower.contains("archived by daily context archive system")
}

/// "1h 23m" from two RFC3339 timestamps; None when either is missing/unparseable.
fn session_duration(started: Option<&str>, ended: Option<&str>) -> Option<String> {
    let s = chrono::DateTime::parse_from_rfc3339(started?).ok()?;
    let e = chrono::DateTime::parse_from_rfc3339(ended?).ok()?;
    let secs = (e - s).num_seconds();
    if secs < 0 {
        return None;
    }
    let mins = secs / 60;
    Some(if mins >= 60 {
        format!("{}h {}m", mins / 60, mins % 60)
    } else {
        format!("{mins}m")
    })
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

            let mut work_count = 0usize;
            let mut review_count = 0usize;
            let mut blocked_count = 0usize;
            let mut day_metrics = DayMetricsDto::default();
            let mut by_project: std::collections::BTreeMap<String, ProjectGainDto> =
                std::collections::BTreeMap::new();
            let mut review_fallbacks: std::collections::BTreeMap<String, Vec<String>> =
                std::collections::BTreeMap::new();
            let mut waste_map: std::collections::BTreeMap<(String, String), (usize, f64)> =
                std::collections::BTreeMap::new();
            let mut consolidated_rules: Vec<ConsolidatedRuleDto> = Vec::new();
            let mut work_learnings: Vec<String> = Vec::new();
            let mut review_learnings: Vec<String> = Vec::new();
            let mut learnings_map: std::collections::BTreeMap<String, ProjectLearningsDto> =
                std::collections::BTreeMap::new();
            let mut cost_by_session: Vec<(String, String, String, f64)> = Vec::new();
            let mut followup_candidates: Vec<(String, String, String)> = Vec::new();
            let mut empty_reviewer_noted = false;

            for r in &rows {
                let title = session_display_title(r);
                let preview = summary_preview(&r.summary_md);
                let files = extract_files_touched(&r.summary_md);
                let (category, blocker_reason) = classify_session(&title, &preview, &files);
                let cost = r.cost_usd.unwrap_or(0.0);

                day_metrics.input_tokens = day_metrics.input_tokens.saturating_add(r.usage.input);
                day_metrics.output_tokens =
                    day_metrics.output_tokens.saturating_add(r.usage.output);
                day_metrics.cache_read_tokens = day_metrics
                    .cache_read_tokens
                    .saturating_add(r.usage.cache_read);
                day_metrics.cache_creation_tokens = day_metrics
                    .cache_creation_tokens
                    .saturating_add(r.usage.cache_creation);
                day_metrics.total_cost_usd += cost;
                day_metrics.test_runs =
                    day_metrics.test_runs.saturating_add(r.aggregates.test_runs);
                day_metrics.failed_tool_calls = day_metrics
                    .failed_tool_calls
                    .saturating_add(r.aggregates.failed_tool_calls);
                day_metrics.interrupted = day_metrics
                    .interrupted
                    .saturating_add(r.aggregates.interrupted);

                if cost > 0.0 {
                    cost_by_session.push((
                        title.clone(),
                        r.project.clone(),
                        r.runtime.clone(),
                        cost,
                    ));
                }

                let lower_md = r.summary_md.to_lowercase();
                if lower_md.contains("empty output") || lower_md.contains("returned no output") {
                    empty_reviewer_noted = true;
                }

                let entry = by_project
                    .entry(r.project.clone())
                    .or_insert_with(|| ProjectGainDto {
                        project: r.project.clone(),
                        ..Default::default()
                    });

                match category {
                    "work" => {
                        work_count += 1;
                        entry.work_count += 1;
                        entry.total_cost_usd += cost;
                        let takeaway = first_sentence(&preview);
                        let line = if takeaway.is_empty() {
                            title.clone()
                        } else {
                            format!("{title}: {takeaway}")
                        };
                        if entry.accomplishments.len() < 5 && !entry.accomplishments.contains(&line)
                        {
                            entry.accomplishments.push(line);
                        }
                        if let Some(dec) = extract_key_decision(&r.summary_md) {
                            if entry.key_decisions.len() < 3 && !entry.key_decisions.contains(&dec)
                            {
                                entry.key_decisions.push(dec.clone());
                            }
                            let dec_l = dec.to_lowercase();
                            if dec_l.contains("findings-only")
                                || dec_l.contains("without applying")
                                || dec_l.contains("deferred")
                                || dec_l.contains("follow-up")
                                || dec_l.contains("later")
                            {
                                followup_candidates.push((r.project.clone(), title.clone(), dec));
                            }
                        } else if lower_md.contains("findings-only")
                            || lower_md.contains("deferred")
                        {
                            followup_candidates.push((
                                r.project.clone(),
                                title.clone(),
                                takeaway.clone(),
                            ));
                        }
                        for f in files {
                            if entry.files_touched.len() < 8 && !entry.files_touched.contains(&f) {
                                entry.files_touched.push(f);
                            }
                        }
                        for l in extract_learnings(&r.summary_md).into_iter().take(3) {
                            push_project_learning(&mut learnings_map, &r.project, &title, &l);
                            if !work_learnings.contains(&l) {
                                work_learnings.push(l);
                            }
                        }
                    }
                    "review" => {
                        review_count += 1;
                        entry.review_count += 1;
                        entry.total_cost_usd += cost;
                        let takeaway = first_sentence(&preview);
                        let line = if takeaway.is_empty() {
                            title.clone()
                        } else {
                            format!("{title}: {takeaway}")
                        };
                        let fb = review_fallbacks.entry(r.project.clone()).or_default();
                        if fb.len() < 2 && !fb.contains(&line) {
                            fb.push(line);
                        }
                        for l in extract_learnings(&r.summary_md).into_iter().take(2) {
                            push_project_learning(&mut learnings_map, &r.project, &title, &l);
                            if !review_learnings.contains(&l) {
                                review_learnings.push(l);
                            }
                        }
                    }
                    _ => {
                        blocked_count += 1;
                        entry.blocked_count += 1;
                        let reason = blocker_reason.unwrap_or_else(|| {
                            "Blocked before execution (environment or permissions)".to_string()
                        });
                        let w = waste_map.entry((r.project.clone(), reason)).or_default();
                        w.0 += 1;
                        w.1 += cost;
                    }
                }

                for rule in extract_consolidated_rules(&r.project, &r.summary_md) {
                    if let Some(existing) = consolidated_rules.iter_mut().find(|c| {
                        c.project.eq_ignore_ascii_case(&rule.project)
                            && c.name.eq_ignore_ascii_case(&rule.name)
                    }) {
                        if existing.trigger.is_none() {
                            existing.trigger = rule.trigger;
                        }
                        if existing.why.is_none() {
                            existing.why = rule.why;
                        }
                    } else if consolidated_rules.len() < 12 {
                        consolidated_rules.push(rule);
                    }
                }
            }

            day_metrics.total_tokens = day_metrics
                .input_tokens
                .saturating_add(day_metrics.output_tokens)
                .saturating_add(day_metrics.cache_read_tokens)
                .saturating_add(day_metrics.cache_creation_tokens);
            day_metrics.cache_hit_rate = crate::ledger::pooled_hit_rate(
                day_metrics.input_tokens,
                day_metrics.cache_read_tokens,
            );

            let commands: Vec<SummaryCardDto> = consolidated_rules
                .iter()
                .map(|rule| {
                    let mut body = rule.summary.clone();
                    if let Some(t) = &rule.trigger {
                        body.push_str(&format!("\n- **Trigger:** {t}"));
                    }
                    if let Some(w) = &rule.why {
                        body.push_str(&format!("\n- **Why:** {w}"));
                    }
                    SummaryCardDto {
                        title: format!("{} — {}", rule.project, rule.name),
                        content: body,
                    }
                })
                .collect();

            let mut learnings_by_project: Vec<ProjectLearningsDto> = learnings_map
                .into_values()
                .filter(|p| !p.pitfalls.is_empty() || !p.learnings.is_empty())
                .collect();
            learnings_by_project.sort_by(|a, b| {
                let count_b = b.pitfalls.len() + b.learnings.len();
                let count_a = a.pitfalls.len() + a.learnings.len();
                count_b
                    .cmp(&count_a)
                    .then_with(|| a.project.cmp(&b.project))
            });

            let mut learnings: Vec<String> = learnings_by_project
                .iter()
                .flat_map(|p| {
                    p.pitfalls
                        .iter()
                        .chain(p.learnings.iter())
                        .map(|item| item.text.clone())
                })
                .collect();
            if learnings.is_empty() {
                learnings = work_learnings;
                for l in review_learnings {
                    if learnings.len() >= 4 {
                        break;
                    }
                    if !learnings.contains(&l) {
                        learnings.push(l);
                    }
                }
            }

            let mut project_gains: Vec<ProjectGainDto> = by_project
                .into_values()
                .filter_map(|mut p| {
                    if p.accomplishments.is_empty() {
                        if let Some(fb) = review_fallbacks.remove(&p.project) {
                            p.accomplishments = fb;
                        }
                    }
                    if p.work_count > 0 || p.review_count > 0 {
                        Some(p)
                    } else {
                        None
                    }
                })
                .collect();
            project_gains.sort_by(|a, b| {
                b.work_count
                    .cmp(&a.work_count)
                    .then_with(|| b.review_count.cmp(&a.review_count))
            });

            // Keep `insights` populated with one card per project so API consumers
            // reading `DailySummaryDto.insights` receive the synthesized project summary.
            let insights: Vec<SummaryCardDto> = project_gains
                .iter()
                .map(|g| SummaryCardDto {
                    title: g.project.clone(),
                    content: g
                        .accomplishments
                        .iter()
                        .map(|a| format!("- {a}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                })
                .collect();

            let mut waste_alerts: Vec<WasteAlertDto> = waste_map
                .into_iter()
                .map(|((project, reason), (count, cost_usd))| {
                    let hint = if reason.contains("bwrap") || reason.contains("RTM_NEWADDR") {
                        "Background survey cannot create a bubblewrap loopback interface inside this container. Disable or reconfigure the background survey job in this environment.".to_string()
                    } else if reason.contains("PONG") {
                        "Automated CLI connectivity probe; safe to ignore.".to_string()
                    } else if reason.contains("403") || reason.contains("model_access_denied") {
                        "API key lacks access to the configured model; update the model in agent settings.".to_string()
                    } else {
                        "Session exited before completing work due to environment or permission constraints.".to_string()
                    };
                    WasteAlertDto {
                        project,
                        count,
                        cost_usd,
                        reason,
                        hint,
                    }
                })
                .collect();
            waste_alerts.sort_by_key(|a| std::cmp::Reverse(a.count));

            let (efficiency_signals, next_steps) = build_workflow_coaching(
                session_count,
                work_count,
                review_count,
                blocked_count,
                &day_metrics,
                &project_gains,
                &waste_alerts,
                &mut cost_by_session,
                &followup_candidates,
                empty_reviewer_noted,
            );

            let overview =
                format_day_overview(&rows.iter().map(|r| r.project.as_str()).collect::<Vec<_>>());

            let summary = DailySummaryDto {
                date: date.clone(),
                overview,
                session_count,
                work_count,
                review_count,
                blocked_count,
                day_metrics,
                efficiency_signals,
                learnings,
                learnings_by_project,
                next_steps,
                consolidated_rules,
                project_gains,
                waste_alerts,
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
            let mut session_briefs: Vec<SessionBrief> = rows
                .iter()
                .map(|r| {
                    let title = session_display_title(r);
                    let preview = summary_preview(&r.summary_md);
                    let files = extract_files_touched(&r.summary_md);
                    let duration =
                        session_duration(r.started_utc.as_deref(), r.ended_utc.as_deref());
                    let (category, blocker_reason) = classify_session(&title, &preview, &files);
                    SessionBrief {
                        name: session_name(r),
                        title,
                        summary_preview: preview,
                        runtime: r.runtime.clone(),
                        project: r.project.clone(),
                        started_utc: r.started_utc.clone(),
                        ended_utc: r.ended_utc.clone(),
                        duration,
                        cost_usd: r.cost_usd,
                        category: category.to_string(),
                        key_decision: if category == "work" {
                            extract_key_decision(&r.summary_md)
                        } else {
                            None
                        },
                        files_touched: if category == "work" {
                            files
                        } else {
                            Vec::new()
                        },
                        blocker_reason,
                    }
                })
                .collect();
            // Order: real work first, then reviews, then blocked noise
            session_briefs.sort_by_key(|s| match s.category.as_str() {
                "work" => 0u8,
                "review" => 1u8,
                _ => 2u8,
            });
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
                duration: session_duration(r.started_utc.as_deref(), r.ended_utc.as_deref()),
                runtime: Some(r.runtime.clone()),
                project: Some(r.project.clone()),
                machine: Some(r.machine.clone()),
                started_utc: r.started_utc.clone(),
                ended_utc: r.ended_utc.clone(),
                cost_usd: r.cost_usd,
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

/// Clean raw session titles: strip surrounding quotes, strip leading HH_MM- / HH:MM-
/// timestamps, and humanize slug-only titles ("00_06-improve-cron-guidance" -> "Improve cron guidance").
fn clean_session_title(raw: &str) -> String {
    let mut s = raw.trim().trim_matches(['"', '\'']).trim();
    if s.len() > 6 {
        let bytes = s.as_bytes();
        if bytes[0].is_ascii_digit()
            && bytes[1].is_ascii_digit()
            && (bytes[2] == b'_' || bytes[2] == b':' || bytes[2] == b'-')
            && bytes[3].is_ascii_digit()
            && bytes[4].is_ascii_digit()
            && (bytes[5] == b'-' || bytes[5] == b'_' || bytes[5] == b' ')
        {
            s = s[6..].trim();
        }
    }
    if s.is_empty() {
        return String::new();
    }
    if !s.contains(' ') && (s.contains('-') || s.contains('_')) {
        let words = s.replace(['-', '_'], " ");
        let mut chars = words.chars();
        match chars.next() {
            Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
            None => words,
        }
    } else {
        s.to_string()
    }
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
            let t = clean_session_title(rest);
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn session_display_title(record: &crate::ledger::SessionBriefRow) -> String {
    let raw = if record.title.trim().is_empty() {
        frontmatter_title(&record.summary_md).unwrap_or_else(|| record.session_id.clone())
    } else {
        clean_session_title(&record.title)
    };
    if raw.is_empty() {
        record.session_id.clone()
    } else {
        raw
    }
}

fn extract_section(summary_md: &str, heading: &str) -> String {
    let marker = format!("## {heading}\n");
    let Some(start) = summary_md.find(&marker).map(|i| i + marker.len()) else {
        return String::new();
    };
    let end = summary_md[start..]
        .find("\n## ")
        .map(|i| start + i)
        .unwrap_or(summary_md.len());
    summary_md[start..end].trim().to_string()
}

fn summary_preview(summary_md: &str) -> String {
    let text = extract_section(summary_md, "Summary");
    if text.chars().count() > 220 {
        format!("{}...", text.chars().take(220).collect::<String>())
    } else {
        text
    }
}

fn first_sentence(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let first_line = trimmed.lines().next().unwrap_or(trimmed).trim();
    if let Some(idx) = first_line.find(". ") {
        format!("{}.", &first_line[..idx])
    } else {
        first_line.to_string()
    }
}

fn extract_key_decision(summary_md: &str) -> Option<String> {
    let sec = extract_section(summary_md, "Key Decisions & Trade-offs");
    for line in sec.lines() {
        let cleaned = line
            .trim()
            .trim_start_matches("- ")
            .trim_start_matches("* ")
            .trim()
            .replace("**", "");
        if is_junk_hint(&cleaned) {
            continue;
        }
        let lower = cleaned.to_lowercase();
        if lower.starts_with("no repository")
            || lower.starts_with("no code")
            || lower.starts_with("no product")
            || lower.starts_with("no files")
        {
            continue;
        }
        let out = if cleaned.chars().count() > 220 {
            format!("{}...", cleaned.chars().take(220).collect::<String>())
        } else {
            cleaned
        };
        return Some(out);
    }
    None
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && s[..prefix.len()].eq_ignore_ascii_case(prefix)
    {
        return Some(&s[prefix.len()..]);
    }
    None
}

fn extract_learnings(summary_md: &str) -> Vec<String> {
    let sec = {
        let s = extract_section(summary_md, "Learnings");
        if s.is_empty() {
            extract_section(summary_md, "Learnings & Pitfalls")
        } else {
            s
        }
    };
    let mut out = Vec::new();
    for line in sec.lines() {
        let cleaned = line
            .trim()
            .trim_start_matches("- ")
            .trim_start_matches("* ")
            .trim()
            .replace("**", "");
        if is_junk_hint(&cleaned) {
            continue;
        }
        let lower = cleaned.to_lowercase();
        if lower.starts_with("no repository")
            || lower.starts_with("no code")
            || lower.starts_with("no product")
            || lower.starts_with("no files")
            || lower.starts_with("read-only review")
            || lower.starts_with("the reviewed plan was ")
            || lower.starts_with("the four verified ")
            || lower.starts_with("the verified ")
        {
            continue;
        }
        let item = if cleaned.chars().count() > 420 {
            let prefix: String = cleaned.chars().take(420).collect();
            match prefix.rfind(' ') {
                Some(idx) if idx > 200 => format!("{}...", &prefix[..idx]),
                _ => format!("{prefix}..."),
            }
        } else {
            cleaned
        };
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

fn classify_learning_kind(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    const PITFALL_MARKERS: &[&str] = &[
        "does not",
        "do not",
        "did not",
        "cannot",
        "can't",
        "won't",
        "never",
        "refused",
        "rejected",
        "failed",
        "failure",
        "error",
        "bug",
        "broken",
        "stale",
        "missing",
        "unsupported",
        "read-only",
        "timeout",
        "timed out",
        "blocked",
        "pitfall",
        "gotcha",
        "caveat",
        "instead of",
        "rather than",
        "still requires",
        "requires configured",
        "alone does not",
        "fallback",
        "avoid",
        "warning",
    ];
    if PITFALL_MARKERS.iter().any(|m| lower.contains(m)) {
        "pitfall"
    } else {
        "learning"
    }
}

fn learning_stem_tokens(text: &str) -> std::collections::BTreeSet<String> {
    const STOPWORDS: &[&str] = &[
        "that", "this", "with", "from", "were", "have", "into", "when", "while", "their", "than",
        "also", "only", "still", "does", "alone", "both", "each", "some", "more", "most", "such",
        "used", "using", "through",
    ];
    text.split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 4 && !STOPWORDS.contains(&w.as_str()))
        .map(|mut w| {
            for suffix in ["ing", "ed", "es", "s"] {
                if w.len() > suffix.len() + 3 && w.ends_with(suffix) {
                    w.truncate(w.len() - suffix.len());
                    break;
                }
            }
            w
        })
        .collect()
}

fn first_subject_token(text: &str) -> Option<String> {
    let first = text
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .find(|w| !w.is_empty())?
        .to_lowercase();
    const GENERIC_STARTS: &[&str] = &[
        "the",
        "both",
        "when",
        "each",
        "some",
        "most",
        "this",
        "that",
        "these",
        "those",
        "after",
        "before",
        "during",
        "while",
        "since",
        "because",
        "initial",
        "component",
    ];
    if first.len() >= 5 && !GENERIC_STARTS.contains(&first.as_str()) {
        Some(first)
    } else {
        None
    }
}

fn is_similar_learning(existing: &str, candidate: &str) -> bool {
    if existing.eq_ignore_ascii_case(candidate) {
        return true;
    }
    if let (Some(a), Some(b)) = (
        first_subject_token(existing),
        first_subject_token(candidate),
    ) {
        if a == b {
            return true;
        }
    }
    let tok_a = learning_stem_tokens(existing);
    let tok_b = learning_stem_tokens(candidate);
    let min_len = tok_a.len().min(tok_b.len());
    if min_len < 4 {
        return false;
    }
    let shared = tok_a.intersection(&tok_b).count();
    (shared as f64 / min_len as f64) >= 0.45
}

fn push_project_learning(
    map: &mut std::collections::BTreeMap<String, ProjectLearningsDto>,
    project: &str,
    session_title: &str,
    text: &str,
) {
    let entry = map
        .entry(project.to_string())
        .or_insert_with(|| ProjectLearningsDto {
            project: project.to_string(),
            pitfalls: Vec::new(),
            learnings: Vec::new(),
        });
    let already_present = entry
        .pitfalls
        .iter()
        .chain(entry.learnings.iter())
        .any(|existing| is_similar_learning(&existing.text, text));
    if already_present {
        return;
    }
    let kind = classify_learning_kind(text);
    let item = LearningItemDto {
        kind: kind.to_string(),
        text: text.to_string(),
        session_title: session_title.to_string(),
    };
    if kind == "pitfall" {
        if entry.pitfalls.len() < 6 {
            entry.pitfalls.push(item);
        }
    } else if entry.learnings.len() < 6 {
        entry.learnings.push(item);
    }
}

/// Parse 3-line `## Potential Skills/Commands` bullets (`- **[name]**: summary` +
/// `Trigger:` + `Why:`) into consolidated rules instead of fragmenting each line into a card.
fn extract_consolidated_rules(project: &str, summary_md: &str) -> Vec<ConsolidatedRuleDto> {
    let mut sec = extract_section(summary_md, "Potential Skills/Commands");
    if sec.is_empty() {
        sec = extract_section(summary_md, "Reusable Skills & Commands");
    }
    let mut rules: Vec<ConsolidatedRuleDto> = Vec::new();
    for raw_line in sec.lines() {
        let cleaned = raw_line
            .trim()
            .trim_start_matches("- ")
            .trim_start_matches("* ")
            .trim();
        if is_junk_hint(cleaned) {
            continue;
        }
        let plain = cleaned.trim_start_matches("**").trim();
        if let Some(rest) = strip_prefix_ci(plain, "trigger:") {
            let val = rest
                .trim_start_matches("**")
                .trim()
                .trim_start_matches(':')
                .trim();
            if !val.is_empty() {
                if let Some(last) = rules.last_mut() {
                    last.trigger = Some(val.to_string());
                }
            }
            continue;
        }
        if let Some(rest) = strip_prefix_ci(plain, "why:") {
            let val = rest
                .trim_start_matches("**")
                .trim()
                .trim_start_matches(':')
                .trim();
            if !val.is_empty() {
                if let Some(last) = rules.last_mut() {
                    last.why = Some(val.to_string());
                }
            }
            continue;
        }

        let (raw_name, raw_summary) = if let Some((l, r)) = cleaned.split_once("**:") {
            (l, r)
        } else if let Some((l, r)) = cleaned.split_once(": ") {
            (l, r)
        } else if let Some((l, r)) = cleaned.split_once(" — ") {
            (l, r)
        } else {
            (cleaned, "")
        };
        let name = raw_name
            .trim_matches(|c: char| matches!(c, '*' | '`' | '[' | ']' | ' ' | ':'))
            .to_string();
        let summary = raw_summary
            .trim()
            .trim_start_matches(['*', ':', '-', '—'])
            .trim()
            .replace("**", "");
        if name.is_empty() || is_junk_hint(&name) {
            continue;
        }
        rules.push(ConsolidatedRuleDto {
            project: project.to_string(),
            name,
            summary: if summary.is_empty() {
                cleaned.replace("**", "")
            } else {
                summary
            },
            trigger: None,
            why: None,
        });
    }
    rules
}

#[allow(clippy::too_many_arguments)]
fn build_workflow_coaching(
    session_count: usize,
    work_count: usize,
    review_count: usize,
    blocked_count: usize,
    day_metrics: &DayMetricsDto,
    project_gains: &[ProjectGainDto],
    waste_alerts: &[WasteAlertDto],
    cost_by_session: &mut [(String, String, String, f64)],
    followup_candidates: &[(String, String, String)],
    empty_reviewer_noted: bool,
) -> (Vec<String>, Vec<NextStepDto>) {
    let mut signals: Vec<String> = Vec::new();
    let mut next_steps: Vec<NextStepDto> = Vec::new();

    for p in project_gains {
        if p.review_count > p.work_count && p.review_count >= 3 {
            let detail = if empty_reviewer_noted {
                format!(
                    "Agents ran {} code-review sessions for {} coding sessions in {} because a reviewer subagent returned empty output and forced extra retry rounds.",
                    p.review_count, p.work_count, p.project
                )
            } else {
                format!(
                    "Agents ran {} code-review sessions for {} coding sessions in {} — batch your edits before running pre-commit review so reviewers run once per change.",
                    p.review_count, p.work_count, p.project
                )
            };
            signals.push(format!(
                "High code-review overhead in {} ({} reviews vs {} coding sessions): {}",
                p.project, p.review_count, p.work_count, detail
            ));
            if next_steps.len() < 3 {
                next_steps.push(NextStepDto {
                    tag: "WORKFLOW".to_string(),
                    title: format!("Streamline pre-commit review gate in {}", p.project),
                    detail: if empty_reviewer_noted {
                        format!(
                            "{} review sessions ran for {} work sessions today because a reviewer subagent returned empty output. Auto-fallback to a secondary model when a reviewer returns empty and freeze staging until reviews finish.",
                            p.review_count, p.work_count
                        )
                    } else {
                        format!(
                            "{} review sessions ran for {} work sessions today. Batch staged edits before running review_triage so reviewers run once per slice.",
                            p.review_count, p.work_count
                        )
                    },
                });
            }
        }
    }

    cost_by_session.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
    if day_metrics.total_cost_usd >= 2.0 && !cost_by_session.is_empty() {
        let top_n = cost_by_session.len().min(2);
        let top_sum: f64 = cost_by_session[..top_n].iter().map(|x| x.3).sum();
        let share_pct = (top_sum / day_metrics.total_cost_usd * 100.0).round();
        let top_labels: Vec<String> = cost_by_session[..top_n]
            .iter()
            .map(|(t, _, rt, c)| format!("{t} [{rt}] (${c:.2})"))
            .collect();
        let cache_explanation = day_metrics
            .cache_hit_rate
            .map(|r| {
                let hit_pct = (r * 100.0).round() as u64;
                let full_pct = 100u64.saturating_sub(hit_pct);
                format!(
                    " {hit_pct}% of input tokens came from prompt cache ({full_pct}% were billed at full input price from new threads or expired cache)."
                )
            })
            .unwrap_or_default();
        signals.push(format!(
            "Biggest cost drivers (${:.2} total today): {} used {:.0}% of today's budget.{}",
            day_metrics.total_cost_usd,
            top_labels.join(" and "),
            share_pct,
            cache_explanation
        ));

        if let Some(rate) = day_metrics.cache_hit_rate {
            if rate < 0.50 && next_steps.len() < 3 {
                next_steps.push(NextStepDto {
                    tag: "COST".to_string(),
                    title: "Reuse warm threads to raise prompt-cache hit rate".to_string(),
                    detail: format!(
                        "Cache hit rate was {:.1}% across {:.2}M tokens (${:.2} total). Keep follow-up fixes in the same session thread instead of starting cold runs.",
                        rate * 100.0,
                        day_metrics.total_tokens as f64 / 1_000_000.0,
                        day_metrics.total_cost_usd
                    ),
                });
            }
        }
    }

    if day_metrics.failed_tool_calls > 0 || day_metrics.interrupted > 0 {
        let mut parts = Vec::new();
        if day_metrics.failed_tool_calls > 0 {
            let noun = if day_metrics.failed_tool_calls == 1 {
                "shell/tool command"
            } else {
                "shell/tool commands"
            };
            parts.push(format!(
                "{} {} exited with an error",
                day_metrics.failed_tool_calls, noun
            ));
        }
        if day_metrics.interrupted > 0 {
            let noun = if day_metrics.interrupted == 1 {
                "agent turn was stopped mid-run"
            } else {
                "agent turns were stopped mid-run"
            };
            parts.push(format!("{} {}", day_metrics.interrupted, noun));
        }
        let test_note = if day_metrics.test_runs > 0 {
            format!("across {} test suite runs", day_metrics.test_runs)
        } else {
            "(0 automated test suites ran)".to_string()
        };
        signals.push(format!(
            "Command errors & stopped turns: {} {}. Repeated command failures usually mean the agent is guessing CLI flags or file paths.",
            parts.join(" and "),
            test_note
        ));
        if day_metrics.failed_tool_calls >= 5 && next_steps.len() < 3 {
            next_steps.push(NextStepDto {
                tag: "WORKFLOW".to_string(),
                title: format!(
                    "Eliminate recurring tool call errors ({} failed today)",
                    day_metrics.failed_tool_calls
                ),
                detail: "Add a preflight check or skill rule for the command flags that failed repeatedly during today's runs.".to_string(),
            });
        }
    }

    for alert in waste_alerts.iter().take(1) {
        signals.push(format!(
            "Blocked background sessions ({}x in {}): Session exited before starting work — {}",
            alert.count, alert.project, alert.reason
        ));
        if next_steps.len() < 3 {
            next_steps.push(NextStepDto {
                tag: "WORKFLOW".to_string(),
                title: format!(
                    "Resolve {} blocked run(s) in {}",
                    alert.count, alert.project
                ),
                detail: alert.hint.clone(),
            });
        }
    }

    for (proj, title, detail) in followup_candidates {
        if next_steps.len() >= 3 {
            break;
        }
        next_steps.push(NextStepDto {
            tag: "FOLLOW-UP".to_string(),
            title: format!("Follow up on {title} ({proj})"),
            detail: detail.clone(),
        });
    }

    if signals.is_empty() && session_count > 0 {
        signals.push(format!(
            "Clean execution ({work_count} shipped work, {review_count} review, {blocked_count} blocked): All sessions completed with 0 command errors."
        ));
    }

    if next_steps.is_empty() {
        if let Some(top_proj) = project_gains.first() {
            if let Some(last_acc) = top_proj.accomplishments.last() {
                next_steps.push(NextStepDto {
                    tag: "FOLLOW-UP".to_string(),
                    title: format!("Continue {} momentum", top_proj.project),
                    detail: format!("Latest shipped milestone: {last_acc}"),
                });
            }
        }
    }

    signals.truncate(4);
    next_steps.truncate(3);
    (signals, next_steps)
}
fn normalize_display_path(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches("./");
    if let Some((_, after_repos)) = trimmed.split_once("/git-repos/") {
        return after_repos.to_string();
    }
    if trimmed.starts_with('/') {
        let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() > 3 {
            return parts[parts.len() - 3..].join("/");
        }
    }
    trimmed.to_string()
}

fn extract_files_touched(summary_md: &str) -> Vec<String> {
    let sec = extract_section(summary_md, "Code Changes");
    let mut files = Vec::new();
    for line in sec.lines() {
        let trimmed = line.trim();
        if is_junk_hint(trimmed) {
            continue;
        }
        let mut rest = trimmed;
        while let Some(open) = rest.find('`') {
            let after_open = &rest[open + 1..];
            let Some(close) = after_open.find('`') else {
                break;
            };
            let candidate = after_open[..close].trim();
            rest = &after_open[close + 1..];
            if !candidate.is_empty()
                && !candidate.contains(' ')
                && (candidate.contains('/') || candidate.contains('.'))
                && !candidate.starts_with("http")
            {
                let clean = normalize_display_path(candidate);
                if !files.iter().any(|f| f == &clean) {
                    files.push(clean);
                    if files.len() >= 6 {
                        return files;
                    }
                }
            }
        }
    }
    files
}

fn classify_session(
    title: &str,
    summary_preview_text: &str,
    files_touched: &[String],
) -> (&'static str, Option<String>) {
    // Any session that modified code is shipped work, never a blocked survey or read-only review.
    if !files_touched.is_empty() {
        return ("work", None);
    }

    let lower_title = title.to_lowercase();
    let preview = summary_preview_text.to_lowercase();

    if (preview.contains("bwrap:") || preview.contains("rtm_newaddr"))
        && (preview.contains("operation not permitted")
            || preview.contains("could not")
            || lower_title.contains("blocked"))
    {
        return (
            "blocked",
            Some(
                "Sandbox loopback error (bwrap: Failed RTM_NEWADDR: Operation not permitted)"
                    .to_string(),
            ),
        );
    }
    if lower_title.contains("pong response")
        || lower_title.contains("pong-response")
        || lower_title.contains("exact pong")
        || lower_title.contains("stdin pong")
    {
        return (
            "blocked",
            Some("Automated CLI connectivity probe (PONG)".to_string()),
        );
    }
    if lower_title.starts_with("blocked ")
        || lower_title.ends_with(" blocked")
        || preview.contains("could not proceed because")
        || preview.contains("could not verify any")
        || preview.contains("never ran because")
    {
        let reason = if preview.contains("403") || preview.contains("model_access_denied") {
            "Model access denied (403 key_model_access_denied)".to_string()
        } else {
            "Blocked before execution (environment or permissions)".to_string()
        };
        return ("blocked", Some(reason));
    }

    if lower_title.contains("code-review")
        || lower_title.contains("code review")
        || lower_title.contains("correctness review")
        || lower_title.contains("review sub-agent")
        || lower_title.ends_with(" review")
        || preview.contains("review pass")
        || preview.contains("three reviewers completed")
    {
        return ("review", None);
    }

    ("work", None)
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

    #[test]
    fn junk_hint_filters_decorators_and_empty_extractions() {
        assert!(is_junk_hint(""));
        assert!(is_junk_hint("---"));
        assert!(is_junk_hint("# heading"));
        assert!(is_junk_hint("None identified in this session."));
        assert!(is_junk_hint("None."));
        assert!(is_junk_hint("None found."));
        assert!(is_junk_hint("(none)"));
        assert!(is_junk_hint("*Archived by Daily Context Archive System*"));
        assert!(!is_junk_hint(
            "Used rg to search the ledger for stale entries"
        ));
        assert!(!is_junk_hint(
            "Ran `kubectl rollout restart deploy/daybook`"
        ));
    }

    #[test]
    fn clean_title_and_classification_extract_signal_from_noise() {
        assert_eq!(
            clean_session_title("\"00_06-improve-cron-guidance\""),
            "Improve cron guidance"
        );
        assert_eq!(
            clean_session_title("00:01-hermes-cron-review"),
            "Hermes cron review"
        );
        assert_eq!(
            clean_session_title("Cron final code-review"),
            "Cron final code-review"
        );

        let blocked_md = "## Summary\nThe session could not proceed because bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted.\n\n## Key Decisions & Trade-offs\n- No repository or memory conclusions were made.\n\n## Code Changes\n- None.";
        let bfiles = extract_files_touched(blocked_md);
        let (cat, reason) = classify_session(
            "Repository survey blocked",
            &summary_preview(blocked_md),
            &bfiles,
        );
        assert_eq!(cat, "blocked");
        assert!(reason.unwrap().contains("bwrap"));
        assert_eq!(extract_key_decision(blocked_md), None);

        // False-positive guard: a real work session titled "Fix blocked pipeline" or mentioning
        // "operation not permitted" while modifying files must stay classified as "work".
        let false_pos_md = "## Summary\nFixed the operation not permitted error in the blocked pipeline.\n\n## Code Changes\n- `src/pipeline.rs`: Fixed permissions.";
        let fp_files = extract_files_touched(false_pos_md);
        let (fp_cat, fp_reason) = classify_session(
            "Fix blocked pipeline",
            &summary_preview(false_pos_md),
            &fp_files,
        );
        assert_eq!(fp_cat, "work");
        assert!(fp_reason.is_none());

        // 403 model_access_denied classification
        let denied_md = "## Summary\nThe session could not proceed because 403 key_model_access_denied.\n\n## Code Changes\n- None.";
        let (dcat, dreason) = classify_session(
            "Blocked survey",
            &summary_preview(denied_md),
            &extract_files_touched(denied_md),
        );
        assert_eq!(dcat, "blocked");
        assert!(dreason.unwrap().contains("403"));

        let work_md = "## Summary\nUpdated important-mail cron to enforce strict allowlist.\n\n## Key Decisions & Trade-offs\n- Kept allowlist in important-mail.yaml instead of global config to avoid false positives.\n\n## Code Changes\n- `/home/agent/git-repos/hermes/cron/important-mail.yaml`: Added sender allowlist.";
        let wfiles = extract_files_touched(work_md);
        let (wcat, wreason) =
            classify_session("Improve cron guidance", &summary_preview(work_md), &wfiles);
        assert_eq!(wcat, "work");
        assert!(wreason.is_none());
        assert_eq!(
            extract_key_decision(work_md).as_deref(),
            Some("Kept allowlist in important-mail.yaml instead of global config to avoid false positives.")
        );
        assert_eq!(wfiles, vec!["hermes/cron/important-mail.yaml".to_string()]);

        let review_md = "## Summary\nCompleted read-only review pass across 3 files.\n\n## Code Changes\n- None.";
        let (rcat, _) = classify_session(
            "Hermes cron review",
            &summary_preview(review_md),
            &extract_files_touched(review_md),
        );
        assert_eq!(rcat, "review");

        // EDT evening (Oct 31 00:15 UTC = Oct 30 20:15 EDT) -> 2026-10-30
        assert_eq!(
            crate::ledger::normalize_journal_date("2026-10-31", Some("2026-10-31T00:15:00Z")),
            "2026-10-30"
        );
        // EST winter boundary (Jan 1 02:30 UTC = Dec 31 21:30 EST) -> 2026-12-31
        assert_eq!(
            crate::ledger::normalize_journal_date("2027-01-01", Some("2027-01-01T02:30:00Z")),
            "2026-12-31"
        );
        // Fallbacks for None / empty / invalid RFC3339
        assert_eq!(
            crate::ledger::normalize_journal_date("2026-10-29", None),
            "2026-10-29"
        );
        assert_eq!(
            crate::ledger::normalize_journal_date("2026-10-29", Some("")),
            "2026-10-29"
        );
        assert_eq!(
            crate::ledger::normalize_journal_date("2026-10-29", Some("not-a-date")),
            "2026-10-29"
        );
        // Preserves explicit non-UTC RFC3339 client offsets as-is
        assert_eq!(
            crate::ledger::normalize_journal_date("2026-10-31", Some("2026-10-30T23:30:00+02:00")),
            "2026-10-30"
        );
    }

    #[test]
    fn session_duration_formats_edges() {
        assert_eq!(
            session_duration(Some("2026-10-30T10:00:00Z"), Some("2026-10-30T11:23:00Z")),
            Some("1h 23m".to_string())
        );
        assert_eq!(
            session_duration(Some("2026-10-30T10:00:00Z"), Some("2026-10-30T10:45:00Z")),
            Some("45m".to_string())
        );
        assert_eq!(
            session_duration(Some("2026-10-30T10:00:00Z"), Some("2026-10-30T10:00:30Z")),
            Some("0m".to_string())
        );
        // ended before started (even sub-minute) is not a duration
        assert_eq!(
            session_duration(Some("2026-10-30T10:00:30Z"), Some("2026-10-30T10:00:00Z")),
            None
        );
        assert_eq!(
            session_duration(Some("garbage"), Some("2026-10-30T11:00:00Z")),
            None
        );
        assert_eq!(session_duration(None, Some("2026-10-30T11:00:00Z")), None);
    }

    #[test]
    fn consolidated_rules_and_learnings_merge_three_line_entries() {
        let md = "## Summary\nAudited Kustomize overlays.\n\n## Learnings & Pitfalls\n- **Kustomize indirect references**: Check nested Kustomize overlays before flagging a directory as an orphan.\n- None.\n\n## Potential Skills/Commands\n- **[verify-api-field-population]**: Verify backend API field population before designing UI sections.\n  - Trigger: When a planned UI section renders empty despite matching API types.\n  - Why: Server handlers may return empty defaults for legacy CLI fields.\n- **`gitops-orphan-detection`**: Trace nested Kustomize overlays before flagging a directory.\n  - Trigger: Directory appears unreferenced in top-level ArgoCD apps.\n  - Why: Kustomize bases reference directories indirectly.\n";
        let rules = extract_consolidated_rules("scratch", md);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].project, "scratch");
        assert_eq!(rules[0].name, "verify-api-field-population");
        assert_eq!(
            rules[0].summary,
            "Verify backend API field population before designing UI sections."
        );
        assert_eq!(
            rules[0].trigger.as_deref(),
            Some("When a planned UI section renders empty despite matching API types.")
        );
        assert_eq!(
            rules[0].why.as_deref(),
            Some("Server handlers may return empty defaults for legacy CLI fields.")
        );
        assert_eq!(rules[1].name, "gitops-orphan-detection");
        assert!(rules[1].trigger.is_some());
        assert!(rules[1].why.is_some());

        let learnings = extract_learnings(md);
        assert_eq!(
            learnings,
            vec!["Kustomize indirect references: Check nested Kustomize overlays before flagging a directory as an orphan.".to_string()]
        );
    }
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

    #[test]
    fn workflow_coaching_computes_signals_and_next_steps() {
        let metrics = DayMetricsDto {
            total_tokens: 1_500_000,
            input_tokens: 900_000,
            output_tokens: 100_000,
            cache_read_tokens: 400_000,
            cache_creation_tokens: 100_000,
            cache_hit_rate: Some(0.308),
            total_cost_usd: 10.0,
            test_runs: 12,
            failed_tool_calls: 6,
            interrupted: 1,
        };
        let gains = vec![ProjectGainDto {
            project: "daybook".to_string(),
            work_count: 5,
            review_count: 8,
            blocked_count: 0,
            total_cost_usd: 8.5,
            accomplishments: vec!["Shipped v0.1.17".to_string()],
            key_decisions: vec![],
            files_touched: vec![],
        }];
        let alerts = vec![WasteAlertDto {
            project: "agent-farm".to_string(),
            count: 2,
            cost_usd: 0.0,
            reason: "Blocked by bwrap".to_string(),
            hint: "Disable survey".to_string(),
        }];
        let mut costs = vec![
            (
                "Session A".to_string(),
                "daybook".to_string(),
                "claude".to_string(),
                6.0,
            ),
            (
                "Session B".to_string(),
                "daybook".to_string(),
                "codex".to_string(),
                2.5,
            ),
        ];
        let followups = vec![(
            "home-ops".to_string(),
            "Flux audit".to_string(),
            "Deferred drift cleanup".to_string(),
        )];
        let (signals, next_steps) = build_workflow_coaching(
            15, 5, 8, 2, &metrics, &gains, &alerts, &mut costs, &followups, true,
        );
        assert_eq!(signals.len(), 4);
        assert!(signals[0]
            .contains("High code-review overhead in daybook (8 reviews vs 5 coding sessions)"));
        assert!(signals[0].contains("returned empty output"));
        assert!(signals[1].contains("Biggest cost drivers ($10.00 total today)"));
        assert!(signals[1].contains("85% of today's budget"));
        assert!(signals[2].contains(
            "Command errors & stopped turns: 6 shell/tool commands exited with an error"
        ));
        assert!(signals[3].contains("Blocked background sessions (2x in agent-farm)"));
        assert_eq!(next_steps.len(), 3);
        assert_eq!(next_steps[0].tag, "WORKFLOW");
        assert_eq!(next_steps[1].tag, "COST");
        assert_eq!(next_steps[2].tag, "WORKFLOW");

        // Clean-execution & momentum fallback when no friction signals or follow-ups exist
        let clean_metrics = DayMetricsDto::default();
        let clean_gains = vec![ProjectGainDto {
            project: "daybook".to_string(),
            work_count: 5,
            review_count: 0,
            blocked_count: 0,
            total_cost_usd: 1.0,
            accomplishments: vec!["Shipped v0.1.17".to_string()],
            key_decisions: vec![],
            files_touched: vec![],
        }];
        let mut no_costs = Vec::new();
        let (clean_signals, clean_steps) = build_workflow_coaching(
            5,
            5,
            0,
            0,
            &clean_metrics,
            &clean_gains,
            &[],
            &mut no_costs,
            &[],
            false,
        );
        assert_eq!(clean_signals.len(), 1);
        assert!(clean_signals[0].contains("Clean execution (5 shipped work, 0 review, 0 blocked)"));
        assert_eq!(clean_steps.len(), 1);
        assert_eq!(clean_steps[0].tag, "FOLLOW-UP");
        assert!(clean_steps[0].title.contains("Continue daybook momentum"));
        assert_eq!(
            extract_learnings("## Learnings\n- Always verify heading prefixes.\n"),
            vec!["Always verify heading prefixes.".to_string()]
        );

        // Project learnings grouping, pitfall classification, and near-duplicate deduplication
        let mut lmap = std::collections::BTreeMap::new();
        push_project_learning(
            &mut lmap,
            "t3code",
            "Fix skill toggle",
            "The text fallback appeared when skill toggling was unsupported or settings were read-only.",
        );
        push_project_learning(
            &mut lmap,
            "t3code",
            "Fix skill toggle",
            "The settings panel is shared by web and desktop, so one change covered both surfaces.",
        );
        push_project_learning(
            &mut lmap,
            "scratch",
            "Survey 1",
            "aidevops combines agent instructions, executable tools, and workflows. Infrastructure access still requires configuration.",
        );
        push_project_learning(
            &mut lmap,
            "scratch",
            "Survey 2",
            "aidevops structures an AI assistant's work through instructions and tools. Installation alone does not grant infrastructure access.",
        );
        assert_eq!(lmap["t3code"].pitfalls.len(), 1);
        assert_eq!(lmap["t3code"].learnings.len(), 1);
        assert_eq!(
            lmap["scratch"].pitfalls.len() + lmap["scratch"].learnings.len(),
            1
        );
    }
}
