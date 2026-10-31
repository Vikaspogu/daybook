import { useState, useEffect, useMemo } from 'react'
import { motion } from 'framer-motion'
import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
  AreaChart,
  Area,
  CartesianGrid,
} from 'recharts'
import { useApi } from '../hooks/useApi'
import type { InsightsData } from '../hooks/useApi'
import { InsightsProblemView } from '../components/InsightsProblemView'
import { InsightsTrends } from '../components/InsightsTrends'
import { UsageTimeline, CostTimeline, ModelPieChart, formatTokenCount, formatCost } from '../components/UsageCharts'
import { cn } from '../lib/utils'
import { useLanguage } from '../contexts/LanguageContext'

type InsightTab = 'charts' | 'sessions'

const COLORS = ['#f97316', '#fb923c', '#fdba74', '#fed7aa', '#ffedd5', '#a3e635', '#4ade80', '#2dd4bf', '#38bdf8', '#818cf8']

function StatCard({ label, value, sub }: { label: string; value: string | number; sub?: string }) {
  return (
    <div className="bg-gray-50 dark:bg-daily-light rounded-xl p-5 border border-gray-200 dark:border-gray-800">
      <p className="text-sm text-gray-500 dark:text-gray-400 mb-1">{label}</p>
      <p className="text-3xl font-bold text-orange-500 dark:text-orange-400 tabular-nums">{value}</p>
      {sub && <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">{sub}</p>}
    </div>
  )
}

function ChartCard({
  title,
  className,
  children,
}: {
  title: string
  className?: string
  children: React.ReactNode
}) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      className={cn(
        'bg-gray-50 dark:bg-daily-light rounded-xl p-5 border border-gray-200 dark:border-gray-800',
        className
      )}
    >
      <h3 className="text-sm font-medium text-gray-500 dark:text-gray-400 mb-4">{title}</h3>
      {children}
    </motion.div>
  )
}

function CustomTooltip({ active, payload, label }: any) {
  if (!active || !payload?.length) return null
  return (
    <div className="bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg px-3 py-2 shadow-lg">
      <p className="text-xs text-gray-500 dark:text-gray-400">{label}</p>
      {payload.map((p: any, i: number) => (
        <p key={i} className="text-sm font-medium tabular-nums" style={{ color: p.color }}>
          {p.value} {p.name || 'sessions'}
        </p>
      ))}
    </div>
  )
}

