import { useState, useEffect, useRef, memo, useMemo } from 'react'
import { motion } from 'framer-motion'
import { useApi } from '../hooks/useApi'
import type { ConversationMessage, ConversationContentBlock } from '../hooks/useApi'
import { MarkdownRenderer } from './MarkdownRenderer'
import { cn } from '../lib/utils'
import { useLanguage } from '../contexts/LanguageContext'

interface ChatViewProps {
  date: string
  name: string
}

interface PairedToolCall {
  id: string
  name: string
  input: unknown
  result?: string
}

type GroupedBlock =
  | { kind: 'text'; text: string }
  | { kind: 'tools'; calls: PairedToolCall[] }

// Group consecutive tool_use + tool_result blocks so multi-tool turns render
// as one collapsible "Ran N tools" bar instead of a wall of JSON.
function groupMessageBlocks(blocks: ConversationContentBlock[]): GroupedBlock[] {
  const out: GroupedBlock[] = []
  let currentTools: PairedToolCall[] = []

  const flushTools = () => {
    if (currentTools.length > 0) {
      out.push({ kind: 'tools', calls: currentTools })
      currentTools = []
    }
  }

  for (const block of blocks) {
    if (block.type === 'text') {
      if (block.text.trim()) {
        flushTools()
        out.push({ kind: 'text', text: block.text })
      }
    } else if (block.type === 'tool_use') {
      currentTools.push({
        id: block.tool_use_id,
        name: block.name,
        input: block.input,
      })
    } else if (block.type === 'tool_result') {
      const match = block.tool_use_id
        ? currentTools.find(t => t.id === block.tool_use_id && t.result === undefined)
        : [...currentTools].reverse().find(t => !t.id && t.result === undefined)
      if (match) {
        match.result = block.content
      } else if (block.content) {
        currentTools.push({
          id: block.tool_use_id,
          name: 'result',
          input: '',
          result: block.content,
        })
      }
    }
  }
  flushTools()
  return out
}

export function ChatView({ date, name }: ChatViewProps) {
  const [messages, setMessages] = useState<ConversationMessage[]>([])
  const [page, setPage] = useState(0)
  const [hasMore, setHasMore] = useState(false)
  const [hasTranscript, setHasTranscript] = useState(true)
  const [totalEntries, setTotalEntries] = useState(0)
  const [initialLoaded, setInitialLoaded] = useState(false)
  const { fetchConversation, loading } = useApi()
  const scrollRef = useRef<HTMLDivElement>(null)
  const { t } = useLanguage()

  useEffect(() => {
    setMessages([])
    setPage(0)
    setInitialLoaded(false)
    loadPage(0)
  }, [date, name])

  const loadPage = async (p: number) => {
    try {
      const data = await fetchConversation(date, name, p)
      if (p === 0) {
        setMessages(data.messages)
      } else {
        setMessages(prev => [...prev, ...data.messages])
      }
      setPage(data.page)
      setHasMore(data.has_more)
      setHasTranscript(data.has_transcript)
      setTotalEntries(data.total_entries)
      setInitialLoaded(true)
    } catch (err) {
      console.error('Failed to load conversation:', err)
      setInitialLoaded(true)
    }
  }

  if (loading && !initialLoaded) {
    return (
      <div className="flex flex-col gap-4 p-4">
        {[1, 2, 3].map(i => (
          <div key={i} className={cn('flex', i % 2 === 0 ? 'justify-end' : 'justify-start')}>
            <div className="animate-pulse h-16 w-2/3 bg-gray-200 dark:bg-daily-dark rounded-lg" />
          </div>
        ))}
      </div>
    )
  }

  if (!hasTranscript && initialLoaded) {
    return (
      <div className="flex items-center justify-center py-16 text-gray-500 dark:text-gray-400">
        <div className="text-center">
          <svg className="size-12 mx-auto mb-3 opacity-40" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
          </svg>
          <p className="text-sm">{t('chatView.noTranscript')}</p>
        </div>
      </div>
    )
  }

  if (messages.length === 0 && initialLoaded) {
    return (
      <div className="flex items-center justify-center py-16 text-gray-500 dark:text-gray-400">
        <p className="text-sm">{t('chatView.noMessages')}</p>
      </div>
    )
  }

  return (
    <div ref={scrollRef} className="flex flex-col gap-3 p-4">
      {messages.map((msg, i) => (
        <MessageBubble key={`${page}-${i}`} message={msg} />
      ))}
      {hasMore && (
        <div className="flex justify-center py-4">
          <button
            onClick={() => loadPage(page + 1)}
            disabled={loading}
            className={cn(
              'px-4 py-2 rounded-lg text-sm font-medium transition-colors',
              'bg-orange-500/10 text-orange-400 hover:bg-orange-500/20',
              'border border-orange-500/20 hover:border-orange-500/40',
              'disabled:opacity-50 disabled:cursor-not-allowed'
            )}
          >
            {loading ? t('chatView.loading') : t('chatView.loadMore', { count: totalEntries - messages.length })}
          </button>
        </div>
      )}
    </div>
  )
}

