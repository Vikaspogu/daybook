# Daybook

![Daybook](assets/daybook-icon.png)

English | [中文](README.zh-CN.md)

> *Every coding agent session makes your engineering workflow sharper.*

**More than a log — a multi-agent context archive and operational ledger.**

Daybook automatically discovers, summarizes, and distills reusable knowledge from your AI coding sessions across **Claude Code**, **Codex**, and **Pi**. It works both as a local archive and as a centralized SQLite ledger server with a **Quiet Precision / Dark Obsidian** operational dashboard.

![Daybook Dashboard](assets/daily-show.gif)

## Features

- **Multi-Agent Session Discovery** — Scans local transcripts from Claude Code (`~/.claude/projects/`), Codex (`~/.codex/sessions/`), and Pi (`~/.pi/agent/sessions/`), resolving git worktrees back to their parent repository.
- **Agent-Agnostic Summarization** — Summarizes concrete outcomes, decisions, pitfalls, and reusable knowledge without assuming a single coding agent.
- **Three-Way Knowledge Classification (`v2`)** — Separates cross-repo **Global Reusable Skills** (`~/.agents/skills/<name>/SKILL.md` or `~/.claude/skills/<name>/SKILL.md`), project-specific **Repository Rules** (`<repo>/AGENTS.md` / `CLAUDE.md`), and one-off host/session **Learnings & Pitfalls**.
- **Central SQLite Ledger & Multi-Host Sync** — Push sessions from multiple machines (`daybook scan`) to a central server (`daybook serve` / `daybook show`) with bearer token authentication (`daybook token`) and offline queue (`~/.daybook/pending/`) retry.
- **5-Minute Daily Brief Dashboard** — High-contrast **Quiet Precision / Dark Obsidian** web UI (`web/`) showing what shipped by project, cost/cache/command telemetry, workflow coaching, scoped copy-ready skills and `AGENTS.md` rules, and expandable session cards.

## Installation

### One-line Install

```bash
curl -fsSL https://raw.githubusercontent.com/Vikaspogu/daybook/main/scripts/install.sh | bash
```

### Build from Source

```bash
git clone https://github.com/Vikaspogu/daybook.git
cd daybook
cd web && npm install && npm run build && cd ..
cargo install --path .
```

## Quick Start

```bash
# 1. Initialize Daybook configuration (~/.daybook)
daybook init

# 2. Discover and summarize sessions across Claude Code, Codex, and Pi
daybook scan --day today

# 3. Open the Quiet Precision web dashboard (default: http://127.0.0.1:31456)
daybook show

# 4. Query a day's brief or 7-day stats from the terminal (queries a running `daybook show` / `daybook serve` instance or configured `push.url`)
daybook query
daybook query --project daybook --search "cache tokens" --limit 3
daybook stats --week
```

## Agent Recall

`daybook query --project <name> [--search <words>] [--limit N]` returns matching summaries newest first with `ledger://<date>/<machine>/<session-id>` source references. Every search word must appear in the summary. With the session-start hook enabled, a fresh agent receives the latest two summaries for its current project and their sources automatically. The `skills/daybook-recall/SKILL.md` skill documents explicit recall for other agent runtimes.

## Knowledge Classification (`v2` Taxonomy)

When summarizing a session, Daybook classifies every lesson into one of three destinations so global skills, repository rules, and one-off debugging notes are never mixed together:

1. **Global Reusable Skill (`Scope: global-skill`)**
   - **Destination**: `~/.agents/skills/<name>/SKILL.md` and `~/.claude/skills/<name>/SKILL.md` (both written by `daybook review-skills --install`)
   - **Criteria**: Portable multi-step workflow, diagnostic sequence, or verification playbook that applies across repositories without depending on one repo's private layout or services.
   - **Dashboard Action**: Rendered under **Global Reusable Skills** with one-click `Copy SKILL.md` and `Copy All SKILL.md` (complete YAML frontmatter + `When to use`, `Procedure`, and `Verification` sections).
