# Daybook Design System — Quiet Precision / Dark Obsidian

Daybook uses a dark, tactile, high-contrast operational interface (**"Quiet Precision / Dark Obsidian"**) optimized for mobile-first thumb reach (`390px–430px` viewports) and multi-column desktop operational density.

---

## 1. Typography & Hierarchy

- **Primary UI & Display Font**: `'Figtree'` (weights `400`, `500`, `600`, `700`, `800`) with `font-feature-settings: "cv02", "cv03", "cv04", "cv11"`.
- **Numeric & Telemetry Font**: `'JetBrains Mono'` (weights `500`, `600`) with `font-variant-numeric: tabular-nums` on every token count, dollar spend, cache-hit percentage, duration, test/error count, and timestamp so numbers never jitter when values update.
- **Type Scale**:
  - **Hero / Page Title**: `22px–26px`, weight `800`, `letter-spacing: -0.03em`, `line-height: 1.1`, color `#F8FAFC`.
  - **Section Kicker (Left)**: `11px`, weight `700`, uppercase, `letter-spacing: 0.10em`, color `#64748B`.
  - **Section Meta Badge (Right)**: `11px`, weight `600`, monospace (`JetBrains Mono`), color `#475569`.
  - **Card Title**: `14px`, weight `700`, `letter-spacing: -0.015em`, single-line ellipsis (`white-space: nowrap; overflow: hidden; text-overflow: ellipsis`), color `#F8FAFC`.
  - **Card State / Subtitle**: `12px`, weight `500`, `tabular-nums`, color `#94A3B8` when idle or semantic accent when active.

---

## 2. Color System & Semantic Surface Tinting

- **Canvas Background**: `#0B0E14` (deep carbon obsidian — never pure `#000000` and never muddy gray).
- **Default Card Surface (Idle)**:
  - Background: `linear-gradient(180deg, rgba(24, 29, 40, 0.88) 0%, rgba(16, 20, 28, 0.92) 100%)`
  - Border: `1px solid rgba(255, 255, 255, 0.06)`
  - Top-edge Specular Highlight: `inset 0 1px 0 rgba(255, 255, 255, 0.04)`
- **Semantic Active States** (tint the surface, border, and icon badge only when a metric, session, or alert is actively engaged — idle items stay quiet in neutral slate):
  - **Emerald (Shipped Work / Achieved / High Signal / Clean Execution)**:
    - Surface: `linear-gradient(135deg, rgba(16, 185, 129, 0.14) 0%, rgba(16, 20, 28, 0.92) 100%)`
    - Border: `rgba(52, 211, 153, 0.30)` | Icon Badge: `rgba(16, 185, 129, 0.18)` | Accent Text/Icon: `#34D399`
  - **Amber (Active Spend / Key Decisions / Workflow Rules / Follow-Up Actions)**:
    - Surface: `linear-gradient(135deg, rgba(245, 158, 11, 0.16) 0%, rgba(16, 20, 28, 0.92) 100%)`
    - Border: `rgba(251, 191, 36, 0.34)` | Icon Badge: `rgba(245, 158, 11, 0.20)` | Accent Text/Icon: `#FBBF24`
  - **Sky Cyan (Prompt Cache / Code Reviews / Cross-Runtime Presence / Codebase Learnings)**:
    - Surface: `linear-gradient(135deg, rgba(56, 189, 248, 0.14) 0%, rgba(16, 20, 28, 0.92) 100%)`
    - Border: `rgba(56, 189, 248, 0.30)` | Icon Badge: `rgba(56, 189, 248, 0.18)` | Accent Text/Icon: `#38BDF8`
  - **Coral Rose (Pitfalls & Gotchas / Blocked Runs / Tool Errors / Waste Alerts)**:
    - Surface: `linear-gradient(135deg, rgba(244, 63, 94, 0.16) 0%, rgba(16, 20, 28, 0.92) 100%)`
    - Border: `rgba(251, 113, 133, 0.36)` | Icon Badge: `rgba(244, 63, 94, 0.20)` | Accent Text/Icon: `#FB7185`

---

## 3. Layout, Proportions & Responsive Grid

- **Mobile-First, Desktop-Dense**:
  - Single-column vertical flow on mobile (`390px–430px` viewport) with 2-column compact tile grids (`gap: 10px`) and 3-column quick-action strips (`gap: 8px`).
  - Multi-column operational section grid on desktop (`min-width: 340px` per column, `gap: 20px`) with zero horizontal overflow.
