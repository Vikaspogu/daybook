import { useState, useEffect, useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { motion, AnimatePresence } from 'framer-motion'
import {
  format,
  isToday,
  isSameMonth,
  startOfMonth,
  endOfMonth,
  startOfWeek,
  endOfWeek,
  eachDayOfInterval,
  addMonths,
  subMonths,
} from 'date-fns'
import { useApi } from '../hooks/useApi'
import type { DateItem } from '../hooks/useApi'
import { cn } from '../lib/utils'
import { formatCost } from '../components/UsageCharts'
import { useLanguage } from '../contexts/LanguageContext'
import { OverviewStrip } from '../components/OverviewStrip'

function cleanTitle(raw: string): string {
  const cleaned = raw
    .replace(/^\d{2}[_:]\d{2}[-_]?/, '')
    .replace(/[-_]+/g, ' ')
    .trim()
  if (!cleaned) return raw
  return cleaned.charAt(0).toUpperCase() + cleaned.slice(1)
}

export function Welcome() {
  const [days, setDays] = useState<DateItem[]>([])
  const [dailyCostMap, setDailyCostMap] = useState<Map<string, number>>(new Map())
  const [loading, setLoading] = useState(true)
  const [currentMonth, setCurrentMonth] = useState(new Date())
  const [slideDirection, setSlideDirection] = useState<'left' | 'right'>('left')
  const { fetchDates, fetchInsights } = useApi()
  const navigate = useNavigate()
  const { t } = useLanguage()

  const WEEKDAYS = [
    t('weekdays.mon'), t('weekdays.tue'), t('weekdays.wed'), t('weekdays.thu'),
    t('weekdays.fri'), t('weekdays.sat'), t('weekdays.sun'),
  ]

  useEffect(() => {
    const loadData = async () => {
      try {
        const dates = await fetchDates()
        setDays(dates)

        try {
          const insights = await fetchInsights(365)
          if (insights.usage_summary?.daily_usage) {
            const costMap = new Map<string, number>()
            for (const du of insights.usage_summary.daily_usage) {
              costMap.set(du.date, du.total_cost_usd)
            }
            setDailyCostMap(costMap)
          }
        } catch {
          // Usage data is optional
        }
      } catch (err) {
        console.error('Failed to load data:', err)
      } finally {
        setLoading(false)
      }
    }

    loadData()
  }, [fetchDates, fetchInsights])

  const archiveMap = useMemo(() => {
    const map = new Map<string, DateItem>()
    for (const day of days) {
      map.set(day.date, day)
    }
    return map
  }, [days])

  const calendarDays = useMemo(() => {
    const monthStart = startOfMonth(currentMonth)
    const monthEnd = endOfMonth(currentMonth)
    const calStart = startOfWeek(monthStart, { weekStartsOn: 1 })
    const calEnd = endOfWeek(monthEnd, { weekStartsOn: 1 })
    return eachDayOfInterval({ start: calStart, end: calEnd })
  }, [currentMonth])

  const monthKey = useMemo(() => format(currentMonth, 'yyyy-MM'), [currentMonth])

  const monthlyCost = useMemo(() => {
    let total = 0
    for (const [date, cost] of dailyCostMap) {
      if (date.startsWith(monthKey)) {
        total += cost
      }
    }
    return total
  }, [dailyCostMap, monthKey])

  const goToPrevMonth = () => {
    setSlideDirection('right')
    setCurrentMonth(prev => subMonths(prev, 1))
  }

  const goToNextMonth = () => {
    setSlideDirection('left')
    setCurrentMonth(prev => addMonths(prev, 1))
  }

  const goToToday = () => {
    const today = new Date()
    if (currentMonth > today) {
      setSlideDirection('right')
    } else {
      setSlideDirection('left')
    }
    setCurrentMonth(today)
  }

  const slideVariants = {
    enter: (direction: 'left' | 'right') => ({
      x: direction === 'left' ? 60 : -60,
      opacity: 0,
    }),
    center: {
      x: 0,
      opacity: 1,
    },
    exit: (direction: 'left' | 'right') => ({
      x: direction === 'left' ? -60 : 60,
      opacity: 0,
    }),
  }

  if (loading) {
    return (
      <div className="max-w-[1160px] mx-auto px-4 sm:px-6 py-6 space-y-5">
        <div className="qp-hero h-28 animate-pulse" />
        <div className="grid grid-cols-2 lg:grid-cols-4 gap-[10px]">
          {[...Array(4)].map((_, i) => (
            <div key={i} className="qp-tile animate-pulse" />
          ))}
        </div>
        <div className="grid grid-cols-7 gap-2">
          {[...Array(35)].map((_, i) => (
            <div key={i} className="h-24 qp-card animate-pulse" />
          ))}
        </div>
      </div>
    )
  }

  if (days.length === 0) {
    return (
      <div className="max-w-3xl mx-auto px-4 sm:px-6 py-10">
        <div className="qp-hero text-center py-14 space-y-4">
          <h1 className="qp-hero-title">
            {t('welcome.emptyTitle')} <span className="text-[#FBBF24]">{t('welcome.emptyTitleHighlight')}</span>
          </h1>
          <p className="text-[#94A3B8] text-sm max-w-md mx-auto">
            {t('welcome.emptySubtitle')}
          </p>
          <div className="pt-4 border-t border-white/[0.06]">
            <p className="font-mono text-xs text-[#64748B]">
              {t('welcome.emptyHint')}
            </p>
          </div>
        </div>
      </div>
    )
  }

  const latestDay = days[0]

  return (
    <div className="max-w-[1160px] mx-auto px-4 sm:px-6 py-6 space-y-6">
      {/* Hero Summary Card */}
      <div className="qp-hero flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div className="space-y-1.5">
          <div className="qp-kicker">QUIET PRECISION · ARCHIVE LEDGER</div>
          <h1 className="qp-hero-title">
            <span className="text-[#FBBF24]">{t('welcome.title')}</span> {t('welcome.titleSuffix')}
          </h1>
          <p className="text-xs font-mono text-[#94A3B8] tabular-nums">
            {t('welcome.subtitle', {
              count: days.length,
              dayWord: days.length === 1 ? t('welcome.day') : t('welcome.days'),
            })}
            {monthlyCost > 0 ? ` · ${formatCost(monthlyCost)} month spend` : ''}
          </p>
        </div>

        {latestDay && (
          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={() => navigate(`/day/${latestDay.date}`)}
              className="qp-pill emerald hover:brightness-110"
            >
              <span className="qp-dot" />
              <span className="font-mono">{latestDay.date}</span>
              <span>Latest Brief →</span>
            </button>
          </div>
        )}
      </div>

      {/* 2-Up / 4-Up Interactive Control Tiles */}
      <OverviewStrip days={days} />

      {/* Month Section Header */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={goToPrevMonth}
            className="size-8 rounded-xl qp-card flex items-center justify-center text-[#94A3B8] hover:text-[#F8FAFC] hover:border-white/15 transition-colors"
            aria-label="Previous month"
          >
            <svg width="16" height="16" viewBox="0 0 20 20" fill="none">
              <path d="M12.5 15L7.5 10L12.5 5" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
          <h2 className="text-base font-extrabold tracking-tight text-[#F8FAFC] min-w-[156px] text-center font-mono">
            {format(currentMonth, 'MMMM yyyy')}
          </h2>
          <button
            type="button"
            onClick={goToNextMonth}
            className="size-8 rounded-xl qp-card flex items-center justify-center text-[#94A3B8] hover:text-[#F8FAFC] hover:border-white/15 transition-colors"
            aria-label="Next month"
          >
            <svg width="16" height="16" viewBox="0 0 20 20" fill="none">
              <path d="M7.5 15L12.5 10L7.5 5" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
        </div>

        <div className="flex items-center gap-2">
          {monthlyCost > 0 && (
            <span className="qp-pill amber font-mono tabular-nums">
              <span className="qp-dot" />
              {formatCost(monthlyCost)}
            </span>
          )}
          <button
            type="button"
            onClick={goToToday}
            className="px-3 py-1.5 text-xs font-bold rounded-xl qp-card text-[#94A3B8] hover:text-[#F8FAFC] hover:border-white/20 transition-colors"
          >
            {t('welcome.today')}
          </button>
        </div>
      </div>

      {/* Weekday headers */}
      <div className="grid grid-cols-7 gap-2">
        {WEEKDAYS.map(d => (
          <div key={d} className="text-center qp-kicker py-1">
            {d}
          </div>
        ))}
      </div>

      {/* Calendar grid */}
      <AnimatePresence mode="wait" custom={slideDirection}>
        <motion.div
          key={monthKey}
          custom={slideDirection}
          variants={slideVariants}
          initial="enter"
          animate="center"
          exit="exit"
          transition={{ duration: 0.18, ease: 'easeInOut' }}
          className="grid grid-cols-7 gap-2"
        >
          {calendarDays.map(day => {
            const dateStr = format(day, 'yyyy-MM-dd')
            const archive = archiveMap.get(dateStr)
            const dayCost = dailyCostMap.get(dateStr)
            const isCurrentMonth = isSameMonth(day, currentMonth)
            const today = isToday(day)
            const hasArchive = !!archive

            return (
              <button
                type="button"
                key={dateStr}
                onClick={() => hasArchive && navigate(`/day/${dateStr}`)}
                disabled={!hasArchive}
                className={cn(
                  'h-24 rounded-[16px] p-2.5 flex flex-col items-start text-left transition-all duration-150 relative overflow-hidden',
                  !isCurrentMonth && 'opacity-30',
                  hasArchive
                    ? archive.has_digest
                      ? 'qp-surface-emerald cursor-pointer hover:brightness-110'
                      : 'qp-surface-sky cursor-pointer hover:brightness-110'
                    : 'qp-card opacity-55 cursor-default'
                )}
              >
                <div className="flex items-center justify-between w-full mb-1">
                  <span
                    className={cn(
                      'font-mono text-xs font-bold leading-none tabular-nums',
                      today && 'px-1.5 py-0.5 rounded-md bg-[#FBBF24] text-[#0B0E14]',
                      !today && hasArchive && 'text-[#F8FAFC]',
                      !today && !hasArchive && 'text-[#64748B]'
                    )}
                  >
                    {format(day, 'd')}
                  </span>

                  {hasArchive && (
                    <span
                      className={cn(
                        'inline-flex items-center gap-1 font-mono text-[10px] font-semibold tabular-nums',
                        archive.has_digest ? 'text-[#34D399]' : 'text-[#38BDF8]'
                      )}
                    >
                      <span className="qp-dot" />
                      {archive.session_count}s
                    </span>
                  )}
                </div>

                {dayCost != null && dayCost > 0 && (
                  <span className="text-[11px] font-mono font-semibold text-[#FBBF24] tabular-nums leading-none mb-1">
                    {formatCost(dayCost)}
                  </span>
                )}

                {(archive?.session_titles?.length || archive?.preview) && (
                  <p className="hidden sm:block text-[10px] leading-snug text-[#94A3B8] line-clamp-2 w-full">
                    {archive.session_titles?.length
                      ? archive.session_titles.slice(0, 2).map(cleanTitle).join(' · ')
                      : archive.preview}
                  </p>
                )}
              </button>
            )
          })}
        </motion.div>
      </AnimatePresence>
    </div>
  )
}
