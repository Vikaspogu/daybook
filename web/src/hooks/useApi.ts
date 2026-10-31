import { useState, useCallback } from 'react'

const API_BASE = '/api'

export interface DateItem {
  date: string
  session_count: number
  has_digest: boolean
  preview?: string
  session_titles: string[]
}

export interface SummaryCard {
  title: string
  content: string
}

export interface ProjectGain {
  project: string
  work_count: number
  review_count: number
  blocked_count: number
  total_cost_usd: number
  accomplishments: string[]
  key_decisions: string[]
  files_touched: string[]
}

export interface WasteAlert {
  project: string
  count: number
  cost_usd: number
  reason: string
  hint: string
}

export interface DayMetrics {
  total_tokens: number
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cache_creation_tokens: number
  cache_hit_rate?: number | null
  total_cost_usd: number
  test_runs: number
  failed_tool_calls: number
  interrupted: number
}

export interface NextStepItem {
  tag: string
  title: string
  detail: string
}

export interface ConsolidatedRule {
  project: string
  name: string
  summary: string
  trigger?: string | null
  why?: string | null
}

export interface LearningItem {
  kind: 'pitfall' | 'learning' | string
  text: string
  session_title: string
}

export interface ProjectLearnings {
  project: string
  pitfalls: LearningItem[]
  learnings: LearningItem[]
}
export interface DailySummary {
  overview?: string
  session_count?: number
  work_count?: number
  review_count?: number
  blocked_count?: number
  day_metrics?: DayMetrics
  efficiency_signals?: string[]
  learnings?: string[]
  learnings_by_project?: ProjectLearnings[]
  next_steps?: NextStepItem[]
  consolidated_rules?: ConsolidatedRule[]
  project_gains?: ProjectGain[]
  waste_alerts?: WasteAlert[]
  insights: SummaryCard[]
  skills: SummaryCard[]
  commands: SummaryCard[]
  tomorrow_focus: SummaryCard[]
  file_path?: string
  raw_content?: string
}

export interface Session {
  name: string
  title?: string
  summary_preview?: string
  runtime?: string
  project?: string
  started_utc?: string
  ended_utc?: string
  duration?: string
  cost_usd?: number
  category?: 'work' | 'review' | 'blocked'
  key_decision?: string
  files_touched?: string[]
  blocker_reason?: string
}
export interface SessionDetail {
  content: string
  metadata?: {
    title?: string
    date?: string
    session_id?: string
    cwd?: string
    git_branch?: string
    duration?: string
    runtime?: string
    project?: string
    machine?: string
    started_utc?: string
    ended_utc?: string
    cost_usd?: number
  }
  file_path?: string
}


export interface PromptTemplates {
  session_summary: string | null
  daily_summary: string | null
  skill_extract: string | null
  command_extract: string | null
}

export interface PromptTemplatesUpdate {
  session_summary?: string | null
  daily_summary?: string | null
  skill_extract?: string | null
  command_extract?: string | null
}

export interface DefaultTemplates {
  session_summary_en: string
  session_summary_zh: string
  daily_summary_en: string
  daily_summary_zh: string
  skill_extract_en: string
  skill_extract_zh: string
  command_extract_en: string
  command_extract_zh: string
}

export interface Config {
  storage_path: string
  model: string
  summary_language: string
  enable_daily_summary: boolean
  enable_extraction_hints: boolean
  auto_digest_enabled: boolean
  digest_time: string
  author: string | null
  prompt_templates: PromptTemplates
  auto_summarize_enabled: boolean
  auto_summarize_on_show: boolean
  auto_summarize_inactive_minutes: number
}

export interface ConfigUpdate {
  summary_language?: string
  model?: string
  enable_daily_summary?: boolean
  enable_extraction_hints?: boolean
  auto_digest_enabled?: boolean
  digest_time?: string
  author?: string
  prompt_templates?: PromptTemplatesUpdate
  auto_summarize_enabled?: boolean
  auto_summarize_on_show?: boolean
  auto_summarize_inactive_minutes?: number
}

export interface SessionUsage {
  session_id: string
  input_tokens: number
  output_tokens: number
  cache_creation_tokens: number
  cache_read_tokens: number
  total_cost_usd: number
  model_calls: ModelUsageCount[]
}

export interface ModelUsageCount {
  model: string
  count: number
}

export interface DailyUsageData {
  date: string
  input_tokens: number
  output_tokens: number
  cache_creation_tokens: number
  cache_read_tokens: number
  total_cost_usd: number
  session_count: number
}

export interface UsageSummary {
  total_input_tokens: number
  total_output_tokens: number
  total_cache_creation_tokens: number
  total_cache_read_tokens: number
  total_cost_usd: number
  total_sessions: number
  model_distribution: ModelUsageCount[]
  daily_usage: DailyUsageData[]
}

export interface SessionInsight {
  session_id: string
  date: string
  session_name: string
  brief_summary: string | null
  outcome: string | null
  goal_categories: string[]
  friction_types: string[]
  friction_detail: string | null
  satisfaction: string | null
  claude_helpfulness: string | null
  session_type: string | null
  token_usage?: SessionUsage
}

export interface WeeklyStat {
  week_label: string
  session_count: number
  friction_rate: number
  success_rate: number
}

export interface TrendData {
  period_label: string
  comparison_label: string
  current_sessions: number
  previous_sessions: number
  sessions_change_pct: number
  current_friction_rate: number
  previous_friction_rate: number
  friction_change_pct: number
  current_success_rate: number
  previous_success_rate: number
  success_change_pct: number
  current_satisfaction_score: number
  previous_satisfaction_score: number
  satisfaction_change_pct: number
  weekly_stats: WeeklyStat[]
}

