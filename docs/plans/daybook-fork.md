# daybook — fork claude-daily into a multi-runtime work journal

**Goal:** daybook-v1 | **Version:** 0.5 | **Status:** ready
**Agreement ref:** pending (user approval of this plan) | **Review ref:** 2026-10-28 independent review, 2 rounds (Codex gpt-6.1-sol, high effort) — round 1: REJECT, 15 findings, all applied in v0.4; round 2: REJECT, 5 remaining findings (F16 stale-revision overwrite, F17 metrics transport, F18 retirement vs pending queue, F7 npm prefix, F19 gate gaps), all applied in v0.5

## Problem

Daily work happens in many independent agent sessions (Pi, Codex, Claude Code, orchestrated by T3 Code). Sessions are durable on disk inside ephemeral workspace VMs, but nothing reads them together, so nobody can answer:

- "What did I work on today / this week / 15 days ago, and why did I decide X?"
- "Where am I struggling, how many tests am I running per day, what's my friction trend?"

Existing tools don't close the gap: LLM observability platforms (Langfuse, Phoenix, OpenLIT) instrument apps, not a developer's own sessions; CLIs (ccusage, abtop) give tokens only; journal projects (claude-daily, cc-session-tools) read Claude Code transcripts only. Full research and evaluation: `draft.md` in this directory (sections 4, 8, 10).

## Solution

Fork **github.com/Oanakiaja/claude-daily** (Rust, MIT, already forked by user as **Vikaspogu/daybook**, commit `f87be02`) into **daybook**: one binary with two roles — a client that runs inside workspace VMs, and a server deployed as a pod in the Talos cluster.

**Architecture (client captures, server owns the ledger):**