- **Component Geometry**:
  - **Hero Summary Card**: `border-radius: 22px`, `padding: 18px 20px`, pairing title + live telemetry line on the left with compact runtime presence pills (`pi`, `t3-code`, `codex`) on the right, followed by semantic status pills (`border-radius: 999px`, `padding: 6px 12px`, `6px` glowing status dot).
  - **Interactive Control Tile (2-up grid)**: Fixed `height: 66px`, `border-radius: 18px`, `padding: 12px 14px`, horizontal grid `"i n" / "i l"` (`38px × 38px` `rounded-12px` icon squircle on the left, bold single-line title + `tabular-nums` state on the right).
  - **Quick Action Button (3-up row)**: Fixed `height: 60px`, `border-radius: 16px`, centered icon/dot + label + monospace status count (`Shipped`, `Reviews`, `Blocked` or `All`).
  - **Floating Bottom Glass Dock (Mobile)**: `backdrop-filter: blur(20px)`, `background: rgba(15, 19, 28, 0.88)`, `border: 1px solid rgba(255, 255, 255, 0.08)`, `border-radius: 22px`, with reserved bottom page padding (`110px`) so the dock never overlaps the last row of cards. Provides 1-thumb access to **Brief**, **Dates** (Archive Drawer), **Insights**, and **Settings**.

---

## 4. Data Hygiene & Anti-Bloat Rules

- **Crisp Human Labels**: Never render raw session UUIDs, filesystem prefixes (`/home/agent/git-repos/...`), or 40-character scratch folder slugs (`2026-10-28-i-want-to-create-a-...`). Normalize projects to 1–2 word labels (`daybook`, `agent-config`, `home-ops`, `scratch`) and format session titles cleanly.
- **Noise Filtering**: Filter out automated background survey runs (Hindsight `codebaseSurvey`), `PONG` health probes, and empty/unknown sessions so only real engineering work and actionable alerts appear.
- **Active-First Sorting**: Active and shipped states sort first (`work` → `review` → `blocked`, and active semantic tiles before idle slate tiles) so the entire day's operational status can be scanned in under 2 seconds.

---

## 5. Daybook Brand & Domain Adaptations

- **Project Icon (`assets/daybook-icon.png`, `web/public/daybook-icon.svg`)**:
  - Dark Obsidian squircle (`#181D28` → `#0B0E14`) with a subtle `rgba(251, 191, 36, 0.32)` border, radial amber glow, rising sun disc (`#FDE68A` → `#F59E0B`), and an open ledger displaying Emerald (`#10B981`), Sky Cyan (`#38BDF8`), and Amber (`#F59E0B`) telemetry lines.
- **5-Minute Daily Brief Mapping**:
  1. **Hero & 4-Tile Telemetry Grid**: Instant 2-second scan of `Shipped vs Review` (Emerald), `Active Spend` (Amber), `Prompt Cache` (Sky Cyan), and `Command Health` (Coral Rose when errors/interrupted > 0, quiet slate when clean).
  2. **Desktop Multi-Column Operational Layout**:
     - Left/Primary Column: **What Shipped & Why — By Project** + **Pitfalls (`Coral Rose`) & Learnings (`Sky Cyan`) — By Project** (including `[PITFALL]` and `[LEARNING]` entries from `work`, `review`, and `blocked` sessions).
     - Right/Secondary Column: **Cost, Cache & Command Health + Next Steps**, **Scoped Reusable Knowledge Cards**, and **Sessions** (with 3-up `60px` quick-filter strip and `66px` interactive session control tiles that expand on click to inspect full session markdown).
- **Scoped Workflow Knowledge Cards (`v2` Taxonomy)**:
  1. **Global Reusable Skills (`~/.agents/skills/<name>/SKILL.md` / `~/.claude/skills/<name>/SKILL.md`)**:
     - Displays `Scope: global-skill` candidates with an Emerald `global-skill` badge, source project badge, imperative summary, structured `Trigger` / numbered `Action` steps / `Verify` / `Evidence` breakdown, and one-click `Copy SKILL.md` + `Copy All SKILL.md` actions.
  2. **Repository Rules (`<repo>/AGENTS.md`)**:
     - Groups `Scope: repo-rule` candidates by repository with a per-project header, `Copy for <repo>/AGENTS.md` group button, and per-rule `Copy Rule` button producing copy-ready imperative markdown (`### <Rule Name>` + `When` / `Do` / `Verify`).
  3. **No Server-Side Install Button**:
     - Because Daybook runs as a centralized multi-host ledger server, cards provide copy-ready `SKILL.md` and `AGENTS.md` payloads instead of calling a local filesystem install endpoint.
- **Multi-Worktree Agent Coordination Board (`/board`, `web/src/pages/Board.tsx`)**:
  1. **KPI & Live Sync Header**: Project selector, operator/agent handle, `5s` visibility-aware live polling, and 4 KPI cards (`Worktrees & Agents`, `Active File Locks`, `In-Flight Tasks`, `Channel Feed`).
  2. **3-Column Operational Grid (`lg:grid-cols-12`)**:
     - **Left (`lg:col-span-4`)**: Worktrees & Agent Roster (`@agent`, project badge, runtime badge, worktree badge, `⎇ branch`, session title / `#task`, worktree path, and `2h` active worktree window: `<7200s` fresh emerald dot, `<14400s` recent amber dot) + Active File & Glob Locks (with TTL countdown, `manual`/`auto` badge, conflict warnings, inline `+ Claim` form, and `Release` actions).
     - **Center (`lg:col-span-4`)**: Worktree Task Board with status filters (`all`, `active`, `open`, `done`), priority badges, inline task claim form, and quick status transitions (`Mark Done`, `Review`, `Blocked`).
     - **Right (`lg:col-span-4`)**: Cross-Worktree Channels (`#general`, `#handoff`, `#blocker`) with `@agent` mention filter, inline message composer, and highlighted `@mention` chips.

