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
     - Left/Primary Column: **What Shipped & Why — By Project** + **Pitfalls (`Coral Rose`) & Learnings (`Sky Cyan`) — By Project**.
     - Right/Secondary Column: **Cost, Cache & Command Health + Next Steps**, **Consolidated Workflow Rules Extracted Today**, and **Sessions** (with 3-up `60px` quick-filter strip and `66px` interactive session control tiles that expand on click to inspect full session markdown).