- Workspace VMs are KubeVirt VMs whose filesystems live in DataVolume disk images — pods cannot mount them. So the server pod cannot read transcripts; capture must stay client-side. The ledger also must outlive ephemeral VMs (recall spans 15–20 days), so it lives on the cluster.
- **Client (`daybook scan`, runs in-VM):** reads Pi `~/.pi/agent/sessions/<slug>/<ts>_<uuid>.jsonl` (skip `*.acp.json`), Codex `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, and Claude Code transcripts; unifies on upstream's `TranscriptData` in `src/transcript/parser.rs` — **widened**, because upstream's struct lacks normalized session identity, project, duration, model, and usage (verified: `parser.rs:27` has only `entries`, `user_messages`, `assistant_messages`, `tool_calls`, `files_modified`, `summary`). Parsers emit a `SessionMeta { session_id, runtime, project_path, started_at, duration, model, usage, content_hash }` alongside `TranscriptData`. `runtime` and `project_path` are **derived from transcript metadata (cwd/session events), never LLM output**.
- **LLM pass, client-side:** upstream's summarizer shells out to `claude --model … --print` (`src/summarizer/engine.rs:44`) — NOT a provider API client, so "the VM already has provider keys" is not sufficient. S3 first verifies a noninteractive LLM call works in the real VM (Claude Code CLI if present and authenticated; otherwise the shell-out is replaced with a direct provider API call configured by env). The condensation step (`to_condensed_text`, `parser.rs:193`) is extended to include **bounded assistant messages and tool outcomes** — upstream drops them, so decisions that exist only in agent replies would never reach the journal.
- **Server (`daybook serve`, the pod):** authenticated ingest API (per-machine tokens minted from the web UI, D7) + SQLite ledger on a PVC + query/stats API + the inherited web UI. Same binary, so local dev is just `daybook serve --data-dir ~/.daybook`.
- **Ingest is revision-aware, not idempotent-blind:** a 9pm scan can summarize a session that is still open; later work must not be discarded. Each push carries `content_hash` plus a monotonic revision marker (transcript byte size + mtime); the server upserts on session id when the hash changed, no-ops when identical, and **rejects pushes whose revision is older than the stored record** — a delayed retry of an older pending record can never overwrite a newer revision already in the ledger. Retries are safe and out-of-order delivery is safe.
- **Privacy, stated precisely:** raw transcripts are never uploaded to the ledger — only summaries, facets, and usage numbers. The LLM pass does send condensed excerpts (including user-message text) to the configured LLM provider; that is inherent to summarization and is not a "never leaves the VM" guarantee.
- **Recall, native to agents:** Pi extension (native tools, no MCP) + companion skill, Codex MCP stdio server (runs in-VM, proxies HTTP), CLI, and the web UI — all thin clients of the same server core. No daemon in the VM: faces call the pod directly.
- **UI/routes:** the inherited router exposes VM-local operations (digest execution, job control, installers, conversation loading from local transcript paths — `src/server/router.rs:14`, `handlers.rs:749`). These are stripped or ported to ledger-backed equivalents in S3; the pod has no transcripts and no LLM config. The inherited frontend sends no auth header (`web/src/hooks/useApi.ts:289`) — S5 adds browser auth.
- **Capture coverage across VMs:** a scan reads only its own VM's filesystem. Coverage model: Pi `session_shutdown` hook (every VM with the extension; fires on shutdown AND reload, so pushes must be revision-aware and cheap) is the primary capture; a per-VM 9pm T3 scheduled task is the catch-up (still-open sessions, Codex sessions which have no hook); a final on-demand `daybook scan` before a VM is retired. Re-runs always dedupe safely.

1. **Journal flow** — 9pm catch-up per VM; `session_shutdown` pushes; on-demand scan anytime. Pending pushes queue on disk (`~/.daybook/pending/`) with retry on next scan/run, so a dead pod or flaky network never loses a record.
2. **Stats** — from the ledger: sessions/day, tokens, cost, test-runs/day, friction/outcome/satisfaction distributions, 30-day trend, thrash/interruption/cache-hit metrics. Event-derived aggregates (test runs, thrash, interruptions, cache-hit) are computed client-side during scan and pushed with the record — the server never sees raw transcripts, so it cannot derive them itself.
3. **Durable-journal thread** — a pinned T3 thread "journal"; the 9pm task posts the day's digest there; user can message it anytime. No long-running agent process (rejected: daemon + second drifting state).
4. **Prompt-improvement report** (later phase) — read-only `analyze-friction` grouping sessions by friction category, feeding any agent the "Analyze" step of the trace→eval→prompt flywheel.
5. **Deployment** — home-ops component following the repo's documented pattern (see `## Deployment`). The pod is internal-only.

### Decisions

- **D1 name — DECIDED:** `daybook`. Fork: https://github.com/Vikaspogu/daybook (fork of Oanakiaja/claude-daily, MIT, branch `main`, commit `f87be02`).
- **D2 ledger location — DECIDED by deployment requirement:** SQLite on the server pod's PVC (`ai` namespace, ceph-block, VolSync backup). `~/.daybook/` remains only the local dev data dir + pending-push queue.
- **D3 deployment — DECIDED (user, m00183):** daybook server runs as a pod in the Talos cluster via the home-ops GitOps repo.
- **D4 summarizer backend — DECIDE IN S3 (verify first):** keep upstream's `claude --print` shell-out only if the Claude Code CLI is installed and noninteractively authenticated in the workspace VMs; otherwise replace `engine.rs:44` with a direct provider API call (env-configured key). Gate: one real noninteractive summarization succeeds in a real VM.
- **D5 ingest contract — DECIDED:** revision-aware upsert keyed on session id with `content_hash` change detection plus freshness: unchanged replay = no-op; appended transcript = updated record; a push carrying an older revision than the stored record is rejected (stale-retry protection — pending-queue retries may arrive out of order). No work is ever discarded because it shares a session id with an earlier scan.
- **D6 day boundaries — DECIDED:** a session's journal date derives from the transcript's own start timestamp (not `Local::now()` at scan time, which upstream uses at `engine.rs:176`), interpreted in the VM's local timezone. A session spanning midnight belongs to its start day; the summary may note the span.
- **D7 machine auth — DECIDED (user, m00293):** per-machine tokens minted from the web UI. Two roles: one admin token (ExternalSecret, UI login, mints/revokes machine tokens) and one machine token per VM (ingest + query only, shown once at creation, stored only as a hash in the ledger with machine label + last-used). Revoking a machine token blocks only that machine; records carry the machine label for attribution. Single shared token rejected: no attribution, no revocation without re-provisioning every VM.