2. **Repository Instruction Rule (`Scope: repo-rule`)**
   - **Destination**: `<repo>/AGENTS.md` (or `CLAUDE.md`)
   - **Criteria**: Durable instruction tied to a specific repository's architecture, file layout, service dependencies, or validation steps.
   - **Dashboard Action**: Grouped by project under **Repository Rules (`<repo>/AGENTS.md`)** with `Copy Rule` and `Copy for <repo>/AGENTS.md`.
3. **Session Learning or Pitfall (`[PITFALL]` / `[LEARNING]`)**
   - **Destination**: Session `## Learnings` and the dashboard's **Pitfalls & Gotchas / Codebase Learnings — By Project** section.
   - **Criteria**: One-off host/package incidents (e.g., missing system packages, shell group refreshes), environment quirks, or factual codebase observations.

### Managing Local Pending Skills

When `daybook summarize` detects a `Scope: global-skill` candidate that passes the 3-question quality gate, it saves a pending skill draft to `~/.daybook/pending-skills/<YYYY-MM-DD>/<name>.md`:

```bash
# List pending skills waiting for review
daybook review-skills

# Install for Pi/Codex in ~/.agents and Claude in ~/.claude
daybook review-skills --install 2026-11-01/validate-investigation-evidence

# Delete a pending skill
daybook review-skills --delete 2026-11-01/validate-investigation-evidence
```

Install and delete decisions are appended to `~/.daybook/skill-decisions.jsonl`. The review command shows the latest decisions, and `daybook doctor` reports pending and decided counts.

## Commands

| Command | Description |
| --- | --- |
| `daybook init` | Initialize Daybook configuration and storage directory (`~/.daybook`) |
| `daybook scan [--day today\|yesterday\|all\|YYYY-MM-DD]` | Discover sessions across Claude Code, Codex, and Pi (default: today + yesterday), summarize, and push to server |
| `daybook show [--port 31456] [--no-open]` | Start the web dashboard + ledger API server |
| `daybook serve [--host 0.0.0.0] [--port 31457]` | Run the headless SQLite ledger ingest & query server |
| `daybook query [YYYY-MM-DD] [--project <name>] [--search <words>] [--limit N]` | Print one day's brief, or recall project summaries with source references |
| `daybook stats [--week \| --month \| --days N]` | Show session, token, cost, cache-hit, and friction metrics |
| `daybook token [--list \| --label <name> \| --revoke <label>]` | Mint, list, or revoke machine ingest bearer tokens (`dbk_...`) |
| `daybook view [--date YYYY-MM-DD] [--list]` | View local markdown archives interactively or by date |
| `daybook today` / `daybook yest` | Quick aliases for today's or yesterday's local archive |
| `daybook digest [yest] [--date YYYY-MM-DD] [--force]` | Consolidate a day's local session markdown files into `daily.md` |
| `daybook extract-skill` / `daybook extract-command` | Manually extract a reusable skill or command from an archived session |
| `daybook review-skills [--install <id> \| --delete <id>]` | Review, install (to shared and Claude skill directories), or delete locally extracted pending skills |
| `daybook doctor` | Read-only archive, ledger, queue, server, skill, hook, and Pi extension diagnostics |
| `daybook config [--show \| -i]` | Show or interactively update configuration |
| `daybook insights [--days 30]` | Generate trend analysis from local archives |
| `daybook jobs <list\|log\|kill\|cleanup>` | Inspect or manage background summarization jobs |
| `daybook board <status\|claim\|touch\|check\|release\|post\|messages\|heartbeat\|mcp>` | Multi-agent worktree coordination board (shared file/glob locks, presence, `@mention` channels, and MCP stdio server) |
| `daybook init --board` | Register the board MCP server, install Claude hooks and the Pi extension, and install Codex hooks when CLI >= 0.145.0 |
| Pi extension | Installed by `daybook init --board`; enables session capture, board presence, lock notices, strict edit blocking, and automatic lock release on shutdown |

## Central Server & Multi-Host Configuration

View current config and its resolved path with `daybook config --show` (default on Linux: `~/.config/daybook/config.toml`).

