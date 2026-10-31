import { useState, useEffect, useMemo } from 'react'
import { useParams, Link } from 'react-router-dom'
import { motion } from 'framer-motion'
import { useApi } from '../hooks/useApi'
import type { ConsolidatedRule, DailySummary, DateInsights, NextStepItem, ProjectGain, Session, SummaryCard, WasteAlert } from '../hooks/useApi'
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
  const duration = session.duration ?? formatDuration(session.started_utc, session.ended_utc)

  return (
    <Link to={`/day/${date}/session/${encodeURIComponent(session.name)}`}>
      <div
        className={cn(
          'p-4 rounded-xl border transition-colors',
          'bg-gray-50 dark:bg-daily-light border-gray-200 dark:border-gray-800 hover:border-orange-400 dark:hover:border-orange-500/40'
        )}
      >
        <div className="flex items-start justify-between gap-3 mb-1.5">
          <div className="flex items-center gap-2 min-w-0">
            {session.project && (
              <span className="px-2 py-0.5 rounded-md bg-orange-500/10 text-orange-600 dark:text-orange-400 border border-orange-500/20 text-xs font-semibold truncate max-w-44">
                {session.project}
              </span>
            )}
            <h3 className="font-semibold text-gray-900 dark:text-gray-100 line-clamp-1">
              {session.title || session.name}
            </h3>
          </div>
          <div className="flex items-center gap-1.5 flex-shrink-0">
            {session.category === 'review' && (
              <span className="text-[11px] font-medium px-2 py-0.5 rounded-full bg-sky-500/10 text-sky-600 dark:text-sky-400 border border-sky-500/20">
                Review
              </span>
            )}
            {session.category === 'blocked' && (
              <span className="text-[11px] font-medium px-2 py-0.5 rounded-full bg-red-500/10 text-red-600 dark:text-red-400 border border-red-500/20">
                Blocked
              </span>
            )}
            {insight?.outcome && (
              <span className={cn('text-xs font-medium px-2 py-0.5 rounded-full whitespace-nowrap', getOutcomeBadgeClass(insight.outcome))}>
                {formatOutcome(insight.outcome)}
              </span>
            )}
          </div>
        </div>

        {session.summary_preview && (
          <p className="text-sm text-gray-600 dark:text-gray-300 line-clamp-2 text-pretty mb-2">
            {session.summary_preview}
          </p>
        )}

        {session.key_decision && (
          <div className="mb-2.5 px-3 py-2 rounded-lg bg-amber-500/5 border border-amber-500/20 text-xs text-gray-700 dark:text-gray-300">
            <span className="font-semibold text-amber-600 dark:text-amber-400">Why: </span>
            {session.key_decision}
          </div>
        )}

        {session.blocker_reason && session.category === 'blocked' && (
          <div className="mb-2.5 px-3 py-1.5 rounded-lg bg-red-500/5 border border-red-500/20 text-xs text-red-600 dark:text-red-400 font-mono">
            {session.blocker_reason}
          </div>
        )}

        {session.files_touched && session.files_touched.length > 0 && (
          <div className="flex flex-wrap gap-1.5 mb-2.5">
            {session.files_touched.slice(0, 4).map(file => (
              <span
                key={file}
                className="px-2 py-0.5 rounded bg-gray-100 dark:bg-gray-800/80 border border-gray-200 dark:border-gray-700/60 font-mono text-[11px] text-gray-600 dark:text-gray-400 truncate max-w-64"
              >
                {file}
              </span>
            ))}
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2 text-xs">
          {session.runtime && (
            <span className="px-1.5 py-0.5 rounded bg-sky-100 text-sky-700 dark:bg-sky-900/30 dark:text-sky-400 font-medium font-mono">
              {session.runtime}
            </span>
          )}
          {duration && (
            <span className="text-gray-500 dark:text-gray-400 tabular-nums font-mono">{duration}</span>
          )}
          {session.cost_usd != null && (
            <span className="text-purple-500 font-medium tabular-nums font-mono">{formatCost(session.cost_usd)}</span>
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

function BoldColonItem({ text, dotColor }: { text: string; dotColor: string }) {
  const colonIdx = text.indexOf(': ')
  const head = colonIdx > 0 ? text.slice(0, colonIdx) : null
  const rest = colonIdx > 0 ? text.slice(colonIdx + 2) : text
  return (
    <li className="flex items-start gap-2 text-xs sm:text-sm text-gray-700 dark:text-gray-300 leading-relaxed">
      <span className={cn('font-bold mt-0.5 flex-shrink-0', dotColor)}>•</span>
      <span>
        {head ? (
          <>
            <strong className="font-semibold text-gray-900 dark:text-gray-100">{head}:</strong>{' '}
            {rest}
          </>
        ) : (
          rest
        )}
      </span>
    </li>
  )
}

function WorkflowCoachingCard({
  signals,
  learnings,
  nextSteps,
}: {
  signals: string[]
  learnings: string[]
  nextSteps: NextStepItem[]
}) {
  return (
    <div className="bg-gray-50 dark:bg-daily-light rounded-2xl p-5 border border-amber-500/30 space-y-4">
      <div className="grid gap-3 md:grid-cols-2">
        <div className="rounded-xl p-4 bg-white dark:bg-black/20 border border-gray-200 dark:border-gray-800 space-y-2">
          <div className="text-[11px] font-bold uppercase tracking-wider text-amber-600 dark:text-amber-400">
            ⚡ Efficiency & Friction Signals
          </div>
          {signals.length > 0 ? (
            <ul className="space-y-2">
              {signals.map((sig, i) => (
                <BoldColonItem key={i} text={sig} dotColor="text-amber-500" />
              ))}
            </ul>
          ) : (
            <p className="text-xs text-gray-500 dark:text-gray-400">
              No workflow friction signals detected today.
            </p>
          )}
        </div>

        <div className="rounded-xl p-4 bg-white dark:bg-black/20 border border-gray-200 dark:border-gray-800 space-y-2">
          <div className="text-[11px] font-bold uppercase tracking-wider text-sky-600 dark:text-sky-400">
            💡 Top Pitfalls & Learnings Today
          </div>
          {learnings.length > 0 ? (
            <ul className="space-y-2">
              {learnings.map((item, i) => (
                <BoldColonItem key={i} text={item} dotColor="text-sky-500" />
              ))}
            </ul>
          ) : (
            <p className="text-xs text-gray-500 dark:text-gray-400">
              No recurring pitfalls logged across today&apos;s sessions.
            </p>
          )}
        </div>
      </div>

      {nextSteps.length > 0 && (
        <div className="rounded-xl p-4 bg-orange-500/5 border border-orange-500/25 space-y-2.5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-[11px] font-bold uppercase tracking-wider text-orange-600 dark:text-orange-400">
              🎯 Recommended Next Steps & Action Items ({nextSteps.length})
            </span>
            <span className="text-[11px] font-mono text-gray-500 dark:text-gray-400">
              Derived from today&apos;s friction & deferred work
            </span>
          </div>
          <ul className="space-y-2">
            {nextSteps.map((step, i) => (
              <li
                key={i}
                className="flex items-start gap-2.5 text-xs sm:text-sm text-gray-700 dark:text-gray-300 leading-relaxed"
              >
                <span className="px-1.5 py-0.5 rounded bg-orange-500/15 text-orange-600 dark:text-orange-400 border border-orange-500/30 font-mono text-[10px] font-bold uppercase flex-shrink-0 mt-0.5">
                  {step.tag}
                </span>
                <span>
                  <strong className="font-semibold text-gray-900 dark:text-gray-100">
                    {step.title}:
                  </strong>{' '}
                  {step.detail}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}

function ConsolidatedRulesCard({
  rules,
  onInstall,
}: {
  rules: ConsolidatedRule[]
  onInstall: (title: string, content: string, cardType: 'skill' | 'command') => Promise<unknown>
}) {
  const [copiedAll, setCopiedAll] = useState(false)
  const [copiedIdx, setCopiedIdx] = useState<number | null>(null)
  const [installedIdx, setInstalledIdx] = useState<Record<number, boolean>>({})

  const formatRuleMarkdown = (r: ConsolidatedRule) => {
    const lines = [`- **[${r.name}]** (${r.project}): ${r.summary}`]
    if (r.trigger) lines.push(`  - Trigger: ${r.trigger}`)
    if (r.why) lines.push(`  - Why: ${r.why}`)
    return lines.join('\n')
  }

  const handleCopyAll = async () => {
    try {
      const text = rules.map(formatRuleMarkdown).join('\n\n')
      await navigator.clipboard.writeText(text)
      setCopiedAll(true)
      setTimeout(() => setCopiedAll(false), 2000)
    } catch {
      // clipboard unavailable on insecure origins
    }
  }

  const handleCopyOne = async (r: ConsolidatedRule, idx: number) => {
    try {
      await navigator.clipboard.writeText(formatRuleMarkdown(r))
      setCopiedIdx(idx)
      setTimeout(() => setCopiedIdx(null), 2000)
    } catch {
      // clipboard unavailable on insecure origins
    }
  }
  const handleInstallOne = async (r: ConsolidatedRule, idx: number) => {
    if (installedIdx[idx]) return
    try {
      await onInstall(r.name, formatRuleMarkdown(r), 'command')
      setInstalledIdx(prev => ({ ...prev, [idx]: true }))
    } catch {
      // handled by hook
    }
  }

  return (
    <div className="bg-gray-50 dark:bg-daily-light rounded-2xl p-5 border border-gray-200 dark:border-gray-800 space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2 pb-2 border-b border-gray-200 dark:border-gray-800">
        <span className="text-xs text-gray-500 dark:text-gray-400">
          Consolidated from session summaries ({rules.length} {rules.length === 1 ? 'rule' : 'rules'})
        </span>
        <button
          onClick={handleCopyAll}
          className="px-2.5 py-1 rounded-lg border border-gray-200 dark:border-gray-700 bg-white dark:bg-black/20 text-xs font-mono text-gray-700 dark:text-gray-300 hover:border-orange-400 transition-colors"
        >
          {copiedAll ? '✓ Copied All' : `Copy All ${rules.length} as Markdown`}
        </button>
      </div>
      <div className="space-y-2.5">
        {rules.map((r, i) => (
          <div
            key={`${r.project}-${r.name}-${i}`}
            className="rounded-xl p-3.5 bg-white dark:bg-black/20 border border-gray-200 dark:border-gray-800 flex flex-col sm:flex-row sm:items-start justify-between gap-3"
          >
            <div className="space-y-1 min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <span className="px-1.5 py-0.5 rounded bg-orange-500/10 text-orange-600 dark:text-orange-400 border border-orange-500/20 font-mono text-[11px] font-semibold">
                  {r.project}
                </span>
                <span className="font-mono text-xs sm:text-sm font-bold text-gray-900 dark:text-gray-100">
                  {r.name}
                </span>
              </div>
              <p className="text-xs sm:text-sm text-gray-700 dark:text-gray-300">
                {r.summary}
              </p>
              {(r.trigger || r.why) && (
                <p className="text-xs text-gray-500 dark:text-gray-400 leading-relaxed">
                  {r.trigger && (
                    <>
                      <strong className="text-gray-700 dark:text-gray-300">Trigger:</strong>{' '}
                      {r.trigger}
                    </>
                  )}
                  {r.trigger && r.why && ' · '}
                  {r.why && (
                    <>
                      <strong className="text-gray-700 dark:text-gray-300">Why:</strong> {r.why}
                    </>
                  )}
                </p>
              )}
            </div>
            <div className="flex items-center gap-1.5 flex-shrink-0">
              <button
                onClick={() => handleCopyOne(r, i)}
                className="px-2.5 py-1 rounded-md border border-gray-200 dark:border-gray-700 text-xs font-mono text-gray-600 dark:text-gray-300 hover:border-orange-400 transition-colors"
              >
                {copiedIdx === i ? '✓ Copied' : 'Copy'}
              </button>
              <button
                onClick={() => handleInstallOne(r, i)}
                disabled={!!installedIdx[i]}
                className="px-2.5 py-1 rounded-md border border-purple-500/30 text-xs font-mono text-purple-600 dark:text-purple-400 hover:bg-purple-500/10 disabled:opacity-50 transition-colors"
              >
                {installedIdx[i] ? '✓ Installed' : 'Install'}
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}

function ProjectGainCard({ gain }: { gain: ProjectGain }) {
  return (
    <div className="bg-gray-50 dark:bg-daily-light rounded-xl p-5 border border-gray-200 dark:border-gray-800 space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2 pb-2.5 border-b border-gray-200 dark:border-gray-800">
        <div className="flex items-center gap-2">
          <span className="size-2 rounded-full bg-orange-500" />
          <h3 className="text-base font-bold text-gray-900 dark:text-gray-100">{gain.project}</h3>
        </div>
        <div className="flex flex-wrap items-center gap-1.5 text-xs font-mono">
          {gain.work_count > 0 && (
            <span className="px-2 py-0.5 rounded-md bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20">
              {gain.work_count} work
            </span>
          )}
          {gain.review_count > 0 && (
            <span className="px-2 py-0.5 rounded-md bg-sky-500/10 text-sky-600 dark:text-sky-400 border border-sky-500/20">
              {gain.review_count} {gain.review_count === 1 ? 'review' : 'reviews'}
            </span>
          )}
          {gain.blocked_count > 0 && (
            <span className="px-2 py-0.5 rounded-md bg-red-500/10 text-red-600 dark:text-red-400 border border-red-500/20">
              {gain.blocked_count} blocked
            </span>
          )}
          {gain.total_cost_usd > 0 && (
            <span className="px-2 py-0.5 rounded-md bg-purple-500/10 text-purple-600 dark:text-purple-400 border border-purple-500/20 tabular-nums">
              {formatCost(gain.total_cost_usd)}
            </span>
          )}
        </div>
      </div>

      {gain.accomplishments.length > 0 && (
        <ul className="space-y-1.5">
          {gain.accomplishments.map((item, idx) => {
            const colonIdx = item.indexOf(': ')
            const head = colonIdx > 0 ? item.slice(0, colonIdx) : null
            const rest = colonIdx > 0 ? item.slice(colonIdx + 2) : item
            return (
              <li key={idx} className="flex items-start gap-2 text-sm text-gray-700 dark:text-gray-300">
                <span className="text-emerald-500 font-bold mt-0.5 flex-shrink-0">•</span>
                <span>
                  {head ? (
                    <>
                      <strong className="font-semibold text-gray-900 dark:text-gray-100">{head}:</strong>{' '}
                      {rest}
                    </>
                  ) : (
                    rest
                  )}
                </span>
              </li>
            )
          })}
        </ul>
      )}

      {gain.key_decisions.length > 0 && (
        <div className="rounded-lg bg-amber-500/5 border border-amber-500/20 p-3 space-y-1">
          <div className="text-[11px] font-bold uppercase tracking-wider text-amber-600 dark:text-amber-400">
            Key Decisions & Why
          </div>
          {gain.key_decisions.map((dec, idx) => (
            <p key={idx} className="text-xs text-gray-700 dark:text-gray-300 leading-relaxed">
              {dec}
            </p>
          ))}
        </div>
      )}

      {gain.files_touched.length > 0 && (
        <div className="flex flex-wrap gap-1.5 pt-1">
          {gain.files_touched.map(file => (
            <span
              key={file}
              className="px-2 py-0.5 rounded bg-gray-100 dark:bg-gray-800/80 border border-gray-200 dark:border-gray-700/60 font-mono text-[11px] text-gray-600 dark:text-gray-400"
            >
              {file}
            </span>
          ))}
        </div>
      )}
    </div>
  )
}

function WasteAlertCard({ alerts }: { alerts: WasteAlert[] }) {
  const totalBlocked = alerts.reduce((s, a) => s + a.count, 0)
  const totalWasteCost = alerts.reduce((s, a) => s + a.cost_usd, 0)
  return (
    <div className="rounded-xl p-5 bg-red-500/5 border border-red-500/25 space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="text-xs font-bold uppercase tracking-wider text-red-600 dark:text-red-400">
            ⚠ {totalBlocked} Blocked / Background Noise Sessions Hidden Below
          </span>
        </div>
        {totalWasteCost > 0 && (
          <span className="text-xs font-mono font-semibold text-red-600 dark:text-red-400 tabular-nums">
            {formatCost(totalWasteCost)} spent on blocked runs
          </span>
        )}
      </div>
      <div className="space-y-2">
        {alerts.map((a, i) => (
          <div
            key={`${a.project}-${i}`}
            className="rounded-lg bg-white/60 dark:bg-black/20 border border-red-500/15 p-3 text-xs space-y-1"
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="font-semibold text-gray-900 dark:text-gray-100">
                {a.count}× <span className="font-mono text-orange-500">{a.project}</span> — {a.reason}
              </span>
              {a.cost_usd > 0 && (
                <span className="font-mono text-gray-500 tabular-nums">{formatCost(a.cost_usd)}</span>
              )}
            </div>
            <p className="text-gray-600 dark:text-gray-400">
              <strong className="text-gray-800 dark:text-gray-200">Action:</strong> {a.hint}
            </p>
          </div>
        ))}
      </div>
    </div>
  )
}

export function DayDetail() {
  const { date } = useParams<{ date: string }>()
  const [summary, setSummary] = useState<DailySummary | null>(null)
  const [insights, setInsights] = useState<DateInsights | null>(null)
  const [sessions, setSessions] = useState<Session[]>([])
  const [runtimeFilter, setRuntimeFilter] = useState<string | null>(null)
  const [categoryFilter, setCategoryFilter] = useState<'work' | 'review' | 'blocked' | 'all'>('work')
  const { fetchDailySummary, fetchDateInsights, fetchSessions, installCard, loading, error } = useApi()
  const { t } = useLanguage()

  useEffect(() => {
    if (!date) return
    let cancelled = false
    setRuntimeFilter(null)
    setCategoryFilter('work')
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
        if (!cancelled) {
          setSessions(res)
          const hasWork = res.some(s => (s.category ?? 'work') === 'work')
          if (!hasWork) {
            setCategoryFilter('all')
          }
        }
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

  const counts = useMemo(() => {
    let work = 0
    let review = 0
    let blocked = 0
    for (const s of sessions) {
      const cat = s.category ?? 'work'
      if (cat === 'work') work++
      else if (cat === 'review') review++
      else blocked++
    }
    return {
      work: summary?.work_count ?? work,
      review: summary?.review_count ?? review,
      blocked: summary?.blocked_count ?? blocked,
    }
  }, [sessions, summary])

  const runtimes = useMemo(
    () => [...new Set(sessions.map(s => s.runtime).filter((r): r is string => !!r))].sort(),
    [sessions]
  )

  const filteredSessions = useMemo(() => {
    return sessions.filter(s => {
      const cat = s.category ?? 'work'
      if (categoryFilter !== 'all' && cat !== categoryFilter) return false
      if (runtimeFilter && s.runtime !== runtimeFilter) return false
      return true
    })
  }, [sessions, categoryFilter, runtimeFilter])

  const projects = useMemo(
    () => [...new Set(sessions.map(s => s.project).filter((p): p is string => !!p))],
    [sessions]
  )
  const totalCost = useMemo(
    () => sessions.reduce((sum, s) => sum + (s.cost_usd ?? 0), 0),
    [sessions]
  )

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
  const dayMetrics = summary?.day_metrics
  const projectGains = summary?.project_gains ?? []
  const wasteAlerts = summary?.waste_alerts ?? []
  const efficiencySignals = summary?.efficiency_signals ?? []
  const learnings = summary?.learnings ?? []
  const nextSteps = summary?.next_steps ?? []
  const consolidatedRules = summary?.consolidated_rules ?? []
  const hasExtracted = (summary?.skills?.length ?? 0) + (summary?.commands?.length ?? 0) > 0

  const totalTokens = dayMetrics?.total_tokens || daySummary?.total_tokens || 0
  const cacheHitPct =
    dayMetrics?.cache_hit_rate != null ? Math.round(dayMetrics.cache_hit_rate * 1000) / 10 : null
  const displayCost = dayMetrics?.total_cost_usd || totalCost
  const pricedSessionCount = sessions.filter(s => (s.cost_usd ?? 0) > 0).length
  const signalDenominator = counts.work + counts.review + counts.blocked
  const signalRatioPct =
    signalDenominator > 0 ? Math.round((counts.work / signalDenominator) * 100) : 100
  return (
    <div className="max-w-4xl mx-auto px-6 py-8 space-y-8">
      {/* Breadcrumb */}
      <nav className="text-sm">
        <Link to="/" className="text-gray-500 hover:text-gray-700 dark:hover:text-gray-300">
          {t('dayDetail.breadcrumbArchives')}
        </Link>
        <span className="text-gray-400 dark:text-gray-600 mx-2">/</span>
        <span className="text-orange-500 dark:text-orange-400 font-mono">{date}</span>
      </nav>

      {error && (
        <div className="bg-red-500/10 border border-red-500/30 rounded-lg p-4 text-red-400">
          {error}
        </div>
      )}

      {/* === 30-Second Scorecard Hero === */}
      <div className="bg-gray-50 dark:bg-daily-light rounded-2xl p-5 border border-gray-200 dark:border-gray-800 space-y-4">
        <div className="flex flex-wrap items-baseline justify-between gap-2 pb-3 border-b border-gray-200 dark:border-gray-800">
          <div>
            <h1 className="text-2xl font-bold text-gray-900 dark:text-gray-100">
              {date} · Daily Workflow Brief
            </h1>
            <p className="text-sm text-gray-500 dark:text-gray-400 mt-0.5">
              {counts.work} shipped work {counts.work === 1 ? 'session' : 'sessions'} across{' '}
              {projectGains.length || projects.length}{' '}
              {(projectGains.length || projects.length) === 1 ? 'project' : 'projects'} ·{' '}
              {counts.review} review {counts.review === 1 ? 'pass' : 'passes'} · {counts.blocked}{' '}
              blocked
            </p>
          </div>
          {runtimes.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {runtimes.map(r => (
                <span
                  key={r}
                  className="text-xs bg-sky-100 text-sky-700 dark:bg-sky-900/30 dark:text-sky-400 px-2.5 py-0.5 rounded-full font-mono tabular-nums"
                >
                  {r} × {sessions.filter(s => s.runtime === r).length}
                </span>
              ))}
            </div>
          )}
        </div>

        {/* 4-Box KPI Scorecard */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5">
          <div className="rounded-xl p-3 bg-white dark:bg-black/20 border border-gray-200 dark:border-gray-800">
            <div className="text-[11px] text-gray-500 dark:text-gray-400">Shipped vs Review</div>
            <div className="text-base font-bold font-mono tabular-nums text-gray-900 dark:text-gray-100 mt-0.5">
              {counts.work} work / {counts.review} rev
            </div>
            <div className="text-[11px] text-gray-500 dark:text-gray-400 mt-0.5">
              {signalRatioPct}% signal ratio
            </div>
          </div>

          <div className="rounded-xl p-3 bg-white dark:bg-black/20 border border-gray-200 dark:border-gray-800">
            <div className="text-[11px] text-gray-500 dark:text-gray-400">Total Spend</div>
            <div className="text-base font-bold font-mono tabular-nums text-gray-900 dark:text-gray-100 mt-0.5">
              {displayCost > 0 ? formatCost(displayCost) : '$0.00'}
            </div>
            <div className="text-[11px] text-gray-500 dark:text-gray-400 mt-0.5">
              {pricedSessionCount} priced {pricedSessionCount === 1 ? 'session' : 'sessions'}
            </div>
          </div>

          <div className="rounded-xl p-3 bg-white dark:bg-black/20 border border-gray-200 dark:border-gray-800">
            <div className="text-[11px] text-gray-500 dark:text-gray-400">Tokens & Cache</div>
            <div className="text-base font-bold font-mono tabular-nums text-gray-900 dark:text-gray-100 mt-0.5">
              {totalTokens > 0 ? `${formatTokenCount(totalTokens)} tok` : '—'}
            </div>
            <div className="text-[11px] text-gray-500 dark:text-gray-400 mt-0.5">
              {cacheHitPct != null
                ? `${cacheHitPct}% cache hit (${formatTokenCount(dayMetrics?.cache_read_tokens ?? 0)})`
                : 'No token telemetry'}
            </div>
          </div>

          <div className="rounded-xl p-3 bg-white dark:bg-black/20 border border-gray-200 dark:border-gray-800">
            <div className="text-[11px] text-gray-500 dark:text-gray-400">Tool Errors / Tests</div>
            <div className="text-base font-bold font-mono tabular-nums text-gray-900 dark:text-gray-100 mt-0.5">
              {dayMetrics?.failed_tool_calls ?? 0} err · {dayMetrics?.test_runs ?? 0} tests
            </div>
            <div className="text-[11px] text-gray-500 dark:text-gray-400 mt-0.5">
              {(dayMetrics?.interrupted ?? 0) > 0
                ? `${dayMetrics?.interrupted} interrupted`
                : (dayMetrics?.failed_tool_calls ?? 0) === 0
                  ? 'Clean execution'
                  : 'Inspect tool failures'}
            </div>
          </div>
        </div>

        {/* Interactive Signal vs Noise Pills */}
        <div className="flex flex-wrap gap-2 pt-1">
          <a
            href="#sessions"
            onClick={() => setCategoryFilter('work')}
            className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-emerald-500/10 border border-emerald-500/25 text-emerald-700 dark:text-emerald-400 text-xs font-medium hover:bg-emerald-500/15 transition-colors"
          >
            <span className="size-2 rounded-full bg-emerald-500" />
            <span className="font-bold font-mono tabular-nums">{counts.work}</span>
            <span>Shipped Work</span>
          </a>
          <a
            href="#sessions"
            onClick={() => setCategoryFilter('review')}
            className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-sky-500/10 border border-sky-500/25 text-sky-700 dark:text-sky-400 text-xs font-medium hover:bg-sky-500/15 transition-colors"
          >
            <span className="size-2 rounded-full bg-sky-500" />
            <span className="font-bold font-mono tabular-nums">{counts.review}</span>
            <span>Code Reviews</span>
          </a>
          {counts.blocked > 0 && (
            <a
              href="#sessions"
              onClick={() => setCategoryFilter('blocked')}
              className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-red-500/10 border border-red-500/25 text-red-700 dark:text-red-400 text-xs font-medium hover:bg-red-500/15 transition-colors"
            >
              <span className="size-2 rounded-full bg-red-500" />
              <span className="font-bold font-mono tabular-nums">{counts.blocked}</span>
              <span>Blocked / Noise</span>
            </a>
          )}
        </div>
      </div>

      {/* === 1. Workflow Coaching, Learnings & Next Steps (~90s read) === */}
      {(efficiencySignals.length > 0 || learnings.length > 0 || nextSteps.length > 0) && (
        <Section id="coaching" title="1 · Workflow Coaching, Learnings & Next Steps">
          <WorkflowCoachingCard
            signals={efficiencySignals}
            learnings={learnings}
            nextSteps={nextSteps}
          />
        </Section>
      )}

      {/* === 2. What You Gained Today (Project Synthesis) === */}
      {projectGains.length > 0 && (
        <Section id="gains" title="2 · What Shipped & Why — By Project">
          <div className="grid gap-4">
            {projectGains.map(gain => (
              <ProjectGainCard key={gain.project} gain={gain} />
            ))}
          </div>
        </Section>
      )}

      {/* === Action Needed: Wasted / Blocked Background Runs === */}
      {wasteAlerts.length > 0 && (
        <Section id="alerts" title="Action Needed — Blocked Background Runs">
          <WasteAlertCard alerts={wasteAlerts} />
        </Section>
      )}

      {/* === 3. Consolidated Workflow Rules (1 card instead of N fragmented cards) === */}
      {consolidatedRules.length > 0 ? (
        <Section
          id="extracted"
          title={`3 · Consolidated Workflow Rules Extracted Today (${consolidatedRules.length})`}
        >
          <ConsolidatedRulesCard rules={consolidatedRules} onInstall={installCard} />
        </Section>
      ) : (
        hasExtracted &&
        summary && (
          <Section id="extracted" title={t('dayDetail.sectionExtracted')}>
            <div className="space-y-3">
              <div className="grid gap-3 sm:grid-cols-2">
                {summary.skills.map((card, i) => (
                  <SkillCommandCard
                    key={`skill-${card.title}-${i}`}
                    card={card}
                    type="skill"
                    onInstall={installCard}
                  />
                ))}
                {summary.commands.map((card, i) => (
                  <SkillCommandCard
                    key={`cmd-${card.title}-${i}`}
                    card={card}
                    type="command"
                    onInstall={installCard}
                  />
                ))}
              </div>
              {skippedProjects.length > 0 && (
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="text-xs text-gray-400 dark:text-gray-500">
                    {t('dayDetail.noExtraction')}
                  </span>
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
          </Section>
        )
      )}

      {/* === 4. Sessions: Segmented by Work / Reviews / Blocked === */}
      <Section id="sessions" title={t('dayDetail.sectionSessions')}>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap gap-1.5">
            {(
              [
                { id: 'work', label: `Shipped Work (${counts.work})` },
                { id: 'review', label: `Reviews (${counts.review})` },
                ...(counts.blocked > 0 ? [{ id: 'blocked', label: `Blocked (${counts.blocked})` }] : []),
                { id: 'all', label: `All (${sessions.length})` },
              ] as { id: 'work' | 'review' | 'blocked' | 'all'; label: string }[]
            ).map(tab => (
              <button
                key={tab.id}
                onClick={() => setCategoryFilter(tab.id)}
                className={cn(
                  'px-3 py-1.5 text-xs font-semibold rounded-lg border transition-colors tabular-nums',
                  categoryFilter === tab.id
                    ? 'bg-orange-500/20 text-orange-600 dark:text-orange-400 border-orange-500/40'
                    : 'text-gray-500 dark:text-gray-400 border-gray-200 dark:border-gray-800 hover:border-orange-400/40'
                )}
              >
                {tab.label}
              </button>
            ))}
          </div>

          {runtimes.length > 1 && (
            <div className="flex flex-wrap gap-1.5">
              <button
                onClick={() => setRuntimeFilter(null)}
                className={cn(
                  'px-2.5 py-1 text-xs font-medium rounded-full border transition-colors',
                  runtimeFilter === null
                    ? 'bg-sky-500/20 text-sky-600 dark:text-sky-400 border-sky-500/30'
                    : 'text-gray-500 dark:text-gray-400 border-gray-200 dark:border-gray-700'
                )}
              >
                {t('dayDetail.filterAll')}
              </button>
              {runtimes.map(r => (
                <button
                  key={r}
                  onClick={() => setRuntimeFilter(r)}
                  className={cn(
                    'px-2.5 py-1 text-xs font-medium rounded-full border transition-colors font-mono tabular-nums',
                    runtimeFilter === r
                      ? 'bg-sky-500/20 text-sky-600 dark:text-sky-400 border-sky-500/30'
                      : 'text-gray-500 dark:text-gray-400 border-gray-200 dark:border-gray-700'
                  )}
                >
                  {r}
                </button>
              ))}
            </div>
          )}
        </div>

        {filteredSessions.length > 0 ? (
          <div className="space-y-2.5">
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
    </div>
  )
}