export function Insights() {
  const [data, setData] = useState<InsightsData | null>(null)
  const [loading, setLoading] = useState(true)
  const [days, setDays] = useState(30)
  const [activeTab, setActiveTab] = useState<InsightTab>('charts')
  const { fetchInsights } = useApi()
  const { t } = useLanguage()

  useEffect(() => {
    const load = async () => {
      setLoading(true)
      try {
        const insights = await fetchInsights(days)
        setData(insights)
      } catch (err) {
        console.error('Failed to load insights:', err)
      } finally {
        setLoading(false)
      }
    }
    load()
  }, [fetchInsights, days])

  const problemSessions = useMemo(() => {
    const list = data?.session_details ?? []
    return list.filter(
      s =>
        s.friction_types.length > 0 ||
        s.outcome === 'not_achieved' ||
        s.outcome === 'partially_achieved' ||
        s.satisfaction === 'frustrated'
    )
  }, [data])
  const problemCount = problemSessions.length

  if (loading) {
    return (
      <div className="max-w-6xl mx-auto px-6 py-8">
        <h1 className="text-3xl font-bold mb-8">
          <span className="text-orange-500 dark:text-orange-400">{t('insights.title')}</span>
        </h1>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mb-8">
          {[...Array(3)].map((_, i) => (
            <div key={i} className="h-28 bg-gray-200 dark:bg-daily-light rounded-xl animate-pulse" />
          ))}
        </div>
        <div className="h-64 bg-gray-200 dark:bg-daily-light rounded-xl animate-pulse" />
      </div>
    )
  }

  if (!data) {
    return (
      <div className="max-w-6xl mx-auto px-6 py-8">
        <h1 className="text-3xl font-bold mb-4">
          <span className="text-orange-500 dark:text-orange-400">{t('insights.title')}</span>
        </h1>
        <p className="text-gray-500">{t('insights.failed')}</p>
      </div>
    )
  }

  const avgSessions = data.total_days > 0
    ? (data.total_sessions / data.total_days).toFixed(1)
    : '0'

  const digestRate = data.total_days > 0
    ? Math.round((data.daily_stats.filter(d => d.has_digest).length / data.total_days) * 100)
    : 0

  const hasLangs = data.language_distribution.length > 0
  const hasTypes = data.session_type_distribution.length > 0
  const bothDist = hasLangs && hasTypes

  return (
    <div className="max-w-6xl mx-auto px-6 py-8">
      {/* Header */}
      <div className="flex flex-wrap items-center justify-between gap-4 mb-8">
        <div>
          <h1 className="text-3xl font-bold">
            <span className="text-orange-500 dark:text-orange-400">{t('insights.title')}</span>
          </h1>
          <p className="text-gray-500 dark:text-gray-400 mt-1">
            {t('insights.subtitle')}
          </p>
        </div>
        <div className="flex gap-2">
          {[7, 14, 30, 90].map(d => (
            <button
              key={d}
              onClick={() => setDays(d)}
              className={`px-3 py-1.5 text-sm rounded-lg transition-colors tabular-nums ${
                days === d
                  ? 'bg-orange-500 text-white'
                  : 'bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-700'
              }`}
            >
              {d}d
            </button>
          ))}
        </div>
      </div>

      {/* Tab bar */}
      <div className="flex gap-1 bg-gray-100 dark:bg-gray-800 rounded-lg p-1 mb-6 w-fit">
        {([
          { key: 'charts' as InsightTab, label: t('insights.tabCharts') },
          { key: 'sessions' as InsightTab, label: t('insights.tabSessions'), count: problemCount },
        ]).map(tab => (
          <button
            key={tab.key}
            onClick={() => setActiveTab(tab.key)}
            className={cn(
              'px-4 py-1.5 text-sm font-medium rounded-md transition-colors flex items-center gap-2',
              activeTab === tab.key
                ? 'bg-white dark:bg-daily-light text-orange-500 dark:text-orange-400 shadow-sm'
                : 'text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300'
            )}
          >
            {tab.label}
            {tab.count != null && (
              <span
                className={cn(
                  'px-1.5 py-0.5 rounded-full text-[11px] font-semibold tabular-nums',
                  tab.count > 0
                    ? 'bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300'
                    : 'bg-gray-200 text-gray-500 dark:bg-gray-700 dark:text-gray-400'
                )}
              >
                {tab.count}
              </span>
            )}
          </button>
        ))}
      </div>

      {/* === Charts Tab === */}
      {activeTab === 'charts' && (
        <>
          {/* Stats Overview */}
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-8">
            <StatCard label={t('insights.totalDays')} value={data.total_days} />
            <StatCard label={t('insights.totalSessions')} value={data.total_sessions} />
            <StatCard label={t('insights.avgSessions')} value={avgSessions} />
            <StatCard label={t('insights.digestRate')} value={`${digestRate}%`} sub={t('insights.digestRateSub')} />
          </div>

          {/* Token Usage Section */}
          {data.usage_summary && data.usage_summary.total_sessions > 0 && (
            <>
              <div className="mb-4">
                <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-200">{t('insights.tokenUsage')}</h2>
                <p className="text-sm text-gray-500 dark:text-gray-400">{t('insights.tokenUsageDesc')}</p>
              </div>

              <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-4">
                <StatCard
                  label={t('insights.totalTokens')}
                  value={formatTokenCount(
                    data.usage_summary.total_input_tokens +
                    data.usage_summary.total_output_tokens +
                    data.usage_summary.total_cache_creation_tokens +
                    data.usage_summary.total_cache_read_tokens
                  )}
                  sub={`${data.usage_summary.total_sessions} sessions`}
                />
                <StatCard
                  label={t('insights.inputTokens')}
                  value={formatTokenCount(data.usage_summary.total_input_tokens)}
                />
                <StatCard
                  label={t('insights.outputTokens')}
                  value={formatTokenCount(data.usage_summary.total_output_tokens)}
                />
                <StatCard
                  label={t('insights.totalCost')}
                  value={formatCost(data.usage_summary.total_cost_usd)}
                  sub={`cache: ${formatTokenCount(data.usage_summary.total_cache_read_tokens)} read`}
                />
              </div>

              <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4">
                <ChartCard title={t('insights.tokenTimeline')}>
                  <UsageTimeline data={data.usage_summary.daily_usage} />
                </ChartCard>
                <ChartCard title={t('insights.dailyCost')}>
                  <CostTimeline data={data.usage_summary.daily_usage} />
                </ChartCard>
              </div>

              {data.usage_summary.model_distribution.length > 0 && (
                <div className="mb-4">
                  <ChartCard title={t('insights.modelDistribution')}>
                    <ModelPieChart data={data.usage_summary.model_distribution} />
                  </ChartCard>
                </div>
              )}
            </>
          )}

          {/* Trend Analysis */}
          <InsightsTrends trends={data.trends} />

          {/* Activity Timeline */}
          <ChartCard title={t('insights.activityTimeline')}>
            {data.daily_stats.length > 0 ? (
              <ResponsiveContainer width="100%" height={200}>
                <AreaChart data={data.daily_stats}>
                  <defs>
                    <linearGradient id="colorSessions" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="5%" stopColor="#f97316" stopOpacity={0.3} />
                      <stop offset="95%" stopColor="#f97316" stopOpacity={0} />
                    </linearGradient>
                  </defs>
                  <CartesianGrid strokeDasharray="3 3" stroke="#374151" opacity={0.2} />
                  <XAxis
                    dataKey="date"
                    tick={{ fontSize: 11, fill: '#9ca3af' }}
                    tickFormatter={(v: string) => v.slice(5)}
                  />
                  <YAxis tick={{ fontSize: 11, fill: '#9ca3af' }} allowDecimals={false} />
                  <Tooltip content={<CustomTooltip />} />
                  <Area
                    type="monotone"
                    dataKey="session_count"
                    stroke="#f97316"
                    strokeWidth={2}
                    fill="url(#colorSessions)"
                  />
                </AreaChart>
              </ResponsiveContainer>
            ) : (
              <p className="text-sm text-gray-500 dark:text-gray-400 py-8 text-center">
                {t('insights.noSessionDetails')}
              </p>
            )}
          </ChartCard>

          {/* Balanced Distribution Grid — pairs Languages + Session Types when both exist, full-width when only one */}
          {(hasLangs || hasTypes) && (
            <div className={cn('grid grid-cols-1 gap-4 mt-4', bothDist && 'md:grid-cols-2')}>
              {hasLangs && (
                <ChartCard title={t('insights.languages')}>
                  <ResponsiveContainer width="100%" height={240}>
                    <BarChart data={data.language_distribution.slice(0, 8)} layout="vertical">
                      <CartesianGrid strokeDasharray="3 3" stroke="#374151" opacity={0.2} />
                      <XAxis type="number" tick={{ fontSize: 11, fill: '#9ca3af' }} allowDecimals={false} />
                      <YAxis
                        type="category"
                        dataKey="name"
                        tick={{ fontSize: 12, fill: '#9ca3af' }}
                        width={100}
                      />
                      <Tooltip content={<CustomTooltip />} />
                      <Bar dataKey="count" fill="#38bdf8" radius={[0, 4, 4, 0]} />
                    </BarChart>
                  </ResponsiveContainer>
                </ChartCard>
              )}

              {hasTypes && (
                <ChartCard title={t('insights.sessionTypes')}>
                  <div className="flex flex-wrap gap-3">
                    {data.session_type_distribution.map((item, i) => (
                      <div
                        key={item.name}
                        className="flex items-center gap-2 px-3 py-2 rounded-lg bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700"
                      >
                        <div
                          className="w-3 h-3 rounded-full"
                          style={{ backgroundColor: COLORS[i % COLORS.length] }}
                        />
                        <span className="text-sm text-gray-600 dark:text-gray-300">{item.name}</span>
                        <span className="text-sm font-bold text-orange-500 dark:text-orange-400 tabular-nums">{item.count}</span>
                      </div>
                    ))}
                  </div>
                </ChartCard>
              )}
            </div>
          )}
        </>
      )}

      {/* === Problem Sessions Tab === */}
      {activeTab === 'sessions' && (
        <>
          {problemSessions.length > 0 ? (
            <InsightsProblemView sessionDetails={problemSessions} />
          ) : (
            <div className="text-center py-12 px-6 bg-gray-50 dark:bg-daily-light rounded-xl border border-dashed border-gray-300 dark:border-gray-700">
              <p className="text-sm font-medium text-gray-600 dark:text-gray-300">
                {t('insights.problemNoIssues')}
              </p>
              <p className="text-xs text-gray-400 dark:text-gray-500 mt-1 max-w-md mx-auto">
                {t('insights.noSessionDetails')}
              </p>
            </div>
          )}
        </>
      )}
    </div>
  )
}
