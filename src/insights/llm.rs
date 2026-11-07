//! LLM-backed insights for the ledger-backed server.
//!
//! The file-based collectors ([`super::collector`], [`super::daily`]) read
//! local archives, facets and transcripts, which do not exist inside the
//! distroless server image. This module answers the same questions from the
//! SQLite ledger: it aggregates session facet data stored by ingest
//! (`sessions.facets_json`) and asks an OpenAI-compatible chat endpoint for a
//! narrative summary. When no LLM is configured (or the call fails) it falls
//! back to the deterministic recommendations in [`super::daily`].

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::daily::{most_common_key, top_n_keys, DayInsightSummary, SessionInsight};
use super::facets::SessionFacet;
use crate::config::InsightsConfig;

#[derive(Debug, Clone, Serialize)]
pub struct LlmInsights {
    pub days: usize,
    pub total_sessions: usize,
    pub sessions_with_facets: usize,
    pub top_goals: Vec<String>,
    pub top_frictions: Vec<String>,
    pub sessions_with_friction: usize,
    pub overall_satisfaction: Option<String>,
    pub llm_summary: Option<String>,
    pub recommendations: Vec<String>,
    /// `llm` or `fallback` (rule-based recommendations; see `error` for why).
    pub source: String,
    /// Why the LLM path was not used (config missing, HTTP error, bad JSON).
    pub error: Option<String>,
    /// Short-lived cache so the UI can poll cheaply. `0` when freshly computed.
    pub cached_seconds: u64,
}

struct CacheEntry {
    computed_at: std::time::Instant,
    data: LlmInsights,
}

static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<usize, CacheEntry>>> =
    std::sync::OnceLock::new();
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);
const MAX_SESSIONS_IN_PROMPT: usize = 20;

fn cache() -> &'static std::sync::Mutex<HashMap<usize, CacheEntry>> {
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Inclusive UTC `[from, to]` covering exactly `days` calendar days ending `today`.
/// Journal dates are bucketed in UTC, and both ledger queries share these bounds
/// so the counts stay comparable.
fn window_dates(today: chrono::NaiveDate, days: usize) -> (String, String) {
    let from = today - chrono::Duration::days(days.saturating_sub(1) as i64);
    (
        from.format("%Y-%m-%d").to_string(),
        today.format("%Y-%m-%d").to_string(),
    )
}

fn build_summary(
    window: &[(String, String, SessionFacet)],
    total_sessions: usize,
) -> (Vec<(String, SessionInsight)>, DayInsightSummary) {
    let mut sessions = Vec::new();
    for (date, session_id, facet) in window {
        sessions.push((
            date.clone(),
            SessionInsight {
                name: session_id.clone(),
                session_id: session_id.clone(),
                brief_summary: facet.brief_summary.clone(),
                outcome: facet.outcome.clone(),
                goal_categories: facet.goal_categories.keys().cloned().collect(),
                friction_types: facet.friction_counts.keys().cloned().collect(),
                friction_detail: facet.friction_detail.clone(),
                satisfaction: facet
                    .user_satisfaction_counts
                    .iter()
                    .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                    .map(|(k, _)| k.clone()),
                claude_helpfulness: facet.claude_helpfulness.clone(),
                token_usage: None,
            },
        ));
    }

    let mut all_goals: HashMap<String, usize> = HashMap::new();
    let mut all_frictions: HashMap<String, usize> = HashMap::new();
    let mut all_satisfaction: HashMap<String, usize> = HashMap::new();
    for (_, _, facet) in window {
        for (k, v) in &facet.goal_categories {
            *all_goals.entry(k.clone()).or_insert(0) += v;
        }
        for (k, v) in &facet.friction_counts {
            *all_frictions.entry(k.clone()).or_insert(0) += v;
        }
        for (k, v) in &facet.user_satisfaction_counts {
            *all_satisfaction.entry(k.clone()).or_insert(0) += v;
        }
    }

    let summary = DayInsightSummary {
        total_sessions,
        sessions_with_friction: sessions
            .iter()
            .filter(|s| !s.1.friction_types.is_empty())
            .count(),
        overall_satisfaction: most_common_key(&all_satisfaction),
        top_goals: top_n_keys(&all_goals, 3),
        top_frictions: top_n_keys(&all_frictions, 3),
        recommendations: Vec::new(),
        total_tokens: None,
        total_cost_usd: None,
        model_distribution: Vec::new(),
    };
    (sessions, summary)
}