## Variables & Scope Boundaries

- `PROJECT_ROOT`: `~/git-repos/daybook` (clone of Vikaspogu/daybook, created by S1)
- `PLAN_FILE`: `docs/plans/daybook-fork.md` (this file, copied into the repo at S1)
- `HOME_OPS_ROOT`: `~/git-repos/home-ops` (existing clone of Vikaspogu/home-ops)
- `WORKING_DIR`: `PROJECT_ROOT`
- `READ_ONLY_DIRS`: this scratch directory (`draft.md` is the research archive; never edited by implementation)
- `ALLOWED_PATHS`: `PROJECT_ROOT/**`, `HOME_OPS_ROOT/components/ai/daybook/**`, `HOME_OPS_ROOT/clusters/talos/apps/20-applications.yaml`, `~/.pi/agent/settings.json`, `~/.pi/agent/skills/daybook/**`, `~/.codex/config.toml`, `~/.daybook/**`
- `READ_ONLY_INPUT_PATHS` (reads required, writes forbidden — reading these is part of the product; modifying or deleting them is instant failure): `~/.pi/agent/sessions/**`, `~/.codex/sessions/**`, `~/.claude/**`
- Note: tests copy small redacted fixtures out of these dirs into `PROJECT_ROOT/tests/fixtures/`; fixtures are committed, originals untouched.

### Relevant Files

- **Existing (upstream, kept):** `src/transcript/parser.rs` (the seam — widened with `SessionMeta`), `src/summarizer/` (prompts ~30KB; `engine.rs:44` shells `claude --print`, `engine.rs:114` invokes the Claude parser directly — both adapted in S3), `src/insights/` (facets, trends; `collector.rs:78` reads Claude usage directly — migrated to ledger), `src/jobs/`, `src/archive/` (markdown + digest), `src/server/` (HTTP + UI; `static_files.rs:11` embeds `web/dist` — frontend build is a prerequisite), `src/usage/`
- **Existing (removed/replaced):** `src/cli/commands/` installer/uninstaller commands (Claude Code lifecycle hooks → our scan+push triggers), VM-local server routes (`router.rs:14` digest/jobs/install, `handlers.rs:749` conversation-from-filesystem)
- **Existing (frontend):** `web/` — TypeScript app; `npm ci && npm run build` must run before Rust build/tests (upstream CI does this at `.github/workflows/ci.yml:44`)
- **Facet schema:** upstream `SessionFacet` has **11** LLM fields; we keep those 11 as LLM output and store `runtime`, `project_path` as session metadata columns (D6-adjacent, never LLM output)
- **New:** `src/transcript/pi.rs`, `src/transcript/codex.rs` (parsers), `src/ledger/` (SQLite store behind the server), `src/push/` (client → server ingest + pending queue), Pi extension file (lives in-repo under `PROJECT_ROOT/extensions/`, referenced from `~/.pi/agent/settings.json` — nothing installs outside ALLOWED_PATHS), `~/.pi/agent/skills/daybook/SKILL.md`, `Dockerfile` + image workflow
- **home-ops (new component, per AGENTS.md pattern):** `components/ai/daybook/{kustomization.yaml,values.yaml,http-route.yaml,externalsecret.yaml}` + registration in `clusters/talos/apps/20-applications.yaml` (sync wave 20)

## Deployment

Follows the home-ops AGENTS.md application pattern exactly:

- **Component:** `components/ai/daybook/` — bjw-s app-template v4.6.2 values (pinned image tag, `readOnlyRootFilesystem: true`, drop ALL caps, requests + memory limits), kustomization with VolSync component for the PVC (storage + snapshot + cache capacity + schedule variables, as in the existing VolSync example at `clusters/talos/apps/20-applications.yaml:683`). Note: hindsight is NOT the reference for this — it uses its own Helm chart with external PostgreSQL.
- **Image:** GitHub Actions in the daybook repo publishes `ghcr.io/Vikaspogu/daybook:<tag>` (must build `web/dist` before the Rust build); home-ops references the pinned tag (Renovate handles bumps).
- **Secrets:** 1Password item → ExternalSecret supplying the **admin token** only (UI login; mints/revokes machine tokens). Machine tokens are created at runtime from the UI and stored only as hashes in the ledger — never in this repo. No LLM config in the cluster — all LLM calls happen client-side in the VMs.
- **Client credentials:** each VM stores its own per-machine token at `~/.daybook/token` (file, 0600), minted from the UI when that VM is enrolled (D7) for scan/push; Pi extension and Codex MCP read it from there (MCP server launched with `DAYBOOK_TOKEN_FILE` env or explicit config). Browser auth: the web UI prompts for the token once and stores it client-side (internal-only route; no OAuth machinery).
- **Ingress:** Gateway API HTTPRoute, internal hostname `daybook.${CLUSTER_DOMAIN}` (no public exposure — journal data is personal).
- **Storage:** `config` PVC on `ceph-block` via VolSync (declared capacity, csi-ceph-blockpool snapshot class).
- **Registration:** entry in `clusters/talos/apps/20-applications.yaml` pointing at `components/ai/daybook`, sync wave 20.

## Workflow

1. **Plan.** User agrees to this plan, independent review runs, status → ready. Then user explicitly selects execution.
2. **Build.** Execute `## Vertical Slices` in order. Update status markers (`[ ]` → `[x]`) at task and slice level; never start a slice while the previous gate fails.
3. **Verify.** Check every `## Definition Of Done` item, fix, rerun.

## Vertical Slices

| Slice | Purpose | Done when |
| --- | --- | --- |
| S1: Fork & hygiene | Own the codebase | Clone builds (frontend + Rust), passes upstream tests, renamed |
| S2: Transcript adapters + discovery | Data in from all runtimes | Non-LLM scan lists Pi + Codex + Claude Code sessions with metadata-derived attribution |
| S3: Journal + server core | Summarize, push, store | Revision-aware push against a running server; VM-local routes stripped |
| S4: Stats & insights | R4 | `daybook stats --week` answers struggle/test-frequency questions against fixtures with expected values |
| S5: Recall surfaces + auth | R2/R3 | Both runtimes answer cross-project recall via the server; auth enforced incl. browser |
| S6: Deploy to cluster | D3 | Pod running in `ai`, UI reachable internally, data survives pod restart |
| S7: Automation & soak | Freshness + real-data proof | Two VMs covered; 9pm digests in journal thread unattended ≥7 days |
| S8: Friction report | R5 phase 2 | First monthly report identifies ≥1 actionable pattern with session links |

### [x] Slice 1 (S1): Fork & hygiene (commit 2221780, review 434b389)

- **Purpose / Done when:** the clone exists as our repo, builds end to end (frontend embedded), and passes its own tests under the chosen name.
- **Blocked by:** none
- **Tasks:**
  - [ ] Clone Vikaspogu/daybook into `~/git-repos/daybook`, strip dead Claude-Code hook installer CLI commands per design, rebrand binary/paths to `daybook`/`~/.daybook/`
  - [ ] `npm --prefix web ci && npm --prefix web run build` (produces `web/dist`, embedded via `static_files.rs:11`; runnable from `WORKING_DIR`), then `cargo test`; register as T3 project; copy this plan into repo as `docs/plans/daybook-fork.md`, draft.md as `docs/design-research.md`

