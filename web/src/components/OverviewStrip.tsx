import { format, parseISO, subDays } from 'date-fns'
import type { DateItem } from '../hooks/useApi'
import { useLanguage } from '../contexts/LanguageContext'

export function OverviewStrip({ days }: { days: DateItem[] }) {
  const { t } = useLanguage()

  const totalDays = days.length
  const totalSessions = days.reduce((sum, d) => sum + d.session_count, 0)
  const avgPerDay = totalDays > 0 ? totalSessions / totalDays : 0

  // Consecutive-day streak ending at the most recent recorded date
  let streak = 0
  if (totalDays > 0) {
    const sorted = [...days].sort((a, b) => b.date.localeCompare(a.date))
    let cursor = sorted[0].date
    for (const d of sorted) {
      if (d.date < cursor) break
      streak++
      cursor = format(subDays(parseISO(cursor), 1), 'yyyy-MM-dd')
    }
  }

  const cards = [
    { value: String(totalDays), label: t('welcome.kpiDays') },
    { value: String(totalSessions), label: t('welcome.kpiSessions') },
    { value: avgPerDay.toFixed(1), label: t('welcome.kpiAvgPerDay') },
    { value: String(streak), label: t('welcome.kpiStreak') },
  ]

  return (
    <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-8">
      {cards.map(c => (
        <div
          key={c.label}
          className="rounded-lg border border-gray-200 dark:border-gray-800 bg-gray-50 dark:bg-daily-light p-3"
        >
          <div className="text-2xl font-bold tabular-nums">{c.value}</div>
          <div className="text-xs text-gray-500 dark:text-gray-400">{c.label}</div>
        </div>
      ))}
    </div>
  )
}
