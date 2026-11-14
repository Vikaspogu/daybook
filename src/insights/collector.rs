use serde::Serialize;
use std::collections::HashMap;

use crate::archive::ArchiveManager;
use crate::config::Config;
use crate::usage::pricing::PricingData;
use crate::usage::scanner;
use crate::usage::types::{SessionUsage, UsageSummary};

use super::facets::SessionFacet;
use super::trends::TrendData;

/// Aggregated insights data from daily archives and Claude facets
#[derive(Debug, Clone, Serialize)]
pub struct InsightsData {
    pub total_days: usize,
    pub total_sessions: usize,
    pub daily_stats: Vec<DailyStat>,
    pub goal_distribution: Vec<CategoryCount>,
    pub friction_distribution: Vec<CategoryCount>,
    pub satisfaction_distribution: Vec<CategoryCount>,
    pub language_distribution: Vec<CategoryCount>,
    pub session_type_distribution: Vec<CategoryCount>,
    pub session_details: Vec<SessionInsight>,
    pub trends: Option<TrendData>,
    pub usage_summary: Option<UsageSummary>,
}

/// Per-session insight combining archive metadata with facet analysis data
#[derive(Debug, Clone, Serialize)]
pub struct SessionInsight {
    pub session_id: String,
    pub date: String,
    pub session_name: String,
    pub brief_summary: Option<String>,
    pub outcome: Option<String>,
    pub goal_categories: Vec<String>,
    pub friction_types: Vec<String>,
    pub friction_detail: Option<String>,
    pub satisfaction: Option<String>,
    pub claude_helpfulness: Option<String>,
    pub session_type: Option<String>,
    pub token_usage: Option<SessionUsage>,
}

/// Statistics for a single day
#[derive(Debug, Clone, Serialize)]
pub struct DailyStat {
    pub date: String,
    pub session_count: usize,
    pub has_digest: bool,
    pub total_tokens: Option<u64>,
    pub total_cost: Option<f64>,
}

/// A category name with its occurrence count
#[derive(Debug, Clone, Serialize)]
pub struct CategoryCount {
    pub name: String,
    pub count: usize,
}

