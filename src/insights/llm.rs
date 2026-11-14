//! LLM-backed insights for the ledger-backed server.
//!
//! The file-based collectors ([`super::collector`], [`super::daily`]) read
//! local archives, facets and transcripts, which do not exist inside the
//! distroless server image. This module answers the same questions from the
//! SQLite ledger: every session's summary markdown and tool aggregates (all
//! runtimes), plus facets when Claude Code's `/insights` wrote them. It asks
//! an OpenAI-compatible chat endpoint for a narrative summary. When no LLM is
//! configured (or the call fails) it falls back to the deterministic
//! recommendations in [`super::daily`].

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::daily::{most_common_key, top_n_keys, DayInsightSummary, SessionInsight};
use crate::config::InsightsConfig;
use crate::ledger::WindowSession;

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

static CACHE: std::sync::OnceLock<
    std::sync::Mutex<HashMap<(usize, chrono::NaiveDate), CacheEntry>>,
> = std::sync::OnceLock::new();
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);
const MAX_SESSIONS_IN_PROMPT: usize = 20;

fn cache() -> &'static std::sync::Mutex<HashMap<(usize, chrono::NaiveDate), CacheEntry>> {
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Inclusive `[from, to]` covering exactly `days` calendar days ending `today`.
/// Journal dates use the configured local journal calendar; both ledger queries share these bounds
/// so the counts stay comparable.
pub(crate) fn window_dates(today: chrono::NaiveDate, days: usize) -> (String, String) {
    let from = today - chrono::Duration::days(days.saturating_sub(1) as i64);
    (
        from.format("%Y-%m-%d").to_string(),
        today.format("%Y-%m-%d").to_string(),
    )
}

/// Plain-text excerpt of a session summary: headings and HTML comments
/// dropped, whitespace collapsed, capped at `max` chars.
pub(crate) fn summary_excerpt(md: &str, max: usize) -> String {
    // YAML frontmatter holds cwd and transcript paths: never send it to the LLM.
    // Unclosed frontmatter (e.g. cut by the SQL 4000-char cap) yields nothing.
    let body = match md.strip_prefix("---") {
        Some(rest) => rest.split_once("\n---").map_or("", |(_, body)| body),
        None => md,
    };
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("<!--"))
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

