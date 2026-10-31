import { useState, useEffect, useRef } from 'react'
import { useNavigate, useLocation } from 'react-router-dom'
import { motion, AnimatePresence } from 'framer-motion'
import { format, parseISO, isToday, isYesterday } from 'date-fns'
import { useApi } from '../hooks/useApi'
import type { DateItem, Session } from '../hooks/useApi'
import { cn } from '../lib/utils'
import { useLanguage } from '../contexts/LanguageContext'

interface DateNodeState {
  expanded: boolean
  sessions: Session[]
  sessionsLoaded: boolean
}

// Clean up raw session slugs into crisp human labels
function formatSessionTitle(raw: string): string {
  const cleaned = raw
    .replace(/^\d{2}[_:]\d{2}[-_]?/, '')
    .replace(/[-_]+/g, ' ')
    .trim()
  if (!cleaned) return raw
  return cleaned.charAt(0).toUpperCase() + cleaned.slice(1)
}

export function ArchiveTree() {
  const [dates, setDates] = useState<DateItem[]>([])
  const [dateStates, setDateStates] = useState<Record<string, DateNodeState>>({})
  const dateRefs = useRef<Record<string, HTMLDivElement | null>>({})
  const { fetchDates, fetchSessions, loading } = useApi()
  const navigate = useNavigate()
  const location = useLocation()
  const { t } = useLanguage()

  useEffect(() => {
    fetchDates()
      .then((data) => {
        setDates(data)
        const today = data.find(d => isToday(parseISO(d.date)))
        const initialDate = today ?? data[0]
        if (initialDate) {
          setDateStates(prev => ({
            ...prev,
            [initialDate.date]: { expanded: true, sessions: [], sessionsLoaded: false }
          }))
        }
      })
      .catch(console.error)
  }, [fetchDates])

  useEffect(() => {
    const match = location.pathname.match(/^\/day\/(\d{4}-\d{2}-\d{2})/)
    if (!match || dates.length === 0) return
    const date = match[1]
    if (!dates.some(d => d.date === date)) return

    setDateStates(prev => {
      if (prev[date]?.expanded) return prev
      const next: Record<string, DateNodeState> = {}
      for (const [key, state] of Object.entries(prev)) {
        next[key] = { ...state, expanded: key === date }
      }
      if (!next[date]) {
        next[date] = { expanded: true, sessions: [], sessionsLoaded: false }
      }
      return next
    })

    requestAnimationFrame(() => {
      dateRefs.current[date]?.scrollIntoView({ behavior: 'smooth', block: 'nearest' })
    })
  }, [location.pathname, dates])

  useEffect(() => {
    const expandedDates = Object.entries(dateStates)
      .filter(([_, state]) => state.expanded && !state.sessionsLoaded)
      .map(([date]) => date)

    expandedDates.forEach(date => {
      fetchSessions(date)
        .then(sessions => {
          // Sort active/shipped work first, then review, then blocked
          const rank = (s: Session) => {
            const c = s.category ?? 'work'
            return c === 'work' ? 0 : c === 'review' ? 1 : 2
          }
          const sorted = [...sessions].sort((a, b) => rank(a) - rank(b))
          setDateStates(prev => ({
            ...prev,
            [date]: { ...prev[date], sessions: sorted, sessionsLoaded: true }
          }))
        })
        .catch(console.error)
    })
  }, [dateStates, fetchSessions])

  const expandOnly = (date: string) => {
    setDateStates(prev => {
      const next: Record<string, DateNodeState> = {}
      for (const [key, state] of Object.entries(prev)) {
        next[key] = { ...state, expanded: key === date }
      }
      if (!next[date]) {
        next[date] = { expanded: true, sessions: [], sessionsLoaded: false }
      }
      return next
    })
  }

  const toggleDate = (date: string) => {
    const wasExpanded = dateStates[date]?.expanded
    if (wasExpanded && location.pathname === `/day/${date}`) {
      setDateStates(prev => ({
        ...prev,
        [date]: { ...prev[date], expanded: false }
      }))
    } else {
      expandOnly(date)
      navigate(`/day/${date}`)
    }
  }

  const getDateLabel = (dateStr: string) => {
    const date = parseISO(dateStr)
    if (isToday(date)) return t('archive.today')
    if (isYesterday(date)) return t('archive.yesterday')
    return format(date, 'EEE')
  }

  const isActive = (path: string) => location.pathname === path

  if (loading && dates.length === 0) {
    return (
      <div className="p-3 space-y-2">
        {[...Array(5)].map((_, i) => (
          <div key={i} className="h-[66px] qp-card animate-pulse" />
        ))}
      </div>
    )
  }

  return (
    <div className="h-full overflow-y-auto p-3 space-y-2">
      {dates.map((dateItem) => {
        const state = dateStates[dateItem.date] || { expanded: false, sessions: [], sessionsLoaded: false }
        const isCurrentDate = location.pathname.startsWith(`/day/${dateItem.date}`)
        const dayShort = dateItem.date.slice(5) // MM-DD

        return (
          <div key={dateItem.date} ref={el => { dateRefs.current[dateItem.date] = el }}>
            {/* Date Control Tile (66px fixed height, "i n" / "i l") */}
            <button
              type="button"
              onClick={() => toggleDate(dateItem.date)}
              className={cn(
                'w-full text-left qp-tile',
                isCurrentDate ? 'amber' : 'hover:border-white/15'
              )}
            >
              <div className="qp-tile-icon">
                {dayShort}
              </div>
              <div className="qp-tile-name flex items-center justify-between gap-2">
                <span className="font-mono">{dateItem.date}</span>
                <span className="text-[11px] font-mono font-semibold text-[#64748B] shrink-0">
                  {getDateLabel(dateItem.date)}
                </span>
              </div>
              <div className="qp-tile-state">
                {dateItem.session_count} {dateItem.session_count === 1 ? t('archive.session') : t('archive.sessions')}
                {dateItem.session_titles.length > 0 ? ` · ${formatSessionTitle(dateItem.session_titles[0])}` : ''}
              </div>
            </button>

            <AnimatePresence>
              {state.expanded && (
                <motion.div
                  initial={{ height: 0, opacity: 0 }}
                  animate={{ height: 'auto', opacity: 1 }}
                  exit={{ height: 0, opacity: 0 }}
                  transition={{ duration: 0.18 }}
                  className="overflow-hidden"
                >
                  <div className="mt-1.5 pl-3 border-l border-white/[0.07] space-y-1">
                    <button
                      type="button"
                      onClick={() => navigate(`/day/${dateItem.date}`)}
                      className={cn(
                        'w-full flex items-center justify-between gap-2 px-3 py-2 rounded-xl text-left text-xs font-bold transition-all',
                        isActive(`/day/${dateItem.date}`)
                          ? 'qp-surface-emerald text-[#34D399]'
                          : 'text-[#94A3B8] hover:text-[#F8FAFC] hover:bg-white/[0.04]'
                      )}
                    >
                      <span className="flex items-center gap-2 truncate">
                        <span className="qp-dot" />
                        <span>{t('archive.dailySummary')}</span>
                      </span>
                      <span className="font-mono text-[10px] opacity-75">BRIEF</span>
                    </button>

                    {state.sessionsLoaded ? (
                      state.sessions.length > 0 && (
                        <div className="pt-1 space-y-1">
                          <div className="px-2 py-0.5 flex items-center justify-between">
                            <span className="qp-kicker">{t('archive.sessionsLabel')}</span>
                            <span className="qp-meta">{state.sessions.length}</span>
                          </div>
                          {state.sessions.map((session) => {
                            const sessionPath = `/day/${dateItem.date}/session/${encodeURIComponent(session.name)}`
                            const active = isActive(sessionPath)
                            const cat = session.category ?? 'work'
                            const dotColor =
                              cat === 'work'
                                ? 'text-[#34D399]'
                                : cat === 'review'
                                  ? 'text-[#38BDF8]'
                                  : 'text-[#FB7185]'
                            return (
                              <button
                                type="button"
                                key={session.name}
                                onClick={() => navigate(sessionPath)}
                                className={cn(
                                  'w-full flex items-center justify-between gap-2 px-2.5 py-1.5 rounded-lg text-left text-xs transition-all',
                                  active
                                    ? 'qp-surface-sky text-[#38BDF8] font-bold'
                                    : 'text-[#94A3B8] hover:text-[#F8FAFC] hover:bg-white/[0.04]'
                                )}
                                title={session.title || session.name}
                              >
                                <span className="flex items-center gap-2 min-w-0">
                                  <span className={cn('text-[10px] shrink-0', dotColor)}>●</span>
                                  <span className="truncate">{formatSessionTitle(session.title || session.name)}</span>
                                </span>
                                {session.project && (
                                  <span className="font-mono text-[10px] text-[#64748B] shrink-0">
                                    {session.project}
                                  </span>
                                )}
                              </button>
                            )
                          })}
                        </div>
                      )
                    ) : (
                      <div className="px-3 py-2 text-xs font-mono text-[#64748B]">{t('archive.loadingSessions')}</div>
                    )}
                  </div>
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        )
      })}
    </div>
  )
}