- **Verification Gate:** `npm --prefix web ci && npm --prefix web run build && cargo build && cargo test` all exit 0 from `WORKING_DIR`; `daybook --help` prints rebranded usage; `web/dist` exists.
### [x] Slice 2 (S2): Transcript adapters + discovery (commits 7519014, 4407a35..HEAD)

- **Purpose / Done when:** all three runtimes' sessions parse into widened `TranscriptData` + `SessionMeta` (session_id, runtime, project_path from cwd metadata, start timestamp, duration, model, usage), and a non-LLM scan command already exists so its gate can run.
- **Blocked by:** S1
- **Tasks:**
  - [ ] Widen `TranscriptData`/add `SessionMeta` in `src/transcript/parser.rs`; derive journal date from transcript start timestamp (D6), never `Local::now()`
  - [ ] Project attribution rule: resolve project from the session's own cwd metadata (local git-root resolution at parse time, no network); a session recorded in a git worktree or a T3 scratch dir attributes to its parent project
  - [ ] `daybook scan --no-llm --local --day <d>`: discovery + parse only, prints session list (runtime, project, start, duration, model, usage) — this command lives here so the S2 gate is runnable
  - [ ] Write `src/transcript/pi.rs` (skip `*.acp.json`), `src/transcript/codex.rs` (session_meta cwd; originator "T3 Code"); keep upstream Claude Code parser
  - [ ] Token accounting policy: prefer `token_usage_record` when present, fall back to `token_count` (both exist in Codex rollouts); Pi usage from token fields on assistant events
  - [ ] Extend `to_condensed_text` (`parser.rs:193`) to include bounded assistant messages + tool outcomes
  - [ ] Unit tests against small redacted fixtures copied into `tests/fixtures/`, including a truncated JSONL, an unknown-event-type line (skipped with warning, never a crash), and worktree/T3-scratch-dir sessions for attribution
- **Verification Gate:** `daybook scan --no-llm --local --day today` lists every Pi + Codex session with metadata-derived project attribution; a Pi and a Codex session from the same project attribute to it; a session recorded in a worktree or T3 scratch dir attributes to its parent project; fixture test with malformed input passes without panic; a session started yesterday and scanned today shows yesterday's date.

### [x] Slice 3 (S3): Journal + server core

- **Purpose / Done when:** summarize → push → revision-aware store works end to end against a locally running server; the summarizer no longer depends on Claude-only assumptions; VM-local routes are gone.
- **Blocked by:** S2
- **Tasks:**
  - [ ] **D4 resolution:** verify a noninteractive LLM call in the real VM (`claude --print` if Claude Code CLI is authenticated there; else replace `engine.rs:44` with a direct provider API client, env-configured). Gate: one real summarization succeeds.
  - [ ] Adapt summarizer `engine.rs:114` to accept pre-parsed `TranscriptData` from any adapter instead of invoking the Claude parser on a file path; keep the 11-field `SessionFacet` prompt
  - [ ] `daybook serve`: ingest API (bearer auth against a machine-token table: token hash, machine label, created/last-used, revocation — D7) + SQLite ledger (`src/ledger/`, schema: sessions incl. `content_hash`, facets, usage, machine label) + query/stats API; ingest upserts on session id when `content_hash` changed, no-op when identical (D5)
  - [ ] Push client in `src/push/`: POST after local LLM pass; on failure the record is written to `~/.daybook/pending/` and retried on next scan/run (idempotent ingest makes retries safe)
  - [ ] Client-side aggregates: compute test runs, thrash, interruptions, cache-hit rate from parsed events during scan and include them in the pushed record (the server cannot derive them from transcripts it never sees)
  - [ ] Strip or port VM-local server routes: digest execution, job control, installer, conversation-from-filesystem (`router.rs:14`, `handlers.rs:749`) — conversation view ports to ledger data or is removed from the UI
  - [ ] `daybook scan --local` dev mode that skips push and writes to `~/.daybook` directly
