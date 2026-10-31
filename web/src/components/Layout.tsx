import { useState } from 'react'
import { NavLink, Outlet, useLocation } from 'react-router-dom'
import { cn } from '../lib/utils'
import { ThemeToggle } from './ThemeToggle'
import { LanguageToggle } from './LanguageToggle'
import { useLanguage } from '../contexts/LanguageContext'

export function Layout() {
  const location = useLocation()
  const [drawerOpen, setDrawerOpen] = useState(false)
  const isArchiveRoute = location.pathname === '/' || location.pathname.startsWith('/day')
  const { t } = useLanguage()

  return (
    <div className="min-h-dvh bg-white dark:bg-daily-dark text-gray-900 dark:text-gray-100 transition-colors">
      {/* Header */}
      <header className="fixed top-0 left-0 right-0 z-50 bg-white/90 dark:bg-daily-dark/90 backdrop-blur-md border-b border-gray-200 dark:border-orange-500/20 transition-colors">
        <div className="px-6 py-3 flex items-center gap-6">
          {isArchiveRoute && (
            <button
              type="button"
              onClick={() => setDrawerOpen(open => !open)}
              className="lg:hidden -ml-2 p-2 rounded-lg text-gray-500 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-daily-light transition-colors"
              aria-label={t('nav.menu')}
            >
              <svg className="size-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
              </svg>
            </button>
          )}
          <NavLink to="/" className="text-xl font-bold text-orange-500 dark:text-orange-400 hover:text-orange-600 dark:hover:text-orange-300 transition-colors">
            Daybook
          </NavLink>
          <nav className="flex gap-4 flex-1">
            <NavLink
              to="/"
              end
              className={({ isActive }) =>
                cn(
                  'text-sm transition-colors',
                  isActive
                    ? 'text-orange-500 dark:text-orange-400'
                    : 'text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200'
                )
              }
            >
              {t('nav.archives')}
            </NavLink>
            <NavLink
              to="/insights"
              className={({ isActive }) =>
                cn(
                  'text-sm transition-colors',
                  isActive
                    ? 'text-orange-500 dark:text-orange-400'
                    : 'text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200'
                )
              }
            >
              {t('nav.insights')}
            </NavLink>
            <NavLink
              to="/settings"
              className={({ isActive }) =>
                cn(
                  'text-sm transition-colors',
                  isActive
                    ? 'text-orange-500 dark:text-orange-400'
                    : 'text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200'
                )
              }
            >
              {t('nav.settings')}
            </NavLink>
          </nav>
          <div className="flex items-center gap-2">
            <LanguageToggle />
            <ThemeToggle />
          </div>
        </div>
      </header>

      {/* Main Content */}
      <main className="pt-16 min-h-dvh">
        <Outlet context={{ drawerOpen, setDrawerOpen }} />
      </main>
    </div>
  )
}
