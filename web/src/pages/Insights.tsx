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

const COLORS = ['#34D399', '#38BDF8', '#FBBF24', '#FB7185', '#A78BFA', '#2DD4BF', '#60A5FA', '#F472B6']

function ControlStatTile({
  icon,
  label,
  value,
  sub,
  tone = '',
}: {
  icon: string
  label: string
  value: string | number
  sub?: string
  tone?: 'emerald' | 'sky' | 'amber' | 'coral' | ''
}) {
  return (
    <div className={cn('qp-tile', tone)}>
      <div className="qp-tile-icon">{icon}</div>
      <div className="qp-tile-name">{label}</div>
      <div className="qp-tile-state font-mono">
        {value}
        {sub ? ` · ${sub}` : ''}
      </div>
    </div>
  )
}

function ChartCard({
  title,
  meta,
  className,
  children,
}: {
  title: string
  meta?: string
  className?: string
  children: React.ReactNode
}) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      className={cn('qp-card p-5 space-y-4', className)}
    >
      <div className="flex items-center justify-between gap-2 pb-2 border-b border-white/[0.06]">
        <h3 className="qp-kicker">{title}</h3>
        {meta && <span className="qp-meta">{meta}</span>}
      </div>
      {children}
    </motion.div>
  )
}

function CustomTooltip({ active, payload, label }: any) {
  if (!active || !payload?.length) return null
  return (
    <div className="qp-card px-3 py-2 shadow-xl">
      <p className="text-[11px] font-mono text-[#94A3B8]">{label}</p>
      {payload.map((p: any, i: number) => (
        <p key={i} className="text-xs font-mono font-bold tabular-nums" style={{ color: p.color }}>
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
      <div className="max-w-[1280px] mx-auto px-4 sm:px-6 py-6 space-y-5">
        <div className="qp-hero h-32 animate-pulse" />
        <div className="grid grid-cols-2 lg:grid-cols-4 gap-[10px]">
          {[...Array(4)].map((_, i) => (
            <div key={i} className="qp-tile animate-pulse" />
          ))}
        </div>
        <div className="qp-card h-64 animate-pulse" />
      </div>
    )
  }

  if (!data) {
    return (
      <div className="max-w-[1280px] mx-auto px-4 sm:px-6 py-6">
        <div className="qp-hero">
          <h1 className="qp-hero-title text-[#FBBF24]">{t('insights.title')}</h1>
          <p className="text-xs font-mono text-[#94A3B8] mt-1">{t('insights.failed')}</p>
        </div>
      </div>
    )
  }

  const avgSessions = data.total_days > 0 ? (data.total_sessions / data.total_days).toFixed(1) : '0'

  const digestRate =
    data.total_days > 0
      ? Math.round((data.daily_stats.filter(d => d.has_digest).length / data.total_days) * 100)
      : 0

  const hasLangs = data.language_distribution.length > 0
  const hasTypes = data.session_type_distribution.length > 0
  const bothDist = hasLangs && hasTypes

  return (
    <div className="max-w-[1280px] mx-auto px-4 sm:px-6 py-6 space-y-6">
      {/* Hero Summary Card */}
      <div className="qp-hero space-y-4">
        <div className="flex flex-wrap items-start justify-between gap-4 pb-3 border-b border-white/[0.06]">
          <div className="space-y-1">
            <div className="qp-kicker">QUIET PRECISION · TELEMETRY & VELOCITY</div>
            <h1 className="qp-hero-title">
              <span className="text-[#FBBF24]">{t('insights.title')}</span> · {days}d Window
            </h1>
            <p className="text-xs font-mono text-[#94A3B8] tabular-nums">
              {t('insights.subtitle')}
            </p>
          </div>

          <div className="flex flex-wrap gap-1.5">
            {[7, 14, 30, 90].map(d => (
              <button
                type="button"
                key={d}
                onClick={() => setDays(d)}
                className={cn(
                  'qp-pill font-mono tabular-nums !py-1 !px-3',
                  days === d ? 'amber' : 'hover:text-[#F8FAFC]'
                )}
              >
                {days === d && <span className="qp-dot" />}
                {d}d
              </button>
            ))}
          </div>
        </div>

        {/* 4-Up Interactive Control Tiles */}
        <div className="grid grid-cols-2 lg:grid-cols-4 gap-[10px]">
          <ControlStatTile
            icon="DY"
            label={t('insights.totalDays')}
            value={`${data.total_days} days`}
            tone="emerald"
          />
          <ControlStatTile
            icon="SN"
            label={t('insights.totalSessions')}
            value={`${data.total_sessions} runs`}
            tone="sky"
          />
          <ControlStatTile
            icon="AVG"
            label={t('insights.avgSessions')}
            value={`${avgSessions} / day`}
            tone="amber"
          />
          <ControlStatTile
            icon="DG"
            label={t('insights.digestRate')}
            value={`${digestRate}%`}
            sub={t('insights.digestRateSub')}
            tone={digestRate >= 70 ? 'emerald' : 'sky'}
          />
        </div>
      </div>

      {/* 2-Up Quick-Action Tab Bar */}
      <div className="grid grid-cols-2 gap-2 max-w-md">
        <button
          type="button"
          onClick={() => setActiveTab('charts')}
          className={cn(
            'qp-quick-btn !h-12',
            activeTab === 'charts' ? 'sky' : 'text-[#94A3B8] hover:text-[#F8FAFC]'
          )}
        >
          <span className="text-xs font-bold">{t('insights.tabCharts')}</span>
          <span className="font-mono text-[10px] opacity-80">Usage & Trends</span>
        </button>

        <button
          type="button"
          onClick={() => setActiveTab('sessions')}
          className={cn(
            'qp-quick-btn !h-12',
            activeTab === 'sessions'
              ? problemCount > 0
                ? 'coral'
                : 'emerald'
              : 'text-[#94A3B8] hover:text-[#F8FAFC]'
          )}
        >
          <span className="text-xs font-bold">{t('insights.tabSessions')}</span>
          <span className="font-mono text-[10px] tabular-nums opacity-85">
            {problemCount} flagged
          </span>
        </button>
      </div>

      {/* === Charts Tab === */}
      {activeTab === 'charts' && (
        <div className="space-y-6">
          {/* Token Usage Section */}
          {data.usage_summary && data.usage_summary.total_sessions > 0 && (
            <div className="space-y-4">
              <div className="flex items-center justify-between px-0.5">
                <h2 className="qp-kicker">{t('insights.tokenUsage')}</h2>
                <span className="qp-meta">{t('insights.tokenUsageDesc')}</span>
              </div>

              <div className="grid grid-cols-2 lg:grid-cols-4 gap-[10px]">
                <ControlStatTile
                  icon="TK"
                  label={t('insights.totalTokens')}
                  value={formatTokenCount(
                    data.usage_summary.total_input_tokens +
                      data.usage_summary.total_output_tokens +
                      data.usage_summary.total_cache_creation_tokens +
                      data.usage_summary.total_cache_read_tokens
                  )}
                  sub={`${data.usage_summary.total_sessions}s`}
                  tone="sky"
                />
                <ControlStatTile
                  icon="IN"
                  label={t('insights.inputTokens')}
                  value={formatTokenCount(data.usage_summary.total_input_tokens)}
                />
                <ControlStatTile
                  icon="OUT"
                  label={t('insights.outputTokens')}
                  value={formatTokenCount(data.usage_summary.total_output_tokens)}
                />
                <ControlStatTile
                  icon="$"
                  label={t('insights.totalCost')}
                  value={formatCost(data.usage_summary.total_cost_usd)}
                  sub={`${formatTokenCount(data.usage_summary.total_cache_read_tokens)} cache`}
                  tone="amber"
                />
              </div>

              <div className="grid grid-cols-1 lg:grid-cols-2 gap-5">
                <ChartCard title={t('insights.tokenTimeline')} meta="TOKENS / DAY">
                  <UsageTimeline data={data.usage_summary.daily_usage} />
                </ChartCard>
                <ChartCard title={t('insights.dailyCost')} meta="USD / DAY">
                  <CostTimeline data={data.usage_summary.daily_usage} />
                </ChartCard>
              </div>

              {data.usage_summary.model_distribution.length > 0 && (
                <ChartCard title={t('insights.modelDistribution')} meta="MODELS">
                  <ModelPieChart data={data.usage_summary.model_distribution} />
                </ChartCard>
              )}
            </div>
          )}

          {/* Trend Analysis */}
          <InsightsTrends trends={data.trends} />

          {/* Activity Timeline */}
          <ChartCard title={t('insights.activityTimeline')} meta="SESSIONS / DAY">
            {data.daily_stats.length > 0 ? (
              <ResponsiveContainer width="100%" height={200}>
                <AreaChart data={data.daily_stats}>
                  <defs>
                    <linearGradient id="colorSessions" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="5%" stopColor="#38BDF8" stopOpacity={0.32} />
                      <stop offset="95%" stopColor="#38BDF8" stopOpacity={0} />
                    </linearGradient>
                  </defs>
                  <CartesianGrid strokeDasharray="3 3" stroke="#1E293B" opacity={0.5} />
                  <XAxis
                    dataKey="date"
                    tick={{ fontSize: 11, fill: '#94A3B8' }}
                    tickFormatter={(v: string) => v.slice(5)}
                  />
                  <YAxis tick={{ fontSize: 11, fill: '#94A3B8' }} allowDecimals={false} />
                  <Tooltip content={<CustomTooltip />} />
                  <Area
                    type="monotone"
                    dataKey="session_count"
                    stroke="#38BDF8"
                    strokeWidth={2}
                    fill="url(#colorSessions)"
                  />
                </AreaChart>
              </ResponsiveContainer>
            ) : (
              <p className="text-xs font-mono text-[#64748B] py-8 text-center">
                {t('insights.noSessionDetails')}
              </p>
            )}
          </ChartCard>

          {/* Balanced Distribution Grid */}
          {(hasLangs || hasTypes) && (
            <div className={cn('grid grid-cols-1 gap-5', bothDist && 'lg:grid-cols-2')}>
              {hasLangs && (
                <ChartCard title={t('insights.languages')} meta="TOP LANGUAGES">
                  <ResponsiveContainer width="100%" height={240}>
                    <BarChart data={data.language_distribution.slice(0, 8)} layout="vertical">
                      <CartesianGrid strokeDasharray="3 3" stroke="#1E293B" opacity={0.5} />
                      <XAxis type="number" tick={{ fontSize: 11, fill: '#94A3B8' }} allowDecimals={false} />
                      <YAxis
                        type="category"
                        dataKey="name"
                        tick={{ fontSize: 12, fill: '#94A3B8' }}
                        width={100}
                      />
                      <Tooltip content={<CustomTooltip />} />
                      <Bar dataKey="count" fill="#34D399" radius={[0, 6, 6, 0]} />
                    </BarChart>
                  </ResponsiveContainer>
                </ChartCard>
              )}

              {hasTypes && (
                <ChartCard title={t('insights.sessionTypes')} meta="DISTRIBUTION">
                  <div className="flex flex-wrap gap-2.5">
                    {data.session_type_distribution.map((item, i) => (
                      <div
                        key={item.name}
                        className="qp-pill !py-1.5 !px-3 font-mono tabular-nums"
                      >
                        <span
                          className="qp-dot"
                          style={{ color: COLORS[i % COLORS.length] }}
                        />
                        <span className="text-[#F8FAFC]">{item.name}</span>
                        <span className="text-[#FBBF24] font-bold">{item.count}</span>
                      </div>
                    ))}
                  </div>
                </ChartCard>
              )}
            </div>
          )}
        </div>
      )}

      {/* === Problem Sessions Tab === */}
      {activeTab === 'sessions' && (
        <>
          {problemSessions.length > 0 ? (
            <InsightsProblemView sessionDetails={problemSessions} />
          ) : (
            <div className="qp-card text-center py-12 px-6">
              <p className="text-sm font-bold text-[#F8FAFC]">
                {t('insights.problemNoIssues')}
              </p>
              <p className="text-xs font-mono text-[#64748B] mt-1 max-w-md mx-auto">
                {t('insights.noSessionDetails')}
              </p>
            </div>
          )}
        </>
      )}
    </div>
  )
}
