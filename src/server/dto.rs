use serde::{Deserialize, Serialize};


/// Generic API response wrapper
#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message.into()),
        }
    }
}

/// Date info for listing
#[derive(Serialize)]
pub struct DateInfo {
    pub date: String,
    pub session_count: usize,
    pub has_digest: bool,
}

/// Brief session info for listing
#[derive(Serialize)]
pub struct SessionBrief {
    pub name: String,
    pub title: String,
    pub summary_preview: String,
}

/// A single card within a daily summary section
#[derive(Serialize)]
pub struct SummaryCardDto {
    pub title: String,
    pub content: String,
}

/// Daily summary DTO
#[derive(Serialize)]
pub struct DailySummaryDto {
    pub date: String,
    pub overview: String,
    pub session_count: usize,
    pub sessions: Vec<String>,
    pub insights: Vec<SummaryCardDto>,
    pub skills: Vec<SummaryCardDto>,
    pub commands: Vec<SummaryCardDto>,
    pub reflections: Option<String>,
    pub tomorrow_focus: Vec<SummaryCardDto>,
    pub raw_content: String,
    pub file_path: String,
}

/// Session detail DTO
#[derive(Serialize)]
pub struct SessionDetailDto {
    pub name: String,
    pub content: String,
    pub metadata: SessionMetadata,
    pub file_path: String,
}

/// Session metadata extracted from frontmatter
#[derive(Serialize, Default)]
pub struct SessionMetadata {
    pub title: String,
    pub date: String,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub duration: Option<String>,
}

/// WebSocket message types
#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WsMessage {
    NewSession { date: String, name: String },
    DigestCompleted { date: String },
    Connected,
}

/// Config DTO for API responses
#[derive(Serialize)]
pub struct ConfigDto {
    pub storage_path: String,
    pub model: String,
    pub summary_language: String,
    pub enable_daily_summary: bool,
    pub enable_extraction_hints: bool,
    pub auto_digest_enabled: bool,
    pub digest_time: String,
    pub author: Option<String>,
    pub prompt_templates: PromptTemplatesDto,
    pub auto_summarize_enabled: bool,
    pub auto_summarize_on_show: bool,
    pub auto_summarize_inactive_minutes: u64,
}

/// Prompt templates DTO for API responses
#[derive(Serialize, Clone)]
pub struct PromptTemplatesDto {
    pub session_summary: Option<String>,
    pub daily_summary: Option<String>,
    pub skill_extract: Option<String>,
    pub command_extract: Option<String>,
}

/// Default templates DTO for API responses
#[derive(Serialize)]
pub struct DefaultTemplatesDto {
    pub session_summary_en: String,
    pub session_summary_zh: String,
    pub daily_summary_en: String,
    pub daily_summary_zh: String,
    pub skill_extract_en: String,
    pub skill_extract_zh: String,
    pub command_extract_en: String,
    pub command_extract_zh: String,
}

/// Insights data for the dashboard
#[derive(Serialize)]
pub struct InsightsDto {
    pub total_days: usize,
    pub total_sessions: usize,
    pub daily_stats: Vec<DailyStatDto>,
    pub goal_distribution: Vec<CategoryCountDto>,
    pub friction_distribution: Vec<CategoryCountDto>,
    pub satisfaction_distribution: Vec<CategoryCountDto>,
    pub language_distribution: Vec<CategoryCountDto>,
    pub session_type_distribution: Vec<CategoryCountDto>,
    pub session_details: Vec<SessionInsightDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trends: Option<TrendDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_summary: Option<UsageSummaryDto>,
}

/// Trend analysis data for period-over-period comparison
#[derive(Serialize)]
pub struct TrendDto {
    pub period_label: String,
    pub comparison_label: String,
    pub current_sessions: usize,
    pub previous_sessions: usize,
    pub sessions_change_pct: f64,
    pub current_friction_rate: f64,
    pub previous_friction_rate: f64,
    pub friction_change_pct: f64,
    pub current_success_rate: f64,
    pub previous_success_rate: f64,
    pub success_change_pct: f64,
    pub current_satisfaction_score: f64,
    pub previous_satisfaction_score: f64,
    pub satisfaction_change_pct: f64,
    pub weekly_stats: Vec<WeeklyStatDto>,
}

/// Weekly breakdown statistics
#[derive(Serialize)]
pub struct WeeklyStatDto {
    pub week_label: String,
    pub session_count: usize,
    pub friction_rate: f64,
    pub success_rate: f64,
}

#[derive(Serialize)]
pub struct DailyStatDto {
    pub date: String,
    pub session_count: usize,
    pub has_digest: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<f64>,
}

#[derive(Serialize)]
pub struct CategoryCountDto {
    pub name: String,
    pub count: usize,
}

/// Per-session insight combining archive metadata with facet analysis data
#[derive(Serialize)]
pub struct SessionInsightDto {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<SessionUsageDto>,
}

/// Per-session insight for a specific date's insights endpoint
#[derive(Serialize)]
pub struct DateSessionInsightDto {
    pub name: String,
    pub session_id: String,
    pub brief_summary: Option<String>,
    pub outcome: Option<String>,
    pub goal_categories: Vec<String>,
    pub friction_types: Vec<String>,
    pub friction_detail: Option<String>,
    pub satisfaction: Option<String>,
    pub claude_helpfulness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<SessionUsageDto>,
}

/// Aggregated day-level insight summary
#[derive(Serialize)]
pub struct DayInsightSummaryDto {
    pub total_sessions: usize,
    pub sessions_with_friction: usize,
    pub overall_satisfaction: Option<String>,
    pub top_goals: Vec<String>,
    pub top_frictions: Vec<String>,
    pub recommendations: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<f64>,
    pub model_distribution: Vec<ModelUsageCountDto>,
}

/// Complete date insights response
#[derive(Serialize)]
pub struct DateInsightsDto {
    pub sessions: Vec<DateSessionInsightDto>,
    pub day_summary: DayInsightSummaryDto,
}

/// Token usage for a single session
#[derive(Serialize)]
pub struct SessionUsageDto {
    pub session_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost_usd: f64,
    pub model_calls: Vec<ModelUsageCountDto>,
}

/// Model usage count entry
#[derive(Serialize)]
pub struct ModelUsageCountDto {
    pub model: String,
    pub count: usize,
}

/// Daily usage data for charts
#[derive(Serialize)]
pub struct DailyUsageDto {
    pub date: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost_usd: f64,
    pub session_count: usize,
}

/// Global usage summary
#[derive(Serialize)]
pub struct UsageSummaryDto {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cost_usd: f64,
    pub total_sessions: usize,
    pub model_distribution: Vec<ModelUsageCountDto>,
    pub daily_usage: Vec<DailyUsageDto>,
}
