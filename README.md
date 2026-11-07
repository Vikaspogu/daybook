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
daybook stats --week
```

## Knowledge Classification (`v2` Taxonomy)

When summarizing a session, Daybook classifies every lesson into one of three destinations so global skills, repository rules, and one-off debugging notes are never mixed together:

1. **Global Reusable Skill (`Scope: global-skill`)**
   - **Destination**: `~/.agents/skills/<name>/SKILL.md` (dashboard copy draft) or `~/.claude/skills/<name>/SKILL.md` (via `daybook review-skills --install`)
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

# Install a pending skill to ~/.claude/skills/<name>/SKILL.md
daybook review-skills --install 2026-11-01/validate-investigation-evidence

# Delete a pending skill
daybook review-skills --delete 2026-11-01/validate-investigation-evidence
```

## Commands

| Command | Description |
| --- | --- |
| `daybook init` | Initialize Daybook configuration and storage directory (`~/.daybook`) |
| `daybook scan [--day today\|yesterday\|all\|YYYY-MM-DD]` | Discover sessions across Claude Code, Codex, and Pi (default: today + yesterday), summarize, and push to server |
| `daybook show [--port 31456] [--no-open]` | Start the web dashboard + ledger API server |
| `daybook serve [--host 0.0.0.0] [--port 31457]` | Run the headless SQLite ledger ingest & query server |
| `daybook query [YYYY-MM-DD]` | Print one day's work overview and session summaries from the ledger server |
| `daybook stats [--week \| --month \| --days N]` | Show session, token, cost, cache-hit, and friction metrics |
| `daybook token [--list \| --label <name> \| --revoke <label>]` | Mint, list, or revoke machine ingest bearer tokens (`dbk_...`) |
| `daybook view [--date YYYY-MM-DD] [--list]` | View local markdown archives interactively or by date |
| `daybook today` / `daybook yest` | Quick aliases for today's or yesterday's local archive |
| `daybook digest [yest] [--date YYYY-MM-DD] [--force]` | Consolidate a day's local session markdown files into `daily.md` |
| `daybook extract-skill` / `daybook extract-command` | Manually extract a reusable skill or command from an archived session |
| `daybook review-skills [--install <id> \| --delete <id>]` | Review, install (to `~/.claude/skills/<name>/SKILL.md`), or delete locally extracted pending skills |
| `daybook config [--show \| -i]` | Show or interactively update configuration |
| `daybook insights [--days 30]` | Generate trend analysis from local archives |
| `daybook jobs <list\|log\|kill\|cleanup>` | Inspect or manage background summarization jobs |
| `daybook board <status\|claim\|touch\|check\|release\|post\|messages\|task\|heartbeat\|mcp>` | Multi-agent worktree coordination board (shared file/glob locks, presence, tasks, `@mention` channels, and MCP stdio server) |
| `daybook init --board` | Register `PreToolUse` / `PostToolUse` lock hooks in `~/.claude/settings.json` and append Agent Coordination Board protocol rules to `AGENTS.md` |

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

## Archive & Ledger Structure

```text
~/.daybook/
├── ledger.db3               # Central SQLite ledger (sessions, usage, machine tokens)
├── pending/                 # Parked ingest records awaiting retry when offline
├── 2026-11-01/              # Local markdown archives by session start date (UTC)
│   ├── daily.md             # Consolidated daily summary
│   └── <session-task>.md    # Individual session archive
├── pending-skills/          # Auto-extracted global skills (<YYYY-MM-DD>/<name>.md)
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
