# Daybook Agent Board (`daybook board`) — Reverse-Engineered Architecture & Implementation Plan

## Human Plan

### Goal
Build a first-class multi-agent coordination board (`daybook board`) inside Daybook in Rust by reverse-engineering and combining the best primitives from [`jalemieux/bot_board`](https://github.com/jalemieux/bot_board) (channels, threaded messages, `@mentions`, cursor polling, agent roster, task claiming, self-describing `agents.md`) and [`ThatHunky/agent-coord`](https://github.com/ThatHunky/agent-coord) (TTL file-glob locks, overlap detection, auto-claim hooks, and zero-dependency stdio MCP tools), so T3 Code, Pi, Claude Code, and Codex agents across worktrees and machines can coordinate without stepping on each other.

### Scope
1. **SQLite Board Storage (`src/ledger/board.rs` + `src/ledger/mod.rs`)**:
   - Extend Daybook's existing SQLite ledger (`src/ledger/mod.rs`, which already enables `WAL` mode and `busy_timeout = 5000`) with 4 tables: `board_agents`, `board_locks`, `board_tasks`, and `board_messages`.
   - Enforce bounded table sizes (`MAX_LOCKS_PER_PROJECT = 200`, `MAX_LOCKS_PER_MACHINE_PROJECT = 50`, `MAX_AUTO_LOCKS_PER_AGENT = 20`, `MAX_TASKS_PER_PROJECT = 500`, `MAX_AGENTS_PER_PROJECT = 100`, `MAX_AGENTS_PER_MACHINE_PROJECT = 25`, `MAX_BOARD_MESSAGES = 500`, `MAX_BOARD_PROJECTS = 100`) and TTL lock expiration pruning inside write transactions.
2. **Worktree-Aware Project & Agent Identity (`src/board/identity.rs`)**:
   - Extract and reuse Daybook's worktree-aware project resolver (`src/transcript/adapters.rs`) so `.t3/worktrees/<project>/<branch>`, linked `.git` file worktrees, and main repo checkouts all resolve to the same canonical `project` name, plus a worktree-root resolver (`find_worktree_root`) to convert absolute file paths into repo-relative paths safely.
   - Deterministic, lowercase-normalized agent handle resolution (`--agent` → `DAYBOOK_AGENT` / `AGENT_NAME` → `<runtime>-<worktree_or_branch>` → `<runtime>-<machine>`, matching `[a-z0-9][a-z0-9_-]{0,63}`).
3. **File-Glob Lock Engine (`src/board/globs.rs`)**:
   - Strict repo-relative glob normalization (rejecting leading `/`, `..` traversal segments, >16 path segments, or >2 `**` wildcards) and bounded glob overlap matching (`*`, `?`, `**`, and directory-prefix containment).
4. **Dual-Mode Client & Bearer-Authenticated Server HTTP API (`src/board/client.rs`, `src/server/router.rs`, `src/server/handlers.rs`, `src/server/dto.rs`)**:
   - All `/api/board/*` routes (both read and write) sit behind Daybook's existing `auth_middleware` (`Bearer dbk_...` machine token or `server.admin_token`), preserving Daybook's S5 authentication model and working seamlessly with `web/src/hooks/useApi.ts`.
   - Machine-token containment: non-admin tokens (`Authed { label, admin: false }`) are bound to their `machine = label`, preventing one machine's token from impersonating an agent handle registered to another machine or releasing another machine's locks.
   - Strict mode selection (no split-brain): when `push.url` is configured (and `--local` is not passed), `BoardClient` operates strictly against the remote server (`/api/board/*`) and never silently falls back to the local SQLite database; when `push.url` is unconfigured (or `--local` is passed), `BoardClient` operates directly against `config.server.db_path` (default `~/.daybook/ledger.db3`).
5. **Agent Interfaces — CLI, Built-in stdio MCP Server, Hooks, and Skill (`src/cli/commands/board.rs`, `src/board/mcp.rs`, `src/hooks/`)**:
   - `daybook board` CLI subcommands: `status`, `post`, `messages`, `claim`, `release`, `check`, `task`, `heartbeat`, `mcp`, `skill`.
   - `daybook board mcp`: Zero-dependency JSON-RPC 2.0 stdio MCP server exposing `board_status`, `board_post`, `board_claim_paths`, `board_release_paths`, `board_check_path`, `board_claim_task`, and `board_update_task`.
   - Hook handlers: `daybook hook pre-tool-use` (advisory by default: warns to stderr when a repo-relative file is locked by another agent; blocks with exit code 2 and fails closed on unreachable remote server when `DAYBOOK_STRICT_LOCKS=1` or `DAYBOOK_LOCK_ENFORCE=strict`), `daybook hook post-tool-use` (auto-touches a 30m lock on the edited repo-relative file and surfaces unread `@mentions`), plus board briefing in `session-start` and auto-lock cleanup in `session-end`.
6. **Quiet Precision Web UI (`web/src/pages/Board.tsx`)**:
   - Add a **Board** navigation pill in `web/src/components/Layout.tsx` (`/board`, updating `web/src/i18n/en.json` and `web/src/i18n/zh.json`) showing project selector, live agent roster, active file locks with TTL countdowns, task claims board, and channel message feed (`#general`, `#handoffs`, `#blockers`, `#decisions`).

### Non-goals
- No VM fleet provisioning, container orchestration, or remote daemon spawning (`bot_board`'s `/api/fleet/*` is skipped).
- No secret vault or TTS/voice synthesis (`bot_board`'s `/api/fleet/secrets` and `/voice` are skipped).
- No hard OS-level mandatory file locking; locks are advisory at `claim` time and enforced at agent tool/hook boundaries (`pre-tool-use` / `board_check_path`).
- No WebSocket/SSE server push in v1; incremental `since_id` cursor polling (`GET /api/board/status?project=...&since_id=...`) and 5-second dashboard polling cover real-time needs with zero stateful connection complexity.

### Behavior
- **Cross-Worktree Unification**: Two agents running in `/home/agent/.t3/worktrees/daybook/t3-fb61e20a` and `/home/agent/.t3/worktrees/daybook/t3-99a1b2c3` both resolve to project `daybook`, see each other in `board_agents`, and share the `daybook` lock table, task list, and message channels.
- **Advisory Claim + Pre-Edit Guard**:
  - Calling `daybook board claim "src/ledger/**" -m "refactoring schema"` creates a TTL lock (default 60 minutes, server-clock epoch) and returns any overlapping locks held by other agents in `conflicts` so the caller can coordinate.
  - Calling `daybook board check src/ledger/mod.rs` when another agent holds an active overlapping lock exits with code `2` and prints the holder handle, note, and expiry. Running `daybook hook pre-tool-use` warns by default (`exit 0`) and blocks (`exit 2`) when `DAYBOOK_STRICT_LOCKS=1` or `DAYBOOK_LOCK_ENFORCE=strict`. Files outside the current git/worktree root are ignored by `pre-tool-use`/`post-tool-use`.
  - Editing a repo file through `daybook hook post-tool-use` creates or refreshes a 30-minute `auto = true` lock on that exact repo-relative path, surfaces unread `@mentions`, and automatically releases auto locks on `session-end`.
- **Task Claiming & Handoffs**:
  - Calling `daybook board task claim T-101 --title "Add SQLite board tables" --note "working in t3-fb61e20a"` atomically claims `T-101` for the calling agent. If another agent already holds `T-101` in `claimed` status, the call returns `{ ok: false, held_by: "<other-agent>", task }` with non-zero CLI exit unless `--force` is set (and permitted by machine/admin ownership).
  - Calling `daybook board task update T-101 --status done --note "Merged in PR #6"` marks the task `done`.
- **Channels, Threading, and `@mentions`**:
  - Calling `daybook board post "Ready for review on @pi-t3-99a1b2c3" --channel handoffs` stores the message in channel `handoffs`, extracts lowercase `["pi-t3-99a1b2c3"]` into `mentions_json`, and assigns a monotonic `id`.
  - Subsequent `daybook board status` or `daybook board messages --since-id <N>` calls return only messages with `id > N`, highlight unread `@mentions` for the current agent, and return `next_since_id = max(since_id, max_id_in_project)` so polling cursors never reset on empty responses.

### Acceptance criteria
1. `cargo test` passes unit and integration tests covering:
   - Worktree-aware project resolution (`.t3/worktrees/<repo>/<id>`, `.git` file pointers, normal repos) and `find_worktree_root` repo-relative path stripping (including rejecting paths outside the worktree).
   - Glob normalization (rejecting leading `/`, `..`, >16 segments, >2 `**`) and wildcard/prefix overlap detection (`*`, `?`, `**`, directory prefixes).
   - SQLite `board_agents`, `board_locks` (TTL filtering on read, write-path pruning, conflict detection, auto-lock touch, ownership-checked release), `board_tasks` (atomic claim conflict, force override, status transitions), `board_messages` (channels, `parent_id` validation, lowercase `@mention` extraction, `since_id` / `next_since_id` cursor stability), and cross-connection concurrency (`Ledger::open` on the same DB file).
   - HTTP `/api/board/*` endpoints: 401 rejection without bearer token, machine-token anti-impersonation (machine A cannot spoof machine B's agent or release machine B's locks), `pr_url` scheme validation (`http://`/`https://` only), and pure read-only `GET /api/board/status`.
   - `BoardClient` local mode vs remote HTTP mode (including fail-closed error propagation when remote server is unreachable, with no silent fallback to local DB).
   - JSON-RPC 2.0 stdio MCP server (`initialize`, `tools/list`, `tools/call` for all 7 tools with both newline-delimited and `Content-Length` framing) and hook handlers (`pre-tool-use` advisory warning + `DAYBOOK_STRICT_LOCKS=1` exit code 2 on conflict, `post-tool-use` auto-lock + mention surfacing, `session-end` auto-lock release).
2. `cargo clippy -- -D warnings` and `cargo fmt --all -- --check` pass with zero warnings.
3. `cd web && npm run build` succeeds with the new `/board` page integrated into `Layout.tsx`, `App.tsx`, `en.json`, and `zh.json`.
4. `README.md` and `design.md` are updated to document `daybook board`, MCP configuration, hooks, and the Board UI view.

### Out-of-scope follow-ups
- Server-Sent Events (SSE) live stream for sub-second push notifications.
- Cross-project global dependency locks (v1 scopes locks, tasks, and channels per canonical `project`).
- GitHub webhook auto-closing of board tasks on PR merge.

---

## Agent Plan

### 1. Reverse-Engineered Reference Architecture & Key Improvements

| Capability | `jalemieux/bot_board` (Python/FastAPI) | `ThatHunky/agent-coord` (Node.js) | Daybook `board` (Rust/Axum/SQLite) |
| :--- | :--- | :--- | :--- |
| **Storage & Concurrency** | SQLite (`WAL` + `busy_timeout=5000`, thread-local connections) | `~/.agent-coord/board.json` + busy-wait `fs.mkdirSync("board.lock")` spinloop | Existing `Ledger` SQLite (`src/ledger/mod.rs:514-517`) with `WAL`, `busy_timeout=5000`, and ACID write transactions |
| **Project Resolution** | Manual `workspace` registration | `basename(git rev-parse --show-toplevel)` — **broken on T3 worktrees** (returns `t3-fb61e20a` instead of `daybook`) | Shared `resolve_project_from_path()` (`src/transcript/adapters.rs:45-100`) resolving `.t3/worktrees/<project>/<branch>` and `.git` file pointers to canonical repo name |
| **Agent Identity & Auth** | Registered handle + bearer token (`bb_...`) | `AGENT_NAME` or `agent_${process.ppid}` — **unstable across CLI subshells, no auth** | Lowercase `[a-z0-9][a-z0-9_-]{0,63}` handle (`<runtime>-<worktree_or_branch>`) bound to authenticated `machine` token (`dbk_...`) |
| **File Locking** | None | `claim_paths`, `release_paths`, `check_path`, `touchPath` (auto-claim on edit), custom `globsOverlap` | `board_locks` table + `src/board/globs.rs` with path traversal guard, bounded DP glob overlap, and ownership-checked release |
| **Tasks** | Channels + posts only | `claim_task` / `complete_task` (flat `claimed`/`done` status) | `board_tasks` table with `open` \| `claimed` \| `review` \| `blocked` \| `done`, priority, `branch`, validated `pr_url`, atomic claim check |
| **Messaging** | Channels, `parent_id` threads, `@mentions`, `next_since` cursor | Flat array of last 100 strings (`messages.slice(-100)`) | `board_messages` table with `channel`, `parent_id`, lowercase `mentions_json`, stable `next_since_id` cursor, and 500-message rolling window |
| **Transport** | Central HTTP server only | Local JSON file only | **Explicit Dual-Mode `BoardClient`**: Remote HTTP `/api/board/*` when `push.url` is set; Local `config.server.db_path` (`~/.daybook/ledger.db3`) when `push.url` is unset (no silent split-brain fallback) |
| **Agent Tooling** | REST API + `/agents.md` prompt | Zero-dep stdio MCP server + Claude Code `PreToolUse`/`PostToolUse` hooks | Built-in Rust stdio MCP server (`daybook board mcp`), CLI (`daybook board ...`), hooks (`pre-tool-use`, `post-tool-use`, `session-start`, `session-end`), and `/api/board/agents.md` |

---

### 2. SQLite Schema & Storage Layer (`src/ledger/board.rs` & `src/ledger/mod.rs`)

`Ledger::open` (`src/ledger/mod.rs:514-517`) already sets `PRAGMA journal_mode=WAL` and `PRAGMA busy_timeout=5000;`. Append the 4 `board_*` tables and indexes to `SCHEMA` in `src/ledger/mod.rs`, using Unix epoch seconds (`INTEGER`, always computed from `Utc::now().timestamp()` on the ledger host) across all tables for consistent ordering and index scans:

```sql
CREATE TABLE IF NOT EXISTS board_agents (
    project          TEXT NOT NULL,
    agent            TEXT NOT NULL,
    machine          TEXT NOT NULL DEFAULT 'local',
    runtime          TEXT NOT NULL DEFAULT 'unknown',
    cwd              TEXT,
    branch           TEXT,
    status           TEXT NOT NULL DEFAULT 'active', -- 'active' | 'idle' | 'blocked' | 'offline'
    current_task     TEXT,
    last_seen_epoch  INTEGER NOT NULL,
    PRIMARY KEY (project, agent)
);
CREATE INDEX IF NOT EXISTS idx_board_agents_project ON board_agents(project, last_seen_epoch DESC);

CREATE TABLE IF NOT EXISTS board_locks (
    id               TEXT PRIMARY KEY,
    project          TEXT NOT NULL,
    agent            TEXT NOT NULL,
    machine          TEXT NOT NULL DEFAULT 'local',
    paths_json       TEXT NOT NULL,                  -- JSON Vec<String> of normalized repo-relative globs
    note             TEXT NOT NULL DEFAULT '',
    auto             INTEGER NOT NULL DEFAULT 0,     -- 1 = auto-claimed by post-tool-use hook, 0 = explicit claim
    created_epoch    INTEGER NOT NULL,               -- Server/ledger Utc::now().timestamp()
    expires_epoch    INTEGER NOT NULL                -- created_epoch + ttl_minutes * 60
);
CREATE INDEX IF NOT EXISTS idx_board_locks_project ON board_locks(project, expires_epoch);
CREATE INDEX IF NOT EXISTS idx_board_locks_expires ON board_locks(expires_epoch);

CREATE TABLE IF NOT EXISTS board_tasks (
    project          TEXT NOT NULL,
    task_id          TEXT NOT NULL,                  -- e.g. 'T-1', 'auth-refactor'
    title            TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'open',   -- 'open' | 'claimed' | 'review' | 'blocked' | 'done'
    priority         TEXT NOT NULL DEFAULT 'normal', -- 'low' | 'normal' | 'high' | 'critical'
    agent            TEXT,                           -- current holder handle when claimed/review/blocked
    machine          TEXT,                           -- machine of current holder
    branch           TEXT,
    pr_url           TEXT,
    note             TEXT NOT NULL DEFAULT '',
    created_epoch    INTEGER NOT NULL,
    updated_epoch    INTEGER NOT NULL,
    PRIMARY KEY (project, task_id)
);
CREATE INDEX IF NOT EXISTS idx_board_tasks_project ON board_tasks(project, status, updated_epoch DESC);

CREATE TABLE IF NOT EXISTS board_messages (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    project          TEXT NOT NULL,
    channel          TEXT NOT NULL DEFAULT 'general',-- 'general' | 'handoffs' | 'blockers' | 'decisions' | slug
    agent            TEXT NOT NULL,
    machine          TEXT NOT NULL DEFAULT 'local',
    body             TEXT NOT NULL,
    parent_id        INTEGER,                        -- soft reference; survives parent window pruning
    mentions_json    TEXT NOT NULL DEFAULT '[]',     -- JSON Vec<String> of lowercase @handles mentioned
    created_epoch    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_board_messages_project_id ON board_messages(project, id DESC);
```

#### Read-Path vs. Write-Path Invariants (`impl Ledger` in `src/ledger/board.rs`)
- **Pure Read Queries (`status`, `messages`, `check_path`)**:
  - `GET` handlers and read methods (`Ledger::board_status`, `Ledger::board_messages`, `Ledger::board_check_path`) execute **read-only `SELECT` queries** and never mutate or prune rows, preventing write-lock contention on polling.
  - Active locks are filtered in SQL via `WHERE project = ?1 AND expires_epoch > ?2` (`now_epoch`).
  - Stale agents (`status == "active"` with `now_epoch - last_seen_epoch > 1800`) are mapped to `status = "idle"` at read serialization time.
  - `next_since_id` is always computed as:
    ```rust
    let max_project_id: i64 = conn.query_row(
        "SELECT COALESCE(MAX(id), 0) FROM board_messages WHERE project = ?1",
        params![project],
        |r| r.get(0),
    )?;
    let next_since_id = since_id.unwrap_or(0).max(max_project_id);
    ```
    so polling with `since_id` when there are no new messages preserves the caller's cursor instead of resetting to `0`.
- **Write-Path Pruning & Caps (`prune_board_tx(tx: &Transaction, project: &str, now_epoch: i64)`)**:
  - Called inside the write transaction of every board mutation (`heartbeat`, `claim_locks`, `touch_lock`, `release_locks`, `claim_task`, `update_task`, `post_message`):
    1. **Project count cap (`MAX_BOARD_PROJECTS = 100`)**: If `project` does not already exist in `sessions`, `board_agents`, `board_locks`, `board_tasks`, or `board_messages`, verify total distinct board projects `< 100` before inserting.
    2. **Lock pruning & cap (`MAX_LOCKS_PER_PROJECT = 200`, `MAX_LOCKS_PER_MACHINE_PROJECT = 50`, `MAX_AUTO_LOCKS_PER_AGENT = 20`)**: `DELETE FROM board_locks WHERE expires_epoch <= ?1` (uses `idx_board_locks_expires`).
       - On `touch_lock`, if the calling `(project, agent)` already has `>= 20` active `auto = 1` locks, evict that same agent's oldest `auto = 1` lock first.
       - If the calling `(project, machine)` has `>= 50` active locks in `project`, evict that same machine's oldest `auto = 1` lock in `project` first; if all 50 locks on that machine are manual (`auto = 0`), reject the new lock claim.
       - If total active locks across all machines in `project` reaches `>= 200`, evict the oldest `auto = 1` lock in `project` first; if all 200 are manual (`auto = 0`), reject the new lock claim.
    3. **Agent roster cap (`MAX_AGENTS_PER_PROJECT = 100`, `MAX_AGENTS_PER_MACHINE_PROJECT = 25`)**: Prune agents in `project` whose `last_seen_epoch < now_epoch - 7 * 86400` AND who hold no active locks or claimed/review/blocked tasks in `project`. To prevent one machine from evicting another machine's agent handles (which would reopen handle impersonation), never evict live rows across machines: reject new agent handle creation when `(project, machine)` already has `>= 25` agent rows or `project` has `>= 100` agent rows.
    4. **Task cap (`MAX_TASKS_PER_PROJECT = 500`)**: Prune `status = 'done'` tasks in `project` older than 7 days (`updated_epoch < now_epoch - 7 * 86400`) when count exceeds 400; reject new task creation if active/open tasks in `project` >= 500.
    5. **Message rolling window (`MAX_BOARD_MESSAGES = 500`)**:
       ```sql
       DELETE FROM board_messages
       WHERE project = ?1
         AND id <= COALESCE(
             (SELECT id FROM board_messages WHERE project = ?1 ORDER BY id DESC LIMIT 1 OFFSET 500),
             0
         );
       ```
       `parent_id` is a soft reference (no `ON DELETE CASCADE`), so replies to messages that aged out of the 500-message window remain intact and display `"reply to #<parent_id>"`.
- **Anti-Impersonation & Machine Ownership Check**:
  - Inside `upsert_agent_tx(tx, project, agent, machine, allow_override)`:
    - Query `SELECT machine FROM board_agents WHERE project = ?1 AND agent = ?2`.
    - If a row exists with `existing_machine != machine` and `!allow_override` (where `allow_override` is `true` only for `authed.admin` or local CLI mode), return `BoardError::AgentOwnedByOtherMachine { agent, existing_machine }` (mapped to HTTP `403 Forbidden`).
  - Similarly, `release_locks` and `update_task` verify that the target lock/task is held by `(agent, machine)` unless `allow_override` (`authed.admin` or `force: true` on the same `machine`) is set.

---

### 3. Worktree-Aware Identity & Glob Engine (`src/board/identity.rs`, `src/board/globs.rs`)

#### `src/board/identity.rs`
- **Project Resolution (`resolve_project(explicit: Option<&str>, cwd: &Path) -> Result<String>`)**:
  - Promotes `resolve_git_project` and `.t3/worktrees/<project>/<branch>` logic in `src/transcript/adapters.rs:45-100` to `pub fn resolve_project_from_path(cwd: &Path) -> Option<String>`.
  - Validates the resolved or explicit project name via `validate_project_name(name: &str) -> Result<String>`: trimmed, 1..=128 chars, ASCII alphanumeric plus `-`, `_`, `.`, no `/` or `..`.
- **Worktree Root & Repo-Relative Path Resolution (`find_worktree_root(start: &Path) -> Option<PathBuf>` and `repo_rel_path(cwd: &Path, target: &Path) -> Option<String>`)**:
  - `find_worktree_root(start)` walks up from `start` to the nearest directory containing `.git` (either directory or worktree `.git` file).
  - `repo_rel_path(cwd, target)` resolves `target` against `cwd` (lexically cleaning `.` and `..` without allowing escape above root), checks `cleaned.strip_prefix(&worktree_root)`, and returns `Some(rel_slash_path)` **only** if `target` resides inside `worktree_root`. Paths outside the repo (e.g. `/etc/passwd`, `/tmp/foo`, `~/.daybook/...`) return `None` so hooks ignore them safely.
- **Branch Detection (`resolve_git_branch(cwd: &Path) -> Option<String>`)**:
  - Reads `.git/HEAD` (or follows `.git` file `gitdir:` pointer to `<gitdir>/HEAD`) and strips `ref: refs/heads/` without spawning a `git` subprocess.
- **Agent Handle Resolution (`resolve_agent_handle(explicit: Option<&str>, cwd: &Path, config: &Config) -> Result<String>`)**:
  1. `explicit` (from `--agent` or MCP tool argument) if non-empty.
  2. `DAYBOOK_AGENT` or `AGENT_NAME` env var if non-empty.
  3. Runtime prefix (`t3` if `T3_THREAD_ID` set, `pi` if `PI_SESSION_ID` set, `claude` if `CLAUDE_PROJECT_DIR` set, `codex` if `CODEX_SANDBOX` set, else `agent`) + `-` + worktree directory slug (when inside `/.t3/worktrees/<proj>/<wt>`) or sanitized branch name, falling back to `machine_label(config)`.
  - Normalized to lowercase `[a-z0-9][a-z0-9_-]{0,63}` (`validate_agent_handle`).

#### `src/board/globs.rs`
- **`normalize_glob(raw: &str) -> Result<String>`**:
  - Replaces `\` with `/`, trims whitespace, strips leading `./`, collapses `//`, strips trailing `/`.
  - Rejects empty strings, leading `/` (`"lock globs must be repo-relative, not absolute paths"`), any segment equal to `..` (`"path traversal '..' is not allowed"`), patterns longer than 512 chars, patterns with `> 16` `/` segments, or patterns containing `> 2` `**` segments.
- **`glob_matches_path(glob: &str, path: &str) -> bool`**:
  - Both inputs normalized.
  - Directory-prefix rule: if `glob` contains no wildcard characters (`*`, `?`) and `path.starts_with(&format!("{glob}/"))`, returns `true` (claiming `src/ledger` locks `src/ledger/mod.rs`).
  - Segment-by-segment iterative DP matching (`<= 16` segments) where `**` matches 0 or more segments, `*` matches 0 or more chars within a segment, and `?` matches 1 char within a segment.
- **`globs_overlap(a: &str, b: &str) -> bool`**:
  - Normalized `a` and `b`.
  - Directory-prefix expansion: any wildcard-free glob `g` (`!g.contains(['*', '?'])`) covers both `g` itself and its subtree `g/**`. Therefore `globs_overlap(a, b)` evaluates:
    ```rust
    raw_globs_overlap(a, b)
        || (!a.contains(['*', '?']) && raw_globs_overlap(&format!("{a}/**"), b))
        || (!b.contains(['*', '?']) && raw_globs_overlap(a, &format!("{b}/**")))
        || (!a.contains(['*', '?']) && !b.contains(['*', '?']) && raw_globs_overlap(&format!("{a}/**"), &format!("{b}/**")))
    ```
    so `globs_overlap("src/**/mod.rs", "src/ledger")` and `globs_overlap("src/ledger", "src/**/mod.rs")` both symmetrically return `true`.
  - `raw_globs_overlap(x, y)` runs a simultaneous 2D DP automaton over the segments of `x` and `y` (`<= 17` segments including any expanded `/**`, so at most `18 x 18 = 324` states), where `**` matches 0 or more segments on the other side and two single-segment patterns `seg_x`, `seg_y` match iff their character-level wildcard automata (`*`, `?`) intersect (`segment_patterns_overlap(seg_x, seg_y)`). This is exact, symmetric, deterministic, and worst-case $< 50\mu\text{s}$ with zero heuristic sample expansion.

---

### 4. Server HTTP API (`src/server/router.rs`, `src/server/handlers.rs`, `src/server/dto.rs`)

#### Route Registration (`src/server/router.rs`)
All `/api/board/*` routes are registered inside the existing `authed` router (`src/server/router.rs:14-36`), requiring a valid bearer token (`dbk_...` machine token or `server.admin_token`) via `auth_middleware`:
- **Read Routes (Pure `SELECT`, no DB writes)**:
  - `GET /api/board/status` — Query params: `project: Option<String>`, `agent: Option<String>`, `since_id: Option<i64>`. Returns `ApiResponse<BoardStatusDto>`:
    - `projects: Vec<String>` (distinct projects present in `board_*` tables)
    - `project: Option<String>`
    - `agents: Vec<BoardAgentDto>`
    - `locks: Vec<BoardLockDto>`
    - `tasks: Vec<BoardTaskDto>`
    - `messages: Vec<BoardMessageDto>`
    - `unread_mentions: Vec<BoardMessageDto>` (messages in the window mentioning `agent` in `mentions_json`)
    - `next_since_id: i64` (`max(since_id.unwrap_or(0), max_message_id_in_project)`)
  - `GET /api/board/messages` — Query params: `project: String`, `channel: Option<String>`, `mention: Option<String>`, `since_id: Option<i64>`, `limit: Option<usize>` (clamped to `1..=200`, default `100`). Returns `ApiResponse<BoardMessagesResponseDto>` (`{ messages: Vec<BoardMessageDto>, next_since_id: i64 }`).
  - `GET /api/board/check` — Query params: `project: String`, `path: String`, `agent: Option<String>`. Returns `ApiResponse<BoardCheckDto>` (`{ blocked: bool, blocking_locks: Vec<BoardLockDto> }`).
  - `GET /api/board/agents.md` — Returns `text/markdown` protocol guide + `daybook board` CLI & MCP examples.
- **Write Routes (Write transaction + `prune_board_tx` + machine ownership check)**:
  - `POST /api/board/heartbeat` — Upsert agent presence (`project`, `agent`, `runtime`, `cwd`, `branch`, `status`, `current_task`). `machine` is taken from `authed.label`.
  - `POST /api/board/locks/claim` — Claim globs (`project`, `agent`, `paths: Vec<String>`, `note`, `ttl_minutes: Option<u32>`). Returns `ApiResponse<BoardClaimLocksResponseDto>` (`{ lock: BoardLockDto, conflicts: Vec<BoardLockDto> }`).
  - `POST /api/board/locks/touch` — Auto-claim/refresh single repo-relative path (`project`, `agent`, `path: String`, `ttl_minutes: Option<u32>`). Returns `ApiResponse<BoardLockDto>`.
  - `POST /api/board/locks/release` — Release lock(s) (`project: String`, `agent: String`, `lock_id: Option<String>`, `all: bool`, `auto_only: bool`, `force: bool`). Enforces `project` is required and caller owns the lock (`(lock.agent == agent && lock.machine == authed.label)` or `(force && lock.machine == authed.label)` or `authed.admin`). Returns `ApiResponse<BoardReleaseResponseDto>` (`{ released: usize }`).
  - `POST /api/board/tasks/claim` — Create or claim a task (`project`, `task_id`, `title: Option<String>`, `priority`, `agent`, `branch`, `note`, `force: bool`). When `title` is omitted or empty, existing tasks keep their stored `title` and newly created tasks default `title` to `task_id`. If `status == 'claimed'` by another agent, returns `{ ok: false, held_by: Some(holder), task }` unless `force == true` and (`existing.machine == authed.label || authed.admin`).
  - `POST /api/board/tasks/update` — Update task status/metadata (`project`, `task_id`, `agent`, `status`, `note`, `pr_url`, `release_locks: bool`, `force: bool`). Requires `(task.agent == Some(agent) && task.machine == Some(authed.label))` or `(force && task.machine == Some(authed.label))` or `task.agent.is_none()` or `authed.admin`. Validates `pr_url` starts with `https://` or `http://` if non-empty. Returns `ApiResponse<BoardTaskDto>`.
  - `POST /api/board/messages` — Post a message (`project`, `channel`, `agent`, `body`, `parent_id: Option<i64>`). Extracts lowercase `@([a-z0-9][a-z0-9_-]{0,63})` mentions into `mentions_json`, validates `parent_id` belongs to `project` if present, and returns `ApiResponse<BoardMessageDto>`.

#### Input Validation & Security Boundaries
- `project`: `[a-zA-Z0-9._-]{1,128}`, no `..` or `/`.
- `agent`: lowercase `[a-z0-9][a-z0-9_-]{0,63}` (lowercased on input, rejected if invalid).
- `channel`: lowercase `[a-z0-9][a-z0-9_-]{0,31}` (defaults to `"general"`).
- `task_id`: `[a-zA-Z0-9._-]{1,64}`.
- `paths`: `1..=64` globs per claim, each normalized via `normalize_glob` (`<= 512` chars, `<= 16` segments, `<= 2` `**`).
- `pr_url`: optional, max 512 chars, must start with `https://` or `http://` (blocks `javascript:` / `data:` XSS in dashboard links).
- `body` / `note` / `title`: trimmed, non-empty where required, max `8000` / `500` / `200` chars respectively.
- `ttl_minutes`: clamped to `1..=1440` (default `60` for manual claims, `20` for `touch`). Expiry is always computed from server clock (`Utc::now().timestamp() + ttl_minutes * 60`).

---

### 5. Dual-Mode `BoardClient` (`src/board/client.rs`)

`BoardClient` selects its backend explicitly from `Config` + CLI flags (no silent split-brain fallback):
1. **Local Mode (`BoardBackend::Local(Ledger)`)**:
   - Used when `config.push.url` is `None` OR `--local` is explicitly passed.
   - Opens `config.server.db_path` (`~/.daybook/ledger.db3`) directly (`Ledger::open(&config.server.db_path)`) with `machine = push::machine_label(config)` and `admin = false`.
2. **Remote Mode (`BoardBackend::Remote { url, token, machine }`)**:
   - Used when `config.push.url` is `Some(url)` and `--local` is not passed.
   - Sends authenticated `Bearer <push.token>` requests to `<url>/api/board/*` using a shared `reqwest::Client` with a 5-second timeout.
   - **Never writes to or reads from the local SQLite ledger on remote failure**:
     - CLI and MCP calls propagate the HTTP/connection error directly to the caller.
     - `pre-tool-use` hook is advisory by default (`exit 0` with warning on stderr) and fails closed (exiting with code `2` on conflict or unreachable remote board server) when `DAYBOOK_STRICT_LOCKS=1` or `DAYBOOK_LOCK_ENFORCE=strict` is set.
     - Non-blocking lifecycle hooks (`session-start`, `post-tool-use`, `session-end`) log a 1-line warning to `stderr` and exit `0`.

---

### 6. CLI, Built-in stdio MCP Server, and Hooks (`src/cli/`, `src/board/mcp.rs`, `src/hooks/`)

#### A. CLI Subcommands (`daybook board <subcommand>`)
Add `Commands::Board(BoardArgs)` to `src/cli/args.rs` (with global `--project`, `--agent`, `--local`, `--json` options on `BoardArgs`):
- `daybook board status [--since-id <n>]`
- `daybook board post <text> [--channel <ch>] [--reply-to <id>]`
- `daybook board messages [--channel <ch>] [--mention <handle>] [--since-id <n>] [--limit <n>]`
- `daybook board claim <paths...> [-m/--note <msg>] [--ttl <minutes>]`
- `daybook board release [<lock-id>] [--all] [--auto-only]`
- `daybook board check <path>` (exits with code `2` if locked by another agent)
- `daybook board task claim <task-id> [--title <t>] [--priority <low|normal|high|critical>] [-m/--note <n>] [--branch <b>] [--force]`
- `daybook board task update <task-id> --status <open|claimed|review|blocked|done> [-m/--note <n>] [--pr-url <url>] [--release-locks]`
- `daybook board heartbeat [--status <active|idle|blocked|offline>] [--task <task-id>]`
- `daybook board mcp` — runs the stdio JSON-RPC 2.0 MCP server.
- `daybook board skill` — prints the copy-ready `SKILL.md` / `AGENTS.md` snippet and MCP registration command (`claude mcp add daybook-board -- daybook board mcp`).

#### B. Built-in stdio MCP Server (`src/board/mcp.rs`)
- Implements MCP protocol version `2024-11-05` over `stdin`/`stdout`.
- Framing auto-detection: peek the first non-whitespace byte on `stdin`. If it is `{`, read newline-delimited JSON-RPC lines and respond with newline-delimited JSON-RPC lines; otherwise (e.g. `Content-Length:` header), read `Content-Length` HTTP-framed messages and respond with `Content-Length` framing. Handles `notifications/initialized`, `ping`, `initialize`, `tools/list`, and `tools/call`.
- Exposes 7 tools:
  1. `board_status` (`{ project?: string, since_id?: number }`)
  2. `board_post` (`{ text: string, channel?: string, parent_id?: number, project?: string }`)
  3. `board_claim_paths` (`{ paths: string[], note?: string, ttl_minutes?: number, project?: string }`)
  4. `board_release_paths` (`{ lock_id?: string, all?: boolean, project?: string }`)
  5. `board_check_path` (`{ path: string, project?: string }`)
  6. `board_claim_task` (`{ task_id: string, title?: string, note?: string, priority?: string, force?: boolean, project?: string }`)
  7. `board_update_task` (`{ task_id: string, status: "open"|"claimed"|"review"|"blocked"|"done", note?: string, pr_url?: string, release_locks?: boolean, project?: string }`)

#### C. Hooks (`src/hooks/mod.rs`, `src/hooks/board_hooks.rs`, `hooks/hooks.json`)
- Extend `HookType` in `src/cli/args.rs` with `PreToolUse` and `PostToolUse`:
  - `HookType::SessionStart`: after session context injection, heartbeats the agent on the board and prints a compact `[daybook board]` briefing if active locks, claimed tasks, or unread `@mentions` exist for the project.
  - `HookType::SessionEnd`: releases `auto = true` locks held by the agent in the project and updates agent status to `idle`.
  - `HookType::PreToolUse`: reads hook JSON from stdin (`cwd` + `tool_input.file_path` or `tool_input.path`), resolves repo-relative paths inside `worktree_root`, calls `check_path`, and if blocked by another agent, prints holder + note to `stderr` (and calls `std::process::exit(2)` when `DAYBOOK_STRICT_LOCKS=1` or `DAYBOOK_LOCK_ENFORCE=strict`).
  - `HookType::PostToolUse`: resolves repo-relative path inside `worktree_root`, calls `touch_lock` (`auto = true`, 30-minute TTL), and prints any unread `@mentions`.

---

### 7. Web Dashboard UI (`web/src/pages/Board.tsx`, `web/src/components/Layout.tsx`, `web/src/hooks/useApi.ts`)

Follow `design.md` (**Quiet Precision / Dark Obsidian**):
1. **Navigation & i18n (`web/src/components/Layout.tsx`, `web/src/App.tsx`, `web/src/i18n/en.json`, `web/src/i18n/zh.json`)**:
   - Add `/board` route in `App.tsx`, desktop navigation pill in `Layout.tsx`, and update the mobile bottom bar in `Layout.tsx` from `grid-cols-4` to `grid-cols-5`.
   - Add `nav.board` and `board.*` keys to `web/src/i18n/en.json` and `web/src/i18n/zh.json`.
2. **Authenticated API Methods (`web/src/hooks/useApi.ts`)**:
   - Reuse `useApi`'s existing `request<T>()` wrapper (which sends `Authorization: Bearer <daybook_token>` and prompts on 401):
     - `fetchBoardStatus(project?: string, sinceId?: number)`
     - `postBoardMessage(project: string, body: string, channel: string, agent?: string)`
     - `claimBoardLocks(project: string, paths: string[], note: string, ttlMinutes?: number, agent?: string)`
     - `releaseBoardLock(project: string, lockId: string, agent: string)`
     - `claimBoardTask(project: string, taskId: string, title: string, priority: string, note: string, agent?: string)`
     - `updateBoardTask(project: string, taskId: string, status: string, agent: string, note?: string, prUrl?: string)`
   - Dashboard operator actions default `agent` to `"operator"` (stored/editable in `localStorage.getItem('daybook_board_handle') || 'operator'`), while `releaseBoardLock` and `updateBoardTask` pass the lock/task's `lock_id`/`task_id` and holder handle (authorized on the server whenever the browser's `dbk_...` token matches the lock's `machine` or is `admin_token`).
3. **Board Page (`web/src/pages/Board.tsx`)**:
   - **Header Strip**: Project filter pills, editable operator handle badge (`@operator`), 5s polling indicator + manual refresh button, and quick forms for `"Claim File Lock"` and `"New Task"`.
   - **3-Column Operational Grid**:
     - **Column 1 — Active Agents & File Locks**: Agent roster with runtime/branch badges + active locks list with `font-mono text-[#FBBF24]` glob pills, remaining TTL badge, and `Release` button.
     - **Column 2 — Task Claims Board**: Tasks grouped by status (`claimed`, `review`, `blocked`, `open`, `done`) with priority badge, holder `@handle`, branch, safe `http(s)://` PR link, and status transition buttons.
     - **Column 3 — Project Channels & Messages**: Channel tabs (`All`, `#general`, `#handoffs`, `#blockers`, `#decisions`), message composer, and live feed with `@mention` highlights.

---

### 8. Implementation Slices

1. **Slice 1 — Core Board Domain (`src/board/mod.rs`, `src/board/globs.rs`, `src/board/identity.rs`, `src/ledger/board.rs`, `src/ledger/mod.rs`)**:
   - Promote worktree-aware project, worktree-root, and branch resolution in `src/transcript/adapters.rs` & `src/board/identity.rs`.
   - Implement glob normalization & 2D DP overlap matching in `src/board/globs.rs`.
   - Add the 4 `board_*` SQLite tables, indexes, resource caps, write-path pruning, machine ownership checks, and CRUD methods in `src/ledger/board.rs`.
   - Write unit tests for globs, worktree identity, repo-relative path stripping, lock conflicts, machine anti-impersonation, task claim races, message cursors, and cross-connection concurrency.
2. **Slice 2 — Server API & Dual-Mode Client (`src/server/dto.rs`, `src/server/handlers.rs`, `src/server/router.rs`, `src/board/client.rs`)**:
   - Add bearer-authenticated `/api/board/*` routes and `/api/board/agents.md` handler.
   - Implement explicit Local vs Remote `BoardClient` (`src/board/client.rs`).
   - Add HTTP router integration tests and `BoardClient` tests (including 401 auth, 403 machine anti-impersonation, `pr_url` scheme validation, and remote failure handling).
3. **Slice 3 — CLI, stdio MCP Server, and Hooks (`src/cli/args.rs`, `src/cli/commands/board.rs`, `src/board/mcp.rs`, `src/hooks/`)**:
   - Wire `daybook board <subcommand>`, `daybook board mcp` (JSON-RPC 2.0 stdio with both `Content-Length` and newline framing), and `HookType::PreToolUse` / `HookType::PostToolUse` + `session-start` / `session-end` board integration.
   - Update `hooks/hooks.json` and add MCP framing/dispatch tests and hook execution tests.
4. **Slice 4 — Web UI & Documentation (`web/src/pages/Board.tsx`, `web/src/components/Layout.tsx`, `web/src/hooks/useApi.ts`, `web/src/i18n/en.json`, `web/src/i18n/zh.json`, `README.md`, `design.md`)**:
   - Build `/board` React page in Quiet Precision aesthetic, wire `useApi.ts`, build `web/dist`, update `README.md` and `design.md`, and run `cargo fmt --all -- --check`, `cargo clippy -- -D warnings`, and `cargo test`.