impl InsightsData {
    /// Collect insights data from archives and facets.
    /// `days` limits the journal-date calendar window ending at `today`.
    pub fn collect(
        config: &Config,
        days: Option<usize>,
        today: chrono::NaiveDate,
        pricing: &PricingData,
    ) -> anyhow::Result<Self> {
        let manager = ArchiveManager::new(config.clone());
        let all_dates = manager.list_dates()?;

        let days_limit = days.unwrap_or(30);
        let (from, to) = super::llm::window_dates(today, days_limit);
        let dates: Vec<String> = all_dates
            .into_iter()
            .filter(|date| date.as_str() >= from.as_str() && date.as_str() <= to.as_str())
            .collect();

        // Scan all usage data upfront
        let all_session_usages = scanner::scan_all_sessions(None, pricing);
        let usage_summary = scanner::aggregate_usage(&all_session_usages, Some(&dates));

        // Build a lookup: date -> DailyUsage for merging into daily_stats
        let daily_usage_map: HashMap<String, &crate::usage::types::DailyUsage> = usage_summary
            .daily_usage
            .iter()
            .map(|d| (d.date.clone(), d))
            .collect();

        let mut daily_stats = Vec::new();
        let mut total_sessions = 0;

        for date in &dates {
            let sessions = manager.list_sessions(date).unwrap_or_default();
            let session_count = sessions.len();
            total_sessions += session_count;

            let has_digest = manager
                .read_daily_summary(date)
                .map(|content| {
                    content.contains("## Overview") && !content.contains("No sessions recorded yet")
                })
                .unwrap_or(false);

            let (total_tokens, total_cost) = if let Some(du) = daily_usage_map.get(date.as_str()) {
                (
                    Some(
                        du.input_tokens
                            + du.output_tokens
                            + du.cache_creation_tokens
                            + du.cache_read_tokens,
                    ),
                    Some(du.total_cost_usd),
                )
            } else {
                (None, None)
            };

            daily_stats.push(DailyStat {
                date: date.clone(),
                session_count,
                has_digest,
                total_tokens,
                total_cost,
            });
        }

        // Reverse so oldest first (for charts)
        daily_stats.reverse();

        // Load facets from Claude Code, indexed by session_id for fast lookup
        let facets = SessionFacet::load_all().unwrap_or_default();
        let facet_map: HashMap<String, &SessionFacet> = facets
            .iter()
            .map(|(id, facet)| (id.clone(), facet))
            .collect();

        // Build per-session details by scanning archive files and matching with facets
        let mut session_details = Vec::new();
        for date in &dates {
            let sessions = manager.list_sessions(date).unwrap_or_default();
            for session_name in &sessions {
                if let Ok(content) = manager.read_session(date, session_name) {
                    if let Some(session_id) = extract_session_id_from_frontmatter(&content) {
                        let token_usage = all_session_usages.get(&session_id).cloned();

                        session_details.push(session_insight(
                            &session_id,
                            date,
                            session_name,
                            facet_map.get(&session_id).copied(),
                            token_usage,
                        ));
                    }
                }
            }
        }
        let selected_facets: Vec<(String, SessionFacet)> = session_details
            .iter()
            .filter_map(|session| {
                facet_map
                    .get(&session.session_id)
                    .map(|facet| (session.session_id.clone(), (**facet).clone()))
            })
            .collect();
        // Aggregate goal_categories (HashMap<String, usize> per facet)
        let goal_distribution = aggregate_hashmap_field(&selected_facets, |f| &f.goal_categories);

        // Aggregate friction_counts (HashMap<String, usize> per facet)
        let friction_distribution =
            aggregate_hashmap_field(&selected_facets, |f| &f.friction_counts);

        // Aggregate user_satisfaction_counts (HashMap<String, usize> per facet)
        let satisfaction_distribution =
            aggregate_hashmap_field(&selected_facets, |f| &f.user_satisfaction_counts);

        // Aggregate session_type (single string per facet)
        let session_type_distribution =
            count_option_field(&selected_facets, |f| f.session_type.as_deref());

        // language_distribution is currently empty since facets don't carry language data
        let language_distribution = Vec::new();

        // Calculate trend data using dates in chronological order (oldest first)
        // daily_stats is already reversed to oldest-first at this point
        let chronological_dates: Vec<String> = daily_stats.iter().map(|s| s.date.clone()).collect();
        let trends = TrendData::calculate(config, &chronological_dates, days_limit);

        Ok(InsightsData {
            total_days: dates.len(),
            total_sessions,
            daily_stats,
            goal_distribution,
            friction_distribution,
            satisfaction_distribution,
            language_distribution,
            session_type_distribution,
            session_details,
            trends,
            usage_summary: Some(usage_summary),
        })
    }

