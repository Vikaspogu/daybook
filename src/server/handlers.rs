use axum::{
    extract::{Extension, Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::{Arc, RwLock};

use crate::archive::ArchiveManager;
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
pub struct AuthedLabel(pub String);

/// Rejects requests without a valid bearer token before axum buffers and
/// parses the JSON body; also stamps the token's last_used_at.
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let authed = match req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        Some(secret) => match state.ledger.check_token(secret) {
            Ok(label) => label,
            // DB errors propagate: a broken ledger must not masquerade as 401.
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
        Some(label) => {
            req.extensions_mut().insert(AuthedLabel(label));
            next.run(req).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<()>::error("invalid or missing token")),
        )
            .into_response(),
    }
}

/// POST /api/ingest — append or refresh a session record pushed by a
/// client machine. Runs behind `auth_middleware` and the 1 MiB body cap
/// set in the router.
pub async fn ingest(
    State(state): State<Arc<AppState>>,
    Extension(authed): Extension<AuthedLabel>,
    Json(record): Json<SessionRecord>,
) -> impl IntoResponse {
    if let Err(msg) = validate_record(&record) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<&str>::error(msg)),
        );
    }
    // Containment: a token minted for one machine must not write rows for another.
    if authed.0 != record.machine {
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
    if chrono::NaiveDate::parse_from_str(&record.journal_date, "%Y-%m-%d").is_err() {
        return Err("journal_date must be a valid YYYY-MM-DD date".into());
    }
    if record.version < 1 {
        return Err("version must be >= 1".into());
    }
    Ok(())
}

/// List all available dates
pub async fn list_dates(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.read().unwrap().clone();
    let manager = ArchiveManager::new(config);

    match manager.list_dates() {
        Ok(dates) => {
            let date_infos: Vec<DateInfo> = dates
                .into_iter()
                .map(|date| {
                    let sessions = manager.list_sessions(&date).unwrap_or_default();
                    let has_digest = manager
                        .read_daily_summary(&date)
                        .map(|content| {
                            content.contains("## Overview")
                                && !content.contains("No sessions recorded yet")
                        })
                        .unwrap_or(false);

                    DateInfo {
                        date,
                        session_count: sessions.len(),
                        has_digest,
                    }
                })
                .collect();

            Json(ApiResponse::success(date_infos))
        }
        Err(e) => Json(ApiResponse::<Vec<DateInfo>>::error(e.to_string())),
    }
}

/// Get daily summary for a specific date
pub async fn get_daily_summary(
    State(state): State<Arc<AppState>>,
    Path(date): Path<String>,
) -> impl IntoResponse {
    let config = state.config.read().unwrap().clone();
    let manager = ArchiveManager::new(config);

    match manager.read_daily_summary(&date) {
        Ok(content) => {
            let file_path = manager.daily_summary_path(&date);
            let mut summary = parse_daily_summary(&date, &content);
            summary.file_path = file_path.to_string_lossy().to_string();
            Json(ApiResponse::success(summary))
        }
        Err(e) => Json(ApiResponse::<DailySummaryDto>::error(e.to_string())),
    }
}

/// List sessions for a specific date
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Path(date): Path<String>,
) -> impl IntoResponse {
    let config = state.config.read().unwrap().clone();
    let manager = ArchiveManager::new(config);

    match manager.list_sessions(&date) {
        Ok(sessions) => {
            let session_briefs: Vec<SessionBrief> = sessions
                .into_iter()
                .filter_map(|name| {
                    manager.read_session(&date, &name).ok().map(|content| {
                        let (title, summary) = extract_session_preview(&content);
                        SessionBrief {
                            name,
                            title,
                            summary_preview: summary,
                        }
                    })
                })
                .collect();

            Json(ApiResponse::success(session_briefs))
        }
        Err(e) => Json(ApiResponse::<Vec<SessionBrief>>::error(e.to_string())),
    }
}

