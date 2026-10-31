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

  const sidebarContent = (
    <>
      <div className="p-4 border-b border-gray-200 dark:border-gray-800">
        <h2 className="text-lg font-semibold text-orange-500 dark:text-orange-400">{t('archive.title')}</h2>
      </div>
      <div className="flex-1 overflow-hidden">
        <ArchiveTree />
      </div>
    </>
  )

  return (
    <div className="flex h-[calc(100vh-4rem)] overflow-hidden">
      {isLg ? (
        <aside className="w-80 shrink-0 border-r border-gray-200 dark:border-gray-800 bg-gray-50 dark:bg-black flex flex-col transition-colors">
          {sidebarContent}
        </aside>
      ) : (
        <>
          {drawerOpen && (
            <div
              className="fixed top-16 inset-x-0 bottom-0 z-40 bg-black/50"
              onClick={() => setDrawerOpen(false)}
            />
          )}
          <aside
            className={cn(
              'fixed top-16 bottom-0 left-0 z-50 w-80 max-w-[85vw] border-r border-gray-200 dark:border-gray-800 bg-gray-50 dark:bg-black flex flex-col transition-transform duration-200',
              drawerOpen ? 'translate-x-0' : '-translate-x-full'
            )}
          >
            {sidebarContent}
          </aside>
        </>
      )}

      {/* Right content area */}
      <main className="flex-1 overflow-y-auto bg-white dark:bg-daily-dark transition-colors">
        <Outlet />
      </main>
    </div>
  )
}