- **Verification Gate:** run against yesterday → records land in the ledger via real HTTP push with persisted markdown summary, all 11 facet fields, and runtime/project metadata columns; (a) second push of unchanged sessions = no-op (row count + content unchanged), (b) push of a session whose transcript grew = updated record, (c) deliver newer revision B, then retry older queued revision A → A is rejected, B remains; kill the server mid-push → record lands in `~/.daybook/pending/` and uploads on the next scan; condensation test: a decision stated only in an assistant message appears in the summary.

### [ ] Slice 4 (S4): Stats & insights

- **Purpose / Done when:** R4 answered from the ledger: daily series, distributions, 30-day trend, plus free transcript metrics (thrash, interruptions, test runs/day, cache hit rate) — computed against fixtures with expected values.
- **Blocked by:** S3
- **Tasks:**
  - [ ] Migrate `src/insights/collector.rs:78` (reads Claude usage directly) to the ledger; compute metrics per runtime
  - [ ] Seed a fixture ledger with known values; stats tests assert expected totals, not just field presence, covering every promised metric: sessions/day, tokens, cost, friction distribution, test runs/day, thrash, interruptions, cache-hit rate, 30-day trend
- **Verification Gate:** `daybook stats --week` (CLI against server) prints sessions/day, tokens, cost, friction distribution, tests/day, and matches the fixture's expected numbers.

### [ ] Slice 5 (S5): Recall surfaces + auth

- **Purpose / Done when:** both runtimes can query the ledger natively; auth is enforced everywhere including the browser.
- **Blocked by:** S3
- **Tasks:**
  - [ ] Pi extension (`~/.pi/agent/settings.json`): native read tools (HTTP queries, no MCP) + `session_shutdown` handler firing a detached per-session scan+push (revision-aware; shutdown fires on reload too, so pushes must be cheap and dedupe-safe) + companion skill `~/.pi/agent/skills/daybook/SKILL.md`; token from `~/.daybook/token`
  - [ ] Codex: MCP stdio server (in-VM, proxies server HTTP) entry in `config.toml`, token via `DAYBOOK_TOKEN_FILE` env
  - [ ] Web UI served by `daybook serve`: admin-token prompt stored client-side (inherited `useApi.ts:289` sends no header — add it); token management page (mint per-machine token → shown once; list machines with last-used; revoke) — admin-token-gated, machine tokens cannot manage tokens; monthly view, stats
  - [ ] Auth gate tests: valid token → 200; missing/wrong token → 401 on both ingest and query APIs
- **Verification Gate:** from a fresh Pi session in an unrelated project, "what did I work on Tuesday and why" answers correctly via the native tool; same question from a fresh Codex session via MCP; closing a Pi session pushes its record without the 9pm job; 401s verified for missing/wrong/revoked tokens — revoking machine A's token blocks only A while machine B still pushes; reload-triggered shutdown causes no duplicate or lost records.

### [ ] Slice 6 (S6): Deploy to cluster

- **Purpose / Done when:** the server runs as a pod in the Talos cluster via home-ops GitOps, reachable from workspace VMs, data durable across pod restarts.
- **Blocked by:** S4, S5
- **Tasks:**
  - [ ] Dockerfile (npm build for `web/dist`, then multi-stage Rust build, distroless/static base) + GitHub Actions publishing `ghcr.io/Vikaspogu/daybook`
  - [ ] home-ops `components/ai/daybook/` per the Deployment section (kustomization, app-template values, http-route, externalsecret for bearer token, VolSync with storage/snapshot/cache-capacity/schedule variables)
  - [ ] Register in `clusters/talos/apps/20-applications.yaml` sync wave 20; ArgoCD sync green
- **Verification Gate:** pod Running in `ai`, `https://daybook.<cluster-domain>` serves the UI internally with working auth; PVC bound; delete the pod → it reschedules and the ledger survives with all records (restart durability); VolSync backup configured; a scan+push from a workspace VM lands in the pod's ledger.