    /// Build insights from ledger rows (all runtimes). Used when the server
    /// has no local archive, e.g. the deployed pod that only holds the ledger.
    pub fn from_ledger(rows: &[crate::ledger::WindowSession], days: usize) -> Self {
        // Distinct session dates, newest first (rows arrive in any order).
        let mut dates: Vec<String> = rows.iter().map(|r| r.journal_date.clone()).collect();
        dates.sort_unstable_by(|a, b| b.cmp(a));
        dates.dedup();

        let mut daily_usage: Vec<crate::usage::types::DailyUsage> = dates
            .iter()
            .rev()
            .map(|d| crate::usage::types::DailyUsage {
                date: d.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
                total_cost_usd: 0.0,
                session_count: 0,
            })
            .collect();
        for r in rows {
            if let Some(du) = daily_usage.iter_mut().find(|d| d.date == r.journal_date) {
                du.input_tokens += r.usage.input;
                du.output_tokens += r.usage.output;
                du.cache_creation_tokens += r.usage.cache_creation;
                du.cache_read_tokens += r.usage.cache_read;
                du.total_cost_usd += r.usage.cost_usd.unwrap_or(0.0);
                du.session_count += 1;
            }
        }

        // Oldest first, for charts.
        let daily_stats: Vec<DailyStat> = daily_usage
            .iter()
            .map(|du| DailyStat {
                date: du.date.clone(),
                session_count: du.session_count,
                has_digest: false,
                total_tokens: Some(
                    du.input_tokens
                        + du.output_tokens
                        + du.cache_creation_tokens
                        + du.cache_read_tokens,
                ),
                total_cost: Some(du.total_cost_usd),
            })
            .collect();

        let facets: Vec<(String, SessionFacet)> = rows
            .iter()
            .filter_map(|r| r.facet.clone().map(|f| (r.session_id.clone(), f)))
            .collect();

        let session_details = rows
            .iter()
            .map(|r| {
                let mut insight = session_insight(
                    &r.session_id,
                    &r.journal_date,
                    &r.project,
                    r.facet.as_ref(),
                    Some(SessionUsage {
                        session_id: r.session_id.clone(),
                        input_tokens: r.usage.input,
                        output_tokens: r.usage.output,
                        cache_creation_tokens: r.usage.cache_creation,
                        cache_read_tokens: r.usage.cache_read,
                        total_cost_usd: r.usage.cost_usd.unwrap_or(0.0),
                        model_calls: HashMap::new(),
                        first_timestamp: None,
                    }),
                );
                if insight.brief_summary.is_none() && !r.summary_md.is_empty() {
                    insight.brief_summary = Some(super::llm::summary_excerpt(&r.summary_md, 300));
                }
                // Sessions without a facet (Pi, Codex) still flag as problems
                // from the scan aggregates.
                if r.facet.is_none() {
                    let a = &r.aggregates;
                    let mut detail = Vec::new();
                    if a.failed_tool_calls > 0 {
                        insight.friction_types.push("tool_error".to_string());
                        detail.push(format!("{} failed tool calls", a.failed_tool_calls));
                    }
                    if a.interrupted > 0 {
                        insight.friction_types.push("interrupted".to_string());
                        detail.push(format!("{} interrupted turns", a.interrupted));
                    }
                    if !detail.is_empty() {
                        insight.friction_detail = Some(detail.join(", "));
                    }
                }
                insight
            })
            .collect();

        let chronological_dates: Vec<String> = daily_stats.iter().map(|s| s.date.clone()).collect();
        let usage_summary = UsageSummary {
            total_input_tokens: daily_usage.iter().map(|d| d.input_tokens).sum(),
            total_output_tokens: daily_usage.iter().map(|d| d.output_tokens).sum(),
            total_cache_creation_tokens: daily_usage.iter().map(|d| d.cache_creation_tokens).sum(),
            total_cache_read_tokens: daily_usage.iter().map(|d| d.cache_read_tokens).sum(),
            total_cost_usd: daily_usage.iter().map(|d| d.total_cost_usd).sum(),
            total_sessions: rows.len(),
            // ponytail: the ledger stores no per-model call counts.
            model_distribution: Vec::new(),
            daily_usage,
        };

        InsightsData {
            total_days: dates.len(),
            total_sessions: rows.len(),
            daily_stats,
            goal_distribution: aggregate_hashmap_field(&facets, |f| &f.goal_categories),
            friction_distribution: aggregate_hashmap_field(&facets, |f| &f.friction_counts),
            satisfaction_distribution: aggregate_hashmap_field(&facets, |f| {
                &f.user_satisfaction_counts
            }),
            language_distribution: Vec::new(),
            session_type_distribution: count_option_field(&facets, |f| f.session_type.as_deref()),
            session_details,
            trends: TrendData::from_window(rows, &chronological_dates, days),
            usage_summary: Some(usage_summary),
        }
    }
}

/// One session row for the timeline; facet fields stay empty without a facet.
fn session_insight(
    session_id: &str,
    date: &str,
    session_name: &str,
    facet: Option<&SessionFacet>,
    token_usage: Option<SessionUsage>,
) -> SessionInsight {
    SessionInsight {
        session_id: session_id.to_string(),
        date: date.to_string(),
        session_name: session_name.to_string(),
        brief_summary: facet.and_then(|f| f.brief_summary.clone()),
        outcome: facet.and_then(|f| f.outcome.clone()),
        goal_categories: facet
            .map(|f| f.goal_categories.keys().cloned().collect())
            .unwrap_or_default(),
        friction_types: facet
            .map(|f| f.friction_counts.keys().cloned().collect())
            .unwrap_or_default(),
        friction_detail: facet.and_then(|f| f.friction_detail.clone()),
        // Most common satisfaction level.
        satisfaction: facet.and_then(|f| {
            f.user_satisfaction_counts
                .iter()
                .max_by_key(|(_, count)| *count)
                .map(|(name, _)| name.clone())
        }),
        claude_helpfulness: facet.and_then(|f| f.claude_helpfulness.clone()),
        session_type: facet.and_then(|f| f.session_type.clone()),
        token_usage,
    }
}

/// Aggregate a HashMap<String, usize> field across all facets
fn aggregate_hashmap_field<F>(facets: &[(String, SessionFacet)], extractor: F) -> Vec<CategoryCount>
where
    F: Fn(&SessionFacet) -> &HashMap<String, usize>,
{
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_, facet) in facets {
        for (key, value) in extractor(facet) {
            *counts.entry(key.clone()).or_insert(0) += value;
        }
    }
    let mut result: Vec<CategoryCount> = counts
        .into_iter()
        .map(|(name, count)| CategoryCount { name, count })
        .collect();
    result.sort_by_key(|a| std::cmp::Reverse(a.count));
    result
}