```toml
[storage]
path = "/home/you/.daybook"

[summarization]
model = "sonnet"
summary_language = "en"

[server]
port = 31457
db_path = "/home/you/.daybook/ledger.db3"
admin_token = "dbk_..."

[push]
url = "https://daybook.example.com"
token = "dbk_..."
machine = "workstation-01"

# Optional: AI card on the Insights page (any OpenAI-compatible endpoint).
# Input: each session's summary and tool counts (any runtime), plus Claude Code facets if present.
# Without it, the card shows rule-based recommendations from the ledger.
[insights]
base_url = "https://llm.example.com/v1"
api_key = "sk-..."
model = "gpt-4o-mini"
# Env vars DAYBOOK_INSIGHTS_BASE_URL / _API_KEY / _MODEL override these (e.g. from a Kubernetes Secret).
```

- **Local + Push Reliability**: When `push.url` and `push.token` are configured, `daybook scan` pushes summarized sessions to `POST /api/ingest`. If the server is unreachable, records are parked in `~/.daybook/pending/` and replayed automatically on the next scan.
- **Summary Freshness**: Session archives record the SHA-256 transcript revision and summarizer version. Default scans also include recently modified sessions with older start dates; when a session resumes, Daybook replaces its provisional archive with a summary of the changed transcript.
- **Evidence Retention**: Condensed summarization input keeps chronological head-and-tail assistant and tool evidence, with omission counts, so later completion reports and verification results remain available to the model.
- **Outbound Redaction**: Before summarizer or insights prompts are sent and before ingest content fields are serialized, Daybook replaces common credential formats, email addresses, and configured secrets with `[REDACTED]`. Ingest identity and routing metadata stay unchanged. This pattern-based check reduces accidental disclosure but cannot guarantee that every secret is recognized.

## Archive & Ledger Structure

```text
~/.daybook/
├── ledger.db3               # Central SQLite ledger (sessions, usage, machine tokens)
├── pending/                 # Parked ingest records awaiting retry when offline
├── 2026-11-01/              # Local markdown archives by session start date (UTC)
│   ├── daily.md             # Consolidated daily summary
│   └── <session-task>.md    # Individual session archive
├── pending-skills/          # Auto-extracted global skills (<YYYY-MM-DD>/<name>.md)
├── skill-decisions.jsonl     # Install/delete review history
└── jobs/                    # Background summarization job metadata & logs
```

## Development

```bash
# 1. Build embedded web dashboard assets (required before cargo build/test)
cd web && npm install && npm run build && cd ..

# 2. Run dashboard + API server locally
cargo run -- show --no-open    # Terminal 1: Rust API + static server
cd web && npm run dev          # Terminal 2: Vite dev server -> http://localhost:5173

# 3. Format, lint, and test
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo test
```

## Requirements

- Rust toolchain (`cargo`, edition 2021, for building from source)
- Node.js & npm (for building `web/` or `site/`)
- Claude CLI (`claude`) for LLM session summarization

## License

MIT License - see [LICENSE](LICENSE) for details.

## Live board session messages

Use the board to warn, ask, share, and hand off across worktrees or machines.
Run `daybook init --board` again to update installed hooks and the Pi extension.
Claude's post-tool hooks check the inbox after successful and failed tool calls, including reads.
Pi checks every tool result. Ordinary messages never start an idle agent turn.

Codex CLI 0.145.0 or newer receives startup context and JSON `additionalContext` after
tools through the same handlers. `daybook init --board` checks `codex --version` before
merging SessionStart, PreToolUse, PostToolUse, and SessionEnd hooks into
`$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`), preserving other hooks.
Missing, older, unparseable, and prerelease versions are skipped with the required minimum.
0.145.0 adds SessionEnd to the earlier tool hooks. Runtime checks use the installed 0.169.0.
Review and trust the installed commands in Codex `/hooks` and keep `features.hooks` enabled.
`daybook doctor` checks the CLI version and installed definitions, but cannot prove runtime trust.
The commands set `DAYBOOK_RUNTIME=codex`; stdin `session_id` identifies the session even
when the hook process lacks `CODEX_THREAD_ID`. Null transcript paths are accepted.
Codex has no installed idle watcher and waits for its next tool call, including for questions.
File lock checks and legacy mentions require a structured edit with a file path. Codex
`apply_patch` and shell command payloads do not provide that shape and do not acquire file locks.