### [ ] Slice 7 (S7): Automation & soak

- **Purpose / Done when:** capture covers every transcript-owning VM with zero manual steps, then proves itself on real data.
- **Blocked by:** S5, S6
- **Tasks:**
  - [ ] Per-VM coverage: enroll each VM with its own machine token minted from the UI (D7 — no shared secret on any disk); Pi extension everywhere (primary), 9pm T3 scheduled task per active workspace as catch-up (still-open sessions, Codex sessions), final `daybook scan` before a VM is retired — retirement waits until `~/.daybook/pending/` is empty (all records acknowledged), otherwise the queue dies with the VM
  - [ ] 9pm task posts digest prompt to pinned "journal" T3 thread; missed-run policy: catch-up scans use `--since last-push`-style incremental mode, and dedupe makes late runs harmless
  - [ ] Pin the journal thread in T3; leave running 1–2 weeks; fix attribution bugs on real data (builds the 30-day baseline trends need)

- **Verification Gate:** sessions from two different workspace VMs both appear in the pod ledger; morning after first run: digest visible in journal thread with no manual command run; 7 consecutive unattended days; simulate a missed 9pm run → next run catches up with no gaps or duplicates; retire a VM with the server down → queue drains and all records deliver after recovery, nothing lost.
### [ ] Slice 8 (S8): Friction report

- **Purpose / Done when:** R5's improvement loop has its Analyze step.
- **Blocked by:** S7 (needs ~30 days of facets)
- **Tasks:**
  - [ ] `daybook analyze-friction`: group sessions by friction category, surface recurring failure patterns, link sessions, output a report any agent consumes to propose prompt/skill changes
- **Verification Gate:** first monthly report identifies at least one actionable pattern backed by session links.

## Definition Of Done

### Workflow Complete

- [ ] Plan agreed, independently reviewed, then explicitly selected for execution.
- [ ] All changes stay within `ALLOWED_PATHS`.

### Capture

- [ ] `daybook scan` parses Pi, Codex, and Claude Code transcripts; unknown/malformed lines are skipped with a warning, never a crash. Verify: run against a fixture set including a truncated JSONL.
- [ ] Session attribution (runtime, project, date) derives from transcript metadata, never from scan time or LLM output. Verify: session started yesterday, scanned today, dates yesterday; sessions from a worktree or T3 scratch dir attribute to the project.
- [ ] Ingest is revision-aware: unchanged replay no-ops; a session that grew after an earlier scan is updated, not discarded. Verify: row content + count checks for both cases.
- [ ] Stale revisions never overwrite newer ledger content. Verify: deliver newer revision B, then retry older pending A → A rejected, B remains (S3 gate).
- [ ] Failed pushes are not lost: killed server mid-push → record in `~/.daybook/pending/`, delivered on next scan. Verify: S3 gate.
- [ ] VM retirement waits for an empty pending queue. Verify: retire with server down → queue drains after recovery, nothing lost (S7 gate).
- [ ] Condensation includes assistant messages/tool outcomes. Verify: a decision stated only in an assistant reply appears in the summary (S3 gate).
- [ ] Raw transcripts are never uploaded to the ledger — only summaries, facets, usage. Verify: inspect request bodies in a capture. (Privacy note: the LLM provider does receive condensed excerpts; that is inherent to summarization.)

### Journal & Stats

- [ ] Each scanned session yields markdown summary + 11-field facet JSON; `runtime` and `project_path` stored as metadata columns.
- [ ] `daybook stats --week` reports sessions/day, tokens, cost, friction distribution, test runs/day, and matches fixture expected values.

### Recall & auth

- [ ] Pi native tool + skill installed; Codex MCP configured; both answer cross-project recall questions (S5 gate).
- [ ] Auth enforced on ingest AND query APIs AND browser: valid → 200, missing/wrong → 401 (S5 gate).

### Deployment

