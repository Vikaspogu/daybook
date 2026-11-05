import { useEffect, useState, useCallback, useMemo } from 'react'
import {
  useApi,
  type BoardStatusData,
  type BoardAgent,
  type BoardLock,
  type BoardTask,
  type BoardMessage,
} from '../hooks/useApi'

function formatRelativeSeconds(epochSec: number): string {
  const nowSec = Math.floor(Date.now() / 1000)
  const diff = nowSec - epochSec
  if (diff < 5) return 'just now'
  if (diff < 60) return `${diff}s ago`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  return `${Math.floor(diff / 86400)}d ago`
}

function formatRemainingTtl(expiresEpoch: number): string {
  const nowSec = Math.floor(Date.now() / 1000)
  const rem = expiresEpoch - nowSec
  if (rem <= 0) return 'expiring'
  if (rem < 60) return `${rem}s left`
  return `${Math.ceil(rem / 60)}m left`
}

function isSafeHttpUrl(url?: string | null): url is string {
  if (!url) return false
  return url.startsWith('https://') || url.startsWith('http://')
}

function renderMessageBodyWithMentions(body: string) {
  const parts = body.split(/(@[a-z0-9][a-z0-9_-]{0,63})/gi)
  return parts.map((part, idx) => {
    if (/^@[a-z0-9][a-z0-9_-]{0,63}$/i.test(part)) {
      return (
        <span
          key={idx}
          className="inline-flex items-center px-1.5 py-0.5 rounded bg-amber-400/15 text-amber-300 font-mono text-[11px] font-semibold"
        >
          {part.toLowerCase()}
        </span>
      )
    }
    return <span key={idx}>{part}</span>
  })
}