fn build_summary(window: &[WindowSession]) -> (Vec<(String, SessionInsight)>, DayInsightSummary) {
    let mut sessions = Vec::new();
    let mut all_goals: HashMap<String, usize> = HashMap::new();
    let mut all_frictions: HashMap<String, usize> = HashMap::new();
    let mut all_satisfaction: HashMap<String, usize> = HashMap::new();
    for w in window {
        let facet = w.facet.clone().unwrap_or_default();
        for (k, v) in &facet.goal_categories {
            *all_goals.entry(k.clone()).or_insert(0) += v;
        }
        for (k, v) in &facet.friction_counts {
            *all_frictions.entry(k.clone()).or_insert(0) += v;
        }
        for (k, v) in &facet.user_satisfaction_counts {
            *all_satisfaction.entry(k.clone()).or_insert(0) += v;
        }
        sessions.push((
            w.journal_date.clone(),
            SessionInsight {
                name: w.session_id.clone(),
                session_id: w.session_id.clone(),
                brief_summary: facet
                    .brief_summary
                    .clone()
                    .or_else(|| Some(summary_excerpt(&w.summary_md, 300))),
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

    let summary = DayInsightSummary {
        total_sessions: window.len(),
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
    window: &[WindowSession],
    sessions: &[(String, SessionInsight)],
    summary: &DayInsightSummary,
    days: usize,
) -> String {
    let failed: u64 = window.iter().map(|w| w.aggregates.failed_tool_calls).sum();
    let interrupted: u64 = window.iter().map(|w| w.aggregates.interrupted).sum();
    let tests: u64 = window.iter().map(|w| w.aggregates.test_runs).sum();
    let mut out = format!(
        "You are summarizing a developer's AI coding-agent sessions over the last {days} days.\n\
         Sessions: {total}. Failed tool calls: {failed}. Interrupted turns: {interrupted}. Test runs: {tests}.\n\
         Sessions with Claude friction tags: {fr}. Top goals: {goals:?}. Top friction: {fri:?}.\n\
         Friction tags exist only for Claude sessions; use the failed and interrupted counts for all sessions.\n\
         Sessions, newest first (one per line):\n",
        total = summary.total_sessions,
        fr = summary.sessions_with_friction,
        goals = summary.top_goals,
        fri = summary.top_frictions,
    );
    for (w, (date, s)) in window.iter().zip(sessions).take(MAX_SESSIONS_IN_PROMPT) {
        let project = w.project.rsplit('/').next().unwrap_or(&w.project);
        out.push_str(&format!(
            "- {date} [{runtime}] {project} failed_tools={f} interrupted={i} tests={t}",
            runtime = w.runtime,
            f = w.aggregates.failed_tool_calls,
            i = w.aggregates.interrupted,
            t = w.aggregates.test_runs,
        ));
        if w.facet.is_some() {
            out.push_str(&format!(
                " goals={:?} outcome={} friction={:?}",
                s.goal_categories,
                s.outcome.as_deref().unwrap_or("unknown"),
                s.friction_types,
            ));
        }
        out.push_str(&format!(
            " summary: {}\n",
            s.brief_summary
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    out.push_str(
        "\nWrite for a busy developer. Use plain words, short sentences, and \"you\". \
         No jargon, no praise, no filler like \"demonstrated\" or \"momentum\". \
         Name real projects and real problems from the data. Do not invent metrics or tags.\n\
         Return JSON: {\"summary\": \"<2-3 short sentences: what you worked on and what went wrong>\", \
         \"recommendations\": [\"<one short sentence, max 15 words>\", ...]}. 3 recommendations, each a concrete next step.",
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
/// uses, fed with counts from the sessions that carry facets.
fn fallback_recommendations(window: &[WindowSession]) -> Vec<String> {
    let mut friction: HashMap<String, usize> = HashMap::new();
    let mut outcomes: HashMap<String, usize> = HashMap::new();
    let mut satisfaction: HashMap<String, usize> = HashMap::new();
    let facets: Vec<_> = window.iter().filter_map(|w| w.facet.as_ref()).collect();
    for facet in &facets {
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
    let with_friction = facets
        .iter()
        .filter(|f| !f.friction_counts.is_empty())
        .count();
    let mut recs = super::daily::generate_recommendations(
        &friction,
        &outcomes,
        &satisfaction,
        with_friction,
        facets.len(),
    );
    if recs.is_empty() && !window.is_empty() {
        recs.push(
            "No strong patterns from session facets. Configure the insights LLM for recommendations from session summaries.".to_string(),
        );
    }
    recs
}

/// Server-side insights: aggregate the ledger window, ask the LLM when
/// configured, otherwise return deterministic recommendations.
pub async fn collect(
    state: &crate::server::handlers::AppState,
    days: usize,
    today: chrono::NaiveDate,
) -> LlmInsights {
    if let Some(hit) = cache()
        .lock()
        .unwrap()
        .get(&(days, today))
        .filter(|e| e.computed_at.elapsed() < CACHE_TTL)
    {
        let mut data = hit.data.clone();
        data.cached_seconds = hit.computed_at.elapsed().as_secs();
        return data;
    }

    let mut config = state.config.read().unwrap().clone();
    config.insights = config.insights.with_env();
    let (from, to) = window_dates(today, days);
    let window = match state
        .ledger
        .insights_window(&from, &to, MAX_SESSIONS_IN_PROMPT)
    {
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

    let (sessions, summary) = build_summary(&window);

    let (llm_summary, recommendations, source, error) = if window.is_empty() {
        (
            None,
            vec![
                "No sessions in this window yet. Run a scan so sessions are pushed to the ledger."
                    .to_string(),
            ],
            "fallback".to_string(),
            None,
        )
    } else {
        let prompt = build_prompt(&window, &sessions, &summary, days);
        let prompt = crate::redaction::redact_for_config(&prompt, &config);
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
        total_sessions: window.len(),
        sessions_with_facets: window.iter().filter(|w| w.facet.is_some()).count(),
        top_goals: summary.top_goals.clone(),
        top_frictions: summary.top_frictions.clone(),
        sessions_with_friction: summary.sessions_with_friction,
        overall_satisfaction: summary.overall_satisfaction.clone(),
        llm_summary,
        recommendations,
        source,
        error,
        cached_seconds: 0,
    };
    // Only cache real LLM answers so config fixes and transient failures retry at once.
    if data.source == "llm" && data.error.is_none() {
        let mut cache = cache().lock().unwrap();
        // ponytail: bounded to eight windows; arbitrary caller dates cannot grow this map.
        if cache.len() >= 8 {
            if let Some(key) = cache.keys().next().copied() {
                cache.remove(&key);
            }
        }
        cache.insert(
            (days, today),
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
    use crate::ledger::AggregatesRecord;

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

    #[test]
    fn test_prompt_uses_summaries_without_facets() {
        let session = |id: &str, summary_md: &str, facet| WindowSession {
            journal_date: "2026-11-01".into(),
            session_id: id.into(),
            runtime: "pi".into(),
            project: "/home/agent/git-repos/daybook".into(),
            summary_md: summary_md.into(),
            facet,
            aggregates: AggregatesRecord {
                failed_tool_calls: 3,
                ..Default::default()
            },
            usage: Default::default(),
        };
        let facet = crate::insights::facets::SessionFacet {
            brief_summary: Some("Facet wins".into()),
            ..Default::default()
        };
        let window = vec![
            session(
                "s1",
                "---\ncwd: /secret/path\n---\n# Session\n<!-- meta -->\n## Summary\nFixed the   insights\nfallback.",
                None,
            ),
            session("s2", "## Summary\nIgnored.", Some(facet)),
        ];
        let (sessions, summary) = build_summary(&window);
        assert_eq!(summary.total_sessions, 2);
        assert_eq!(
            sessions[0].1.brief_summary.as_deref(),
            Some("Fixed the insights fallback.")
        );
        assert_eq!(sessions[1].1.brief_summary.as_deref(), Some("Facet wins"));
        let prompt = build_prompt(&window, &sessions, &summary, 7);
        assert!(prompt.contains("[pi] daybook failed_tools=3"));
        assert!(prompt.contains("summary: Fixed the insights fallback."));
        assert!(
            !prompt.contains("/secret/path"),
            "frontmatter never reaches the LLM"
        );
        assert_eq!(summary_excerpt("---\ncwd: /secret/path", 300), "");
        assert_eq!(
            fallback_recommendations(&window[..1]).len(),
            1,
            "no facets still yields a hint"
        );
        // Ratio rules count only facet-bearing sessions, so facet-less rows do not dilute them.
        let rough = crate::insights::facets::SessionFacet {
            friction_counts: HashMap::from([("tool_error".to_string(), 1)]),
            ..Default::default()
        };
        let mixed = vec![session("a", "", None), session("b", "", Some(rough))];
        assert!(fallback_recommendations(&mixed)
            .iter()
            .any(|r| r.starts_with("More than half")));
    }
}
