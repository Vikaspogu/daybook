import { useLanguage } from '../contexts/LanguageContext'
import { cn } from '../lib/utils'

export function LanguageToggle({ className }: { className?: string }) {
  const { language, setLanguage } = useLanguage()

  return (
    <button
      onClick={() => setLanguage(language === 'en' ? 'zh' : 'en')}
      className={cn(
        'px-2.5 py-1 rounded-lg border border-white/[0.08] bg-[#131822]/90 transition-colors',
        'text-[#94A3B8] hover:text-[#F8FAFC] hover:border-white/20',
        className
      )}
      title={language === 'en' ? '切换到中文' : 'Switch to English'}
    >
      <span className="font-mono text-[11px] font-semibold leading-none">
        {language === 'en' ? 'EN' : '中'}
      </span>
    </button>
  )
}