/// Count occurrences of an Option<&str> field across all facets
fn count_option_field<F>(facets: &[(String, SessionFacet)], extractor: F) -> Vec<CategoryCount>
where
    F: Fn(&SessionFacet) -> Option<&str>,
{
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_, facet) in facets {
        if let Some(value) = extractor(facet) {
            *counts.entry(value.to_string()).or_insert(0) += 1;
        }
    }
    let mut result: Vec<CategoryCount> = counts
        .into_iter()
        .map(|(name, count)| CategoryCount { name, count })
        .collect();
    result.sort_by_key(|a| std::cmp::Reverse(a.count));
    result
}

/// Extract session_id from YAML frontmatter in a session archive markdown file.
/// Looks for `session_id: <value>` between `---` markers.
fn extract_session_id_from_frontmatter(content: &str) -> Option<String> {
    if let Some(stripped) = content.strip_prefix("---\n") {
        if let Some(end) = stripped.find("\n---") {
            let frontmatter = &stripped[..end];
            for line in frontmatter.lines() {
                if let Some((key, value)) = line.split_once(':') {
                    let key = key.trim();
                    if key == "session_id" {
                        let value = value.trim().trim_matches('"');
                        if !value.is_empty() {
                            return Some(value.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{UsageRecord, WindowSession};

    fn row(date: &str, id: &str, summary: &str, facet: Option<SessionFacet>) -> WindowSession {
        WindowSession {
            journal_date: date.into(),
            session_id: id.into(),
            runtime: "pi".into(),
            project: "daybook".into(),
            summary_md: summary.into(),
            facet,
            aggregates: Default::default(),
            usage: UsageRecord {
                input: 10,
                output: 5,
                cost_usd: Some(0.5),
                ..Default::default()
            },
        }
    }

    #[test]
    fn from_ledger_fills_panels_without_archive() {
        let facet = SessionFacet {
            brief_summary: Some("facet brief".into()),
            friction_counts: HashMap::from([("tool_error".to_string(), 1)]),
            ..Default::default()
        };
        let mut rows = vec![
            row(
                "2026-11-07",
                "a",
                "---\ncwd: /secret\n---\nShipped the fix.",
                None,
            ),
            row("2026-11-07", "b", "", Some(facet)),
            row("2026-11-08", "c", "", None),
        ];
        rows[2].aggregates.failed_tool_calls = 2;
        let data = InsightsData::from_ledger(&rows, 7);

        assert_eq!(data.total_days, 2);
        assert_eq!(data.total_sessions, 3);
        // Oldest first, counts and tokens per day.
        assert_eq!(data.daily_stats[0].date, "2026-11-07");
        assert_eq!(data.daily_stats[0].session_count, 2);
        assert_eq!(data.daily_stats[0].total_tokens, Some(30));
        assert_eq!(data.daily_stats[1].session_count, 1);
        assert!(data.trends.is_some());
        assert_eq!(data.friction_distribution[0].name, "tool_error");
        assert_eq!(data.usage_summary.as_ref().unwrap().total_cost_usd, 1.5);

        // Timeline excerpt comes from the summary, never the frontmatter.
        let a = data
            .session_details
            .iter()
            .find(|s| s.session_id == "a")
            .unwrap();
        assert_eq!(a.brief_summary.as_deref(), Some("Shipped the fix."));
        let b = data
            .session_details
            .iter()
            .find(|s| s.session_id == "b")
            .unwrap();
        assert_eq!(b.brief_summary.as_deref(), Some("facet brief"));

        // One day of history is not enough for trends.
        let one_day = InsightsData::from_ledger(&rows[..2], 7);
        assert!(one_day.trends.is_none());

        // Pi/Codex rows (no facet) flag as problems from scan aggregates.
        let c = data
            .session_details
            .iter()
            .find(|s| s.session_id == "c")
            .unwrap();
        assert_eq!(c.friction_types, vec!["tool_error".to_string()]);
        assert_eq!(c.friction_detail.as_deref(), Some("2 failed tool calls"));
        let a = data
            .session_details
            .iter()
            .find(|s| s.session_id == "a")
            .unwrap();
        assert!(a.friction_types.is_empty());
    }
}