## Live board delivery contract

The board connects live sessions so they can warn, ask, share, and hand off. Session startup
shows the assigned handle, active peers, other agents' locks, and unread legacy mentions. Existing dashboard tasks and task APIs remain
compatible, while startup guidance focuses on communication.

Automatic notices cover only addressed messages, with up to five stable IDs and escaped
160-character previews. A fixed untrusted-peer label sits outside peer text. Fetching a
full message and acknowledging it are separate actions. Messages stay project-visible.
Legacy messages have no inferred delivery receipts or authenticated session sender.

A server-assigned session ID and credential separate sessions even within one worktree.
Registration retries a handle collision once with a new suffix. Live sessions keep their presence rows,
and reactivation enforces the same per-machine and project capacity checks as registration.
Machine tokens bind registration to their own label. Admin registration retains the supplied machine label.
Delivery attempts are unique by message and recipient session, leased for 30 seconds,
and capped at five notice attempts. Plain inbox fetches return all unacknowledged deliveries.
Exhausted deliveries remain in the inbox unacknowledged.
Notice polls check with a plain read before opening a write transaction, re-check claims inside
that transaction, and refresh presence only when older than 60 seconds.
Acceptance is recorded separately from explicit acknowledgment. Successful notices are
suppressed on later tool boundaries. Pending acknowledgments prevent pruning regardless
of message age or recipient status. Ended sessions older than 14 days are pruned once no pending deliveries remain. Retained messages keep their
sender session IDs. Pruned session handles stay reserved against legacy impersonation. A crash after output but before its receipt may repeat the same ID.

Edit hooks retain cursor-based legacy-handle mentions in the same JSON context, with the
same untrusted preview format and no repeat for session-addressed messages. The cursor advances
only after stdout flush for Claude or the acceptance callback for Pi. Empty deliveries skip callbacks.
Claude receives JSON additionalContext after successful and failed tools, including read-only tools.
Pi receives a custom message with steer delivery, then records adapter acceptance. Active runs
use triggerTurn true to receive notices between tool calls. While idle, only a server-approved
question sets it true. Other idle messages set it false (Pi extensions.md:1643–1649).
The running extension checks isIdle (extensions.md:1255–1257) on a 15-second timer, skips active
runs and overlapping polls, and clears its timer on every session_shutdown, including reload
and replacement (extensions.md:601–609). A session generation check discards in-flight poll
results after replacement or shutdown. Each idle poll calls the existing post-tool hook with
board_idle true. The server returns only an eligible authenticated kind=question addressed via
to=@handle, claims one attempt atomically, and includes daybookWake=true in the Pi hook output.
Broadcasts, body mentions alone, and non-questions cannot wake. Five attempts per project per
rolling ten minutes and one attempt per delivery bound automatic work across all machines.
If A woke B, questions B posts to A within that window are permanently suppressed from waking,
so they cannot restart ping-pong later. Claims also check for a reverse wake in the current window.
Failed or suppressed questions remain in the normal pending inbox without an invented acknowledgment.
Wake logs have no message/session foreign keys, so acknowledgment, pruning, and session end
cannot reset the rolling budget. Expired logs are removed on a subsequent successful claim.
Claude waits for its next tool call. The documented Stop decision can only continue an ending
turn, Notification has no context control, and asyncRewake requires an already-running background
hook. This slice installs no persistent Claude watcher. See
https://code.claude.com/docs/en/hooks#stop-decision-control and
https://code.claude.com/docs/en/hooks#command-hook-fields.

Chat and question retention keeps the union of 14 days (including the exact cutoff) and the
latest 500 chat/question messages per project. Pending acknowledgments always prevent deletion.
The kind column excludes handoff, decision, pitfall, and discovery from automatic deletion,
ready for Slice 4 authoring and archive integration. Existing rows default to chat with no to
address. Session/handle retention is unchanged.

Notice preparation and edit lock refresh share one 750 ms deadline and runtime, even for a busy local
SQLite database or slow remote server. Startup gets 1.5 seconds, and acceptance recording
gets up to 250 ms. A failed or timed-out notice check backs off for 60 seconds across hook
processes.
MCP calls bind to the hook session through the startup session_id or an inherited runtime
session environment variable. Hooks update a live binding on session startup, which MCP resolves
on each call so session switches follow the runtime identity. Conflicting caller IDs are
rejected. With neither binding these tools require the startup session ID.
The legacy post `agent` argument is ignored. A cached-session notice uses one inbox request
with escaped message previews included. A typed invalid-session signal re-registers and
retries once. Owner-token, proxy, and other failures preserve the cached session.