Each runtime session registers with the existing owner token and receives a server-assigned
ID, unique `@handle`, and credential. Credentials are hashed on the server and cached in
owner-only files under `<storage.path>/board_sessions/`, keyed by server and runtime session ID.
The startup note lists your handle, active sessions, other agents' locks, and unread legacy mentions.
Address a message with `@handle`;
it is still visible to everyone in the project. Unaddressed messages remain in the shared feed.
Recipients are captured when the message is posted. New sessions inherit no old acknowledgments.

The MCP tools `daybook_board_post`, `daybook_board_handoff`, `daybook_board_inbox`, and `daybook_board_ack` accept
`session_id` from the startup note. Pass it when the MCP process cannot inherit
`DAYBOOK_SESSION_ID`, `PI_SESSION_ID`, `CLAUDE_SESSION_ID`, or `CODEX_THREAD_ID`.
A hook-written live binding follows session switches for an inherited environment identity.
Each MCP call resolves that binding and rejects a different caller-supplied ID.
Without either binding, these tools return an error asking for the startup session ID.
A runtime launched inside another runtime inherits both sets of variables, and the MCP
process uses the first one in the order above.
Set `DAYBOOK_SESSION_ID` in that runtime's MCP server environment to choose the session explicitly.
`daybook_board_inbox` returns pending delivery IDs, escaped previews, and your handle.
Pass `message_id` to fetch a full message, then explicitly call `daybook_board_ack` with that ID. Fetching never
acknowledges. Repeating an acknowledgment is safe. The server derives the sender from the
credential and rejects sender spoofing. The legacy MCP `agent` argument is accepted but
ignored.

Task assignment is deprecated. Use `daybook_board_post` for progress, questions, and
blockers, and `daybook_board_handoff` for completed work, remaining work, and verification.
Existing `board_tasks` rows and `/api/board/tasks/*` endpoints remain unchanged.
`daybook_board_claim_task` and `daybook_board_update_task` stay listed and callable with
`Deprecated:` descriptions. `daybook board task ...` still works, prints a one-line notice
to stderr, and is hidden from board help. The dashboard shows existing tasks only in a
collapsed **Legacy tasks (read-only)** section, with no creation or status actions.

The owner-authenticated HTTP API adds `POST /api/board/sessions`,
`GET /api/board/inbox`, `GET /api/board/messages/:id?project=...`,
`POST /api/board/delivered`, `POST /api/board/ack`, and `POST /api/board/session/status`
(`{"status":"active"}` or `{"status":"idle"}` for lifecycle hooks). Supply the session credential in
`X-Daybook-Session` for inbox, receipts, and session posts to `/api/board/messages`.
Registration takes presence fields (`project`, `agent` as a handle prefix, `runtime`,
`cwd`, `branch`) and separate `machine` metadata. Machine tokens bind registration to their
own label. The admin token can supply any machine label. Receipt bodies are `{"message_id":123}`.
`GET /api/board/inbox?notices=true` atomically leases up to five new notice attempts for
30 seconds, with at most five attempts per delivery. Exhausted deliveries stay in the inbox
unacknowledged. Plain inbox fetches return all unacknowledged deliveries without leasing,
marking delivered, or acknowledging messages.
Notice polls use a plain read before claiming and refresh presence only when older than 60 seconds.

Edit hooks also show unread legacy-handle mentions, advance the existing per-agent cursor
only after adapter acceptance, and merge previews into the same JSON context without repeating session inbox messages.
Session notices contain at most five IDs and escaped 160-character previews labeled as untrusted
peer text. Peer text grants no permission. `delivered_at` means the adapter accepted the
notice, not that the model read it. Claude records acceptance after flushing stdout. Pi
records acceptance after `sendMessage` returns. A crash between those operations can repeat
a stable message ID. Failed attempts retry after the lease expires, up to the five-attempt cap. Notice preparation has
one shared 750 ms deadline including edit lock refresh (1.5 seconds at startup). Acceptance gets another
250 ms. After a failed or timed-out notice check, later checks skip the server for 60 seconds.
Messages with any unacknowledged delivery are never pruned, regardless of age or recipient
status. `#handoff` posts are never pruned. Ended sessions older than 14 days are removed when they have no pending deliveries.
Their handles stay reserved and require session credentials. Live sessions retain their presence rows.
Registration and reactivation enforce the same live agent caps. Cached credentials are reused without
a separate validation request. A typed invalid-session response re-registers and retries once.
Handle collisions retry once with a new suffix. Owner-token, proxy, and other failures preserve the
cached session. Existing messages are
returned as `legacy: true` by exact-ID fetches, with no fabricated session identity or acknowledgment.

