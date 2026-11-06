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

function shortPathLabel(path: string): string {
  const cleaned = path.trim()
  const parts = cleaned.split('/').filter(Boolean)
  if (parts.length <= 2) return cleaned
  return parts.slice(-2).join('/')
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

  const [selectedProject, setSelectedProject] = useState<string>('all')
  const [targetProject, setTargetProject] = useState<string>('')
  const [board, setBoard] = useState<BoardStatusData | null>(null)
  const [initialLoading, setInitialLoading] = useState(true)
  const [actionError, setActionError] = useState<string | null>(null)
  const [lastRefreshed, setLastRefreshed] = useState<number>(Date.now())

  // View toggles & filters
  const [showInactive, setShowInactive] = useState(false)
  const [tasksDrawerOverride, setTasksDrawerOverride] = useState<boolean | null>(null)
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

  useEffect(() => {
    const projects = board?.projects ?? []
    if (projects.length > 0 && (!targetProject || !projects.includes(targetProject))) {
      setTargetProject(projects[0])
    }
  }, [board?.projects, targetProject])

  const activeProject = selectedProject || board?.project || 'all'
  const writeProject =
    activeProject === 'all'
      ? targetProject && board?.projects?.includes(targetProject)
        ? targetProject
        : board?.projects?.[0] || ''
      : activeProject

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
    if (!writeProject) {
      setActionError('No active project on the board yet')
      return
    }
    try {
      const res = await claimBoardLocks({
        project: writeProject,
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

  const handleReleaseAgentLocks = async (locksToRelease: BoardLock[]) => {
    setActionError(null)
    const results = await Promise.allSettled(
      locksToRelease.map((lock) =>
        releaseBoardLocks({
          project: lock.project,
          agent: lock.agent,
          lock_id: lock.id,
          force: true,
        })
      )
    )
    const failed = results.find((r): r is PromiseRejectedResult => r.status === 'rejected')
    if (failed) {
      setActionError(
        failed.reason instanceof Error ? failed.reason.message : 'Failed to release locks'
      )
    }
    await refreshBoard()
  }

  const handleClaimTask = async (e: React.FormEvent) => {
    e.preventDefault()
    setActionError(null)
    if (!taskIdInput.trim()) return
    if (!writeProject) {
      setActionError('No active project on the board yet')
      return
    }
    try {
      const res = await claimBoardTask({
        project: writeProject,
        task_id: taskIdInput.trim(),
        title: taskTitleInput.trim() || taskIdInput.trim(),
        priority: taskPriorityInput,
        agent: operatorHandle || 'operator',
        branch: taskBranchInput.trim() || undefined,
        note: taskNoteInput.trim() || undefined,
      })
      if (!res.ok) {
        setActionError(`Task [${res.task.task_id}] is already claimed by @${res.held_by}`)
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
        task_id: task.task_id,
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
    if (!writeProject) {
      setActionError('No active project on the board yet')
      return
    }
    try {
      await postBoardMessage({
        project: writeProject,
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

  const handleMentionAgent = (project: string, agent: string) => {
    setTargetProject(project)
    const prefix = `@${agent} `
    setMsgBody((prev) => (prev.split(/\s+/).includes(`@${agent}`) ? prev : `${prefix}${prev}`))
  }

  const nowSec = Math.floor(lastRefreshed / 1000)
  const agents = board?.agents ?? []
  const locks = board?.locks ?? []
  const tasks = board?.tasks ?? []
  const messages = board?.messages ?? []

  const knownTaskIds = useMemo(
    () => new Set(tasks.map((t) => `${t.project}:${t.task_id}`)),
    [tasks]
  )

  const formatCurrentTask = useCallback(
    (proj: string, task?: string | null, branch?: string | null): string => {
      if (task && task.trim()) {
        const trimmed = task.trim()
        if (knownTaskIds.has(`${proj}:${trimmed}`)) {
          return `#${trimmed}`
        }
        // Humanize multi-word hyphenated session slugs while preserving ticket IDs (e.g. AUTH-1)
        if (
          !trimmed.includes(' ') &&
          (trimmed.match(/-/g)?.length ?? 0) >= 2 &&
          !/^[A-Z0-9]+-\d+$/.test(trimmed)
        ) {
          return trimmed.replace(/-+/g, ' ')
        }
        return trimmed
      }
      if (branch && branch !== 'unknown' && branch !== 'HEAD') {
        return `Working on ${branch}`
      }
      return 'Coding session'
    },
    [knownTaskIds]
  )

  const agentMetaMap = useMemo(() => {
    const map = new Map<string, BoardAgent>()
    for (const ag of agents) {
      map.set(`${ag.project}:${ag.agent}`, ag)
    }
    return map
  }, [agents])

  const locksByAgentKey = useMemo(() => {
    const map = new Map<string, BoardLock[]>()
    for (const lock of locks) {
      const key = `${lock.project}:${lock.agent}`
      const existing = map.get(key)
      if (existing) {
        existing.push(lock)
      } else {
        map.set(key, [lock])
      }
    }
    return map
  }, [locks])

  const isSessionActive = useCallback(
    (ag: BoardAgent) => {
      const hasLocks = (locksByAgentKey.get(`${ag.project}:${ag.agent}`)?.length ?? 0) > 0
      if (hasLocks) return true
      return nowSec - ag.last_seen_epoch < 7200 && ag.status !== 'idle' && ag.status !== 'done'
    },
    [locksByAgentKey, nowSec]
  )

  const activeAgents = useMemo(
    () => agents.filter((ag) => isSessionActive(ag)),
    [agents, isSessionActive]
  )
  const inactiveAgents = useMemo(
    () => agents.filter((ag) => !isSessionActive(ag)),
    [agents, isSessionActive]
  )

  const visibleAgents = showInactive ? agents : activeAgents

  // Group visible sessions and standalone locks by project
  const projectGroups = useMemo(() => {
    const map = new Map<
      string,
      {
        project: string
        sessions: BoardAgent[]
        orphanLocks: BoardLock[]
        totalLocks: number
        latestSeen: number
      }
    >()

    const ensureGroup = (proj: string) => {
      let g = map.get(proj)
      if (!g) {
        g = {
          project: proj,
          sessions: [],
          orphanLocks: [],
          totalLocks: 0,
          latestSeen: 0,
        }
        map.set(proj, g)
      }
      return g
    }

    const visibleSessionKeys = new Set<string>()
    for (const ag of visibleAgents) {
      const g = ensureGroup(ag.project)
      g.sessions.push(ag)
      visibleSessionKeys.add(`${ag.project}:${ag.agent}`)
      if (ag.last_seen_epoch > g.latestSeen) {
        g.latestSeen = ag.last_seen_epoch
      }
    }

    for (const lock of locks) {
      const g = ensureGroup(lock.project)
      g.totalLocks += lock.paths.length
      if (!visibleSessionKeys.has(`${lock.project}:${lock.agent}`)) {
        g.orphanLocks.push(lock)
      }
    }

    return Array.from(map.values()).sort((a, b) => {
      if (a.totalLocks !== b.totalLocks) return b.totalLocks - a.totalLocks
      return b.latestSeen - a.latestSeen
    })
  }, [visibleAgents, locks])

  const activeProjectsCount = useMemo(
    () => new Set(activeAgents.map((a) => a.project)).size,
    [activeAgents]
  )
  const lockedFilesCount = useMemo(
    () => locks.reduce((acc, l) => acc + l.paths.length, 0),
    [locks]
  )
  const activeTasksCount = tasks.filter(
    (t) => t.status === 'claimed' || t.status === 'review' || t.status === 'blocked'
  ).length
  const blockerMessagesCount = messages.filter(
    (m) => m.channel === 'blocker' && nowSec - m.created_epoch < 7200
  ).length
  const blockedSessionsCount = activeAgents.filter((a) => a.status === 'blocked').length
  const totalBlockers = blockerMessagesCount + blockedSessionsCount
  const tasksExpanded =
    showTaskForm || (tasksDrawerOverride !== null ? tasksDrawerOverride : activeTasksCount > 0)

  return (
    <div className="max-w-[1440px] mx-auto px-4 sm:px-6 py-5 space-y-4">
      {/* Compact Plain-English Command & Summary Bar */}
      <div className="qp-card p-4 flex flex-col xl:flex-row xl:items-center xl:justify-between gap-3.5">
        <div className="space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-lg sm:text-xl font-extrabold tracking-tight text-slate-100">
              {activeAgents.length} Active {activeAgents.length === 1 ? 'Session' : 'Sessions'} Across{' '}
              {activeProjectsCount} {activeProjectsCount === 1 ? 'Project' : 'Projects'}
            </h1>

            {totalBlockers > 0 ? (
              <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-semibold bg-rose-500/15 text-rose-300 border border-rose-500/30">
                <span className="size-2 rounded-full bg-rose-400 animate-pulse" />
                {totalBlockers} {totalBlockers === 1 ? 'Blocker Needs' : 'Blockers Need'} Attention
              </span>
            ) : lockedFilesCount > 0 ? (
              <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-semibold bg-amber-400/15 text-amber-300 border border-amber-400/30">
                <span className="size-2 rounded-full bg-amber-400 animate-pulse" />
                {lockedFilesCount} {lockedFilesCount === 1 ? 'file' : 'files'} being edited right now
              </span>
            ) : (
              <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-semibold bg-emerald-500/15 text-emerald-300 border border-emerald-500/25">
                <span className="size-2 rounded-full bg-emerald-400" />
                No blockers or file conflicts
              </span>
            )}

            <span className="text-[11px] font-mono text-slate-500">
              · live ({Math.max(0, Math.floor((Date.now() - lastRefreshed) / 1000))}s ago)
            </span>
          </div>

          <p className="text-xs text-slate-400">
            Live view of what every AI coding session is building right now, grouped by project.
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* Project filter */}
          <div className="flex items-center gap-1.5 bg-[#0B0E14] border border-white/[0.08] rounded-xl px-2.5 py-1.5">
            <span className="text-[10px] font-mono uppercase tracking-wider text-slate-400">
              View
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
              <option value="all" className="bg-[#131822] text-slate-100">
                All Projects ({board?.projects?.length ?? 0})
              </option>
              {(board?.projects ?? []).map((proj) => (
                <option key={proj} value={proj} className="bg-[#131822] text-slate-100">
                  {proj}
                </option>
              ))}
            </select>
          </div>

          {/* Toggle idle / >2h sessions */}
          {inactiveAgents.length > 0 && (
            <button
              type="button"
              onClick={() => setShowInactive((v) => !v)}
              className={`px-2.5 py-1.5 rounded-xl border text-xs font-semibold transition-colors ${
                showInactive
                  ? 'bg-amber-400/15 border-amber-400/35 text-amber-300'
                  : 'bg-[#0B0E14] border-white/[0.08] text-slate-400 hover:text-slate-200'
              }`}
            >
              {showInactive
                ? `Hide ${inactiveAgents.length} older/idle`
                : `Show ${inactiveAgents.length} older/idle (>2h)`}
            </button>
          )}

          {/* Quick operator actions */}
          <button
            type="button"
            onClick={() => setShowLockForm((v) => !v)}
            className="px-2.5 py-1.5 rounded-xl bg-white/[0.05] hover:bg-white/[0.1] border border-white/[0.08] text-xs font-semibold text-amber-300 transition-colors"
          >
            {showLockForm ? 'Close Lock Form' : '+ Lock Files'}
          </button>

          <button
            type="button"
            onClick={() => {
              setShowTaskForm((v) => !v)
              setTasksDrawerOverride(true)
            }}
            className="px-2.5 py-1.5 rounded-xl bg-white/[0.05] hover:bg-white/[0.1] border border-white/[0.08] text-xs font-semibold text-sky-300 transition-colors"
          >
            {showTaskForm ? 'Close Task Form' : '+ New Task'}
          </button>

          <button
            type="button"
            onClick={() => setShowSetupDrawer((v) => !v)}
            className="px-2.5 py-1.5 rounded-xl qp-surface-amber text-xs font-bold text-amber-300 transition-colors"
          >
            {showSetupDrawer ? 'Hide Setup' : 'Setup'}
          </button>
        </div>
      </div>

      {/* Collapsible CLI / Hooks / MCP Quick-Start Guide */}
      {showSetupDrawer && (
        <div className="qp-card p-4 border border-amber-400/25 space-y-3">
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
                Automatically locks files while an agent edits them and warns other worktrees before conflicts happen.
              </p>
            </div>
            <div className="p-3 rounded-xl bg-[#0B0E14] border border-white/[0.06] space-y-1.5">
              <div className="font-bold text-slate-200">2. Register MCP Server</div>
              <pre className="font-mono text-[11px] text-emerald-300 overflow-x-auto">
                {`{ "mcpServers": { "daybook-board": { "command": "daybook", "args": ["board", "mcp"] } } }`}
              </pre>
              <p className="text-[11px] text-slate-400">
                Gives agents native tools to check status, lock files, claim tasks, and post updates.
              </p>
            </div>
            <div className="p-3 rounded-xl bg-[#0B0E14] border border-white/[0.06] space-y-1.5">
              <div className="font-bold text-slate-200">3. CLI Coordination</div>
              <pre className="font-mono text-[11px] text-sky-300 overflow-x-auto">
                {`daybook board status\ndaybook board claim "src/server/**"\ndaybook board post -c handoff "@t3-a1 done"`}
              </pre>
              <p className="text-[11px] text-slate-400">
                Every worktree automatically maps back to its parent repository.
              </p>
            </div>
          </div>
        </div>
      )}

      {actionError && (
        <div className="p-3 rounded-xl bg-rose-500/10 border border-rose-500/30 flex items-center justify-between gap-3 text-xs text-rose-200">
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

      {initialLoading ? (
        <div className="qp-card p-12 text-center text-sm text-slate-400 font-mono">
          Loading coordination board...
        </div>
      ) : (
        <div className="grid grid-cols-1 lg:grid-cols-12 gap-4 items-start">
          {/* LEFT 8 COLUMNS: Project-Grouped Live Sessions & Inline File Edits */}
          <div className="lg:col-span-8 space-y-3">
            {projectGroups.length === 0 ? (
              <div className="qp-card p-8 text-center space-y-2">
                <div className="text-sm font-semibold text-slate-200">
                  No active sessions in the last 2 hours
                </div>
                <p className="text-xs text-slate-400">
                  {inactiveAgents.length > 0
                    ? `There are ${inactiveAgents.length} older or idle sessions hidden.`
                    : 'Sessions will appear here automatically as soon as work begins in any repository.'}
                </p>
                {inactiveAgents.length > 0 && !showInactive && (
                  <button
                    type="button"
                    onClick={() => setShowInactive(true)}
                    className="mt-2 px-3 py-1.5 rounded-xl bg-white/[0.06] hover:bg-white/[0.1] text-xs font-semibold text-amber-300"
                  >
                    Show {inactiveAgents.length} older/idle sessions
                  </button>
                )}
              </div>
            ) : (
              <div className="grid grid-cols-1 xl:grid-cols-2 gap-3.5 items-start">
                {projectGroups.map((group) => (
                  <section
                    key={group.project}
                    className="qp-card p-3.5 space-y-2.5 border border-white/[0.07]"
                  >
                    {/* Project Group Header */}
                    <div className="flex items-center justify-between gap-2 pb-2 border-b border-white/[0.06]">
                      <div className="flex items-center gap-2 min-w-0">
                        <h2 className="text-sm font-bold text-slate-100 truncate">
                          {group.project}
                        </h2>
                        <span className="px-2 py-0.5 rounded-full bg-white/[0.05] text-[11px] font-mono text-slate-300">
                          {group.sessions.length}{' '}
                          {group.sessions.length === 1 ? 'session' : 'sessions'}
                        </span>
                      </div>

                      <div className="flex items-center gap-1.5 shrink-0">
                        {group.totalLocks > 0 && (
                          <span className="px-2 py-0.5 rounded-full bg-amber-400/15 border border-amber-400/30 text-[11px] font-semibold text-amber-300">
                            Editing {group.totalLocks} {group.totalLocks === 1 ? 'file' : 'files'}
                          </span>
                        )}
                      </div>
                    </div>

                    {/* Standalone / manual locks not attached to a visible session */}
                    {group.orphanLocks.map((lock) => (
                      <div
                        key={lock.id}
                        className="p-2.5 rounded-xl bg-amber-400/10 border border-amber-400/25 space-y-1.5"
                      >
                        <div className="flex items-center justify-between gap-2 text-xs">
                          <span className="font-semibold text-amber-200">
                            Locked by @{lock.agent} · {formatRemainingTtl(lock.expires_epoch)}
                          </span>
                          <button
                            type="button"
                            onClick={() => handleReleaseLock(lock)}
                            className="px-2 py-0.5 rounded bg-rose-500/20 hover:bg-rose-500/30 text-rose-200 text-[10px] font-mono font-semibold"
                          >
                            Release
                          </button>
                        </div>
                        <div className="flex flex-wrap gap-1">
                          {lock.paths.map((p) => (
                            <code
                              key={p}
                              title={p}
                              className="px-1.5 py-0.5 rounded bg-[#0B0E14]/80 text-[11px] font-mono text-amber-200"
                            >
                              {shortPathLabel(p)}
                            </code>
                          ))}
                        </div>
                      </div>
                    ))}

                    {/* Session Rows inside this Project */}
                    <div className="space-y-2">
                      {group.sessions.map((ag) => {
                        const active = isSessionActive(ag)
                        const sessionLocks = locksByAgentKey.get(`${ag.project}:${ag.agent}`) ?? []
                        const lockedPaths = Array.from(
                          new Set(sessionLocks.flatMap((l) => l.paths))
                        )
                        const humanTitle = formatCurrentTask(
                          ag.project,
                          ag.current_task,
                          ag.branch
                        )

                        return (
                          <div
                            key={`${ag.project}:${ag.agent}`}
                            className={`p-2.5 rounded-xl bg-[#0B0E14]/90 border transition-colors space-y-1.5 ${
                              lockedPaths.length > 0
                                ? 'border-amber-400/30'
                                : ag.status === 'blocked'
                                  ? 'border-rose-500/35'
                                  : 'border-white/[0.06]'
                            } ${!active ? 'opacity-60' : ''}`}
                          >
                            {/* Line 1: Plain-English Task Title + Status Badge */}
                            <div className="flex items-start justify-between gap-2">
                              <div className="flex items-start gap-2 min-w-0">
                                <span
                                  className={`mt-1.5 size-2 rounded-full shrink-0 ${
                                    ag.status === 'blocked'
                                      ? 'bg-rose-400 animate-pulse'
                                      : lockedPaths.length > 0
                                        ? 'bg-amber-400 animate-pulse'
                                        : active
                                          ? 'bg-emerald-400'
                                          : 'bg-slate-600'
                                  }`}
                                />
                                <span className="text-[13px] font-bold text-slate-100 leading-snug break-words">
                                  {humanTitle}
                                </span>
                              </div>

                              <div className="flex items-center gap-1.5 shrink-0">
                                {ag.status === 'blocked' ? (
                                  <span className="px-2 py-0.5 rounded-full bg-rose-500/15 text-rose-300 text-[10px] font-semibold">
                                    Blocked
                                  </span>
                                ) : lockedPaths.length > 0 ? (
                                  <span className="px-2 py-0.5 rounded-full bg-amber-400/15 text-amber-300 text-[10px] font-semibold">
                                    Editing {lockedPaths.length}{' '}
                                    {lockedPaths.length === 1 ? 'file' : 'files'}
                                  </span>
                                ) : active ? (
                                  <span className="px-2 py-0.5 rounded-full bg-emerald-500/10 text-emerald-300 text-[10px] font-semibold">
                                    Online
                                  </span>
                                ) : (
                                  <span className="px-2 py-0.5 rounded-full bg-white/[0.05] text-slate-400 text-[10px] font-mono">
                                    {ag.status === 'idle' ? 'Idle' : formatRelativeSeconds(ag.last_seen_epoch)}
                                  </span>
                                )}
                              </div>
                            </div>

                            {/* Inline Active File Edits (if any locks held by this session) */}
                            {sessionLocks.length > 0 && (
                              <div className="pl-4 flex flex-wrap items-center justify-between gap-1.5 pt-0.5">
                                <div className="flex flex-wrap items-center gap-1 min-w-0">
                                  <span className="text-[10px] font-mono text-amber-300/90">
                                    Files:
                                  </span>
                                  {lockedPaths.slice(0, 3).map((p) => (
                                    <code
                                      key={p}
                                      title={p}
                                      className="px-1.5 py-0.5 rounded bg-amber-400/10 border border-amber-400/20 text-[10px] font-mono text-amber-200 truncate max-w-[180px]"
                                    >
                                      {shortPathLabel(p)}
                                    </code>
                                  ))}
                                  {lockedPaths.length > 3 && (
                                    <span
                                      title={lockedPaths.slice(3).join('\n')}
                                      className="text-[10px] font-mono text-amber-300"
                                    >
                                      +{lockedPaths.length - 3} more
                                    </span>
                                  )}
                                </div>
                                <button
                                  type="button"
                                  onClick={() => handleReleaseAgentLocks(sessionLocks)}
                                  className="px-1.5 py-0.5 rounded bg-rose-500/15 hover:bg-rose-500/25 text-rose-300 text-[10px] font-mono"
                                >
                                  Unlock
                                </button>
                              </div>
                            )}

                            {/* Line 2: Helpful Technical Context (Runtime, Branch, @Handle, Relative Time) */}
                            <div className="pl-4 flex flex-wrap items-center justify-between gap-2 text-[11px] font-mono text-slate-400">
                              <div className="flex flex-wrap items-center gap-1.5 min-w-0">
                                <span className="px-1.5 py-0.5 rounded bg-white/[0.05] text-[10px] text-slate-300">
                                  {ag.runtime}
                                </span>
                                {ag.branch && ag.branch !== 'unknown' && (
                                  <span
                                    title={`Git branch: ${ag.branch}`}
                                    className="text-sky-300/90 truncate max-w-[190px]"
                                  >
                                    ⎇ {ag.branch}
                                  </span>
                                )}
                                <button
                                  type="button"
                                  title={
                                    ag.cwd
                                      ? `Click to mention @${ag.agent}\nWorktree: ${ag.cwd}`
                                      : `Click to mention @${ag.agent}`
                                  }
                                  onClick={() => handleMentionAgent(ag.project, ag.agent)}
                                  className="text-amber-300/90 hover:text-amber-200 hover:underline"
                                >
                                  @{ag.agent}
                                </button>
                              </div>

                              <span className="text-[10px] text-slate-500 shrink-0">
                                {formatRelativeSeconds(ag.last_seen_epoch)}
                              </span>
                            </div>
                          </div>
                        )
                      })}
                    </div>
                  </section>
                ))}
              </div>
            )}
          </div>

          {/* RIGHT 4 COLUMNS: Team Activity Dock (Forms, Collapsible Tasks, Message Stream) */}
          <div className="lg:col-span-4 space-y-3.5">
            {/* Inline Manual File Lock Drawer (shown when "+ Lock Files" clicked) */}
            {showLockForm && (
              <section className="qp-card p-3.5 space-y-2.5 border border-amber-400/30">
                <div className="flex items-center justify-between">
                  <h2 className="text-xs font-mono font-bold uppercase tracking-wider text-amber-300">
                    Lock Files Manually
                  </h2>
                  <button
                    type="button"
                    onClick={() => setShowLockForm(false)}
                    className="text-[11px] font-mono text-slate-400 hover:text-slate-200"
                  >
                    Close
                  </button>
                </div>
                <form onSubmit={handleClaimLock} className="space-y-2">
                  <input
                    type="text"
                    value={lockPathsInput}
                    onChange={(e) => setLockPathsInput(e.target.value)}
                    placeholder="Files or globs: src/server/**, src/main.rs"
                    className="w-full px-2.5 py-1.5 rounded-lg bg-[#0B0E14] border border-white/[0.08] text-xs font-mono text-slate-100"
                    required
                  />
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={lockNoteInput}
                      onChange={(e) => setLockNoteInput(e.target.value)}
                      placeholder="Reason (optional)"
                      className="flex-1 px-2.5 py-1.5 rounded-lg bg-[#0B0E14] border border-white/[0.08] text-xs text-slate-100"
                    />
                    <input
                      type="number"
                      min={1}
                      max={240}
                      value={lockTtlInput}
                      onChange={(e) => setLockTtlInput(Number(e.target.value) || 30)}
                      aria-label="Lock TTL minutes"
                      className="w-20 px-2 py-1.5 rounded-lg bg-[#0B0E14] border border-white/[0.08] text-xs font-mono text-slate-100"
                    />
                  </div>
                  {activeProject === 'all' && (board?.projects?.length ?? 0) > 0 && (
                    <div className="flex items-center justify-between gap-2 text-xs font-mono text-slate-400">
                      <span>Target Project:</span>
                      <select
                        value={writeProject}
                        onChange={(e) => setTargetProject(e.target.value)}
                        aria-label="Select project for lock"
                        className="px-2 py-1 rounded-lg bg-[#131822] border border-white/[0.08] text-xs font-mono text-amber-300"
                      >
                        {(board?.projects ?? []).map((proj) => (
                          <option key={proj} value={proj}>
                            {proj}
                          </option>
                        ))}
                      </select>
                    </div>
                  )}
                  <button
                    type="submit"
                    className="w-full py-1.5 rounded-lg bg-amber-400 text-slate-950 font-bold text-xs hover:bg-amber-300 transition-colors"
                  >
                    Lock in {writeProject || 'project'} ({lockTtlInput}m)
                  </button>
                </form>
              </section>
            )}

            {/* Shared Tasks Section (Compact 1-line bar when 0 tasks, expandable on click) */}
            <section className="qp-card p-3.5 space-y-3">
              <div className="flex items-center justify-between gap-2">
                <button
                  type="button"
                  onClick={() => {
                    setShowTaskForm(false)
                    setTasksDrawerOverride(!tasksExpanded)
                  }}
                  className="flex items-center gap-2 text-left"
                >
                  <h2 className="text-xs font-mono font-bold uppercase tracking-wider text-sky-300">
                    Shared Tasks ({activeTasksCount} active)
                  </h2>
                  <span className="text-[11px] font-mono text-slate-500">
                    {tasksExpanded ? '▾' : '▸'}
                  </span>
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setShowTaskForm((v) => !v)
                    setTasksDrawerOverride(true)
                  }}
                  className="px-2 py-0.5 rounded-lg bg-sky-500/15 hover:bg-sky-500/25 border border-sky-500/30 text-[11px] font-mono font-bold text-sky-300"
                >
                  {showTaskForm ? 'Cancel' : '+ Claim Task'}
                </button>
              </div>

              {tasksExpanded && (
                <>
                  {showTaskForm && (
                    <form
                      onSubmit={handleClaimTask}
                      className="p-3 rounded-xl bg-[#0B0E14] border border-white/[0.08] space-y-2"
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
                        placeholder="Plain-English task description"
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
                          placeholder="Note (optional)"
                          className="w-1/2 px-2.5 py-1.5 rounded-lg bg-[#131822] border border-white/[0.08] text-xs text-slate-100"
                        />
                      </div>
                      {activeProject === 'all' && (board?.projects?.length ?? 0) > 0 && (
                        <div className="flex items-center justify-between gap-2 text-xs font-mono text-slate-400">
                          <span>Target Project:</span>
                          <select
                            value={writeProject}
                            onChange={(e) => setTargetProject(e.target.value)}
                            aria-label="Select project for task"
                            className="px-2 py-1 rounded-lg bg-[#131822] border border-white/[0.08] text-xs font-mono text-sky-300"
                          >
                            {(board?.projects ?? []).map((proj) => (
                              <option key={proj} value={proj}>
                                {proj}
                              </option>
                            ))}
                          </select>
                        </div>
                      )}
                      <button
                        type="submit"
                        className="w-full py-1.5 rounded-lg bg-sky-400 text-slate-950 font-bold text-xs hover:bg-sky-300 transition-colors"
                      >
                        Claim Task in {writeProject || 'project'}
                      </button>
                    </form>
                  )}

                  {tasks.length > 0 && (
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
                  )}

                  {filteredTasks.length === 0 ? (
                    <div className="p-4 rounded-xl bg-[#0B0E14]/60 border border-white/[0.04] text-center text-xs text-slate-500">
                      No formal tasks claimed yet. Session titles appear automatically on the left.
                    </div>
                  ) : (
                    <div className="space-y-2">
                      {filteredTasks.map((task) => (
                        <div
                          key={`${task.project}:${task.task_id}`}
                          className="p-3 rounded-xl bg-[#0B0E14]/90 border border-white/[0.07] space-y-1.5"
                        >
                          <div className="flex items-start justify-between gap-2">
                            <div className="space-y-1 min-w-0">
                              <div className="flex flex-wrap items-center gap-1.5">
                                <span className="px-1.5 py-0.5 rounded bg-violet-500/15 border border-violet-500/25 text-[10px] font-mono font-semibold text-violet-300">
                                  {task.project}
                                </span>
                                <span className="px-1.5 py-0.5 rounded bg-white/[0.07] font-mono text-[11px] font-bold text-slate-200">
                                  {task.task_id}
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
                </>
              )}
            </section>

            {/* Team Messages & Handoffs Stream */}
            <section className="qp-card p-3.5 space-y-3">
              <div className="flex items-center justify-between">
                <div>
                  <h2 className="text-xs font-mono font-bold uppercase tracking-wider text-slate-200">
                    Team Updates & Handoffs ({filteredMessages.length})
                  </h2>
                  <p className="text-[11px] text-slate-400">
                    Click any @agent on the left to mention them here
                  </p>
                </div>
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

              {/* Post Message Form with integrated Project + Handle selectors */}
              <form
                onSubmit={handlePostMessage}
                className="p-2.5 rounded-xl bg-[#0B0E14] border border-white/[0.08] space-y-2"
              >
                <div className="flex flex-wrap items-center gap-1.5 text-[11px] font-mono">
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

                  {activeProject === 'all' && (board?.projects?.length ?? 0) > 0 && (
                    <select
                      value={writeProject}
                      onChange={(e) => setTargetProject(e.target.value)}
                      aria-label="Select target project for writes"
                      className="px-2 py-1 rounded-lg bg-[#131822] border border-white/[0.08] text-xs font-mono text-violet-300"
                    >
                      {(board?.projects ?? []).map((proj) => (
                        <option key={proj} value={proj} className="bg-[#131822] text-slate-100">
                          in {proj}
                        </option>
                      ))}
                    </select>
                  )}

                  <div className="flex items-center gap-1 bg-[#131822] border border-white/[0.08] rounded-lg px-2 py-1 ml-auto">
                    <span className="text-slate-500">@</span>
                    <input
                      type="text"
                      value={operatorHandle}
                      onChange={(e) => handleOperatorChange(e.target.value)}
                      placeholder="operator"
                      aria-label="Agent handle"
                      className="w-20 bg-transparent text-xs font-mono text-slate-200 focus:outline-none"
                    />
                  </div>
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
                <div className="p-6 rounded-xl bg-[#0B0E14]/60 border border-white/[0.04] text-center text-xs text-slate-500">
                  No messages in this channel yet.
                </div>
              ) : (
                <div className="space-y-2 max-h-[520px] overflow-y-auto pr-1">
                  {[...filteredMessages].reverse().map((m: BoardMessage) => (
                    <div
                      key={m.id}
                      className="p-2.5 rounded-xl bg-[#0B0E14]/90 border border-white/[0.07] space-y-1.5"
                    >
                      <div className="flex items-center justify-between gap-2 text-[10px] font-mono">
                        <div className="flex flex-wrap items-center gap-1.5 min-w-0">
                          <span className="px-1.5 py-0.5 rounded bg-violet-500/15 border border-violet-500/25 text-[10px] font-mono font-semibold text-violet-300">
                            {m.project}
                          </span>
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
                          {(() => {
                            const msgAgent = agentMetaMap.get(`${m.project}:${m.agent}`)
                            const msgAgentTitle = msgAgent?.current_task
                            return msgAgentTitle ? (
                              <span className="text-[11px] font-sans font-semibold text-slate-300 truncate">
                                · {formatCurrentTask(m.project, msgAgentTitle, msgAgent?.branch)}
                              </span>
                            ) : null
                          })()}
                        </div>
                        <span className="text-slate-500 shrink-0">
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
