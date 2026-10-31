import { useEffect, useState } from 'react'

const QUERY = '(min-width: 1024px)'

export function useIsLg() {
  const [isLg, setIsLg] = useState(() => window.matchMedia(QUERY).matches)

  useEffect(() => {
    const mql = window.matchMedia(QUERY)
    const onChange = (e: MediaQueryListEvent) => setIsLg(e.matches)
    mql.addEventListener('change', onChange)
    return () => mql.removeEventListener('change', onChange)
  }, [])

  return isLg
}
