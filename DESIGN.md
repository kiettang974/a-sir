# ASR Studio — Design System (`Sáng Thanh Lịch · Cobalt / Slate`)

**Approved:** 2026-11-26 · **Owner:** `kietta` · **Scope:** All `ui/src/` screens.

## Aesthetic World
**Sáng Thanh Lịch (Cobalt / Slate):** a crisp, modern macOS studio interface inspired by Linear and Things. Soft cool-slate sidebar (`--paper`), clean bright reading/writing canvas (`--sheet`), elevated white cards (`--card`), deep slate ink (`--ink`), and **one** unified cobalt-blue accent (`--accent`) across primary buttons, selection, timestamps, and table headers.

## Semantic Tokens (`ui/src/style.css` `:root`)
- **Surfaces:** `--paper` (`#f1f5f9`), `--paper-hover` (`#e2e8f0`), `--sheet` (`#fafbfd`), `--card` (`#ffffff`), `--card-selected` (`#f5f8ff`), `--row-alt` (`#f8fafc`)
- **Ink:** `--ink` (`#0f172a`), `--ink-soft` (`#475569`), `--ink-faint` (`#64748b`), `--on-accent` (`#ffffff`)
- **Intent / Semantic:** `--accent` (`#1d4ed8`), `--accent-hover` (`#1e40af`), `--accent-soft`, `--accent-ring`, `--fail` (`#dc2626`), `--fail-soft`
- **Interactive washes:** `--selection`, `--hover-wash`, `--hover-subtle`, `--line-hover`, `--scroll-thumb`, `--scroll-thumb-hover`
- **Rules & Elevation:** `--rule` (`#e2e8f0`), `--rule-strong` (`#cbd5e1`), `--shadow-sm`, `--shadow-menu`
- **Radii:** `--radius-sm` (`5px`), `--radius-md` (`8px`), `--radius-lg` (`10px`)
- **Typography:** `--ui` (`Be Vietnam Pro`), `--read` (`Literata`)

## Typography & Reading Measure
- **UI Chrome:** `Be Vietnam Pro`, fixed scale (`12px`–`16px`), `-0.015em` heading tracking, `tabular-nums` on all timestamps/durations/percentages/indices.
- **Editorial Reading:** `Literata` serif at `16.5px / 1.8` (`66ch` measure) for transcripts and `15.5px / 1.65` for meeting notes.

## Rules & Governance
See [AI/rules/DESIGN-DECISIONS.md](AI/rules/DESIGN-DECISIONS.md) for the closed status/action registries, slots, hard rules (`UI-01`..`UI-08`), and automated build gate (`ui/scripts/lint-ui-rules.mjs`).