- [ ] Pod runs in `ai` with pinned image tag, `readOnlyRootFilesystem: true`, requests + memory limits set.
- [ ] UI reachable at internal `daybook.<cluster-domain>`; PVC on ceph-block with VolSync backup; ExternalSecret resolves from 1Password.
- [ ] Durability proven: pod deleted → rescheduled → ledger intact (S6 gate).
- [ ] Workspace VMs can reach the service in-cluster; a scan+push from a VM lands in the pod ledger.

### Automation

- [ ] Capture covers every transcript-owning VM (extension + per-VM 9pm catch-up + retirement scan). Verify: two VMs' sessions in the ledger (S7 gate).
- [ ] 9pm T3 task ran unattended for ≥7 consecutive days producing digests in the journal thread.

## How You're Graded

- Graded continuously on every completed bullet in `## Definition Of Done`.
- Workflow steps in order: Plan, Build (slices S1→S8), Verify.
- Instant failure: deliverables outside `ALLOWED_PATHS` (except `PLAN_FILE`), **writing to or deleting anything under `READ_ONLY_INPUT_PATHS`** (reads are required; the transcripts are the product's input, never its output), marking a slice done with unchecked gates, or steering away from the plan (unrequested abstractions, new dependencies, out-of-scope refactors).
- On boundary violation or unresolvable failure: mark `[f]`, stop, preserve work, report.

## Change / review record

| Version | Change | Agreement / review |
| --- | --- | --- |
| 0.1 | Initial draft from research + discussion rounds (draft.md §1–13, conversation m00001–m00151) | pending |
| 0.2 | D1 resolved: user forked upstream as Vikaspogu/daybook; S1 clones instead of forking | fork created by user |
| 0.3 | D3 added (user, m00183): client-captures/server-owns-ledger architecture; ledger on pod PVC (D2); S6 deploy slice; home-ops paths in ALLOWED_PATHS | pending |
| 0.4 | Applied all 15 findings from independent review (Codex gpt-6.1-sol 2026-10-28): F1 read-only transcript boundary (reads allowed, writes forbidden); F2/F12 revision-aware ingest (D5) + pending-push queue + shutdown-on-reload handling; F3 widened `SessionMeta` seam + summarizer/insights migration tasks; F4 D4 summarizer-backend verification (`engine.rs:44` shells `claude --print`); F5 condensation includes assistant messages; F6 metadata-derived attribution + D6 day boundaries; F7 frontend build prerequisite (S1, image); F8 non-LLM scan moved into S2; F9 full bearer-auth client flow (token file, browser, MCP env, 401 gates); F10 VM-local routes stripped/ported; F11 per-VM coverage model; F13 fixture-expected-value gates, token accounting policy, Codex recall check, restart-durability gate; F14 precise privacy wording; F15 hindsight-not-the-reference correction, VolSync example ref, installer path fix, 11-field facet fix | review 2026-10-28: REJECT on v0.3, findings applied; re-review pending |
| 0.5 | Applied round-2 findings: F16 freshness rule — push carries revision marker (size+mtime), server rejects older-than-stored revisions, S3 gate delivers B then A and asserts B remains; F17 client-side aggregates (test runs, thrash, interruptions, cache-hit) computed at scan and pushed with the record, S4 fixtures cover every promised metric; F18 retirement waits for empty pending queue + S7 gate with server down; F7 S1 command uses `npm --prefix web`; F19 worktree/T3-scratch attribution rule + fixtures (S2), S3 gate asserts markdown/11 facets/metadata columns; S6 blocked-by includes S5; Pi extension file lives in-repo | round 2: REJECT on v0.4, 5 findings applied; status → ready |
| 0.6 | D7 resolved (user, m00293): per-machine tokens minted from the web UI — two roles (admin token via ExternalSecret for UI login + token management; per-VM machine tokens for ingest/query only, stored as hashes with machine label, revocable per machine); Solution/Deployment/S3/S5/S7 updated; records carry machine label | user decision, applied same session |