export function Board() {
  const {
    fetchBoardStatus,
    postBoardMessage,
    claimBoardLocks,
    releaseBoardLocks,
    claimBoardTask,
    updateBoardTask,
  } = useApi()

  const [selectedProject, setSelectedProject] = useState<string>('')
  const [board, setBoard] = useState<BoardStatusData | null>(null)
  const [initialLoading, setInitialLoading] = useState(true)
  const [actionError, setActionError] = useState<string | null>(null)
  const [lastRefreshed, setLastRefreshed] = useState<number>(Date.now())

  // Filters
  const [taskFilter, setTaskFilter] = useState<'all' | 'active' | 'open' | 'done'>('all')
  const [channelFilter, setChannelFilter] = useState<'all' | 'general' | 'handoff' | 'blocker'>('all')
  const [mentionFilter, setMentionFilter] = useState<string>('')

  // Operator / Agent handle persisted in localStorage
  const [operatorHandle, setOperatorHandle] = useState<string>(() => {
    if (typeof window !== 'undefined') {
      return localStorage.getItem('daybook_board_agent') || 'operator'
    }
    return 'operator'
  })

  // Inline forms
  const [showLockForm, setShowLockForm] = useState(false)
  const [lockPathsInput, setLockPathsInput] = useState('')
  const [lockNoteInput, setLockNoteInput] = useState('')
  const [lockTtlInput, setLockTtlInput] = useState(30)

  const [showTaskForm, setShowTaskForm] = useState(false)
  const [taskIdInput, setTaskIdInput] = useState('')
  const [taskTitleInput, setTaskTitleInput] = useState('')
  const [taskPriorityInput, setTaskPriorityInput] = useState<'high' | 'medium' | 'low'>('medium')
  const [taskBranchInput, setTaskBranchInput] = useState('')
  const [taskNoteInput, setTaskNoteInput] = useState('')

  const [msgChannel, setMsgChannel] = useState<'general' | 'handoff' | 'blocker'>('general')
  const [msgBody, setMsgBody] = useState('')
  const [showSetupDrawer, setShowSetupDrawer] = useState(false)

  const handleOperatorChange = (val: string) => {
    setOperatorHandle(val)
    if (typeof window !== 'undefined') {
      localStorage.setItem('daybook_board_agent', val)
    }
  }

  const refreshBoard = useCallback(
    async (projOverride?: string) => {
      try {
        const target = projOverride !== undefined ? projOverride : selectedProject
        const snap = await fetchBoardStatus(target || undefined)
        setBoard(snap)
        if (!selectedProject && snap.project) {
          setSelectedProject(snap.project)
        }
        setLastRefreshed(Date.now())
      } catch (err) {
        setActionError(err instanceof Error ? err.message : 'Failed to load board')
      } finally {
        setInitialLoading(false)
      }
    },
    [fetchBoardStatus, selectedProject]
  )

  useEffect(() => {
    refreshBoard()
    const timer = window.setInterval(() => {
      if (document.visibilityState === 'visible') {
        refreshBoard()
      }
    }, 5000)
    const onVisChange = () => {
      if (document.visibilityState === 'visible') {
        refreshBoard()
      }
    }
    document.addEventListener('visibilitychange', onVisChange)
    return () => {
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', onVisChange)
    }
  }, [refreshBoard])

  const activeProject = selectedProject || board?.project || 'daybook'

  const filteredTasks = useMemo(() => {
    const list = board?.tasks ?? []
    return list.filter((t) => {
      if (taskFilter === 'all') return true
      if (taskFilter === 'active') {
        return t.status === 'claimed' || t.status === 'review' || t.status === 'blocked'
      }
      return t.status === taskFilter
    })
  }, [board?.tasks, taskFilter])

  const filteredMessages = useMemo(() => {
    const list = board?.messages ?? []
    const cleanMention = mentionFilter.trim().replace(/^@/, '').toLowerCase()
    return list.filter((m) => {
      if (channelFilter !== 'all' && m.channel !== channelFilter) return false
      if (cleanMention && !m.mentions.some((x) => x.toLowerCase().includes(cleanMention))) {
        return false
      }
      return true
    })
  }, [board?.messages, channelFilter, mentionFilter])

  const handleClaimLock = async (e: React.FormEvent) => {
    e.preventDefault()
    setActionError(null)
    const paths = lockPathsInput
      .split(/[\s,]+/)
      .map((s) => s.trim())
      .filter(Boolean)
    if (paths.length === 0) return
    try {
      const res = await claimBoardLocks({
        project: activeProject,
        agent: operatorHandle || 'operator',
        paths,
        note: lockNoteInput,
        ttl_minutes: lockTtlInput,
      })
      if (res.conflicts.length > 0) {
        setActionError(
          `Lock claimed, but overlaps with ${res.conflicts.length} existing lock(s) held by ${res.conflicts
            .map((c) => `@${c.agent}`)
            .join(', ')}`
        )
      }
      setLockPathsInput('')
      setLockNoteInput('')
      setShowLockForm(false)
      await refreshBoard()
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to claim lock')
    }
  }

  const handleReleaseLock = async (lock: BoardLock) => {
    setActionError(null)
    try {
      await releaseBoardLocks({
        project: lock.project,
        agent: lock.agent,
        lock_id: lock.id,
        force: true,
      })
      await refreshBoard()
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to release lock')
    }
  }

  const handleClaimTask = async (e: React.FormEvent) => {
    e.preventDefault()
    setActionError(null)
    if (!taskIdInput.trim()) return
    try {
      const res = await claimBoardTask({
        project: activeProject,
        task_id: taskIdInput.trim(),
        title: taskTitleInput.trim() || taskIdInput.trim(),
        priority: taskPriorityInput,
        agent: operatorHandle || 'operator',
        branch: taskBranchInput.trim() || undefined,
        note: taskNoteInput.trim() || undefined,
      })
      if (!res.ok) {
        setActionError(`Task [${res.task.id}] is already claimed by @${res.held_by}`)
      } else {
        setTaskIdInput('')
        setTaskTitleInput('')
        setTaskBranchInput('')
        setTaskNoteInput('')
        setShowTaskForm(false)
      }
      await refreshBoard()
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to claim task')
    }
  }

  const handleQuickTaskStatus = async (task: BoardTask, nextStatus: string) => {
    setActionError(null)
    try {
      await updateBoardTask({
        project: task.project,
        task_id: task.id,
        agent: task.agent || operatorHandle || 'operator',
        status: nextStatus,
        release_locks: nextStatus === 'done',
        force: true,
      })
      await refreshBoard()
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to update task')
    }
  }

  const handlePostMessage = async (e: React.FormEvent) => {
    e.preventDefault()
    setActionError(null)
    if (!msgBody.trim()) return
    try {
      await postBoardMessage({
        project: activeProject,
        channel: msgChannel,
        agent: operatorHandle || 'operator',
        body: msgBody.trim(),
      })
      setMsgBody('')
      await refreshBoard()
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to post message')
    }
  }

  const nowSec = Math.floor(Date.now() / 1000)
  const agents = board?.agents ?? []
  const locks = board?.locks ?? []
  const tasks = board?.tasks ?? []
  const messages = board?.messages ?? []
  const onlineAgentsCount = agents.filter((a) => nowSec - a.last_seen_epoch < 600 && a.status !== 'idle').length
  const activeTasksCount = tasks.filter(
    (t) => t.status === 'claimed' || t.status === 'review' || t.status === 'blocked'
  ).length
  const blockerMessagesCount = messages.filter((m) => m.channel === 'blocker').length

  return (
    <div className="max-w-[1440px] mx-auto px-4 sm:px-6 py-6 space-y-6">
      {/* Top Bar: Title + Project Switcher + Operator Handle */}
      <div className="qp-card p-5 flex flex-col lg:flex-row lg:items-center lg:justify-between gap-4">
        <div>
          <div className="flex items-center gap-2.5">
            <span className="inline-flex items-center px-2 py-0.5 rounded-md text-[10px] font-mono font-bold uppercase tracking-wider bg-amber-400/15 text-amber-300 border border-amber-400/25">
              Multi-Worktree Coordination
            </span>
            <span className="inline-flex items-center gap-1.5 text-[11px] font-mono text-slate-400">
              <span className="size-2 rounded-full bg-emerald-400 animate-pulse" />
              5s live sync · updated {Math.max(0, Math.floor((Date.now() - lastRefreshed) / 1000))}s ago
            </span>
          </div>
          <h1 className="text-xl sm:text-2xl font-extrabold tracking-tight text-slate-100 mt-1.5">
            Agent Coordination Board
          </h1>
          <p className="text-xs text-slate-400 mt-0.5">
            Shared file locks, worktree presence, task claims, and @mention channels across T3 Code, Claude Code, Codex, and Pi.
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2.5">
          {/* Project selector */}
          <div className="flex items-center gap-1.5 bg-[#0B0E14] border border-white/[0.08] rounded-xl px-2.5 py-1.5">
            <span className="text-[10px] font-mono uppercase tracking-wider text-slate-400">
              Project
            </span>
            <select
              value={activeProject}
              onChange={(e) => {
                setSelectedProject(e.target.value)
                refreshBoard(e.target.value)
              }}
              aria-label="Select project"
              className="bg-transparent text-xs font-mono font-bold text-amber-300 focus:outline-none"
            >
              {(board?.projects && board.projects.length > 0 ? board.projects : [activeProject]).map(
                (proj) => (
                  <option key={proj} value={proj} className="bg-[#131822] text-slate-100">
                    {proj}
                  </option>
                )
              )}
            </select>
          </div>

          {/* Operator / Agent handle input */}
          <div className="flex items-center gap-1.5 bg-[#0B0E14] border border-white/[0.08] rounded-xl px-2.5 py-1.5">
            <span className="text-[10px] font-mono uppercase tracking-wider text-slate-400">
              Handle
            </span>
            <input
              type="text"
              value={operatorHandle}
              onChange={(e) => handleOperatorChange(e.target.value)}
              placeholder="operator"
              aria-label="Agent handle"
              className="w-28 bg-transparent text-xs font-mono font-semibold text-slate-200 focus:outline-none"
            />
          </div>

          <button
            type="button"
            onClick={() => refreshBoard()}
            className="px-3 py-1.5 rounded-xl bg-white/[0.05] hover:bg-white/[0.1] border border-white/[0.08] text-xs font-semibold text-slate-200 transition-colors"
          >
            Refresh
          </button>

          <button
            type="button"
            onClick={() => setShowSetupDrawer((v) => !v)}
            className="px-3 py-1.5 rounded-xl qp-surface-amber text-xs font-bold text-amber-300 transition-colors"
          >
            {showSetupDrawer ? 'Hide CLI / MCP Setup' : 'CLI & MCP Setup'}
          </button>
        </div>
      </div>

      {/* Collapsible CLI / Hooks / MCP Quick-Start Guide */}
      {showSetupDrawer && (
        <div className="qp-card p-5 border border-amber-400/25 space-y-3">
          <div className="flex items-center justify-between">
            <h2 className="text-xs font-mono font-bold uppercase tracking-wider text-amber-300">
              Multi-Worktree Agent Setup (T3 Code · Claude Code · Codex · Pi)
            </h2>
            <a
              href="/api/board/agents.md"
              target="_blank"
              rel="noreferrer"
              className="text-xs font-mono text-amber-300 hover:underline"
            >
              Open /api/board/agents.md ↗
            </a>
          </div>
          <div className="grid grid-cols-1 md:grid-cols-3 gap-3 text-xs">
            <div className="p-3 rounded-xl bg-[#0B0E14] border border-white/[0.06] space-y-1.5">
              <div className="font-bold text-slate-200">1. Install Hooks & Rules</div>
              <pre className="font-mono text-[11px] text-amber-300 overflow-x-auto">
                daybook init --board
              </pre>
              <p className="text-[11px] text-slate-400">
                Registers Claude Code PreToolUse/PostToolUse file-lock hooks and writes the Agent Board section into AGENTS.md.
              </p>
            </div>
            <div className="p-3 rounded-xl bg-[#0B0E14] border border-white/[0.06] space-y-1.5">
              <div className="font-bold text-slate-200">2. Register MCP Server</div>
              <pre className="font-mono text-[11px] text-emerald-300 overflow-x-auto">
                {`{ "mcpServers": { "daybook-board": { "command": "daybook", "args": ["board", "mcp"] } } }`}
              </pre>
              <p className="text-[11px] text-slate-400">
                Exposes 7 native MCP tools (`daybook_board_status`, `claim`, `release`, `check`, `post`, `claim_task`, `update_task`).
              </p>
            </div>
            <div className="p-3 rounded-xl bg-[#0B0E14] border border-white/[0.06] space-y-1.5">
              <div className="font-bold text-slate-200">3. CLI Coordination</div>
              <pre className="font-mono text-[11px] text-sky-300 overflow-x-auto">
                {`daybook board status\ndaybook board claim "src/server/**"\ndaybook board post -c handoff "@t3-a1 done"`}
              </pre>
              <p className="text-[11px] text-slate-400">
                Every worktree resolves back to `{activeProject}` automatically while keeping its own worktree handle.
              </p>
            </div>
          </div>
        </div>
      )}

      {actionError && (
        <div className="p-3.5 rounded-xl bg-rose-500/10 border border-rose-500/30 flex items-center justify-between gap-3 text-xs text-rose-200">
          <span>{actionError}</span>
          <button
            type="button"
            onClick={() => setActionError(null)}
            className="text-rose-300 hover:text-white font-mono text-[11px]"
          >
            Dismiss
          </button>
        </div>
      )}

      {/* KPI Strip */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-3.5">
        <div className="qp-card p-4">
          <div className="text-[10px] font-mono uppercase tracking-wider text-slate-400">
            Worktrees & Agents
          </div>
          <div className="mt-1 flex items-baseline gap-2">
            <span className="text-2xl font-extrabold font-mono text-emerald-400">
              {onlineAgentsCount}
            </span>
            <span className="text-xs text-slate-400">active / {agents.length} registered</span>
          </div>
        </div>

        <div className="qp-card p-4">
          <div className="text-[10px] font-mono uppercase tracking-wider text-slate-400">
            Active File Locks
          </div>
          <div className="mt-1 flex items-baseline gap-2">
            <span className="text-2xl font-extrabold font-mono text-amber-300">
              {locks.length}
            </span>
            <span className="text-xs text-slate-400">
              {locks.filter((l) => !l.auto).length} manual · {locks.filter((l) => l.auto).length} auto
            </span>
          </div>
        </div>

        <div className="qp-card p-4">
          <div className="text-[10px] font-mono uppercase tracking-wider text-slate-400">
            In-Flight Tasks
          </div>
          <div className="mt-1 flex items-baseline gap-2">
            <span className="text-2xl font-extrabold font-mono text-sky-300">
              {activeTasksCount}
            </span>
            <span className="text-xs text-slate-400">of {tasks.length} total tasks</span>
          </div>
        </div>

        <div className="qp-card p-4">
          <div className="text-[10px] font-mono uppercase tracking-wider text-slate-400">
            Channel Feed
          </div>
          <div className="mt-1 flex items-baseline gap-2">
            <span className="text-2xl font-extrabold font-mono text-slate-100">
              {messages.length}
            </span>
            <span className="text-xs text-slate-400">
              {blockerMessagesCount > 0 ? (
                <span className="text-rose-400 font-semibold">{blockerMessagesCount} #blocker</span>
              ) : (
                '0 blockers'
              )}
            </span>
          </div>
        </div>
      </div>

      {initialLoading ? (
        <div className="qp-card p-12 text-center text-sm text-slate-400 font-mono">
          Loading coordination board...
        </div>
      ) : (
        <div className="grid grid-cols-1 lg:grid-cols-12 gap-5 items-start">
          {/* LEFT COLUMN: Active Worktrees & File Locks */}
          <div className="lg:col-span-4 space-y-5">
            {/* Active File Locks */}
            <section className="qp-card p-4 space-y-3.5">
              <div className="flex items-center justify-between">
                <div>
                  <h2 className="text-xs font-mono font-bold uppercase tracking-wider text-amber-300">
                    Active File & Glob Locks ({locks.length})
                  </h2>
                  <p className="text-[11px] text-slate-400">
                    Advisory / strict PreToolUse conflict protection across worktrees
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => setShowLockForm((v) => !v)}
                  className="px-2.5 py-1 rounded-lg bg-amber-400/15 hover:bg-amber-400/25 border border-amber-400/30 text-[11px] font-mono font-bold text-amber-300"
                >
                  {showLockForm ? 'Cancel' : '+ Claim'}
                </button>
              </div>

              {showLockForm && (
                <form
                  onSubmit={handleClaimLock}
                  className="p-3 rounded-xl bg-[#0B0E14] border border-white/[0.08] space-y-2.5"
                >
                  <input
                    type="text"
                    value={lockPathsInput}
                    onChange={(e) => setLockPathsInput(e.target.value)}
                    placeholder="Globs: src/server/**, src/ledger/board.rs"
                    className="w-full px-2.5 py-1.5 rounded-lg bg-[#131822] border border-white/[0.08] text-xs font-mono text-slate-100"
                    required
                  />
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={lockNoteInput}
                      onChange={(e) => setLockNoteInput(e.target.value)}
                      placeholder="Reason / task note"
                      className="flex-1 px-2.5 py-1.5 rounded-lg bg-[#131822] border border-white/[0.08] text-xs text-slate-100"
                    />
                    <input
                      type="number"
                      min={1}
                      max={240}
                      value={lockTtlInput}
                      onChange={(e) => setLockTtlInput(Number(e.target.value) || 30)}
                      aria-label="Lock TTL minutes"
                      className="w-20 px-2 py-1.5 rounded-lg bg-[#131822] border border-white/[0.08] text-xs font-mono text-slate-100"
                    />
                  </div>
                  <button
                    type="submit"
                    className="w-full py-1.5 rounded-lg bg-amber-400 text-slate-950 font-bold text-xs hover:bg-amber-300 transition-colors"
                  >
                    Claim Lock ({lockTtlInput}m TTL)
                  </button>
                </form>
              )}

              {locks.length === 0 ? (
                <div className="p-6 rounded-xl bg-[#0B0E14]/60 border border-white/[0.04] text-center text-xs text-slate-500">
                  No active file locks in <span className="font-mono text-slate-400">{activeProject}</span>.
                </div>
              ) : (
                <div className="space-y-2.5">
                  {locks.map((lock) => (
                    <div
                      key={lock.id}
                      className="p-3 rounded-xl bg-[#0B0E14]/90 border border-white/[0.07] space-y-2"
                    >
                      <div className="flex items-center justify-between gap-2">
                        <div className="flex items-center gap-1.5 min-w-0">
                          <span className="font-mono text-xs font-bold text-amber-300 truncate">
                            @{lock.agent}
                          </span>
                          <span className="px-1.5 py-0.5 rounded text-[10px] font-mono bg-white/[0.05] text-slate-400">
                            {lock.auto ? 'auto' : 'manual'}
                          </span>
                          <span className="text-[10px] font-mono text-slate-500">
                            {formatRemainingTtl(lock.expires_epoch)}
                          </span>
                        </div>
                        <button
                          type="button"
                          onClick={() => handleReleaseLock(lock)}
                          className="px-2 py-0.5 rounded bg-rose-500/15 hover:bg-rose-500/25 text-rose-300 text-[10px] font-mono font-semibold"
                        >
                          Release
                        </button>
                      </div>

                      <div className="flex flex-wrap gap-1.5">
                        {lock.paths.map((p) => (
                          <code
                            key={p}
                            className="px-2 py-0.5 rounded bg-amber-400/10 border border-amber-400/20 text-[11px] font-mono text-amber-200 break-all"
                          >
                            {p}
                          </code>
                        ))}
                      </div>

                      {lock.note && (
                        <div className="text-[11px] text-slate-400">{lock.note}</div>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </section>

            {/* Active Agents & Worktrees */}
            <section className="qp-card p-4 space-y-3.5">
              <div>
                <h2 className="text-xs font-mono font-bold uppercase tracking-wider text-emerald-300">
                  Worktrees & Agent Roster ({agents.length})
                </h2>
                <p className="text-[11px] text-slate-400">
                  Every worktree session mapped to its branch and active task
                </p>
              </div>

              {agents.length === 0 ? (
                <div className="p-6 rounded-xl bg-[#0B0E14]/60 border border-white/[0.04] text-center text-xs text-slate-500">
                  No agent heartbeats recorded yet.
                </div>
              ) : (
                <div className="space-y-2.5">
                  {agents.map((ag: BoardAgent) => {
                    const ageSec = nowSec - ag.last_seen_epoch
                    const isFresh = ageSec < 600 && ag.status !== 'idle'
                    return (
                      <div
                        key={ag.agent}
                        className="p-3 rounded-xl bg-[#0B0E14]/90 border border-white/[0.07] space-y-1.5"
                      >
                        <div className="flex items-center justify-between gap-2">
                          <div className="flex items-center gap-2 min-w-0">
                            <span
                              className={`size-2 rounded-full shrink-0 ${
                                isFresh
                                  ? 'bg-emerald-400'
                                  : ageSec < 1800
                                    ? 'bg-amber-400'
                                    : 'bg-slate-600'
                              }`}
                            />
                            <span className="font-mono text-xs font-bold text-slate-100 truncate">
                              @{ag.agent}
                            </span>
                            <span className="px-1.5 py-0.5 rounded bg-white/[0.06] text-[10px] font-mono text-slate-300">
                              {ag.runtime}
                            </span>
                          </div>
                          <span className="text-[10px] font-mono text-slate-400 shrink-0">
                            {formatRelativeSeconds(ag.last_seen_epoch)}
                          </span>
                        </div>

                        <div className="flex flex-wrap items-center gap-1.5 text-[11px] font-mono">
                          {ag.branch && (
                            <span className="px-1.5 py-0.5 rounded bg-sky-500/10 border border-sky-500/20 text-sky-300">
                              ⎇ {ag.branch}
                            </span>
                          )}
                          {ag.current_task && (
                            <span className="px-1.5 py-0.5 rounded bg-amber-400/15 border border-amber-400/25 text-amber-300">
                              #{ag.current_task}
                            </span>
                          )}
                          <span className="text-slate-500">{ag.status}</span>
                        </div>

                        {ag.cwd && (
                          <div className="text-[10px] font-mono text-slate-500 truncate" title={ag.cwd}>
                            {ag.cwd}
                          </div>
                        )}
                      </div>
                    )
                  })}
                </div>
              )}
            </section>
          </div>

          {/* CENTER COLUMN: Shared Task Board */}
          <div className="lg:col-span-4 space-y-4">
            <section className="qp-card p-4 space-y-3.5">
              <div className="flex items-center justify-between gap-2">
                <div>
                  <h2 className="text-xs font-mono font-bold uppercase tracking-wider text-sky-300">
                    Worktree Tasks ({filteredTasks.length})
                  </h2>
                  <p className="text-[11px] text-slate-400">
                    Prevent duplicate work across parallel worktrees
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => setShowTaskForm((v) => !v)}
                  className="px-2.5 py-1 rounded-lg bg-sky-500/15 hover:bg-sky-500/25 border border-sky-500/30 text-[11px] font-mono font-bold text-sky-300"
                >
                  {showTaskForm ? 'Cancel' : '+ Claim Task'}
                </button>
              </div>

              {/* Filter pills */}
              <div className="flex items-center gap-1 bg-[#0B0E14] p-1 rounded-xl border border-white/[0.06]">
                {(['all', 'active', 'open', 'done'] as const).map((f) => (
                  <button
                    key={f}
                    type="button"
                    onClick={() => setTaskFilter(f)}
                    className={`flex-1 py-1 rounded-lg text-[11px] font-mono font-semibold capitalize transition-colors ${
                      taskFilter === f
                        ? 'bg-white/[0.1] text-slate-100'
                        : 'text-slate-400 hover:text-slate-200'
                    }`}
                  >
                    {f}
                  </button>
                ))}
              </div>

              {showTaskForm && (
                <form
                  onSubmit={handleClaimTask}
                  className="p-3 rounded-xl bg-[#0B0E14] border border-white/[0.08] space-y-2.5"
                >
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={taskIdInput}
                      onChange={(e) => setTaskIdInput(e.target.value)}
                      placeholder="Task ID (e.g. AUTH-1)"
                      className="w-32 px-2.5 py-1.5 rounded-lg bg-[#131822] border border-white/[0.08] text-xs font-mono text-slate-100"
                      required
                    />
                    <select
                      value={taskPriorityInput}
                      onChange={(e) =>
                        setTaskPriorityInput(e.target.value as 'high' | 'medium' | 'low')
                      }
                      aria-label="Task priority"
                      className="px-2 py-1.5 rounded-lg bg-[#131822] border border-white/[0.08] text-xs font-mono text-slate-200"
                    >
                      <option value="high">high</option>
                      <option value="medium">medium</option>
                      <option value="low">low</option>
                    </select>
                  </div>
                  <input
                    type="text"
                    value={taskTitleInput}
                    onChange={(e) => setTaskTitleInput(e.target.value)}
                    placeholder="Task title"
                    className="w-full px-2.5 py-1.5 rounded-lg bg-[#131822] border border-white/[0.08] text-xs text-slate-100"
                  />
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={taskBranchInput}
                      onChange={(e) => setTaskBranchInput(e.target.value)}
                      placeholder="Branch (optional)"
                      className="w-1/2 px-2.5 py-1.5 rounded-lg bg-[#131822] border border-white/[0.08] text-xs font-mono text-slate-100"
                    />
                    <input
                      type="text"
                      value={taskNoteInput}
                      onChange={(e) => setTaskNoteInput(e.target.value)}
                      placeholder="Progress note"
                      className="w-1/2 px-2.5 py-1.5 rounded-lg bg-[#131822] border border-white/[0.08] text-xs text-slate-100"
                    />
                  </div>
                  <button
                    type="submit"
                    className="w-full py-1.5 rounded-lg bg-sky-400 text-slate-950 font-bold text-xs hover:bg-sky-300 transition-colors"
                  >
                    Claim Task as @{operatorHandle || 'operator'}
                  </button>
                </form>
              )}

              {filteredTasks.length === 0 ? (
                <div className="p-8 rounded-xl bg-[#0B0E14]/60 border border-white/[0.04] text-center text-xs text-slate-500">
                  No tasks matching filter.
                </div>
              ) : (
                <div className="space-y-2.5">
                  {filteredTasks.map((task) => (
                    <div
                      key={task.id}
                      className="p-3.5 rounded-xl bg-[#0B0E14]/90 border border-white/[0.07] space-y-2"
                    >
                      <div className="flex items-start justify-between gap-2">
                        <div className="space-y-1 min-w-0">
                          <div className="flex flex-wrap items-center gap-1.5">
                            <span className="px-1.5 py-0.5 rounded bg-white/[0.07] font-mono text-[11px] font-bold text-slate-200">
                              {task.id}
                            </span>
                            <span
                              className={`px-1.5 py-0.5 rounded font-mono text-[10px] font-bold uppercase ${
                                task.status === 'claimed'
                                  ? 'bg-emerald-500/15 text-emerald-300'
                                  : task.status === 'blocked'
                                    ? 'bg-rose-500/15 text-rose-300'
                                    : task.status === 'review'
                                      ? 'bg-amber-400/15 text-amber-300'
                                      : task.status === 'done'
                                        ? 'bg-slate-700/40 text-slate-400'
                                        : 'bg-sky-500/15 text-sky-300'
                              }`}
                            >
                              {task.status}
                            </span>
                            <span
                              className={`text-[10px] font-mono ${
                                task.priority === 'high'
                                  ? 'text-rose-400 font-bold'
                                  : task.priority === 'low'
                                    ? 'text-slate-500'
                                    : 'text-amber-300'
                              }`}
                            >
                              {task.priority}
                            </span>
                          </div>
                          <div className="text-xs font-semibold text-slate-100 break-words">
                            {task.title}
                          </div>
                        </div>
                        <span className="text-[10px] font-mono text-slate-500 shrink-0">
                          {formatRelativeSeconds(task.updated_epoch)}
                        </span>
                      </div>

                      <div className="flex flex-wrap items-center gap-2 text-[11px] font-mono text-slate-400">
                        {task.agent && <span className="text-amber-300">@{task.agent}</span>}
                        {task.branch && <span className="text-sky-300">⎇ {task.branch}</span>}
                        {isSafeHttpUrl(task.pr_url) && (
                          <a
                            href={task.pr_url}
                            target="_blank"
                            rel="noreferrer"
                            className="text-emerald-300 hover:underline"
                          >
                            PR ↗
                          </a>
                        )}
                      </div>

                      {task.note && (
                        <div className="text-[11px] text-slate-400 bg-white/[0.02] p-2 rounded-lg border border-white/[0.04]">
                          {task.note}
                        </div>
                      )}

                      <div className="flex items-center gap-1.5 pt-1">
                        {task.status !== 'done' && (
                          <button
                            type="button"
                            onClick={() => handleQuickTaskStatus(task, 'done')}
                            className="px-2 py-0.5 rounded bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-300 text-[10px] font-mono font-semibold"
                          >
                            Mark Done
                          </button>
                        )}
                        {task.status !== 'review' && task.status !== 'done' && (
                          <button
                            type="button"
                            onClick={() => handleQuickTaskStatus(task, 'review')}
                            className="px-2 py-0.5 rounded bg-amber-400/15 hover:bg-amber-400/25 text-amber-300 text-[10px] font-mono font-semibold"
                          >
                            Review
                          </button>
                        )}
                        {task.status !== 'blocked' && task.status !== 'done' && (
                          <button
                            type="button"
                            onClick={() => handleQuickTaskStatus(task, 'blocked')}
                            className="px-2 py-0.5 rounded bg-rose-500/15 hover:bg-rose-500/25 text-rose-300 text-[10px] font-mono font-semibold"
                          >
                            Blocked
                          </button>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </section>
          </div>

          {/* RIGHT COLUMN: Cross-Worktree Message Stream */}
          <div className="lg:col-span-4 space-y-4">
            <section className="qp-card p-4 space-y-3.5">
              <div>
                <h2 className="text-xs font-mono font-bold uppercase tracking-wider text-slate-200">
                  Cross-Worktree Channels ({filteredMessages.length})
                </h2>
                <p className="text-[11px] text-slate-400">
                  #general · #handoff · #blocker with @agent mention routing
                </p>
              </div>

              {/* Channel filter tabs + @mention filter */}
              <div className="flex flex-col sm:flex-row gap-2">
                <div className="flex-1 flex items-center gap-1 bg-[#0B0E14] p-1 rounded-xl border border-white/[0.06]">
                  {(['all', 'general', 'handoff', 'blocker'] as const).map((ch) => (
                    <button
                      key={ch}
                      type="button"
                      onClick={() => setChannelFilter(ch)}
                      className={`flex-1 py-1 rounded-lg text-[10px] font-mono font-semibold transition-colors ${
                        channelFilter === ch
                          ? 'bg-white/[0.1] text-amber-300'
                          : 'text-slate-400 hover:text-slate-200'
                      }`}
                    >
                      {ch === 'all' ? 'All' : `#${ch}`}
                    </button>
                  ))}
                </div>
                <input
                  type="text"
                  value={mentionFilter}
                  onChange={(e) => setMentionFilter(e.target.value)}
                  placeholder="Filter @agent"
                  aria-label="Filter by @agent mention"
                  className="sm:w-28 px-2.5 py-1 rounded-xl bg-[#0B0E14] border border-white/[0.06] text-xs font-mono text-slate-200"
                />
              </div>

              {/* Post Message Form */}
              <form
                onSubmit={handlePostMessage}
                className="p-3 rounded-xl bg-[#0B0E14] border border-white/[0.08] space-y-2"
              >
                <div className="flex items-center gap-2">
                  <select
                    value={msgChannel}
                    onChange={(e) =>
                      setMsgChannel(e.target.value as 'general' | 'handoff' | 'blocker')
                    }
                    aria-label="Message channel"
                    className="px-2 py-1 rounded-lg bg-[#131822] border border-white/[0.08] text-xs font-mono text-amber-300"
                  >
                    <option value="general">#general</option>
                    <option value="handoff">#handoff</option>
                    <option value="blocker">#blocker</option>
                  </select>
                  <span className="text-[11px] font-mono text-slate-400">
                    as <strong className="text-slate-200">@{operatorHandle || 'operator'}</strong>
                  </span>
                </div>
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={msgBody}
                    onChange={(e) => setMsgBody(e.target.value)}
                    placeholder="Post update or @agent handoff..."
                    className="flex-1 px-2.5 py-1.5 rounded-lg bg-[#131822] border border-white/[0.08] text-xs text-slate-100"
                  />
                  <button
                    type="submit"
                    className="px-3 py-1.5 rounded-lg bg-amber-400 text-slate-950 font-bold text-xs hover:bg-amber-300 transition-colors"
                  >
                    Post
                  </button>
                </div>
              </form>

              {/* Message Feed */}
              {filteredMessages.length === 0 ? (
                <div className="p-8 rounded-xl bg-[#0B0E14]/60 border border-white/[0.04] text-center text-xs text-slate-500">
                  No messages in this channel yet.
                </div>
              ) : (
                <div className="space-y-2.5 max-h-[560px] overflow-y-auto pr-1">
                  {[...filteredMessages].reverse().map((m: BoardMessage) => (
                    <div
                      key={m.id}
                      className="p-3 rounded-xl bg-[#0B0E14]/90 border border-white/[0.07] space-y-1.5"
                    >
                      <div className="flex items-center justify-between gap-2 text-[10px] font-mono">
                        <div className="flex items-center gap-1.5">
                          <span className="text-slate-500">#{m.id}</span>
                          <span
                            className={`px-1.5 py-0.5 rounded font-bold ${
                              m.channel === 'blocker'
                                ? 'bg-rose-500/15 text-rose-300'
                                : m.channel === 'handoff'
                                  ? 'bg-sky-500/15 text-sky-300'
                                  : 'bg-white/[0.06] text-slate-300'
                            }`}
                          >
                            #{m.channel}
                          </span>
                          <span className="font-bold text-amber-300">@{m.agent}</span>
                        </div>
                        <span className="text-slate-500">
                          {formatRelativeSeconds(m.created_epoch)}
                        </span>
                      </div>
                      <div className="text-xs text-slate-200 leading-relaxed break-words">
                        {renderMessageBodyWithMentions(m.body)}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </section>
          </div>
        </div>
      )}
    </div>
  )
}