fn build_prompt(
    sessions: &[(String, SessionInsight)],
    summary: &DayInsightSummary,
    days: usize,
) -> String {
    let mut out = format!(
        "You are analyzing a developer's AI coding-agent sessions over the last {days} days.\n\
         Sessions: {total}, with friction: {fr}, top goals: {goals:?}, top friction: {fri:?}.\n\
         Sessions (one per line):\n",
        total = summary.total_sessions,
        fr = summary.sessions_with_friction,
        goals = summary.top_goals,
        fri = summary.top_frictions,
    );
    for (date, s) in sessions.iter().take(MAX_SESSIONS_IN_PROMPT) {
        out.push_str(&format!(
            "- {} goals={:?} outcome={} friction={:?} satisfaction={} summary={}\n",
            date,
            s.goal_categories,
            s.outcome.as_deref().unwrap_or("unknown"),
            s.friction_types,
            s.satisfaction.as_deref().unwrap_or("unknown"),
            s.brief_summary
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect::<String>(),
        ));
    }
    out.push_str(
        "\nReturn JSON: {\"summary\": \"<3-4 sentence paragraph on work patterns, friction, and momentum>\", \
         \"recommendations\": [\"<concrete change>\", ...]}. 3-6 recommendations, each actionable in this repo's context.",
    );
    out
}

#[derive(Deserialize)]
struct LlmResponse {
    summary: String,
    #[serde(default)]
    recommendations: Vec<String>,
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

async fn call_llm(cfg: &InsightsConfig, prompt: &str) -> Result<LlmResponse> {
    let base_url = cfg
        .base_url
        .as_deref()
        .context("insights.base_url not configured")?;
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let model = cfg.model.as_deref().unwrap_or("gpt-4o-mini");

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.3,
        "max_tokens": cfg.max_tokens,
    });

    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("reqwest client")
    });
    let mut req = client.post(&url).json(&body);
    if let Some(key) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }
    let resp: serde_json::Value = req.send().await?.error_for_status()?.json().await?;
    let content = resp["choices"][0]["message"]["content"]
        .as_str()
        .context("LLM response missing choices[0].message.content")?;
    let json = extract_json_object(content).context("LLM response had no JSON object")?;
    let parsed: LlmResponse =
        serde_json::from_str(json).context("LLM JSON did not match schema")?;
    Ok(parsed)
}

/// Deterministic recommendations from the ledger window (used when no LLM is
/// configured or the call fails). Delegates to the same rules the local CLI
/// uses, fed with counts aggregated from the ledger facets.
fn fallback_recommendations(window: &[(String, String, SessionFacet)]) -> Vec<String> {
    let mut friction: HashMap<String, usize> = HashMap::new();
    let mut outcomes: HashMap<String, usize> = HashMap::new();
    let mut satisfaction: HashMap<String, usize> = HashMap::new();
    for (_, _, facet) in window {
        for (k, v) in &facet.friction_counts {
            *friction.entry(k.clone()).or_insert(0) += v;
        }
        if let Some(outcome) = &facet.outcome {
            *outcomes.entry(outcome.clone()).or_insert(0) += 1;
        }
        for (k, v) in &facet.user_satisfaction_counts {
            *satisfaction.entry(k.clone()).or_insert(0) += v;
        }
    }
    let with_friction = window
        .iter()
        .filter(|(_, _, f)| !f.friction_counts.is_empty())
        .count();
    super::daily::generate_recommendations(
        &friction,
        &outcomes,
        &satisfaction,
        with_friction,
        window.len(),
    )
}