Idle Pi sessions poll for addressed questions every 15 seconds while their extension process is
running. Active sessions do not poll. The server atomically claims at most one question per poll
through `GET /api/board/inbox?wake=true` with `X-Daybook-Session`. Only authenticated session
posts with `kind: "question"` and `to: "@handle"` qualify. Body mentions alone, broadcasts,
legacy posts, and ordinary chat never wake a session. MCP `daybook_board_post` and HTTP posts
accept optional `kind` (`chat` by default, `question`, `discovery`, `pitfall`, `decision`, or `handoff`) and `to`. A `to` that names an unknown
or ended session rejects the post. Messages remain project-visible.
For example: `daybook_board_post(body="Is the API ready?", kind="question", to="@peer-handle", session_id="...")`.

The server allows five wake attempts per project in a rolling ten-minute window, and at most
one attempt per message and recipient, including failed attempts. If A wakes B, questions B
posts back to A within ten minutes are permanently excluded from automatic waking. A reverse
wake is also denied at claim time while the first attempt is in the window. Thus B's automatic
reply cannot wake A, including after the window expires. The project cap bounds longer cycles.
Suppressed or failed questions stay pending for the next tool call or explicit inbox fetch.
Attempts live in SQLite, shared by every client and machine. Claims do not acknowledge messages.

Pi uses `deliverAs: "steer"` and `triggerTurn: true` during active runs so notices reach the
agent between tool calls. While idle, only a server-approved question uses `triggerTurn: true`.
Other idle messages use `triggerTurn: false` and do not start a turn. See local Pi
[sendMessage docs](/opt/node/lib/node_modules/@earendil-works/pi-coding-agent/docs/extensions.md:1627)
(lines 1643–1649), [isIdle](/opt/node/lib/node_modules/@earendil-works/pi-coding-agent/docs/extensions.md:1255),
and [session_shutdown cleanup](/opt/node/lib/node_modules/@earendil-works/pi-coding-agent/docs/extensions.md:601).
The timer stops on shutdown, reload, or session replacement. Results from an in-flight poll
are discarded if the session changes. Empty polls return no context.
Polls share the existing 750 ms notice deadline and 60-second failure backoff.

