import { useState, useEffect, useMemo } from 'react'
import { useParams, Link } from 'react-router-dom'
import { motion } from 'framer-motion'
import { useApi } from '../hooks/useApi'
import type {
  ConsolidatedRule,
  DailySummary,
  DateInsights,
  NextStepItem,
  ProjectGain,
  ProjectLearnings,
  Session,
  SummaryCard,
  WasteAlert,
} from '../hooks/useApi'
import { MarkdownRenderer } from '../components/MarkdownRenderer'
import { cn } from '../lib/utils'
import { formatTokenCount, formatCost } from '../components/UsageCharts'
import { useLanguage } from '../contexts/LanguageContext'

const formatOutcome = (outcome: string): string =>
  outcome.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase())

const getOutcomePillTone = (outcome: string | null): string => {
  switch (outcome) {
    case 'achieved':
      return 'emerald'
    case 'partially_achieved':
      return 'amber'
    case 'not_achieved':
      return 'coral'
    default:
      return ''
  }
}

// Normalize project label to crisp 1-2 word name (never raw paths or scratch slugs)
const normalizeProjectLabel = (raw?: string): string => {
  if (!raw) return 'workspace'
  const base = raw.split('/').filter(Boolean).pop() ?? raw
  if (/^\d{4}-\d{2}-\d{2}-/.test(base) || base.length > 26) {
    return 'scratch'
  }
  return base
}

// Clean up raw session slugs into crisp human labels
const cleanSessionTitle = (raw: string): string => {
  const cleaned = raw
    .replace(/^\d{2}[_:]\d{2}[-_]?/, '')
    .replace(/[-_]+/g, ' ')
    .trim()
  if (!cleaned) return raw
  return cleaned.charAt(0).toUpperCase() + cleaned.slice(1)
}

const formatDuration = (started?: string, ended?: string): string | null => {
  if (!started || !ended) return null
  const s = Date.parse(started)
  const e = Date.parse(ended)
  if (Number.isNaN(s) || Number.isNaN(e) || e < s) return null
  const mins = Math.floor((e - s) / 60000)
  return mins >= 60 ? `${Math.floor(mins / 60)}h ${mins % 60}m` : `${mins}m`
}