/// Server-side insights: aggregate the ledger window, ask the LLM when
/// configured, otherwise return deterministic recommendations.
pub async fn collect(state: &crate::server::handlers::AppState, days: usize) -> LlmInsights {
    if let Some(hit) = cache()
        .lock()
        .unwrap()
        .get(&days)
        .filter(|e| e.computed_at.elapsed() < CACHE_TTL)
    {
        let mut data = hit.data.clone();
        data.cached_seconds = hit.computed_at.elapsed().as_secs();
        return data;
    }

    let mut config = state.config.read().unwrap().clone();
    config.insights = config.insights.with_env();
    let (from, to) = window_dates(chrono::Utc::now().date_naive(), days);
    let window = match state.ledger.facets_window(&from, &to) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[daybook] insights ledger query failed: {e:#}");
            return LlmInsights {
                days,
                total_sessions: 0,
                sessions_with_facets: 0,
                top_goals: vec![],
                top_frictions: vec![],
                sessions_with_friction: 0,
                overall_satisfaction: None,
                llm_summary: None,
                recommendations: vec![],
                source: "fallback".into(),
                error: Some("ledger query failed".into()),
                cached_seconds: 0,
            };
        }
    };

    let mut count_error = None;
    let total_sessions = match state.ledger.count_sessions_range(&from, &to) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("[daybook] insights session count failed: {e:#}");
            count_error = Some("session count unavailable".to_string());
            window.len()
        }
    };
    let (sessions, summary) = build_summary(&window, total_sessions);

    let (llm_summary, recommendations, source, error) = if window.is_empty() {
        (
            None,
            vec!["No sessions with facet data in this window yet. Run a scan so sessions are pushed to the ledger.".to_string()],
            "fallback".to_string(),
            None,
        )
    } else {
        let prompt = build_prompt(&sessions, &summary, days);
        match call_llm(&config.insights, &prompt).await {
            Ok(resp) => (
                Some(resp.summary),
                resp.recommendations,
                "llm".to_string(),
                None,
            ),
            Err(e) => {
                // Full error (may include internal base_url) stays in server logs only.
                eprintln!("[daybook] insights LLM call failed: {e:#}");
                let reason = if config.insights.base_url.as_deref().unwrap_or("").is_empty() {
                    "LLM not configured"
                } else {
                    "LLM call failed"
                };
                (
                    None,
                    fallback_recommendations(&window),
                    "fallback".to_string(),
                    Some(reason.to_string()),
                )
            }
        }
    };

    let data = LlmInsights {
        days,
        total_sessions,
        sessions_with_facets: window.len(),
        top_goals: summary.top_goals.clone(),
        top_frictions: summary.top_frictions.clone(),
        sessions_with_friction: summary.sessions_with_friction,
        overall_satisfaction: summary.overall_satisfaction.clone(),
        llm_summary,
        recommendations,
        source,
        error: error.or(count_error),
        cached_seconds: 0,
    };
    // Only cache real LLM answers so config fixes and transient failures retry at once.
    if data.source == "llm" && data.error.is_none() {
        cache().lock().unwrap().insert(
            days,
            CacheEntry {
                computed_at: std::time::Instant::now(),
                data: data.clone(),
            },
        );
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_object_handles_prose_and_fences() {
        let text =
            "Sure!\n```json\n{\"summary\": \"ok {nested}\", \"recommendations\": [\"a\"]}\n```";
        let json = extract_json_object(text).unwrap();
        let parsed: LlmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.summary, "ok {nested}");
        assert_eq!(parsed.recommendations, vec!["a"]);
        assert!(extract_json_object("no json here").is_none());
        assert!(extract_json_object("} backwards {").is_none());
    }

    #[test]
    fn test_window_dates_covers_exactly_n_days() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        assert_eq!(
            window_dates(today, 1),
            ("2026-03-01".into(), "2026-03-01".into())
        );
        assert_eq!(
            window_dates(today, 7),
            ("2026-02-23".into(), "2026-03-01".into())
        );
        assert_eq!(window_dates(today, 0).0, "2026-03-01");
    }

    #[test]
    fn test_llm_response_recommendations_default_empty() {
        let parsed: LlmResponse = serde_json::from_str(r#"{"summary": "s"}"#).unwrap();
        assert!(parsed.recommendations.is_empty());
    }
}