/// Get session details
pub async fn get_session(
    State(state): State<Arc<AppState>>,
    Path((date, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let config = state.config.read().unwrap().clone();
    let manager = ArchiveManager::new(config);

    match manager.read_session(&date, &name) {
        Ok(content) => {
            let metadata = extract_session_metadata(&content);
            let file_path = manager.session_archive_path(&date, &name);
            let detail = SessionDetailDto {
                name,
                content,
                metadata,
                file_path: file_path.to_string_lossy().to_string(),
            };
            Json(ApiResponse::success(detail))
        }
        Err(e) => Json(ApiResponse::<SessionDetailDto>::error(e.to_string())),
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

fn parse_daily_summary(date: &str, content: &str) -> DailySummaryDto {
    let extract_section = |header: &str| -> Option<String> {
        let pattern = format!("## {}\n", header);
        if let Some(start) = content.find(&pattern) {
            let start = start + pattern.len();
            let end = content[start..]
                .find("\n## ")
                .map(|i| start + i)
                .unwrap_or(content.len());
            let section = content[start..end].trim().to_string();
            if section.is_empty() || section == "No sessions recorded yet." {
                None
            } else {
                Some(section)
            }
        } else {
            None
        }
    };

    // Parse a section into cards by splitting on ### subsections.
    // Backward compat: if no ### found, wrap entire section as a single card.
    let extract_cards = |header: &str| -> Vec<SummaryCardDto> {
        let section = match extract_section(header) {
            Some(s) => s,
            None => return Vec::new(),
        };

        let mut cards: Vec<SummaryCardDto> = Vec::new();
        let mut current_title: Option<String> = None;
        let mut current_lines: Vec<String> = Vec::new();

        for line in section.lines() {
            if let Some(title) = line.strip_prefix("### ") {
                // Flush previous card
                if let Some(prev_title) = current_title.take() {
                    let body = current_lines.join("\n").trim().to_string();
                    if !body.is_empty() {
                        cards.push(SummaryCardDto {
                            title: prev_title,
                            content: body,
                        });
                    }
                }
                current_title = Some(title.trim().to_string());
                current_lines.clear();
            } else {
                current_lines.push(line.to_string());
            }
        }

        // Flush last card
        if let Some(prev_title) = current_title {
            let body = current_lines.join("\n").trim().to_string();
            if !body.is_empty() {
                cards.push(SummaryCardDto {
                    title: prev_title,
                    content: body,
                });
            }
        }

        // Backward compatibility: if no ### subsections found, wrap as single card
        if cards.is_empty() && !section.is_empty() {
            cards.push(SummaryCardDto {
                title: header.to_string(),
                content: section,
            });
        }

        cards
    };

    // Extract session names from frontmatter or content
    let sessions: Vec<String> = if let Some(start) = content.find("sessions:") {
        let start = start + 9;
        let end = content[start..]
            .find("\n---")
            .or_else(|| content[start..].find("\ntags:"))
            .map(|i| start + i)
            .unwrap_or(content.len());
        content[start..end]
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                line.strip_prefix("- ")
                    .map(|stripped| stripped.trim_matches('"').to_string())
            })
            .collect()
    } else {
        Vec::new()
    };

    // Skills & Commands live under "## Skills & Commands Identified" with
    // "### Potential Skills" and "### Potential Commands" as sub-markers.
    // The LLM-generated card content may contain rogue ## headings (e.g. "## 可复用技能"),
    // so we locate these markers directly in the full content rather than relying on
    // extract_section which would be broken by embedded ## headings.
    let extract_skills_commands = || -> (Vec<SummaryCardDto>, Vec<SummaryCardDto>) {
        let skills_marker = "\n### Potential Skills\n";
        let commands_marker = "\n### Potential Commands\n";

        // Also support file starting with the marker (no leading newline)
        let skills_start = content
            .find(skills_marker)
            .map(|i| i + skills_marker.len())
            .or_else(|| {
                content
                    .strip_prefix("### Potential Skills\n")
                    .map(|_| "### Potential Skills\n".len())
            });
        let commands_start = content
            .find(commands_marker)
            .map(|i| i + commands_marker.len());

        // Find the footer marker: "---" followed by "*Generated" or "*Archived"
        let find_footer = |from: usize| -> usize {
            let mut search_from = from;
            loop {
                match content[search_from..].find("\n---\n") {
                    Some(pos) => {
                        let after = search_from + pos + 4; // skip past "\n---\n"
                        if content[after..].starts_with('*') {
                            return search_from + pos;
                        }
                        // This "---" is a content separator (e.g. between cards), skip it
                        search_from = after;
                    }
                    None => return content.len(),
                }
            }
        };

        // Skills section: from after "### Potential Skills\n" to "### Potential Commands\n" (or footer)
        let skills_text = skills_start.map(|start| {
            let end = content[start..]
                .find(commands_marker)
                .map(|i| start + i)
                .unwrap_or_else(|| find_footer(start));
            content[start..end].trim()
        });

        // Commands section: from after "### Potential Commands\n" to the footer
        let commands_text = commands_start.map(|start| {
            let end = find_footer(start);
            content[start..end].trim()
        });

        let parse_cards_from_text = |text: Option<&str>| -> Vec<SummaryCardDto> {
            let section = match text {
                Some(s) if !s.is_empty() => s,
                _ => return Vec::new(),
            };
            let mut cards: Vec<SummaryCardDto> = Vec::new();
            let mut current_title: Option<String> = None;
            let mut current_lines: Vec<String> = Vec::new();

            for line in section.lines() {
                // Card titles are ### level; skip any ## lines (LLM noise)
                if let Some(title) = line.strip_prefix("### ") {
                    if let Some(prev_title) = current_title.take() {
                        let body = current_lines.join("\n").trim().to_string();
                        if !body.is_empty() {
                            cards.push(SummaryCardDto {
                                title: prev_title,
                                content: body,
                            });
                        }
                    }
                    current_title = Some(title.trim().to_string());
                    current_lines.clear();
                } else if line.starts_with("## ") {
                    // Skip rogue H2 headings embedded in LLM content
                    continue;
                } else {
                    current_lines.push(line.to_string());
                }
            }
            if let Some(prev_title) = current_title {
                let body = current_lines.join("\n").trim().to_string();
                if !body.is_empty() {
                    cards.push(SummaryCardDto {
                        title: prev_title,
                        content: body,
                    });
                }
            }
            cards
        };

        let skills = parse_cards_from_text(skills_text);
        let commands = parse_cards_from_text(commands_text);
        (skills, commands)
    };

    let (skills, commands) = extract_skills_commands();

    DailySummaryDto {
        date: date.to_string(),
        overview: extract_section("Overview").unwrap_or_default(),
        session_count: sessions.len(),
        sessions,
        insights: extract_cards("Key Insights"),
        skills,
        commands,
        reflections: extract_section("Reflections"),
        tomorrow_focus: extract_cards("Tomorrow's Focus"),
        raw_content: content.to_string(),
        file_path: String::new(), // Will be set by caller
    }
}

fn extract_session_preview(content: &str) -> (String, String) {
    // Extract title from frontmatter or first heading
    let title = if let Some(start) = content.find("title:") {
        let start = start + 6;
        let end = content[start..]
            .find('\n')
            .map(|i| start + i)
            .unwrap_or(content.len());
        content[start..end].trim().trim_matches('"').to_string()
    } else if let Some(start) = content.find("# ") {
        let start = start + 2;
        let end = content[start..]
            .find('\n')
            .map(|i| start + i)
            .unwrap_or(content.len());
        content[start..end].trim().to_string()
    } else {
        "Untitled".to_string()
    };

    // Extract summary section preview
    let summary = if let Some(start) = content.find("## Summary\n") {
        let start = start + 11;
        let end = content[start..]
            .find("\n## ")
            .map(|i| start + i)
            .unwrap_or_else(|| (start + 300).min(content.len()));
        let text = content[start..end].trim();
        if text.chars().count() > 200 {
            let truncated: String = text.chars().take(200).collect();
            format!("{}...", truncated)
        } else {
            text.to_string()
        }
    } else {
        String::new()
    };

    (title, summary)
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

fn extract_session_metadata(content: &str) -> SessionMetadata {
    let mut metadata = SessionMetadata::default();

    // Parse YAML frontmatter
    if let Some(stripped) = content.strip_prefix("---\n") {
        if let Some(end) = stripped.find("\n---") {
            let frontmatter = &stripped[..end];
            for line in frontmatter.lines() {
                if let Some((key, value)) = line.split_once(':') {
                    let key = key.trim();
                    let value = value.trim().trim_matches('"');
                    match key {
                        "title" => metadata.title = value.to_string(),
                        "date" => metadata.date = value.to_string(),
                        "session_id" => metadata.session_id = Some(value.to_string()),
                        "cwd" => metadata.cwd = Some(value.to_string()),
                        "git_branch" => metadata.git_branch = Some(value.to_string()),
                        "duration" => metadata.duration = Some(value.to_string()),
                        _ => {}
                    }
                }
            }
        }
    }

    metadata
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
        let state = Arc::new(AppState {
            config: RwLock::new(Config::default()),
            pricing: PricingData::from_map(std::collections::HashMap::new()),
            ledger,
        });
        (create_router(state.clone()), state, secret)
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
}
