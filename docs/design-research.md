# Project draft — cross-session work journal & recall

Status: DRAFT (research done, direction not decided)

## 1. Problem

Daily work happens in many independent agent sessions (Pi, Codex, orchestrated by T3 Code).
No single place knows what was worked on across the whole day. Consequences:

- Cannot answer "what did I work on today / this week / 15 days ago?" without reopening sessions.
- No recall of past decisions and their why.
- No stats: where I struggle, tests run per day, frequency of issues/bugs/features per project.
- No simple view of prompt → response efficiency to tune prompts over time.

## 2. Requirements (from problem statement)

- R1: Holistic daily view across ALL sessions and projects, queryable at any point in the day.
- R2: Any agent (Pi or Codex) can query it — not tied to one runtime.
- R3: Long-term recall (weeks): what was done, decisions made, why.
- R4: Frequency stats: struggles, tests/day, issues/bugs/features per project.
- R5: Simple observability: prompts, responses, efficiency; prompt-improvement loop. Explicitly "not too complicated".
- R6: Draft-first: pick the right solution, existing tool if it fits.

## 3. Local infrastructure facts (verified)

- Sessions live as local JSONL transcripts:
  - Codex: `~/.codex/sessions/<year>/...`
  - Pi: `~/.pi/agent/sessions/<project-dir-slug>/...`
  - Both are per-project, append-only, timestamps + prompts + tool calls included.
- T3 Code orchestrates sessions; it supports scheduled tasks (fixed_time / interval) — a daily digest agent needs no new infra.
- Hindsight memory is already installed here, but it is per-repo knowledge; it does not give the cross-project, day-level "what did I do" view. This project complements it.

## 4. Landscape (research, Oct 2026)

### A. Heavy LLM observability platforms
Langfuse, OpenLIT, Phoenix, AgentOps, Opik, MLflow Tracing, LangWatch, OpenLIT.
- Full-featured tracing/eval/prompt-management, self-hostable.
- Wrong fit: built for instrumenting LLM *apps*, not for reading a developer's coding-agent session transcripts. Requires SDK instrumentation, a DB, Docker stack. Massive overkill for a personal journal.

### B. Grafana Agent Observability — closest off-the-shelf observability option
- Grafana Cloud Agent Observability has a free, open-source **local app** that keeps sessions on your machine, with integrations for **Claude Code, Codex, Cursor, GitHub Copilot, OpenCode, and Pi**.
- Gives: cost, tokens, tool usage by user/repo/branch; Prometheus metrics; custom dashboards.
- Does NOT give: content-level journal ("what did I do, what did I decide, why").
- Worth a 30-minute evaluation before building any dashboard ourselves (covers R5, partially R4).

