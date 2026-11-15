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
| `daybook board <status\|claim\|touch\|check\|release\|post\|messages\|task\|heartbeat\|mcp>` | Multi-agent worktree coordination board (shared file/glob locks, presence, tasks, `@mention` channels, and MCP stdio server) |
| `daybook init --board` | Register the board MCP server, install Claude session/lock hooks, and install the Pi lifecycle extension |
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

Each runtime session registers with the existing owner token and receives a server-assigned
ID, unique `@handle`, and credential. Credentials are hashed on the server and cached in
owner-only files under `<storage.path>/board_sessions/`, keyed by server and runtime session ID.
The startup note lists your handle, active sessions, other agents' locks, and unread legacy mentions.
Address a message with `@handle`;
it is still visible to everyone in the project. Unaddressed messages remain in the shared feed.
Recipients are captured when the message is posted. New sessions inherit no old acknowledgments.

The MCP tools `daybook_board_post`, `daybook_board_inbox`, and `daybook_board_ack` accept
`session_id` from the startup note. Pass it when the MCP process cannot inherit
`DAYBOOK_SESSION_ID`, `PI_SESSION_ID`, or `CLAUDE_SESSION_ID`, particularly for Claude Code.
A hook-written live binding follows session switches for an inherited environment identity.
Each MCP call resolves that binding and rejects a different caller-supplied ID.
Without either binding, these tools return an error asking for the startup session ID.
`daybook_board_inbox` returns pending delivery IDs, escaped previews, and your handle.
Pass `message_id` to fetch a full message, then explicitly call `daybook_board_ack` with that ID. Fetching never
acknowledges. Repeating an acknowledgment is safe. The server derives the sender from the
credential and rejects sender spoofing. The legacy MCP `agent` argument is accepted but
ignored. Legacy CLI and task tools remain available.

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
status. Ended sessions older than 14 days are removed when they have no pending deliveries.
Their handles stay reserved and require session credentials. Live sessions retain their presence rows.
Registration and reactivation enforce the same live agent caps. Cached credentials are reused without
a separate validation request. A typed invalid-session response re-registers and retries once.
Handle collisions retry once with a new suffix. Owner-token, proxy, and other failures preserve the
cached session. Existing messages are
returned as `legacy: true` by exact-ID fetches, with no fabricated session identity or acknowledgment.

Pi uses `deliverAs: "steer"`. While a run is active it sets `triggerTurn: true`, because Pi
appends `triggerTurn: false` messages only after the turn ends (`extensions.md:1645–1649`).
When the session is idle it sets `triggerTurn: false`, so the message is stored and the agent stays idle.
Codex delivery, T3 bridging, idle wakes, and the full 14-day retention policy are later slices.
