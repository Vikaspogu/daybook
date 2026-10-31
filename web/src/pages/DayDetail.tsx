import { useState, useEffect, useMemo } from 'react'
import { useParams, Link } from 'react-router-dom'
import { motion } from 'framer-motion'
import { useApi } from '../hooks/useApi'
import type { DailySummary, DateInsights, Session, SummaryCard } from '../hooks/useApi'
import { MarkdownRenderer } from '../components/MarkdownRenderer'
import { cn } from '../lib/utils'
import { formatTokenCount, formatCost } from '../components/UsageCharts'
import { useLanguage } from '../contexts/LanguageContext'

// Format outcome into a readable badge label
const formatOutcome = (outcome: string): string => {
  return outcome.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase())
}

// Get badge color class based on outcome
const getOutcomeBadgeClass = (outcome: string | null): string => {
  switch (outcome) {
    case 'achieved':
      return 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
    case 'partially_achieved':
      return 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400'
    case 'not_achieved':
      return 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400'
    default:
      return 'bg-gray-100 text-gray-600 dark:bg-gray-700/30 dark:text-gray-400'
  }
}

// "1h 23m" from two ISO timestamps; null when either is missing/unparseable
const formatDuration = (started?: string, ended?: string): string | null => {
  if (!started || !ended) return null
  const s = Date.parse(started)
  const e = Date.parse(ended)
  if (Number.isNaN(s) || Number.isNaN(e) || e < s) return null
  const mins = Math.floor((e - s) / 60000)
  return mins >= 60 ? `${Math.floor(mins / 60)}h ${mins % 60}m` : `${mins}m`
}

// Section header with a tiny anchor link
function Section({ id, title, children }: { id: string; title: string; children: React.ReactNode }) {
  return (
    <motion.section
      id={id}
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      className="scroll-mt-20 space-y-3"
    >
      <div className="flex items-center gap-2">
        <h2 className="text-sm font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
          {title}
        </h2>
        <a
          href={`#${id}`}
          aria-label={`Link to ${title}`}
          className="text-gray-300 dark:text-gray-600 hover:text-orange-500 dark:hover:text-orange-400 transition-colors"
        >
          #
        </a>
      </div>
      {children}
    </motion.section>
  )
}

// Collapsible summary card component
function CollapsibleCard({ card, defaultOpen = true }: { card: SummaryCard; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen)

  return (
    <div className="border border-gray-200 dark:border-gray-700/50 rounded-lg overflow-hidden">
      <button
        onClick={() => setOpen(!open)}
        className="w-full flex items-center justify-between px-4 py-3 text-left bg-white dark:bg-daily-light hover:bg-gray-50 dark:hover:bg-gray-800/50 transition-colors"
      >
        <span className="text-sm font-semibold text-gray-800 dark:text-gray-200">{card.title}</span>
        <svg
          className={cn('size-4 text-gray-400 transition-transform', open && 'rotate-180')}
          fill="none" viewBox="0 0 24 24" stroke="currentColor"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>
      {open && (
        <div className="px-4 pb-4 pt-1 bg-white dark:bg-daily-light">
          <div className="text-sm text-gray-600 dark:text-gray-400 markdown-content">
            <MarkdownRenderer content={card.content} />
          </div>
        </div>
      )}
    </div>
  )
}

