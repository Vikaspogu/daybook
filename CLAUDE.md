# CLAUDE.md

This file provides guidance to coding agents when working with code in this repository.

**Important**: When making changes to interfaces, features, architecture, or workflows, review and update `README.md` and `design.md` accordingly.

## Build and Development Commands

```bash
# Build web frontend first (embedded into the Rust binary via rust-embed)
cd web && npm install && npm run build && cd ..

# Build Rust binary (`daybook`)
cargo build
cargo build --release

# Run CLI commands
cargo run -- <command>

# Test & Lint (requires web/dist from build step above)
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo test
cargo test <test_name>         # Run specific test
cargo test -- --nocapture      # Show test output

# Install locally
cargo install --path .
```

## Project Structure

```text
daybook/
├── src/              # Rust CLI & server source code (`daybook` binary)
├── web/              # Quiet Precision / Dark Obsidian dashboard UI (`daybook show`) - React + Vite
│                     # Embedded into the Rust server via `rust-embed` (`web/dist`)
├── site/             # Public marketing/docs website - React + Vite
├── hooks/            # Optional Claude Code hook registration (hooks/hooks.json; handlers live in src/hooks/)
├── commands/         # Slash command templates
├── scripts/          # Installation scripts (`install.sh`)
├── assets/           # Brand icons (`daybook-icon.png`, `logo.png`) and docs assets
├── design.md         # Quiet Precision / Dark Obsidian UI & component specification
└── .github/          # CI, Docker (`ghcr.io/vikaspogu/daybook`), and Release workflows
```

**Important distinction**:
- `web/` = Operational dashboard embedded in `daybook show` (governed by `design.md`)
- `site/` = Public marketing website

## Architecture Overview

Daybook is a multi-agent context archive and operational ledger that discovers, summarizes, and indexes AI coding sessions across **Claude Code**, **Codex**, and **Pi**.

### Data Flow

1. **Multi-Runtime Discovery (`src/transcript/adapters.rs`, `src/cli/commands/scan.rs`)**:
   - Discovers transcripts from `~/.claude/projects/`, `~/.codex/sessions/`, and `~/.pi/agent/sessions/`.
   - Resolves git worktrees (`.git` file pointers and `.t3/worktrees/<repo>/...` paths) back to the parent repository and buckets sessions by their UTC start timestamp.
2. **Agent-Agnostic Summarization (`src/summarizer/engine.rs`, `src/summarizer/prompts.rs`)**:
   - Summarizes sessions into `topic`, `summary`, `decisions`, `learnings` (`[PITFALL]` / `[LEARNING]`), and `skill_hints` (`<!-- daybook:workflow-candidates:v2 -->` with `Scope: global-skill` or `Scope: repo-rule`).
3. **Local Archive & Central Ledger Push (`src/archive/`, `src/push/mod.rs`, `src/ledger/mod.rs`)**:
   - Writes local markdown archives under `~/.daybook/YYYY-MM-DD/` and pushes `SessionRecord` payloads to `POST /api/ingest` (parking offline records in `~/.daybook/pending/`).
4. **Server Synthesis & Dashboard (`src/server/handlers.rs`, `web/src/pages/DayDetail.tsx`)**:
   - Synthesizes the 5-minute daily brief, project-grouped shipped outcomes, pitfalls/learnings, workflow coaching, and scoped copy-ready `SKILL.md` vs `<repo>/AGENTS.md` rules for the web dashboard and `daybook query` / `daybook stats`.

### Module Structure

- `src/cli/` - Clap CLI definitions (`args.rs`) and command handlers (`scan`, `serve`, `show`, `query`, `stats`, `token`, `init`, `view`, `digest`, `summarize`, `extract`, `skills`, `jobs`, `config`, `insights`)
- `src/transcript/` - Transcript parsing (`parser.rs`) and multi-runtime adapters for Claude Code, Codex, and Pi (`adapters.rs`)
- `src/summarizer/` - LLM summarization engine (`engine.rs`) and agent-agnostic v2 prompt templates (`prompts.rs`, `template.rs`)
- `src/ledger/mod.rs` - SQLite ledger schema, session upsert, token management (`dbk_...`), and stats queries
- `src/push/mod.rs` - HTTP ingest client and offline `~/.daybook/pending/` retry queue
- `src/server/` - Axum router (`router.rs`), API & synthesis handlers (`handlers.rs`), DTOs (`dto.rs`), and embedded static asset serving (`static_files.rs`)
- `src/archive/` - Local markdown archive management (`manager.rs`, `session.rs`, `daily.rs`, `templates.rs`)
- `src/usage/` - Token and model pricing calculation (`pricing.rs`, `scanner.rs`)
- `src/hooks/` - Claude Code hook handlers (`session_start.rs`, `session_end.rs`) and JSON stdin parser (`input.rs`)
- `src/jobs/` - Background summarization job manager (`manager.rs`)
- `src/config/` - Configuration loading via `confy` (`settings.rs`, `~/.config/daybook/config.toml`)

### Testing

Tests use `tempfile::TempDir` for isolated filesystem and SQLite ledger tests. Always ensure `web/dist` exists before running `cargo test`.