### C. Session-journal / daily-digest projects (the exact pattern we want)
- **claude-daily** (github.com/Oanakiaja/claude-daily): hooks Claude Code SessionEnd → background LLM summarization → per-session markdown → daily digest `daily.md`; extracts "skills" from sessions. Claude Code only.
- **cc-session-tools plugin** (github.com/Suto-Michimasa/cc-session-tools, and the matching Claude Code feature request #29585): reads `~/.claude/projects/*.jsonl`, generates cross-project `/daily-report` (tasks by project, learnings, next actions). Claude Code only.
- **claude-code-log** (github.com/daaain/claude-code-log): JSONL → readable HTML/Markdown + TUI, date-range filtering. Claude Code only.
- **agentBrain** (github.com/frontmatters/agentBrain): portable markdown "brain" read by many agents (claims Claude Code, Copilot, Pi, Cursor) + `session-journal` add-on via hooks. Closest in *philosophy*, but journal capture relies on hooks, not transcript parsing.
- **work-buddy** (github.com/KadenMc/work-buddy): local-first runtime on Claude Code + Obsidian; journal, tasks, projects, searchable session record, React dashboard. Heavy: a whole runtime, Claude Code-centric.
- **dsh-auto-memory** (github.com/Aik358/dsh-auto-memory): three-layer memory (user rules → project notes → daily logs), self-writing daily logs via a judge subagent, 30-day distillation into project notes. Interesting ideas, host-specific.

### D. Token/cost CLIs & local dashboards
ccusage, abtop, AgentsView, Claude Code Agent Monitor, agentacct, ClawMetry ("reads session files of many agent runtimes into one timeline of tool calls and cost"), Claude Code Hooks Multi-Agent Observability, agento11y.
- Stats and dashboards: yes. Content/decision recall: no. Several are Claude Code/Codex-only; ClawMetry claims multi-runtime transcript reading — worth a quick look before building a parser.

## 5. Gap analysis

No off-the-shelf project does the whole job:
1. Reads **both Pi and Codex** local transcripts (most tools read only Claude Code's format).
2. Produces a **cross-project daily journal with decisions and why** (platforms give metrics; CLIs give tokens).
3. Is **agent-agnostic on recall** — any session, Pi or Codex, can query the record.

But the gap is small: transcripts are already on disk in both runtimes. The core is a thin reader + LLM summarizer + files on disk.

## 6. Candidate directions

- **O1 — Thin custom ledger (build, small):** parse Pi + Codex session JSONL, extract per-session facts (time, project, prompts, tool calls, files touched, commits, test runs), write a daily Markdown journal (`YYYY-MM-DD.md`) + optional SQLite for stats. Recall = any agent reads the files, or a scheduled T3 task answers queries. Ladder rungs 6–7: a few hundred lines.
- **O2 — Adopt Grafana local app for R5, skip building dashboards:** evaluate first; if its Pi/Codex views satisfy the observability want, we never build one.
- **O3 — Fork claude-daily / cc-session-tools:** proven digest UX, but everything Claude-Code-specific (paths, hooks, formats) gets rewritten; ends up the same size as O1 with more legacy.
- **O4 — Full platform (Langfuse/OpenLIT):** rejected for R6 ("not too complicated"); personal journal does not need an eval/prompt-management platform.

Recommended shape (updated after discussion): **fork claude-daily (O3)** — see section 8. Grafana (O2) rejected: it means shipping metrics to an external dashboard stack; the user wants a single contained app with front and backend together.

## 7. Open questions (to refine)

- Q1: Direction — **DECIDED** (m00027): fork claude-daily, evaluate and rewrite for Pi + Codex (section 8). Grafana (O2) rejected by user.
- Q2: Ledger format — **DECIDED**: Markdown journal + SQLite for stats.
- Q4: Trigger — digest via T3 scheduled task (fixed_time daily), on-demand only, or both?
- Q5: Should the ledger live per-machine in one place (e.g. `~/.worklog/`) or per-project in git?
- Q6: Name for the project (user undecided).

## 8. Evaluation: claude-daily as fork candidate (verified against source)

Repo facts: github.com/Oanakiaja/claude-daily — Rust, MIT, ~410KB of src across ~50 files, v0.1.8, 31 stars, last push 2026-02 (stale → a fork is fully ours from day one).

Architecture matches the "single contained app" requirement:
- CLI (`daily`) + built-in HTTP server with static UI (src/server/, handlers ~53KB) — front and backend in one binary.
- src/summarizer/ — LLM summarization engine + prompt templates (prompts ~30KB).
- src/archive/ — per-session markdown + daily digest (daily.md pattern).
- src/insights/ — collector, daily, facets, trends: the frequency stats for R4.
- src/jobs/ — background job manager (non-blocking summarization).
- src/usage/ — token/cost tracking with LiteLLM pricing table.

The seam: `src/transcript/parser.rs` (~10KB) defines `TranscriptData { user_messages, tool_calls, files_modified, summary }`. Everything downstream consumes that struct — no Claude-Code coupling past the parser.

Claude-Code-specific parts to replace/port:
- Parser: keep the Claude Code reader, add two. Formats verified against real local files:
  - Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` — event log (session_meta carries cwd, timestamps, payloads). T3-spawned sessions land here too (originator: "T3 Code").
  - Pi: `~/.pi/agent/sessions/<project-slug>/<timestamp>_<uuid>.jsonl` — event log (session / model_change / message / tool events, timestamps, cwd). Skip `*.acp.json` sidecars (compression metadata, not transcripts).
- Hooks (`src/hooks/`, install.rs, uninstall.rs): Claude Code lifecycle hooks. Pi/Codex need no hooks — their transcripts are already durable, so capture = scheduled or on-demand scan.
- Add: SQLite ledger alongside the markdown archive (D2), and a recall/query path ("what did I work on / why did I decide X") — a `query` command + API route.

Effort estimate: the fork keeps summarizer prompts, insights, jobs, server/UI (~80% of the code). New work: two parsers (~10KB each), scheduler entry, SQLite layer, recall command. Rough: a focused week for a working multi-runtime v0.

Risks:
- Rust codebase. If we do not want to maintain Rust, fallback is rewriting this architecture in ~1K lines of Python — losing the working UI, insights code, and tuned prompts.
- Upstream stale 8 months — irrelevant under MIT, but no upstream fixes to lean on.
- LLM summarizer needs provider API-key config.

### 8.1 Upstream observability mechanics (how "over time" works)

Three data sources, all recomputed on the fly from files (no time-series DB):
1. Its own markdown archive — per-session summaries + daily digests, built day by day by the summarizer. This is the longitudinal record.
2. Claude Code "facets" — per-session JSON in `~/.claude/usage-data/facets/`: goal_categories, outcome (achieved/partially/not), user_satisfaction_counts, friction_counts, session_type. Claude Code itself generates these; claude-daily only reads them (src/insights/facets.rs).
3. Usage scan — tokens/cost per session per day.

src/insights then computes over a default 30-day window:
- `DailyStat` per day: session count, tokens, cost → chart series.
- Distributions aggregated across all facets: goal categories, friction types, satisfaction, session type — the "what am I working on / where am I struggling" view (R4).
- `TrendData`: splits the window into two halves and reports % change for session count, friction rate (share of sessions with any friction), success rate (share achieved/partially_achieved), and a weighted satisfaction score (happy=100, frustrated=25). Plus per-ISO-week stats (session count, friction rate, success rate).

**Gap for our fork:** facets are a Claude Code feature. Pi and Codex produce no facet files, so friction/satisfaction/trends would be empty for them. Fix: the summarizer LLM already reads every session — extend it to also emit a facet JSON per session. Then facet generation becomes runtime-agnostic and the `~/.claude` dependency disappears entirely, Claude Code included.

## 9. Current direction (after first refinement round)

Fork claude-daily → rebrand → multi-runtime (Pi + Codex + optionally Claude Code) daily work ledger:
- Capture: read existing local transcripts (no new hooks for Pi/Codex); digest on a schedule (T3 fixed_time task or the app's own scheduler) plus on demand.
- Store: markdown daily journal + SQLite for stats queries.
- Recall: **DECIDED (m00105)** — native to the coding agents, following Hindsight's pattern (hindsight.vectorize.io/sdks/integrations/coding-agents): one shared core, a thin agent-facing surface per host. Concretely: for Pi, a Pi extension entry in `~/.pi/agent/settings.json` exposing the ledger as native tools (Hindsight proves this needs no MCP for Pi) + a companion skill in `~/.pi/agent/skills` so the agent knows when/how to consult the journal; for Codex, `~/.codex/hooks.json` + `[mcp_servers]` in `config.toml`. The CLI (`show`/`query`/`stats`) stays the underlying core and the fallback for humans.

Decided (m00105):
- Q7: **Keep Rust.** Fork claude-daily as-is, no Python rewrite.
- Q8: **Keep the Claude Code reader** alongside the new Pi and Codex parsers (all three runtimes).
- Q9/Q6: name — candidates proposed (section 13), user picks.
- Q10: **Native agent recall** (extension/skill wiring, above), not a plain CLI-only surface.

Still open:
- Q4: Digest trigger — T3 fixed_time task, app scheduler, on-demand, or both.
- Q5: Ledger location — one place per machine (e.g. `~/.daybook/`) vs per-project in git.

## 10. Research: how people use observability for coding agents (metrics catalog)

Sources: grafana.com/blog/agent-observability-faq (2026-10), docs.datadoghq.com LLM metrics (2026-09), codebridge.tech agent-observability guide, blaxel.ai/blog/ai-observability, langfuse.com agent-evaluation + metrics docs, langchain.com/blog/traces-start-agent-improvement-loop (2026-03), developers.openai.com trace-grading + evaluation-flywheel + eval-skills, arize.com agent-evals-from-traces (2026-09), Claude Code docs (monitoring-usage, analytics API, costs), github.com/opencolin/agentic-engineering.

### When people actually open the observability tools (the occasions)
1. Debug one bad run — inspect a single trace: which tool call, which prompt, which step broke.
2. Cost hunting — which sessions/models burn tokens, cache hit rate, spend per workflow, spotting pricier model than needed.
3. Systematic improvement loop — the core pattern, repeated across LangChain/OpenAI/Arize/HALO: trace → curate failures → build eval/dataset → fix prompt/tool/harness → re-run eval → ship. Trace is the input; eval is the memory.
4. Regression gating — run the eval suite against every prompt/harness change as a merge check; compare prompt versions.
5. Org-level reporting — adoption, productivity, spend dashboards (Claude Code team analytics).

### The metric catalog (what they look at)

**A. Cost & usage** (nearly everyone, first thing they track):
- tokens split: input / output / cache read / cache write; cost per session, per run, per accepted task
- cache hit rate (Claude Code shows "share of input tokens served from cache" per session)
- sub-agent spend (teams in plan mode ≈7x tokens of a standard session — Claude Code docs)

**B. Performance**: latency, time-to-first-token, error/timeout/retry rate, throughput.

**C. Trajectory & behavior** (the agent-specific layer — Langfuse/OpenAI):
- step count / tool calls per task, unnecessary tool calls, loop & thrash detection (repeated identical tool calls)
- tool-call success rate, tool selection correctness, argument validity, recovery after a failed call
- escalation/handoff rate, human override/interruption rate

**D. Quality & outcome** (what actually drives improvement decisions):
- task completion: achieved / partially / not achieved (claude-daily facets use exactly this)
- user feedback: thumbs, satisfaction counts, abandonment, rephrasing patterns
- edit acceptance rate (Claude Code: edit_tool.accepted/(accepted+rejected) — their headline productivity metric)
- LOC accepted, commits, PRs, cost per commit/PR
- friction categories (misunderstood request, tool misuse, looping...) — labels on traces
- eval pass rate on the accumulated suite

**E. Context efficiency** (closest to our R5 "tweak my prompts"):
- prompt bloat / token budget drift across versions, context window usage, compression/compaction rate
- trace→fix mapping: wrong tool chosen → fix tool descriptions/routing; reasoning drift mid-task → tighten system prompt or split steps; correct-but-misses-intent → clarify instructions; structural problem → different tool or human checkpoint (LangChain, Arize table)

**F. Process/biz outcome**: successful task completion, time saved, cost per resolved issue, attempts per accepted task.

### How the prompt-improvement loop works in practice (OpenAI flywheel / LangChain loop)
1. Analyze: read ~50 failing traces, open-code failure labels, group into categories (axial coding) — LLM-as-judge automates the labeling at scale.
2. Measure: turn recurring failure modes into graders/evals + a dataset; set baseline.
3. Improve: targeted prompt/tool/harness change.
4. Re-run: eval suite gates the change; passing evals join the permanent suite.
- LangChain's concrete quote of the workflow: a developer tells a coding agent to pull 30 days of traces, isolate negative-feedback ones, identify failure patterns, draft evals, and propose prompt changes — in one terminal session. Our ledger is exactly the substrate for that step.

### What this means for our fork
- Validated: claude-daily's facets = category D labels; its insights = D+B aggregation. That's the industry-standard core, not a quirky one-off.
- Metrics computable from Pi/Codex transcripts with NO extra LLM pass: sessions/day, tokens & cost, cache hit rate (Pi exposes acp_cache), tool calls/session, tool error rate, loop/thrash (repeated identical calls), interruption/steer rate, compression events, duration, test-command runs (bash invocations matching test commands), edit volume, commits.
- Metrics needing the facet LLM pass (D): outcome, friction categories, satisfaction. One summarizer pass per session feeds both the journal and the stats.
- R5 prompt improvement = a later read-only command over the ledger: group sessions by friction category, list failure patterns, hand the report to any agent to propose prompt/skill changes. The ledger already contains everything the flywheel's "Analyze" step needs; no separate tracing infra required.
- Grafana Agent Observability now ships a "Coding agents view" (Claude Code, Codex, Cursor, Copilot, OpenCode, Pi) forwarding sessions to their cloud — confirms this is a real category, and confirms the cloud/SaaS shape we deliberately rejected for a local single-user tool.

## 12. Post-fork build phases (roadmap)

Assumes Phase 1 decisions (Q6–Q10) are settled and the fork exists. Each phase ends with something runnable/checkable.

**P0 — Repo hygiene (half a day).**
Strip or keep Claude-Code pieces per Q8 decision; rebrand (name, binary, paths per Q9); make it build and pass its own tests; register as T3 project; drop the draft into the repo as docs/design.md.

**P1 — Transcript adapters: data in (the critical path, ~2 days).**
Unify on the existing `TranscriptData` struct. Write the Pi parser (`~/.pi/agent/sessions/<slug>/<ts>_<uuid>.jsonl`, skip `*.acp.json`) and the Codex parser (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`), each ~10KB against formats already verified against real local files. Non-LLM extraction: timestamps, duration, project path, model, tool calls + errors, test-command runs, commits.
Check: `daily scan --day today` prints every session from both runtimes with correct project attribution.

**P2 — Journal generation: the LLM pass (~1 day).**
Extend the existing summarizer prompt to also emit the facet JSON per session (12 upstream fields + `runtime` + `project_path`). Wire the scan → summarize → archive pipeline for a chosen date; add SQLite rows alongside markdown (D2). Configure provider key.
Check: running on yesterday produces per-session markdown + facet JSON + daily.md digest.

**P3 — Stats & insights (R4, ~1 day).**
Load facets + usage into SQLite queries: daily series (sessions, tokens, cost), distributions (goal categories, friction, outcome, satisfaction), 30-day trend split. Add the free metrics from transcripts (thrash, interruptions, test runs/day, cache hit rate).
Check: `daily stats --week` answers "where am I struggling, how many tests/day".

**P4 — Recall surfaces, native to the agents (R2/R3, ~1-2 days).**
Shared core: `show`/`query`/`stats` subcommands + API routes for the built-in web UI. Agent-native wiring per the Hindsight pattern (Q10 decision): Pi extension in `~/.pi/agent/settings.json` exposing native read tools (no MCP needed) + companion skill in `~/.pi/agent/skills`; Codex hooks in `~/.codex/hooks.json` + MCP server entry in `config.toml`.
Check: from a fresh Pi session in an unrelated project, "what did I work on Tuesday and why" gets a correct answer through the native tool.
**P5 — Automation + real-data soak (days of calendar time, little work).**
T3 fixed_time task (or app scheduler per Q4) runs the daily digest; leave it alone for 1–2 weeks; fix attribution bugs on real data. This produces the 30-day baseline the trends need.

**P6 — Prompt-improvement report (R5, phase 2 feature).**
A read-only `daily analyze-friction` command: group sessions by friction category, surface recurring failure patterns, output a report any agent consumes to propose prompt/skill changes (the OpenAI/LangChain flywheel "Analyze" step — no eval infra of our own).
Check: first monthly report identifies at least one actionable pattern backed by session links.

Explicitly out of scope unless asked later: eval framework/CI gating, cloud sync, multi-user, team dashboards, latency metrics.

## 13. Name candidates (Q9/Q6 — pick one)

Binary name doubles as the command every agent and human types (`<name> show`, `<name> stats --week`), so it wants short, lowercase, unambiguous in a terminal. Personal tool — global uniqueness not required, only no clash on this machine and in the Rust crate ecosystem we care about.

- **N1: `daybook` (recommended)** — the literal term for a daily record book (accounting's day-by-day ledger before posting to the ledger). Says exactly what it is: one book, one day at a time, totals roll up. Reads as "journal + ledger" without the genericness of "log".
- **N2: `logbook`** — record of events kept day by day, nautical/engineering flavor. Slightly more generic, more existing packages named logbook.
- **N3: `almanac`** — a compendium of recorded conditions and statistics over time. Emphasizes the stats/recall side over the journal side.
- **N4: `debrief`** — verb-shaped: every session gets debriefed into the book. Less descriptive of storage, memorable command.