// Skill/Command card with distinctive visual style and action buttons
function SkillCommandCard({
  card,
  type,
  onInstall,
}: {
  card: SummaryCard
  type: 'skill' | 'command'
  onInstall: (title: string, content: string, cardType: 'skill' | 'command') => Promise<unknown>
}) {
  const [expanded, setExpanded] = useState(false)
  const [copyOk, setCopyOk] = useState(false)
  const [installing, setInstalling] = useState(false)
  const [installed, setInstalled] = useState(false)
  const [installPath, setInstallPath] = useState<string | null>(null)
  const { t } = useLanguage()
  const isSkill = type === 'skill'
  const colorClass = isSkill
    ? 'border-green-200 dark:border-green-800/40 hover:border-green-300 dark:hover:border-green-700/60'
    : 'border-purple-200 dark:border-purple-800/40 hover:border-purple-300 dark:hover:border-purple-700/60'
  const badgeClass = isSkill
    ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
    : 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400'
  const iconColor = isSkill ? 'text-green-500' : 'text-purple-500'

  const handleCopy = async (e: React.MouseEvent) => {
    e.stopPropagation()
    const text = `# ${card.title}\n\n${card.content}`
    await navigator.clipboard.writeText(text)
    setCopyOk(true)
    setTimeout(() => setCopyOk(false), 2000)
  }

  const handleInstall = async (e: React.MouseEvent) => {
    e.stopPropagation()
    if (installing || installed) return
    setInstalling(true)
    try {
      await onInstall(card.title, card.content, type)
      setInstalled(true)
      // Show path briefly
      const home = '~/.claude'
      setInstallPath(
        type === 'skill'
          ? `${home}/skills/`
          : `${home}/commands/`
      )
    } catch {
      // error handled by parent
    } finally {
      setInstalling(false)
    }
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      className={cn(
        'bg-white dark:bg-daily-light rounded-xl border transition-colors',
        colorClass
      )}
    >
      <div
        className="p-4 cursor-pointer"
        onClick={() => setExpanded(!expanded)}
      >
        <div className="flex items-start gap-3">
          <div className={cn('mt-0.5 flex-shrink-0', iconColor)}>
            {isSkill ? (
              <svg className="size-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
              </svg>
            ) : (
              <svg className="size-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
              </svg>
            )}
          </div>
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 mb-1">
              <h4 className="text-sm font-semibold text-gray-900 dark:text-gray-100 truncate">
                {card.title}
              </h4>
              <span className={cn('text-[10px] font-medium px-1.5 py-0.5 rounded-full flex-shrink-0 uppercase tracking-wide', badgeClass)}>
                {isSkill ? t('dayDetail.skill') : t('dayDetail.cmd')}
              </span>
            </div>
            {!expanded && card.content && (
              <p className="text-xs text-gray-500 dark:text-gray-400 line-clamp-2">
                {card.content.replace(/[#*`\->\n]/g, ' ').replace(/\s+/g, ' ').trim().slice(0, 120)}
              </p>
            )}
          </div>
          <svg
            className={cn('size-4 text-gray-400 transition-transform flex-shrink-0 mt-0.5', expanded && 'rotate-180')}
            fill="none" viewBox="0 0 24 24" stroke="currentColor"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
          </svg>
        </div>
      </div>
      {expanded && (
        <div className="border-t border-gray-100 dark:border-gray-700/30">
          <div className="px-4 pb-3 pt-3 text-sm text-gray-600 dark:text-gray-400 markdown-content">
            <MarkdownRenderer content={card.content} />
          </div>
          {/* Action buttons */}
          <div className="px-4 pb-3 flex items-center gap-2">
            <button
              onClick={handleCopy}
              className={cn(
                'flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors',
                'border border-gray-200 dark:border-gray-700 text-gray-600 dark:text-gray-400',
                'hover:bg-gray-50 dark:hover:bg-gray-800/50',
                copyOk && 'border-green-300 dark:border-green-700 text-green-600 dark:text-green-400'
              )}
            >
              {copyOk ? (
                <>
                  <svg className="size-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  {t('dayDetail.copied')}
                </>
              ) : (
                <>
                  <svg className="size-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
                  </svg>
                  {t('dayDetail.copy')}
                </>
              )}
            </button>
            <button
              onClick={handleInstall}
              disabled={installing || installed}
              className={cn(
                'flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors',
                installed
                  ? 'border border-green-300 dark:border-green-700 text-green-600 dark:text-green-400 bg-green-50 dark:bg-green-900/10'
                  : isSkill
                    ? 'border border-green-200 dark:border-green-800/40 text-green-700 dark:text-green-400 hover:bg-green-50 dark:hover:bg-green-900/20'
                    : 'border border-purple-200 dark:border-purple-800/40 text-purple-700 dark:text-purple-400 hover:bg-purple-50 dark:hover:bg-purple-900/20',
                'disabled:opacity-50'
              )}
            >
              {installing ? (
                <>
                  <svg className="animate-spin size-3.5" viewBox="0 0 24 24" fill="none">
                    <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                    <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                  </svg>
                  {t('dayDetail.installing')}
                </>
              ) : installed ? (
                <>
                  <svg className="size-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  {t('dayDetail.installed')}
                </>
              ) : (
                <>
                  <svg className="size-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
                  </svg>
                  {t('dayDetail.install')}
                </>
              )}
            </button>
            {installed && installPath && (
              <span className="text-[10px] text-gray-400 dark:text-gray-500 truncate">
                {installPath}
              </span>
            )}
          </div>
        </div>
      )}
    </motion.div>
  )
}

// One row in the Sessions section
function SessionRow({
  session,
  date,
  insight,
}: {
  session: Session
  date: string
  insight?: { outcome: string | null; friction_types: string[] }
}) {
  const duration = formatDuration(session.started_utc, session.ended_utc)

  return (
    <Link to={`/day/${date}/session/${encodeURIComponent(session.name)}`}>
      <div
        className={cn(
          'p-4 rounded-lg border transition-colors',
          'bg-gray-50 dark:bg-daily-light border-gray-200 dark:border-orange-500/20 hover:border-orange-400 dark:hover:border-orange-500/40'
        )}
      >
        <div className="flex items-start justify-between gap-3 mb-1">
          <h3 className="font-medium text-gray-900 dark:text-gray-100 line-clamp-1">
            {session.title || session.name}
          </h3>
          {insight?.outcome && (
            <span className={cn('text-xs font-medium px-2 py-0.5 rounded-full whitespace-nowrap', getOutcomeBadgeClass(insight.outcome))}>
              {formatOutcome(insight.outcome)}
            </span>
          )}
        </div>
        {session.summary_preview && (
          <p className="text-sm text-gray-600 dark:text-gray-400 line-clamp-2 text-pretty mb-2">
            {session.summary_preview}
          </p>
        )}
        <div className="flex flex-wrap items-center gap-2 text-xs">
          {session.runtime && (
            <span className="px-1.5 py-0.5 rounded bg-sky-100 text-sky-700 dark:bg-sky-900/30 dark:text-sky-400 font-medium">
              {session.runtime}
            </span>
          )}
          {session.project && (
            <span className="px-1.5 py-0.5 rounded bg-gray-100 text-gray-600 dark:bg-gray-700/40 dark:text-gray-300 font-medium truncate max-w-48">
              {session.project}
            </span>
          )}
          {duration && (
            <span className="text-gray-500 dark:text-gray-400 tabular-nums">{duration}</span>
          )}
          {session.cost_usd != null && (
            <span className="text-purple-500 font-medium tabular-nums">{formatCost(session.cost_usd)}</span>
          )}
          {insight && insight.friction_types.length > 0 && (
            <span className="flex items-center gap-1 text-amber-600 dark:text-amber-400">
              <svg className="size-3.5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" />
              </svg>
              {insight.friction_types.map(f => f.replace(/_/g, ' ')).join(', ')}
            </span>
          )}
        </div>
      </div>
    </Link>
  )
}

export function DayDetail() {
  const { date } = useParams<{ date: string }>()
  const [summary, setSummary] = useState<DailySummary | null>(null)
  const [insights, setInsights] = useState<DateInsights | null>(null)
  const [sessions, setSessions] = useState<Session[]>([])
  const [runtimeFilter, setRuntimeFilter] = useState<string | null>(null)
  const { fetchDailySummary, fetchDateInsights, fetchSessions, installCard, loading, error } = useApi()
  const { t } = useLanguage()

  const getSatisfactionIndicator = (satisfaction: string | null): { label: string; className: string } => {
    switch (satisfaction) {
      case 'happy':
        return { label: t('dayDetail.happy'), className: 'text-green-600 dark:text-green-400' }
      case 'likely_satisfied':
        return { label: t('dayDetail.satisfied'), className: 'text-blue-600 dark:text-blue-400' }
      case 'neutral':
        return { label: t('dayDetail.neutral'), className: 'text-gray-500 dark:text-gray-400' }
      case 'frustrated':
        return { label: t('dayDetail.frustrated'), className: 'text-red-600 dark:text-red-400' }
      default:
        return { label: t('dayDetail.unknown'), className: 'text-gray-400' }
    }
  }

  useEffect(() => {
    if (!date) return
    let cancelled = false
    setRuntimeFilter(null)
    setSummary(null)
    setInsights(null)
    setSessions([])
    fetchDailySummary(date)
      .then(res => {
        if (!cancelled) setSummary(res)
      })
      .catch(console.error)
    fetchDateInsights(date)
      .then(res => {
        if (!cancelled) setInsights(res)
      })
      .catch(() => {})
    fetchSessions(date)
      .then(res => {
        if (!cancelled) setSessions(res)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [date, fetchDailySummary, fetchDateInsights, fetchSessions])

  const insightByName = useMemo(() => {
    const map = new Map<string, { outcome: string | null; friction_types: string[] }>()
    for (const s of insights?.sessions ?? []) {
      map.set(s.name, { outcome: s.outcome, friction_types: s.friction_types })
    }
    return map
  }, [insights])

  const runtimes = useMemo(
    () => [...new Set(sessions.map(s => s.runtime).filter((r): r is string => !!r))].sort(),
    [sessions]
  )
  const filteredSessions = useMemo(
    () => (runtimeFilter ? sessions.filter(s => s.runtime === runtimeFilter) : sessions),
    [sessions, runtimeFilter]
  )

  const projects = useMemo(
    () => [...new Set(sessions.map(s => s.project).filter((p): p is string => !!p))],
    [sessions]
  )
  const totalCost = useMemo(
    () => sessions.reduce((sum, s) => sum + (s.cost_usd ?? 0), 0),
    [sessions]
  )

  // ponytail: extraction hints carry their project prefix ("project — hint"),
  // so a project with no card yielded nothing extractable (junk/empty) that day.
  const skippedProjects = useMemo(() => {
    const cards = [...(summary?.skills ?? []), ...(summary?.commands ?? [])]
    if (cards.length === 0) return []
    return projects.filter(
      p => !cards.some(c => c.title === p || c.title.startsWith(`${p} — `) || c.title.startsWith(`${p} - `))
    )
  }, [summary, projects])

  if (loading && !summary) {
    return (
      <div className="max-w-4xl mx-auto px-6 py-8">
        <div className="animate-pulse space-y-4">
          <div className="h-8 w-48 bg-gray-200 dark:bg-daily-light rounded" />
          <div className="h-32 bg-gray-200 dark:bg-daily-light rounded-lg" />
          <div className="h-24 bg-gray-200 dark:bg-daily-light rounded-lg" />
        </div>
      </div>
    )
  }

  const daySummary = insights?.day_summary
  const hasButFirst =
    !!daySummary &&
    (daySummary.sessions_with_friction > 0 ||
      daySummary.overall_satisfaction != null ||
      daySummary.top_frictions.length > 0)
  const hasOverview =
    Boolean(summary?.overview) || (summary?.insights?.length ?? 0) > 0
  const hasNextSteps =
    (summary?.tomorrow_focus?.length ?? 0) > 0 ||
    (daySummary?.recommendations?.length ?? 0) > 0

  const anchorNav: { id: string; label: string }[] = [
    ...(hasButFirst ? [{ id: 'but-first', label: t('dayDetail.sectionButFirst') }] : []),
    { id: 'stats', label: t('dayDetail.sectionStats') },
    ...(hasOverview ? [{ id: 'overview', label: t('dayDetail.sectionOverview') }] : []),
    ...(hasNextSteps ? [{ id: 'next-steps', label: t('dayDetail.sectionNextSteps') }] : []),
    { id: 'sessions', label: t('dayDetail.sectionSessions') },
    { id: 'extracted', label: t('dayDetail.sectionExtracted') },
  ]

  return (
    <div className="max-w-4xl mx-auto px-6 py-8 space-y-8">
      {/* Breadcrumb */}
      <nav className="text-sm">
        <Link to="/" className="text-gray-500 hover:text-gray-700 dark:hover:text-gray-300">
          {t('dayDetail.breadcrumbArchives')}
        </Link>
        <span className="text-gray-400 dark:text-gray-600 mx-2">/</span>
        <span className="text-orange-500 dark:text-orange-400">{date}</span>
      </nav>

      {/* Anchor nav */}
      <nav className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-gray-400 dark:text-gray-500">
        {anchorNav.map(s => (
          <a key={s.id} href={`#${s.id}`} className="hover:text-orange-500 dark:hover:text-orange-400 transition-colors">
            {s.label}
          </a>
        ))}
      </nav>

      {error && (
        <div className="bg-red-500/10 border border-red-500/30 rounded-lg p-4 text-red-400">
          {error}
        </div>
      )}

      {/* === But first: friction / satisfaction folded from the Insights tab === */}
      {hasButFirst && daySummary && (
        <Section id="but-first" title={t('dayDetail.sectionButFirst')}>
          <div className="bg-gray-50 dark:bg-daily-light rounded-xl p-5 border border-gray-200 dark:border-orange-500/20 space-y-2">
            {daySummary.sessions_with_friction > 0 && (
              <p className="text-sm text-gray-600 dark:text-gray-400">
                {t('dayDetail.frictionSummary', {
                  withFriction: daySummary.sessions_with_friction,
                  total: daySummary.total_sessions,
                })}
              </p>
            )}
            {daySummary.top_frictions.length > 0 && (
              <div className="flex flex-wrap gap-1.5">
                {daySummary.top_frictions.map(friction => (
                  <span
                    key={friction}
                    className="text-xs bg-amber-100 text-amber-700 dark:bg-amber-900/20 dark:text-amber-400 px-2 py-0.5 rounded-full"
                  >
                    {friction.replace(/_/g, ' ')}
                  </span>
                ))}
              </div>
            )}
            {daySummary.overall_satisfaction != null && (
              <p className="text-sm text-gray-600 dark:text-gray-400">
                {t('dayDetail.satisfaction')}:{' '}
                <span className={cn('font-medium', getSatisfactionIndicator(daySummary.overall_satisfaction).className)}>
                  {getSatisfactionIndicator(daySummary.overall_satisfaction).label}
                </span>
              </p>
            )}
          </div>
        </Section>
      )}

      {/* === Stats: KPI strip === */}
      <Section id="stats" title={t('dayDetail.sectionStats')}>
        <div className="bg-gray-50 dark:bg-daily-light rounded-xl p-5 border border-gray-200 dark:border-orange-500/20">
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-4">
            <div className="text-center">
              <div className="text-2xl font-bold text-orange-500 tabular-nums">
                {sessions.length}
              </div>
              <div className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">{t('dayDetail.sessions')}</div>
            </div>
            <div className="text-center">
              <div className="text-2xl font-bold text-sky-500 tabular-nums">{projects.length}</div>
              <div className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">{t('dayDetail.projects')}</div>
            </div>
            {daySummary?.total_tokens != null && (
              <div className="text-center">
                <div className="text-2xl font-bold text-gray-700 dark:text-gray-200 tabular-nums">
                  {formatTokenCount(daySummary.total_tokens)}
                </div>
                <div className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">{t('dayDetail.tokens')}</div>
              </div>
            )}
            <div className="text-center">
              <div className="text-2xl font-bold text-purple-500 tabular-nums">{formatCost(totalCost)}</div>
              <div className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">{t('dayDetail.cost')}</div>
            </div>
          </div>
          {runtimes.length > 0 && (
            <div className="flex flex-wrap gap-1.5 mt-4 pt-3 border-t border-gray-200 dark:border-gray-700/50">
              {runtimes.map(r => (
                <span
                  key={r}
                  className="text-xs bg-sky-100 text-sky-700 dark:bg-sky-900/30 dark:text-sky-400 px-2 py-0.5 rounded-full tabular-nums"
                >
                  {r} × {sessions.filter(s => s.runtime === r).length}
                </span>
              ))}
            </div>
          )}
        </div>
      </Section>

      {/* === Overview + Key Insights === */}
      {hasOverview && (
        <Section id="overview" title={t('dayDetail.sectionOverview')}>
          <div className="space-y-3">
            {summary?.overview && (
              <div className="bg-gray-50 dark:bg-daily-light rounded-xl p-5 border border-gray-200 dark:border-gray-800">
                <MarkdownRenderer content={summary.overview} />
              </div>
            )}
            {(summary?.insights ?? []).map((card, i) => (
              <CollapsibleCard key={`insight-${card.title}-${i}`} card={card} defaultOpen={i < 3} />
            ))}
          </div>
        </Section>
      )}

      {/* === Next steps: Focus tab + Insights recommendations === */}
      {hasNextSteps && (
        <Section id="next-steps" title={t('dayDetail.sectionNextSteps')}>
          <div className="space-y-3">
            {(summary?.tomorrow_focus ?? []).map((card, i) => (
              <CollapsibleCard key={`focus-${card.title}-${i}`} card={card} defaultOpen={true} />
            ))}
            {(daySummary?.recommendations.length ?? 0) > 0 && (
              <div className="bg-gray-50 dark:bg-daily-light rounded-xl p-5 border border-gray-200 dark:border-orange-500/20">
                <div className="flex items-center gap-1.5 mb-2">
                  <svg className="size-4 text-orange-500" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z" />
                  </svg>
                  <span className="text-sm font-semibold text-gray-700 dark:text-gray-300">{t('dayDetail.recommendations')}</span>
                </div>
                <ul className="space-y-1.5">
                  {daySummary!.recommendations.map((rec, i) => (
                    <li key={i} className="flex items-start gap-2 text-sm text-gray-600 dark:text-gray-400">
                      <span className="text-orange-400 mt-0.5 flex-shrink-0">-</span>
                      <span>{rec}</span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        </Section>
      )}

      {/* === Sessions: list + runtime filter chips === */}
      <Section id="sessions" title={t('dayDetail.sectionSessions')}>
        {runtimes.length > 1 && (
          <div className="flex flex-wrap gap-1.5">
            <button
              onClick={() => setRuntimeFilter(null)}
              className={cn(
                'px-2.5 py-1 text-xs font-medium rounded-full border transition-colors',
                runtimeFilter === null
                  ? 'bg-orange-500/20 text-orange-500 dark:text-orange-400 border-orange-500/30'
                  : 'text-gray-500 dark:text-gray-400 border-gray-200 dark:border-gray-700 hover:border-orange-400 dark:hover:border-orange-500/40'
              )}
            >
              {t('dayDetail.filterAll')}
            </button>
            {runtimes.map(r => (
              <button
                key={r}
                onClick={() => setRuntimeFilter(r)}
                className={cn(
                  'px-2.5 py-1 text-xs font-medium rounded-full border transition-colors tabular-nums',
                  runtimeFilter === r
                    ? 'bg-orange-500/20 text-orange-500 dark:text-orange-400 border-orange-500/30'
                    : 'text-gray-500 dark:text-gray-400 border-gray-200 dark:border-gray-700 hover:border-orange-400 dark:hover:border-orange-500/40'
                )}
              >
                {r} × {sessions.filter(s => s.runtime === r).length}
              </button>
            ))}
          </div>
        )}
        {filteredSessions.length > 0 ? (
          <div className="space-y-2">
            {filteredSessions.map(session => (
              <SessionRow
                key={session.name}
                session={session}
                date={date!}
                insight={insightByName.get(session.name)}
              />
            ))}
          </div>
        ) : (
          <div className="text-center py-8 bg-gray-50 dark:bg-daily-light rounded-xl border border-gray-200 dark:border-gray-800">
            <p className="text-gray-500 dark:text-gray-400">{t('dayDetail.noSessions')}</p>
          </div>
        )}
      </Section>

      {/* === Extracted: skills/commands hint cards + skipped sources === */}
      <Section id="extracted" title={t('dayDetail.sectionExtracted')}>
        {summary && (summary.skills.length > 0 || summary.commands.length > 0) ? (
          <div className="space-y-3">
            <div className="grid gap-3 sm:grid-cols-2">
              {summary.skills.map((card, i) => (
                <SkillCommandCard key={`skill-${card.title}-${i}`} card={card} type="skill" onInstall={installCard} />
              ))}
              {summary.commands.map((card, i) => (
                <SkillCommandCard key={`cmd-${card.title}-${i}`} card={card} type="command" onInstall={installCard} />
              ))}
            </div>
            {skippedProjects.length > 0 && (
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="text-xs text-gray-400 dark:text-gray-500">{t('dayDetail.noExtraction')}</span>
                {skippedProjects.map(p => (
                  <span
                    key={p}
                    className="text-xs text-gray-400 dark:text-gray-500 bg-gray-100 dark:bg-gray-800/50 px-2 py-0.5 rounded-full"
                  >
                    {p}
                  </span>
                ))}
              </div>
            )}
          </div>
        ) : (
          <div className="text-center py-8 bg-gray-50 dark:bg-daily-light rounded-xl border border-gray-200 dark:border-gray-800">
            <p className="text-gray-500 dark:text-gray-400">{t('dayDetail.noSkills')}</p>
          </div>
        )}
      </Section>
    </div>
  )
}