const MessageBubble = memo(function MessageBubble({ message }: { message: ConversationMessage }) {
  const isUser = message.role === 'user'
  const { t } = useLanguage()
  const grouped = useMemo(() => groupMessageBlocks(message.content), [message.content])

  if (grouped.length === 0) return null

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2 }}
      className={cn('flex', isUser ? 'justify-end' : 'justify-start')}
    >
      <div
        className={cn(
          'max-w-[85%] rounded-xl px-4 py-3',
          isUser
            ? 'bg-orange-500/15 border border-orange-500/25 rounded-br-sm'
            : 'bg-gray-100 dark:bg-daily-dark/80 border border-gray-200 dark:border-gray-700/50 rounded-bl-sm'
        )}
      >
        {/* Role label */}
        <div
          className={cn(
            'text-[11px] font-semibold uppercase tracking-wider mb-1.5 flex items-center gap-2',
            isUser ? 'text-orange-500 dark:text-orange-400' : 'text-sky-600 dark:text-sky-400'
          )}
        >
          <span>{isUser ? t('chatView.you') : t('chatView.claude')}</span>
          {message.timestamp && (
            <span className="text-gray-400 dark:text-gray-500 font-mono font-normal normal-case tracking-normal tabular-nums">
              {formatTimestamp(message.timestamp)}
            </span>
          )}
        </div>

        {/* Grouped content blocks */}
        <div className="space-y-2">
          {grouped.map((g, i) =>
            g.kind === 'text' ? (
              <div key={i} className="text-sm leading-relaxed text-gray-800 dark:text-gray-200">
                <MarkdownRenderer content={g.text} />
              </div>
            ) : (
              <ToolCallGroup key={i} calls={g.calls} />
            )
          )}
        </div>
      </div>
    </motion.div>
  )
})

function ToolCallGroup({ calls }: { calls: PairedToolCall[] }) {
  const [groupOpen, setGroupOpen] = useState(false)

  const countsByName = useMemo(() => {
    const map = new Map<string, number>()
    for (const c of calls) {
      map.set(c.name, (map.get(c.name) ?? 0) + 1)
    }
    return [...map.entries()]
  }, [calls])

  if (calls.length === 1) {
    return <SingleToolCall call={calls[0]} />
  }

  return (
    <div className="my-1 rounded-xl border border-gray-200 dark:border-gray-700/60 bg-white/60 dark:bg-black/30 overflow-hidden">
      <button
        type="button"
        onClick={() => setGroupOpen(o => !o)}
        className="w-full flex flex-wrap items-center gap-2 px-3 py-2 text-xs text-gray-600 dark:text-gray-300 hover:bg-gray-100/70 dark:hover:bg-white/5 transition-colors text-left"
      >
        <span className="text-gray-400 dark:text-gray-500 font-mono">
          {groupOpen ? '▾' : '▸'}
        </span>
        <span className="font-medium tabular-nums">
          Ran {calls.length} tools
        </span>
        <div className="flex flex-wrap items-center gap-1.5">
          {countsByName.map(([toolName, count]) => (
            <span
              key={toolName}
              className="px-1.5 py-0.5 rounded bg-gray-100 dark:bg-gray-800 text-gray-700 dark:text-gray-300 font-mono text-[11px] tabular-nums"
            >
              {toolName}{count > 1 ? ` ×${count}` : ''}
            </span>
          ))}
        </div>
        <span className="ml-auto text-[11px] text-gray-400 dark:text-gray-500">
          {groupOpen ? 'collapse' : 'expand'}
        </span>
      </button>
      {groupOpen && (
        <div className="px-3 pb-2.5 pt-1 space-y-1.5 border-t border-gray-200/70 dark:border-gray-700/50">
          {calls.map((c, idx) => (
            <SingleToolCall key={`${c.id}-${idx}`} call={c} />
          ))}
        </div>
      )}
    </div>
  )
}

