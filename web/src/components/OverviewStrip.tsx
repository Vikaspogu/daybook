import { format, parseISO, subDays } from 'date-fns'
import type { DateItem } from '../hooks/useApi'
import { useLanguage } from '../contexts/LanguageContext'
import { cn } from '../lib/utils'

export function OverviewStrip({ days }: { days: DateItem[] }) {
  const { t } = useLanguage()

  const totalDays = days.length
  const totalSessions = days.reduce((sum, d) => sum + d.session_count, 0)
  const avgPerDay = totalDays > 0 ? totalSessions / totalDays : 0

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

  const tiles = [
    {
      icon: 'DY',
      name: t('welcome.kpiDays'),
      state: `${totalDays} archived`,
      tone: totalDays > 0 ? 'emerald' : '',
    },
    {
      icon: 'SN',
      name: t('welcome.kpiSessions'),
      state: `${totalSessions} logged`,
      tone: totalSessions > 0 ? 'sky' : '',
    },
    {
      icon: 'AVG',
      name: t('welcome.kpiAvgPerDay'),
      state: `${avgPerDay.toFixed(1)} / day`,
      tone: avgPerDay > 0 ? 'amber' : '',
    },
    {
      icon: '🔥',
      name: t('welcome.kpiStreak'),
      state: `${streak}d active`,
      tone: streak > 1 ? 'emerald' : '',
    },
  ]

  return (
    <div className="grid grid-cols-2 lg:grid-cols-4 gap-[10px] mb-6">
      {tiles.map(tile => (
        <div key={tile.name} className={cn('qp-tile', tile.tone)}>
          <div className="qp-tile-icon">{tile.icon}</div>
          <div className="qp-tile-name">{tile.name}</div>
          <div className="qp-tile-state font-mono">{tile.state}</div>
        </div>
      ))}
    </div>
  )
}
