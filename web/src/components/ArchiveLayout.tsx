import { useEffect } from 'react'
import { Outlet, useLocation, useOutletContext } from 'react-router-dom'
import { ArchiveTree } from './ArchiveTree'
import { useLanguage } from '../contexts/LanguageContext'
import { useIsLg } from '../hooks/useIsLg'
import { cn } from '../lib/utils'

export interface LayoutOutletContext {
  drawerOpen: boolean
  setDrawerOpen: (open: boolean) => void
}

export function ArchiveLayout() {
  const { t } = useLanguage()
  const { drawerOpen, setDrawerOpen } = useOutletContext<LayoutOutletContext>()
  const isLg = useIsLg()
  const location = useLocation()

  // Close the mobile drawer after any navigation
  useEffect(() => {
    setDrawerOpen(false)
  }, [location.pathname, setDrawerOpen])

  const timeZoneLabel = (Intl.DateTimeFormat().resolvedOptions().timeZone || 'LOCAL').toUpperCase()

  const sidebarContent = (
    <>
      <div className="px-4 py-3.5 border-b border-white/[0.06] flex items-center justify-between">
        <h2 className="qp-kicker">{t('archive.title')}</h2>
        <span className="qp-meta">{timeZoneLabel}</span>
      </div>
      <div className="flex-1 overflow-hidden">
        <ArchiveTree />
      </div>
    </>
  )

  return (
    <div className="flex lg:h-[calc(100dvh-3.5rem)] lg:overflow-hidden">
      {isLg ? (
        <aside className="w-72 xl:w-80 shrink-0 border-r border-white/[0.06] bg-[#0B0E14] flex flex-col">
          {sidebarContent}
        </aside>
      ) : (
        <>
          {drawerOpen && (
            <div
              className="fixed top-14 inset-x-0 bottom-0 z-40 bg-black/70 backdrop-blur-sm"
              onClick={() => setDrawerOpen(false)}
            />
          )}
          <aside
            className={cn(
              'fixed top-14 bottom-0 left-0 z-40 w-80 max-w-[86vw] border-r border-white/[0.08] bg-[#0B0E14] flex flex-col pb-24 transition-transform duration-200',
              drawerOpen ? 'translate-x-0' : '-translate-x-full'
            )}
          >
            {sidebarContent}
          </aside>
        </>
      )}

      {/* Right content area */}
      <div className="flex-1 lg:overflow-y-auto qp-canvas min-w-0">
        <Outlet />
      </div>
    </div>
  )
}