function SingleToolCall({ call }: { call: PairedToolCall }) {
  const [expanded, setExpanded] = useState(false)
  const { t } = useLanguage()
  const summary = getToolSummary(call.name, call.input)

  return (
    <div className="my-1 border border-gray-300/50 dark:border-gray-700/60 rounded-lg overflow-hidden">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className={cn(
          'w-full flex items-center gap-2 px-3 py-1.5 text-xs',
          'bg-gray-50/80 dark:bg-daily-dark/60 hover:bg-gray-100 dark:hover:bg-daily-dark',
          'text-gray-600 dark:text-gray-400 transition-colors text-left'
        )}
      >
        <span className="text-orange-500 dark:text-orange-400 font-semibold font-mono shrink-0">
          {call.name}
        </span>
        {summary && <span className="truncate font-mono opacity-75">{summary}</span>}
        <svg
          className={cn('size-3 shrink-0 ml-auto transition-transform', expanded && 'rotate-180')}
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>
      {expanded && (
        <div className="divide-y divide-gray-200 dark:divide-gray-700/60">
          {call.input !== '' && call.input != null && (
            <div className="px-3 py-2 text-xs font-mono bg-gray-100/50 dark:bg-daily-dark max-h-60 overflow-auto">
              <pre className="whitespace-pre-wrap break-all text-gray-600 dark:text-gray-400">
                {typeof call.input === 'string' ? call.input : JSON.stringify(call.input, null, 2)}
              </pre>
            </div>
          )}
          {call.result && (
            <div className="px-3 py-2 text-xs font-mono bg-green-500/5 max-h-60 overflow-auto">
              <div className="text-[10px] font-semibold text-green-600 dark:text-green-500 uppercase tracking-wider mb-1">
                {t('chatView.result')}
              </div>
              <pre className="whitespace-pre-wrap break-all text-gray-600 dark:text-gray-400">
                {call.result}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

function getToolSummary(name: string, input: unknown): string {
  const inp = input as Record<string, unknown>
  if (!inp || typeof inp !== 'object') return ''
  const lower = name.toLowerCase()
  if (lower === 'read' || lower === 'write' || lower === 'edit' || lower === 'replace' || lower === 'replace_match' || lower === 'insert') {
    return String(inp.file_path || inp.path || '')
  }
  if (lower === 'bash' || lower === 'hypa_shell') {
    return String(inp.command || '').slice(0, 80)
  }
  if (lower === 'glob' || lower === 'grep' || lower === 'hypa_grep' || lower === 'hypa_find') {
    return String(inp.pattern || inp.path || '')
  }
  if (lower === 'task' || lower === 'subagent' || lower.endsWith('delegate_task')) {
    return String(inp.description || inp.title || inp.task || '').slice(0, 60)
  }
  if (lower === 'webfetch' || lower === 'websearch' || lower === 'web_search') {
    return String(inp.url || inp.query || '').slice(0, 60)
  }
  const fallback = inp.file_path ?? inp.path ?? inp.command ?? inp.pattern ?? inp.query ?? inp.title
  return typeof fallback === 'string' ? fallback.slice(0, 60) : ''
}

function formatTimestamp(ts: string): string {
  try {
    const date = new Date(ts)
    return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
  } catch {
    return ts
  }
}
