import { useState } from 'react'
import { NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom'
import { cn } from '../lib/utils'
import { LanguageToggle } from './LanguageToggle'
import { useLanguage } from '../contexts/LanguageContext'

export function Layout() {
  const location = useLocation()
  const navigate = useNavigate()
  const [drawerOpen, setDrawerOpen] = useState(false)
  const isArchiveRoute = location.pathname === '/' || location.pathname.startsWith('/day')
  const { t } = useLanguage()

  const handleMobileDatesClick = () => {
    if (!isArchiveRoute) {
      navigate('/')
      setTimeout(() => setDrawerOpen(true), 60)
    } else {
      setDrawerOpen(open => !open)
    }
  }

  return (
    <div className="min-h-dvh qp-canvas transition-colors">
      {/* Top Operational Bar */}
      <header className="fixed top-0 left-0 right-0 z-50 bg-[#0B0E14]/90 backdrop-blur-xl border-b border-white/[0.06]">
        <div className="max-w-[1440px] mx-auto px-4 sm:px-6 h-14 flex items-center justify-between gap-4">
          <div className="flex items-center gap-3 min-w-0">
            {isArchiveRoute && (
              <button
                type="button"
                onClick={() => setDrawerOpen(open => !open)}
                className="lg:hidden -ml-1 p-2 rounded-xl text-slate-400 hover:text-slate-100 hover:bg-white/[0.05] transition-colors"
                aria-label={t('nav.menu')}
              >
                <svg className="size-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
                </svg>
              </button>
            )}
            <NavLink
              to="/"
              className="flex items-center gap-2.5 group min-w-0"
            >
              <img
                src="/daybook-icon.svg"
                alt=""
                className="size-8 rounded-[10px] border border-amber-400/30 shadow-sm shrink-0"
              />
              <div className="leading-none min-w-0">
                <div className="text-[16px] font-extrabold tracking-[-0.025em] text-[#F8FAFC] group-hover:text-amber-300 transition-colors">
                  Daybook
                </div>
                <div className="hidden sm:block text-[10px] font-bold uppercase tracking-[0.10em] text-[#64748B] mt-0.5">
                  Quiet Precision · Ledger
                </div>
              </div>
            </NavLink>
          </div>

          {/* Desktop Navigation Pills */}
          <nav className="hidden lg:flex items-center gap-1.5 p-1 rounded-xl bg-[#131822]/90 border border-white/[0.06]">
            <NavLink
              to="/"
              end
              className={({ isActive }) =>
                cn(
                  'px-3.5 py-1.5 rounded-lg text-xs font-bold transition-all',
                  isActive || location.pathname.startsWith('/day')
                    ? 'qp-surface-amber text-[#FBBF24]'
                    : 'text-[#94A3B8] hover:text-[#F8FAFC]'
                )
              }
            >
              {t('nav.archives')}
            </NavLink>
            <NavLink
              to="/insights"
              className={({ isActive }) =>
                cn(
                  'px-3.5 py-1.5 rounded-lg text-xs font-bold transition-all',
                  isActive
                    ? 'qp-surface-amber text-[#FBBF24]'
                    : 'text-[#94A3B8] hover:text-[#F8FAFC]'
                )
              }
            >
              {t('nav.insights')}
            </NavLink>
            <NavLink
              to="/settings"
              className={({ isActive }) =>
                cn(
                  'px-3.5 py-1.5 rounded-lg text-xs font-bold transition-all',
                  isActive
                    ? 'qp-surface-amber text-[#FBBF24]'
                    : 'text-[#94A3B8] hover:text-[#F8FAFC]'
                )
              }
            >
              {t('nav.settings')}
            </NavLink>
          </nav>

          <div className="flex items-center gap-2">
            <span className="hidden sm:inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-[11px] font-mono font-semibold text-[#34D399] bg-emerald-500/10 border border-emerald-400/25 tabular-nums">
              <span className="qp-dot" />
              LIVE
            </span>
            <LanguageToggle />
          </div>
        </div>
      </header>

      {/* Main Content (reserves 110px bottom padding on mobile so Floating Glass Dock never overlaps) */}
      <main className="pt-14 pb-[110px] lg:pb-0 min-h-dvh">
        <Outlet context={{ drawerOpen, setDrawerOpen }} />
      </main>

      {/* Floating Bottom Glass Dock (Mobile Thumb Zone) */}
      <nav
        aria-label="Mobile navigation"
        className="lg:hidden fixed left-3.5 right-3.5 bottom-3.5 z-50 h-16 qp-dock grid grid-cols-4 p-1.5 gap-1"
      >
        <NavLink
          to="/"
          end
          onClick={() => setDrawerOpen(false)}
          className={({ isActive }) =>
            cn(
              'rounded-[16px] flex flex-col items-center justify-center gap-0.5 text-[11px] font-bold transition-all',
              (isActive || location.pathname.startsWith('/day')) && !drawerOpen
                ? 'qp-surface-amber text-[#FBBF24]'
                : 'text-[#64748B] hover:text-[#94A3B8]'
            )
          }
        >
          <span className="font-mono text-xs leading-none">◈</span>
          <span>Brief</span>
        </NavLink>

        <button
          type="button"
          onClick={handleMobileDatesClick}
          className={cn(
            'rounded-[16px] flex flex-col items-center justify-center gap-0.5 text-[11px] font-bold transition-all',
            drawerOpen
              ? 'qp-surface-sky text-[#38BDF8]'
              : 'text-[#64748B] hover:text-[#94A3B8]'
          )}
        >
          <span className="font-mono text-xs leading-none">▤</span>
          <span>Dates</span>
        </button>

        <NavLink
          to="/insights"
          onClick={() => setDrawerOpen(false)}
          className={({ isActive }) =>
            cn(
              'rounded-[16px] flex flex-col items-center justify-center gap-0.5 text-[11px] font-bold transition-all',
              isActive
                ? 'qp-surface-amber text-[#FBBF24]'
                : 'text-[#64748B] hover:text-[#94A3B8]'
            )
          }
        >
          <span className="font-mono text-xs leading-none">◫</span>
          <span>{t('nav.insights')}</span>
        </NavLink>

        <NavLink
          to="/settings"
          onClick={() => setDrawerOpen(false)}
          className={({ isActive }) =>
            cn(
              'rounded-[16px] flex flex-col items-center justify-center gap-0.5 text-[11px] font-bold transition-all',
              isActive
                ? 'qp-surface-amber text-[#FBBF24]'
                : 'text-[#64748B] hover:text-[#94A3B8]'
            )
          }
        >
          <span className="font-mono text-xs leading-none">⚙</span>
          <span>{t('nav.settings')}</span>
        </NavLink>
      </nav>
    </div>
  )
}