export interface InsightsData {
  total_days: number
  total_sessions: number
  daily_stats: DailyStat[]
  goal_distribution: CategoryCount[]
  friction_distribution: CategoryCount[]
  satisfaction_distribution: CategoryCount[]
  language_distribution: CategoryCount[]
  session_type_distribution: CategoryCount[]
  session_details: SessionInsight[]
  trends?: TrendData
  usage_summary?: UsageSummary
}

export interface DailyStat {
  date: string
  session_count: number
  has_digest: boolean
  total_tokens?: number
  total_cost?: number
}

export interface CategoryCount {
  name: string
  count: number
}

export interface DateSessionInsight {
  name: string
  session_id: string
  brief_summary: string | null
  outcome: string | null
  goal_categories: string[]
  friction_types: string[]
  friction_detail: string | null
  satisfaction: string | null
  claude_helpfulness: string | null
  token_usage?: SessionUsage
}

export interface DayInsightSummary {
  total_sessions: number
  sessions_with_friction: number
  overall_satisfaction: string | null
  top_goals: string[]
  top_frictions: string[]
  recommendations: string[]
  total_tokens?: number
  total_cost?: number
  model_distribution?: ModelUsageCount[]
}

export interface DateInsights {
  sessions: DateSessionInsight[]
  day_summary: DayInsightSummary
}

export type ConversationContentBlock =
  | { type: 'text'; text: string }
  | { type: 'tool_use'; tool_use_id: string; name: string; input: unknown }
  | { type: 'tool_result'; tool_use_id: string; content: string }

export interface ConversationMessage {
  role: 'user' | 'assistant'
  content: ConversationContentBlock[]
  timestamp?: string
}

export interface ConversationData {
  messages: ConversationMessage[]
  total_entries: number
  has_transcript: boolean
  page: number
  page_size: number
  has_more: boolean
}

export interface InstallCardResponse {
  name: string
  path: string
  message: string
}

interface ApiResponse<T> {
  success: boolean
  data?: T
  error?: string
}

interface RequestOptions extends RequestInit {
  headers?: Record<string, string>
}

export function useApi() {
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const request = useCallback(async <T>(endpoint: string, options: RequestOptions = {}): Promise<T> => {
    setLoading(true)
    setError(null)
    try {
      const fetchWithAuth = (token: string | null, headers: Record<string, string>) =>
        fetch(`${API_BASE}${endpoint}`, {
          ...options,
          headers: {
            'Content-Type': 'application/json',
            Authorization: `Bearer ${token ?? ''}`,
            ...options.headers,
            ...headers,
          },
        })

      let res = await fetchWithAuth(localStorage.getItem('daybook_token'), {})
      if (res.status === 401) {
        // ponytail: window.prompt gate — replace with a login screen if the dashboard is ever shared
        const token = window.prompt('Daybook: enter your ingest token (dbk_...)')
        if (token) {
          res = await fetchWithAuth(token.trim(), {})
          if (res.ok) localStorage.setItem('daybook_token', token.trim())
        }
      }
      const data: ApiResponse<T> = await res.json()
      if (!data.success) {
        throw new Error(data.error || 'Request failed')
      }
      return data.data as T
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unknown error'
      setError(message)
      throw err
    } finally {
      setLoading(false)
    }
  }, [])

  const fetchDates = useCallback(() => request<DateItem[]>('/dates'), [request])

  const fetchDailySummary = useCallback(
    (date: string) => request<DailySummary>(`/dates/${date}`),
    [request]
  )

  const fetchSessions = useCallback(
    (date: string) => request<Session[]>(`/dates/${date}/sessions`),
    [request]
  )

  const fetchSession = useCallback(
    (date: string, name: string) => request<SessionDetail>(`/dates/${date}/sessions/${encodeURIComponent(name)}`),
    [request]
  )


  const fetchConfig = useCallback(() => request<Config>('/config'), [request])

  const updateConfig = useCallback(
    (config: ConfigUpdate) =>
      request<Config>('/config', {
        method: 'PATCH',
        body: JSON.stringify(config),
      }),
    [request]
  )

  const fetchDefaultTemplates = useCallback(
    () => request<DefaultTemplates>('/config/templates/defaults'),
    [request]
  )

  const fetchInsights = useCallback(
    (days: number = 30) => request<InsightsData>(`/insights?days=${days}`),
    [request]
  )

  const fetchConversation = useCallback(
    (date: string, name: string, page: number = 0, pageSize: number = 50) =>
      request<ConversationData>(
        `/dates/${date}/sessions/${encodeURIComponent(name)}/conversation?page=${page}&page_size=${pageSize}`
      ),
    [request]
  )

  const fetchDateInsights = useCallback(
    (date: string) => request<DateInsights>(`/dates/${date}/insights`),
    [request]
  )

  const installCard = useCallback(
    (title: string, content: string, cardType: 'skill' | 'command') =>
      request<InstallCardResponse>('/install', {
        method: 'POST',
        body: JSON.stringify({ title, content, card_type: cardType }),
      }),
    [request]
  )

  return {
    loading,
    error,
    fetchDates,
    fetchDailySummary,
    fetchSessions,
    fetchSession,
    fetchConfig,
    updateConfig,
    fetchDefaultTemplates,
    fetchInsights,
    fetchConversation,
    fetchDateInsights,
    installCard,
  }
}