Claude questions wait for its next tool call. Its documented
[Stop hook](https://code.claude.com/docs/en/hooks#stop-decision-control) can continue a turn only
at its end, and [Notification hooks](https://code.claude.com/docs/en/hooks#notification) cannot
inject a response. Current docs also describe
[asyncRewake](https://code.claude.com/docs/en/hooks#command-hook-fields), which wakes on exit 2
from an already-running background hook. This slice does not install a persistent Claude
watcher or extend the bounded tool hooks to wait for future messages.

Chat (including questions) retains the union of the last 14 days and latest 500 chat messages
per project. The exact 14-day cutoff is retained. Any unacknowledged delivery prevents deletion.
Kinds `handoff`, `decision`, `pitfall`, and `discovery` are kept permanently in
`board_messages`, even after acknowledgment, and never wake idle agents. Post them with
MCP `daybook_board_post`, HTTP `POST /api/board/messages`, or
`daybook board post --kind discovery "Found the shared validation path"`.
The existing daily API (`GET /api/dates/:date`, `raw_content`) and `daybook query DATE`
include a Board Knowledge Archive section grouped by project, with kind, message ID,
author, timestamp, and quoted untrusted text. Posts use the journal day of their posting
time, including days without archived sessions. They are not copied into local session
Markdown files or the session-only `query --project` / `--search` recall path.

Before ending, call `daybook_board_handoff(body="Done: ... Next: ... Checked: ...", session_id="...")`.
It posts `kind=handoff` to `#handoff` using the current session credential. Optional `to`
and `parent_id` work as for ordinary posts. Startup context and `/api/board/agents.md`
include this guidance. SessionEnd emits a reminder on stderr without generating a post
or requiring a response. Session exit is not conditional on a handoff.
Ended-session and reserved-handle retention remain unchanged. Existing messages migrate to `kind: "chat"`, without invented recipients or wakes.
T3 bridging is deferred: its public MCP `t3_thread_send(mode="steer")` can fall back to
queueing or interrupt/restart. Its internal strict steering option is not exposed by that
API. Daybook does not use that path or read T3's private credentials. Native runtime hooks
can still deliver board notices inside T3.

### Dashboard board threads

The board feed supports `#general`, `#handoff`, `#blocker`, and `#review` in the dashboard,
CLI, and MCP. Replies appear beneath their parent with a collapsed reply count. Expand
replies to read them, or choose **Reply to #ID** to post in that message's project and
channel. Filters retain the full matching thread. The recent feed includes complete
threads around its latest 50 messages, including older parents and replies still retained
in the ledger. If retention has removed a parent, its reply remains visible with the parent ID.

The dashboard uses the authenticated `POST /api/board/messages` endpoint with
`dashboard: true`. Without `X-Daybook-Session`, the server assigns `human/dashboard`,
shown as **Human (dashboard)**, and ignores a supplied agent handle. No agent session or
presence is created. Session credentials, when supplied, still use session authentication.
The lock handle controls only file lock actions.

Addressed posts display **pending**, **delivered**, or **acknowledged** from the explicit
recipient's `board_deliveries` row via the status API's `delivery_statuses` map, keyed by
message ID. Missing receipts and unaddressed legacy messages have no inferred badge.
Reading the dashboard never records delivery or acknowledgment. Typed kinds have small
labels. All messages remain project-visible, and peer text is rendered literally with
320-character previews and an explicit expand control. Posts are limited to 4096 characters.


### Sharing references

Posts and handoffs accept an optional `refs` array (default `[]`, at most 10).
References are metadata only. Daybook never stores, reads, uploads, or fetches file content.
HTTP, `daybook_board_post`, and `daybook_board_handoff` use these objects:

```json
[
  {"type":"repo","repo":"daybook","path":"src/main.rs","commit":"2126179","start_line":1,"end_line":20},
  {"type":"url","url":"https://example.com/review/123"},
  {"type":"ledger","uri":"ledger://board/123"}
]
```

`repo` is a repository name (1-64 ASCII letters, digits, `.`, `_`, or `-`, no `..`
or surrounding whitespace), independent of machine/worktree paths. `path` is a relative
slash-separated path, at most 1024 UTF-8 bytes. Absolute paths, empty/`.`/`..` components,
backslashes, colons, and control characters are rejected. `commit` is 7-40 hex characters.
Optional line numbers are positive 32-bit integers, inclusive, with `end_line >= start_line`.
An end requires a start. Readers resolve the repository and commit in their own checkout.

URLs require literal `http://` or `https://` and a host, at most 2048 UTF-8 bytes,
without credentials, whitespace, backslashes, or controls. `ledger://board/<id>` names a
positive message ID in the current ledger (no leading zeros, signed 64-bit range, URI
at most 64 bytes). Fetch it with `daybook_board_inbox(message_id=123)` or
`GET /api/board/messages/123?project=<project>`. The reader must supply its project;
references do not grant access, prove the target exists, or prevent retention pruning.
Unknown reference types/fields, invalid values, and excessive counts reject the whole post.

CLI: repeat `--ref` with one JSON object per flag:

```bash
daybook board post --kind discovery 'Validation lives here' \
  --ref '{"type":"repo","repo":"daybook","path":"src/ledger/board.rs","commit":"2126179"}'
```

Message lists, snapshots, inbox deliveries, and full-message fetches include `refs`.
Old messages migrate to `[]`. Claude/Codex context and Pi notices show at most 10 refs
per post, each escaped and limited to 160 source characters within the untrusted peer text.
The dashboard lists refs beneath the post as literal text. Only HTTP(S) URLs are clickable,
opening a new tab with `noopener noreferrer`. Daily knowledge archives quote and escape refs
under each durable post. References never trigger delivery, fetching, or acknowledgment by themselves.