// Section Header with Left Kicker (11px uppercase #64748B) and Right Monospace Meta Badge (#475569)
function Section({
  id,
  title,
  meta,
  children,
}: {
  id: string
  title: string
  meta?: string
  children: React.ReactNode
}) {
  return (
    <motion.section
      id={id}
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      className="scroll-mt-20 space-y-3"
    >
      <div className="flex items-center justify-between gap-2 px-0.5">
        <div className="flex items-center gap-2">
          <h2 className="qp-kicker">{title}</h2>
          <a
            href={`#${id}`}
            aria-label={`Link to ${title}`}
            className="font-mono text-[11px] text-[#475569] hover:text-[#FBBF24] transition-colors"
          >
            #
          </a>
        </div>
        {meta && <span className="qp-meta">{meta}</span>}
      </div>
      {children}
    </motion.section>
  )
}

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
  const { t } = useLanguage()
  const isSkill = type === 'skill'

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
    } catch {
      // handled by hook
    } finally {
      setInstalling(false)
    }
  }

  return (
    <div className="qp-card overflow-hidden">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className={cn(
          'w-full text-left qp-tile rounded-none border-0 shadow-none',
          isSkill ? 'emerald' : 'amber'
        )}
      >
        <div className="qp-tile-icon">{isSkill ? 'SK' : 'CM'}</div>
        <div className="qp-tile-name">{card.title}</div>
        <div className="qp-tile-state font-mono">
          {isSkill ? t('dayDetail.skill') : t('dayDetail.cmd')} · {expanded ? 'hide' : 'inspect'}
        </div>
      </button>

      {expanded && (
        <div className="border-t border-white/[0.06] p-4 space-y-3">
          <div className="text-xs text-[#94A3B8] markdown-content">
            <MarkdownRenderer content={card.content} />
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={handleCopy}
              className="px-2.5 py-1 rounded-lg border border-white/10 text-xs font-mono text-[#94A3B8] hover:text-[#F8FAFC] hover:border-white/25 transition-colors"
            >
              {copyOk ? t('dayDetail.copied') : t('dayDetail.copy')}
            </button>
            <button
              type="button"
              onClick={handleInstall}
              disabled={installing || installed}
              className="px-2.5 py-1 rounded-lg border border-amber-400/30 text-xs font-mono text-[#FBBF24] hover:bg-amber-500/10 disabled:opacity-50 transition-colors"
            >
              {installing
                ? t('dayDetail.installing')
                : installed
                  ? t('dayDetail.installed')
                  : t('dayDetail.install')}
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

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
  const cat = session.category ?? 'work'
  const tileTone = cat === 'work' ? 'emerald' : cat === 'review' ? 'sky' : 'coral'
  const projLabel = normalizeProjectLabel(session.project)
  const badgeText =
    cat === 'review'
      ? 'RV'
      : cat === 'blocked'
        ? '!'
        : projLabel.slice(0, 2).toUpperCase()

  const telemetryParts = [
    projLabel,
    session.runtime,
    duration,
    session.cost_usd != null && session.cost_usd > 0 ? formatCost(session.cost_usd) : null,
  ].filter(Boolean)

  return (
    <Link
      to={`/day/${date}/session/${encodeURIComponent(session.name)}`}
      className="block group"
    >
      <div className="qp-card overflow-hidden hover:border-white/20 transition-all">
        {/* 66px Interactive Control Tile Header ("i n" / "i l") */}
        <div className={cn('qp-tile rounded-none border-0 shadow-none', tileTone)}>
          <div className="qp-tile-icon">{badgeText}</div>
          <div className="qp-tile-name flex items-center justify-between gap-2">
            <span className="truncate">{cleanSessionTitle(session.title || session.name)}</span>
            {insight?.outcome && (
              <span
                className={cn(
                  'qp-pill !py-0.5 !px-2 !text-[10px] shrink-0 font-mono',
                  getOutcomePillTone(insight.outcome)
                )}
              >
                {formatOutcome(insight.outcome)}
              </span>
            )}
          </div>
          <div className="qp-tile-state font-mono">
            {telemetryParts.join(' · ')}
          </div>
        </div>

        {/* Supplementary Digest Context */}
        {(session.summary_preview || session.key_decision || (session.files_touched?.length ?? 0) > 0) && (
          <div className="px-3.5 py-3 border-t border-white/[0.06] space-y-2">
            {session.summary_preview && (
              <p className="text-xs text-[#94A3B8] line-clamp-2 leading-relaxed">
                {session.summary_preview}
              </p>
            )}

            {session.key_decision && (
              <div className="px-3 py-2 rounded-xl qp-surface-amber text-xs text-[#F8FAFC] leading-relaxed">
                <span className="font-mono font-bold text-[#FBBF24] uppercase text-[10px] mr-1.5">
                  WHY
                </span>
                {session.key_decision}
              </div>
            )}

            {session.blocker_reason && session.category === 'blocked' && (
              <div className="px-3 py-1.5 rounded-xl qp-surface-coral text-xs text-[#FB7185] font-mono">
                {session.blocker_reason}
              </div>
            )}

            {session.files_touched && session.files_touched.length > 0 && (
              <div className="flex flex-wrap gap-1.5 pt-0.5">
                {session.files_touched.slice(0, 4).map(file => (
                  <span
                    key={file}
                    className="px-2 py-0.5 rounded-md bg-[#0B0E14]/70 border border-white/[0.07] font-mono text-[11px] text-[#94A3B8] truncate max-w-60"
                  >
                    {file}
                  </span>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </Link>
  )
}

function BoldColonItem({ text, dotColor }: { text: string; dotColor: string }) {
  const colonIdx = text.indexOf(': ')
  const head = colonIdx > 0 ? text.slice(0, colonIdx) : null
  const rest = colonIdx > 0 ? text.slice(colonIdx + 2) : text
  return (
    <li className="flex items-start gap-2 text-xs text-[#94A3B8] leading-relaxed">
      <span className={cn('font-bold mt-0.5 flex-shrink-0', dotColor)}>•</span>
      <span>
        {head ? (
          <>
            <strong className="font-bold text-[#F8FAFC]">{head}:</strong> {rest}
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
  nextSteps,
}: {
  signals: string[]
  nextSteps: NextStepItem[]
}) {
  return (
    <div className="qp-card p-4 space-y-3.5">
      <div className="rounded-xl p-3.5 bg-[#0B0E14]/65 border border-white/[0.06] space-y-2">
        <div className="flex items-center justify-between">
          <span className="qp-kicker !text-[#FBBF24]">⚡ Cost, Cache & Command Health</span>
          <span className="qp-meta">{signals.length} SIGNALS</span>
        </div>
        {signals.length > 0 ? (
          <ul className="space-y-1.5">
            {signals.map((sig, i) => (
              <BoldColonItem key={i} text={sig} dotColor="text-[#FBBF24]" />
            ))}
          </ul>
        ) : (
          <p className="text-xs text-[#64748B]">
            No workflow friction signals detected today.
          </p>
        )}
      </div>

      {nextSteps.length > 0 && (
        <div className="rounded-xl p-3.5 qp-surface-amber space-y-2.5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="qp-kicker !text-[#FBBF24]">
              🎯 Recommended Next Steps
            </span>
            <span className="font-mono text-[11px] font-semibold text-[#FBBF24]/80 tabular-nums">
              {nextSteps.length} ACTIONS
            </span>
          </div>
          <ul className="space-y-2">
            {nextSteps.map((step, i) => (
              <li
                key={i}
                className="flex items-start gap-2.5 text-xs text-[#F8FAFC]/90 leading-relaxed"
              >
                <span className="px-1.5 py-0.5 rounded bg-amber-500/20 text-[#FBBF24] border border-amber-400/30 font-mono text-[10px] font-bold uppercase flex-shrink-0 mt-0.5">
                  {step.tag}
                </span>
                <span>
                  <strong className="font-bold text-[#F8FAFC]">
                    {step.title}:
                  </strong>{' '}
                  <span className="text-[#94A3B8]">{step.detail}</span>
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}

function ProjectLearningsSection({
  groups,
  fallbackLearnings,
}: {
  groups: ProjectLearnings[]
  fallbackLearnings: string[]
}) {
  const effectiveGroups: ProjectLearnings[] =
    groups.length > 0
      ? groups
      : fallbackLearnings.length > 0
        ? [
            {
              project: 'workspace',
              pitfalls: [],
              learnings: fallbackLearnings.map(text => ({
                kind: 'learning',
                text,
                session_title: '',
              })),
            },
          ]
        : []

  if (effectiveGroups.length === 0) return null

  return (
    <div className="grid gap-3.5">
      {effectiveGroups.map(group => {
        const pitfallCount = group.pitfalls.length
        const learningCount = group.learnings.length
        const projectName = normalizeProjectLabel(group.project)
        return (
          <div key={group.project} className="qp-card p-4 space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-2 pb-2.5 border-b border-white/[0.06]">
              <div className="flex items-center gap-2">
                <span className="qp-dot text-[#38BDF8]" />
                <h3 className="qp-card-title">{projectName}</h3>
              </div>
              <div className="flex flex-wrap items-center gap-1.5 text-[11px] font-mono tabular-nums">
                {pitfallCount > 0 && (
                  <span className="qp-pill coral !py-0.5 !px-2 !text-[11px]">
                    {pitfallCount} {pitfallCount === 1 ? 'pitfall' : 'pitfalls'}
                  </span>
                )}
                {learningCount > 0 && (
                  <span className="qp-pill sky !py-0.5 !px-2 !text-[11px]">
                    {learningCount} {learningCount === 1 ? 'learning' : 'learnings'}
                  </span>
                )}
              </div>
            </div>

            {pitfallCount > 0 && (
              <div className="rounded-xl qp-surface-coral p-3.5 space-y-2">
                <div className="qp-kicker !text-[#FB7185]">⚠ Pitfalls & Gotchas</div>
                <ul className="space-y-2">
                  {group.pitfalls.map((item, idx) => (
                    <li
                      key={idx}
                      className="flex items-start gap-2 text-xs text-[#F8FAFC]/90 leading-relaxed"
                    >
                      <span className="text-[#FB7185] font-bold mt-0.5 flex-shrink-0">•</span>
                      <div className="space-y-0.5">
                        <span>{item.text}</span>
                        {item.session_title && (
                          <div className="text-[11px] font-mono text-[#94A3B8]">
                            Session: {cleanSessionTitle(item.session_title)}
                          </div>
                        )}
                      </div>
                    </li>
                  ))}
                </ul>
              </div>
            )}

            {learningCount > 0 && (
              <div className="rounded-xl qp-surface-sky p-3.5 space-y-2">
                <div className="qp-kicker !text-[#38BDF8]">💡 System & Codebase Learnings</div>
                <ul className="space-y-2">
                  {group.learnings.map((item, idx) => (
                    <li
                      key={idx}
                      className="flex items-start gap-2 text-xs text-[#F8FAFC]/90 leading-relaxed"
                    >
                      <span className="text-[#38BDF8] font-bold mt-0.5 flex-shrink-0">•</span>
                      <div className="space-y-0.5">
                        <span>{item.text}</span>
                        {item.session_title && (
                          <div className="text-[11px] font-mono text-[#94A3B8]">
                            Session: {cleanSessionTitle(item.session_title)}
                          </div>
                        )}
                      </div>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}

function ConsolidatedRulesCard({
  rules,
}: {
  rules: ConsolidatedRule[]
}) {
  const [copiedKey, setCopiedKey] = useState<string | null>(null)

  const resolveScope = (r: ConsolidatedRule): 'global-skill' | 'repo-rule' => {
    if (r.scope === 'global-skill' || r.scope === 'repo-rule') return r.scope
    const proj = (r.project ?? '').trim().toLowerCase()
    return !proj || proj === 'scratch' || proj === 'default' || proj === 'unknown'
      ? 'global-skill'
      : 'repo-rule'
  }

  const formatRuleDraft = (r: ConsolidatedRule) =>
    r.draft_markdown?.trim() || `- **${r.name}**: ${r.summary}`

  const copyText = async (key: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopiedKey(key)
      setTimeout(() => setCopiedKey(prev => (prev === key ? null : prev)), 2000)
    } catch {
      // clipboard unavailable
    }
  }

  const globalSkills = rules.filter(r => resolveScope(r) === 'global-skill')
  const repoRules = rules.filter(r => resolveScope(r) === 'repo-rule')
  const repoGroups = Array.from(
    repoRules.reduce((acc, rule) => {
      const proj = normalizeProjectLabel(rule.project)
      const list = acc.get(proj) ?? []
      list.push(rule)
      acc.set(proj, list)
      return acc
    }, new Map<string, ConsolidatedRule[]>()),
  )

  return (
    <div className="space-y-4">
      {globalSkills.length > 0 && (
        <div className="qp-card p-4 space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-2 pb-2.5 border-b border-white/[0.06]">
            <div className="space-y-0.5">
              <div className="flex items-center gap-2">
                <span className="qp-dot text-[#34D399]" />
                <h3 className="qp-card-title">Global Reusable Skills</h3>
                <span className="px-1.5 py-0.5 rounded bg-emerald-500/15 text-[#34D399] border border-emerald-400/30 font-mono text-[10px] font-semibold">
                  ~/.agents/skills/&lt;name&gt;/SKILL.md
                </span>
              </div>
              <p className="text-xs text-[#94A3B8]">
                Cross-repository workflows portable across any project ({globalSkills.length}{' '}
                {globalSkills.length === 1 ? 'skill' : 'skills'})
              </p>
            </div>
            <button
              type="button"
              onClick={() =>
                copyText(
                  'all-skills',
                  globalSkills.map(formatRuleDraft).join('\n\n'),
                )
              }
              className="px-2.5 py-1 rounded-lg border border-emerald-400/30 bg-emerald-500/10 text-xs font-mono text-[#34D399] hover:bg-emerald-500/20 transition-colors tabular-nums"
            >
              {copiedKey === 'all-skills'
                ? '✓ Copied All SKILL.md'
                : `Copy All SKILL.md (${globalSkills.length})`}
            </button>
          </div>
          <div className="space-y-2.5">
            {globalSkills.map((r, i) => {
              const key = `skill-${r.name}-${i}`
              const actionText =
                r.actions && r.actions.length > 0 ? r.actions.join(' ') : r.why
              return (
                <div
                  key={key}
                  className="rounded-xl p-3 bg-[#0B0E14]/65 border border-white/[0.06] flex flex-col sm:flex-row sm:items-start justify-between gap-3"
                >
                  <div className="space-y-1 min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="px-1.5 py-0.5 rounded bg-emerald-500/15 text-[#34D399] border border-emerald-400/30 font-mono text-[10px] font-semibold">
                        global-skill
                      </span>
                      <span className="font-mono text-xs font-bold text-[#F8FAFC]">
                        {r.name}
                      </span>
                      <span className="text-[10px] font-mono text-[#64748B]">
                        from {normalizeProjectLabel(r.project)}
                      </span>
                    </div>
                    <p className="text-xs text-[#94A3B8] leading-relaxed">
                      {r.summary}
                    </p>
                    {(r.trigger || actionText || r.verification) && (
                      <div className="text-[11px] text-[#64748B] leading-relaxed space-y-0.5 pt-0.5">
                        {r.trigger && (
                          <p>
                            <strong className="text-[#94A3B8]">When:</strong> {r.trigger}
                          </p>
                        )}
                        {actionText && (
                          <p>
                            <strong className="text-[#94A3B8]">Procedure:</strong> {actionText}
                          </p>
                        )}
                        {r.verification && (
                          <p>
                            <strong className="text-[#94A3B8]">Verify:</strong> {r.verification}
                          </p>
                        )}
                      </div>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5 flex-shrink-0">
                    <button
                      type="button"
                      onClick={() => copyText(key, formatRuleDraft(r))}
                      className="px-2.5 py-1 rounded-lg border border-emerald-400/30 bg-emerald-500/10 text-[11px] font-mono text-[#34D399] hover:bg-emerald-500/20 transition-colors"
                    >
                      {copiedKey === key ? '✓ Copied SKILL.md' : 'Copy SKILL.md'}
                    </button>
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      )}

      {repoGroups.map(([projectName, groupRules]) => {
        const groupKey = `repo-group-${projectName}`
        return (
          <div key={groupKey} className="qp-card p-4 space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-2 pb-2.5 border-b border-white/[0.06]">
              <div className="space-y-0.5">
                <div className="flex items-center gap-2">
                  <span className="qp-dot text-[#FBBF24]" />
                  <h3 className="qp-card-title">{projectName} Repository Rules</h3>
                  <span className="px-1.5 py-0.5 rounded bg-amber-500/15 text-[#FBBF24] border border-amber-400/30 font-mono text-[10px] font-semibold">
                    {projectName}/AGENTS.md
                  </span>
                </div>
                <p className="text-xs text-[#94A3B8]">
                  Project-specific instructions to append to <code className="font-mono text-[#F8FAFC]">{projectName}/AGENTS.md</code> ({groupRules.length}{' '}
                  {groupRules.length === 1 ? 'rule' : 'rules'})
                </p>
              </div>
              <button
                type="button"
                onClick={() =>
                  copyText(
                    groupKey,
                    `## Workflow & Validation Rules\n\n${groupRules.map(formatRuleDraft).join('\n')}`,
                  )
                }
                className="px-2.5 py-1 rounded-lg border border-amber-400/30 bg-amber-500/10 text-xs font-mono text-[#FBBF24] hover:bg-amber-500/20 transition-colors tabular-nums"
              >
                {copiedKey === groupKey
                  ? `✓ Copied ${projectName}/AGENTS.md`
                  : `Copy for ${projectName}/AGENTS.md (${groupRules.length})`}
              </button>
            </div>
            <div className="space-y-2.5">
              {groupRules.map((r, i) => {
                const key = `repo-${projectName}-${r.name}-${i}`
                const doText =
                  r.actions && r.actions.length > 0 ? r.actions.join(' ') : r.why
                return (
                  <div
                    key={key}
                    className="rounded-xl p-3 bg-[#0B0E14]/65 border border-white/[0.06] flex flex-col sm:flex-row sm:items-start justify-between gap-3"
                  >
                    <div className="space-y-1 min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="px-1.5 py-0.5 rounded bg-amber-500/15 text-[#FBBF24] border border-amber-400/30 font-mono text-[10px] font-semibold">
                          {projectName}/AGENTS.md
                        </span>
                        <span className="font-mono text-xs font-bold text-[#F8FAFC]">
                          {r.name}
                        </span>
                      </div>
                      <p className="text-xs text-[#94A3B8] leading-relaxed">
                        {r.summary}
                      </p>
                      {(r.trigger || doText || r.verification) && (
                        <div className="text-[11px] text-[#64748B] leading-relaxed space-y-0.5 pt-0.5">
                          {r.trigger && (
                            <p>
                              <strong className="text-[#94A3B8]">When:</strong> {r.trigger}
                            </p>
                          )}
                          {doText && (
                            <p>
                              <strong className="text-[#94A3B8]">Do:</strong> {doText}
                            </p>
                          )}
                          {r.verification && (
                            <p>
                              <strong className="text-[#94A3B8]">Verify:</strong> {r.verification}
                            </p>
                          )}
                        </div>
                      )}
                    </div>
                    <div className="flex items-center gap-1.5 flex-shrink-0">
                      <button
                        type="button"
                        onClick={() => copyText(key, formatRuleDraft(r))}
                        className="px-2.5 py-1 rounded-lg border border-white/10 text-[11px] font-mono text-[#94A3B8] hover:text-[#F8FAFC] hover:border-white/25 transition-colors"
                      >
                        {copiedKey === key ? '✓ Copied Rule' : 'Copy Rule'}
                      </button>
                    </div>
                  </div>
                )
              })}
            </div>
          </div>
        )
      })}
    </div>
  )
}

function ProjectGainCard({ gain }: { gain: ProjectGain }) {
  const projectName = normalizeProjectLabel(gain.project)
  return (
    <div className="qp-card p-4 space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2 pb-2.5 border-b border-white/[0.06]">
        <div className="flex items-center gap-2">
          <span className="qp-dot text-[#34D399]" />
          <h3 className="qp-card-title">{projectName}</h3>
        </div>
        <div className="flex flex-wrap items-center gap-1.5 text-[11px] font-mono tabular-nums">
          {gain.work_count > 0 && (
            <span className="qp-pill emerald !py-0.5 !px-2 !text-[11px]">
              {gain.work_count} work
            </span>
          )}
          {gain.review_count > 0 && (
            <span className="qp-pill sky !py-0.5 !px-2 !text-[11px]">
              {gain.review_count} {gain.review_count === 1 ? 'review' : 'reviews'}
            </span>
          )}
          {gain.blocked_count > 0 && (
            <span className="qp-pill coral !py-0.5 !px-2 !text-[11px]">
              {gain.blocked_count} blocked
            </span>
          )}
          {gain.total_cost_usd > 0 && (
            <span className="qp-pill amber !py-0.5 !px-2 !text-[11px]">
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
              <li key={idx} className="flex items-start gap-2 text-xs sm:text-[13px] text-[#94A3B8] leading-relaxed">
                <span className="text-[#34D399] font-bold mt-0.5 flex-shrink-0">•</span>
                <span>
                  {head ? (
                    <>
                      <strong className="font-bold text-[#F8FAFC]">{head}:</strong> {rest}
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
        <div className="rounded-xl qp-surface-amber p-3 space-y-1.5">
          <div className="qp-kicker !text-[#FBBF24]">Key Decisions & Why</div>
          {gain.key_decisions.map((dec, idx) => (
            <p key={idx} className="text-xs text-[#F8FAFC]/90 leading-relaxed">
              {dec}
            </p>
          ))}
        </div>
      )}

      {gain.files_touched.length > 0 && (
        <div className="flex flex-wrap gap-1.5 pt-0.5">
          {gain.files_touched.map(file => (
            <span
              key={file}
              className="px-2 py-0.5 rounded-md bg-[#0B0E14]/75 border border-white/[0.07] font-mono text-[11px] text-[#94A3B8]"
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
    <div className="rounded-[18px] p-4 qp-surface-coral space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="qp-kicker !text-[#FB7185]">
          ⚠ {totalBlocked} Blocked / Background Noise Runs Hidden Below
        </span>
        {totalWasteCost > 0 && (
          <span className="text-xs font-mono font-semibold text-[#FB7185] tabular-nums">
            {formatCost(totalWasteCost)} wasted
          </span>
        )}
      </div>
      <div className="space-y-2">
        {alerts.map((a, i) => (
          <div
            key={`${a.project}-${i}`}
            className="rounded-xl bg-[#0B0E14]/70 border border-rose-400/20 p-3 text-xs space-y-1"
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="font-bold text-[#F8FAFC]">
                {a.count}× <span className="font-mono text-[#FB7185]">{normalizeProjectLabel(a.project)}</span> — {a.reason}
              </span>
              {a.cost_usd > 0 && (
                <span className="font-mono text-[#94A3B8] tabular-nums">{formatCost(a.cost_usd)}</span>
              )}
            </div>
            <p className="text-[#94A3B8]">
              <strong className="text-[#F8FAFC]">Action:</strong> {a.hint}
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
          // Sort active/shipped work first, then review, then blocked
          const rank = (s: Session) => {
            const c = s.category ?? 'work'
            return c === 'work' ? 0 : c === 'review' ? 1 : 2
          }
          const sorted = [...res].sort((a, b) => rank(a) - rank(b))
          setSessions(sorted)
          const hasWork = sorted.some(s => (s.category ?? 'work') === 'work')
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
    () => [...new Set(sessions.map(s => s.project).filter((p): p is string => !!p).map(normalizeProjectLabel))],
    [sessions]
  )
  const totalCost = useMemo(
    () => sessions.reduce((sum, s) => sum + (s.cost_usd ?? 0), 0),
    [sessions]
  )

  if (loading && !summary) {
    return (
      <div className="max-w-[1280px] mx-auto px-4 sm:px-6 py-6 space-y-5">
        <div className="qp-hero h-36 animate-pulse" />
        <div className="grid grid-cols-2 lg:grid-cols-4 gap-[10px]">
          {[...Array(4)].map((_, i) => (
            <div key={i} className="qp-tile animate-pulse" />
          ))}
        </div>
        <div className="grid grid-cols-1 xl:grid-cols-2 gap-5">
          <div className="qp-card h-64 animate-pulse" />
          <div className="qp-card h-64 animate-pulse" />
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
  const learningsByProject = summary?.learnings_by_project ?? []
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
  const failedCalls = dayMetrics?.failed_tool_calls ?? 0
  const interruptedCount = dayMetrics?.interrupted ?? 0
  const testRuns = dayMetrics?.test_runs ?? 0

  return (
    <div className="max-w-[1280px] mx-auto px-4 sm:px-6 py-6 space-y-6">
      {/* Breadcrumb */}
      <nav className="flex items-center justify-between text-xs font-mono">
        <div className="flex items-center gap-2">
          <Link to="/" className="text-[#64748B] hover:text-[#F8FAFC] transition-colors">
            {t('dayDetail.breadcrumbArchives')}
          </Link>
          <span className="text-[#475569]">/</span>
          <span className="text-[#FBBF24] font-semibold tabular-nums">{date}</span>
        </div>
        <span className="qp-meta">QUIET PRECISION · DAILY BRIEF</span>
      </nav>

      {error && (
        <div className="qp-surface-coral rounded-2xl p-4 text-xs font-mono text-[#FB7185]">
          {error}
        </div>
      )}

      {/* === Hero Summary Card (border-radius: 22px, padding: 18px 20px) === */}
      <div className="qp-hero space-y-4">
        <div className="flex flex-wrap items-start justify-between gap-3 pb-3.5 border-b border-white/[0.06]">
          <div className="space-y-1">
            <h1 className="qp-hero-title tabular-nums">
              {date} · Daily Brief
            </h1>
            <p className="text-xs font-mono text-[#94A3B8] tabular-nums">
              {counts.work} shipped · {projectGains.length || projects.length}{' '}
              {(projectGains.length || projects.length) === 1 ? 'project' : 'projects'} ·{' '}
              {counts.review} {counts.review === 1 ? 'review' : 'reviews'} · {signalRatioPct}% signal
            </p>
          </div>

          {runtimes.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {runtimes.map(r => (
                <span key={r} className="qp-pill sky font-mono tabular-nums !py-1 !px-2.5 !text-[11px]">
                  <span className="qp-dot" />
                  {r} × {sessions.filter(s => s.runtime === r).length}
                </span>
              ))}
            </div>
          )}
        </div>

        {/* 2-Up / 4-Up Interactive Control Tiles (66px fixed height, "i n" / "i l") */}
        <div className="grid grid-cols-2 lg:grid-cols-4 gap-[10px]">
          <div className={cn('qp-tile', counts.work > 0 ? 'emerald' : '')}>
            <div className="qp-tile-icon">WK</div>
            <div className="qp-tile-name">Shipped Work</div>
            <div className="qp-tile-state font-mono">
              {counts.work} work · {signalRatioPct}% signal
            </div>
          </div>

          <div className={cn('qp-tile', displayCost > 0 ? 'amber' : '')}>
            <div className="qp-tile-icon">$</div>
            <div className="qp-tile-name">Active Spend</div>
            <div className="qp-tile-state font-mono">
              {displayCost > 0 ? formatCost(displayCost) : '$0.00'} · {pricedSessionCount} priced
            </div>
          </div>

          <div className={cn('qp-tile', totalTokens > 0 ? 'sky' : '')}>
            <div className="qp-tile-icon">⚡</div>
            <div className="qp-tile-name">Prompt Cache</div>
            <div className="qp-tile-state font-mono">
              {totalTokens > 0
                ? `${formatTokenCount(totalTokens)}${cacheHitPct != null ? ` · ${cacheHitPct}% hit` : ''}`
                : 'No telemetry'}
            </div>
          </div>

          <div className={cn('qp-tile', failedCalls > 0 || interruptedCount > 0 ? 'coral' : '')}>
            <div className="qp-tile-icon">{failedCalls > 0 || interruptedCount > 0 ? '!' : '✓'}</div>
            <div className="qp-tile-name">Command Health</div>
            <div className="qp-tile-state font-mono">
              {failedCalls} err · {testRuns} tests
              {interruptedCount > 0 ? ` · ${interruptedCount} int` : ''}
            </div>
          </div>
        </div>

        {/* Semantic Status Pills (border-radius: 999px, padding: 6px 12px, 6px glowing status dot) */}
        <div className="flex flex-wrap gap-2 pt-0.5">
          <a
            href="#sessions"
            onClick={() => setCategoryFilter('work')}
            className="qp-pill emerald font-mono tabular-nums hover:brightness-110"
          >
            <span className="qp-dot" />
            <span>{counts.work} Shipped Work</span>
          </a>
          <a
            href="#sessions"
            onClick={() => setCategoryFilter('review')}
            className="qp-pill sky font-mono tabular-nums hover:brightness-110"
          >
            <span className="qp-dot" />
            <span>{counts.review} Code Reviews</span>
          </a>
          {counts.blocked > 0 && (
            <a
              href="#sessions"
              onClick={() => setCategoryFilter('blocked')}
              className="qp-pill coral font-mono tabular-nums hover:brightness-110"
            >
              <span className="qp-dot" />
              <span>{counts.blocked} Blocked</span>
            </a>
          )}
        </div>
      </div>

      {/* === Multi-Column Operational Section Grid (min-width: 340px per column, gap: 20px) === */}
      <div className="grid grid-cols-1 xl:grid-cols-[minmax(340px,1.12fr)_minmax(340px,0.88fr)] gap-5 items-start">
        {/* Primary Operational Column: Shipped Outcomes + Pitfalls & Learnings + Waste Alerts */}
        <div className="space-y-6 min-w-0">
          {projectGains.length > 0 && (
            <Section
              id="gains"
              title="1 · What Shipped & Why"
              meta={`${projectGains.length} ${projectGains.length === 1 ? 'PROJECT' : 'PROJECTS'}`}
            >
              <div className="grid gap-3.5">
                {projectGains.map(gain => (
                  <ProjectGainCard key={gain.project} gain={gain} />
                ))}
              </div>
            </Section>
          )}

          {(learningsByProject.length > 0 || learnings.length > 0) && (
            <Section
              id="learnings"
              title="2 · Pitfalls & Learnings"
              meta="BY PROJECT"
            >
              <ProjectLearningsSection
                groups={learningsByProject}
                fallbackLearnings={learnings}
              />
            </Section>
          )}

          {wasteAlerts.length > 0 && (
            <Section id="alerts" title="Action Needed — Blocked Runs" meta="CORAL ALERT">
              <WasteAlertCard alerts={wasteAlerts} />
            </Section>
          )}
        </div>

        {/* Secondary Operational Column: Coaching + Workflow Rules + Live Session Feed */}
        <div className="space-y-6 min-w-0">
          {(efficiencySignals.length > 0 || nextSteps.length > 0) && (
            <Section
              id="coaching"
              title="3 · Coaching & Next Steps"
              meta={`${nextSteps.length} NEXT`}
            >
              <WorkflowCoachingCard
                signals={efficiencySignals}
                nextSteps={nextSteps}
              />
            </Section>
          )}

          {consolidatedRules.length > 0 ? (
            <Section
              id="extracted"
              title="4 · Reusable Skills & Repo AGENTS.md Rules"
              meta={`${consolidatedRules.length} CANDIDATES`}
            >
              <ConsolidatedRulesCard rules={consolidatedRules} />
            </Section>
          ) : (
            hasExtracted &&
            summary && (
              <Section
                id="extracted"
                title={t('dayDetail.sectionExtracted')}
                meta={`${summary.skills.length + summary.commands.length} ITEMS`}
              >
                <div className="grid gap-2.5">
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
              </Section>
            )
          )}

          {/* Sessions Feed with 3-Up Quick-Action Strip (height: 60px, border-radius: 16px, gap: 8px) */}
          <Section
            id="sessions"
            title={t('dayDetail.sectionSessions')}
            meta={`${filteredSessions.length} / ${sessions.length}`}
          >
            <div className={cn('grid gap-2', counts.blocked > 0 ? 'grid-cols-2 sm:grid-cols-4' : 'grid-cols-3')}>
              <button
                type="button"
                onClick={() => setCategoryFilter('all')}
                className={cn(
                  'qp-quick-btn',
                  categoryFilter === 'all' ? 'amber' : 'text-[#94A3B8] hover:text-[#F8FAFC]'
                )}
              >
                <span className="text-xs font-bold">All Runs</span>
                <span className="font-mono text-[11px] tabular-nums opacity-85">
                  {sessions.length} total
                </span>
              </button>

              <button
                type="button"
                onClick={() => setCategoryFilter(prev => (prev === 'work' ? 'all' : 'work'))}
                className={cn(
                  'qp-quick-btn',
                  categoryFilter === 'work' ? 'emerald' : 'text-[#94A3B8] hover:text-[#F8FAFC]'
                )}
              >
                <span className="text-xs font-bold">Shipped</span>
                <span className="font-mono text-[11px] tabular-nums opacity-85">
                  {counts.work} runs
                </span>
              </button>

              <button
                type="button"
                onClick={() => setCategoryFilter(prev => (prev === 'review' ? 'all' : 'review'))}
                className={cn(
                  'qp-quick-btn',
                  categoryFilter === 'review' ? 'sky' : 'text-[#94A3B8] hover:text-[#F8FAFC]'
                )}
              >
                <span className="text-xs font-bold">Reviews</span>
                <span className="font-mono text-[11px] tabular-nums opacity-85">
                  {counts.review} passes
                </span>
              </button>

              {counts.blocked > 0 && (
                <button
                  type="button"
                  onClick={() => setCategoryFilter(prev => (prev === 'blocked' ? 'all' : 'blocked'))}
                  className={cn(
                    'qp-quick-btn',
                    categoryFilter === 'blocked' ? 'coral' : 'text-[#94A3B8] hover:text-[#F8FAFC]'
                  )}
                >
                  <span className="text-xs font-bold">Blocked</span>
                  <span className="font-mono text-[11px] tabular-nums opacity-85">
                    {counts.blocked} blocked
                  </span>
                </button>
              )}
            </div>

            {runtimes.length > 1 && (
              <div className="flex flex-wrap gap-1.5 pt-1">
                <button
                  type="button"
                  onClick={() => setRuntimeFilter(null)}
                  className={cn(
                    'qp-pill !py-1 !px-2.5 !text-[11px] font-mono',
                    runtimeFilter === null ? 'sky' : 'hover:text-[#F8FAFC]'
                  )}
                >
                  {t('dayDetail.filterAll')}
                </button>
                {runtimes.map(r => (
                  <button
                    type="button"
                    key={r}
                    onClick={() => setRuntimeFilter(r)}
                    className={cn(
                      'qp-pill !py-1 !px-2.5 !text-[11px] font-mono tabular-nums',
                      runtimeFilter === r ? 'sky' : 'hover:text-[#F8FAFC]'
                    )}
                  >
                    {r}
                  </button>
                ))}
              </div>
            )}

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
              <div className="qp-card text-center py-8">
                <p className="text-xs font-mono text-[#64748B]">{t('dayDetail.noSessions')}</p>
              </div>
            )}
          </Section>
        </div>
      </div>
    </div>
  )
}
