# P2b chat-journal: calculation-pad chat and Agent & Kernel Journal

Read `../../WAVE.md` first, then `../shell/REPORT.md`, which defines the binding prop interfaces and store contract. Work only in `/home/v9i/Projects/Kala-Agent/studio`. You own `src/components/chat/**`, `src/components/journal/**`, `src/styles/chat.css`, `src/styles/journal.css`, and the matching tests. The P2a worker is editing viewer/drafting files at the same time, so don't touch those. AppShell/store edits are limited to minimal additive wiring: re-read the file right before patching and list the edit in your REPORT.

Reference: `stitch/kala_studio_the_vellum_drafting_board_master_workspace/` (left column and right-bottom journal), `hash_chain_ledger_audit/` (ledger row styling), and `kala_agent_floating_overlay_system_3_states/` (message/status card states).

## ChatPanel (left 340px column, bound engineer's calculation pad)
- Header stamp "KALA // MECHANICAL DESIGN AGENT" with a connection dot and label taken from REAL connection state (desktop / browser fallback / disconnected).
- Starter chips styled as taped paper tabs ("Size L-Bracket", "Size Stepped Shaft", "Housing Envelope", "STEP Handoff"). Each one fills the composer with a real, useful prompt and does not auto-send.
- Message cards: user notes and agent responses with a left rule, role label, and timestamp. Inline mono parameter tables wherever the agent result REALLY contains structured parameters, tool calls, or estimates (read the existing ChatPanel and the agent response types to see what exists). Any digest chip shows only a real artifact digest.
- Composer: textarea ("Describe geometry, tolerance, or prompt…"), Enter sends and Shift+Enter adds a newline, plus the [Draft Part Geometry ↵] charcoal button. Show a remote-egress consent stamp only if the existing agent/provider flow has a real consent/egress concept (wire it to that). Otherwise show the real active provider status. Mic/sketch icons appear only if they actually work; do not add dead buttons.
- Preserve all existing chat behaviour and tests: pending/error states, abort, and keyboard handling.
## JournalPanel (right bottom; engineer's ledger)
- Header "AGENT & KERNEL JOURNAL" with a status stamp. Show "HASH-CHAIN OK" ONLY if the store/backend really reports a verified chain; otherwise show a neutral "SESSION LOG" stamp.
- Mono rows: tag in colour by kind (plan/sketch/preview/export/policy/connection/error), time, message, and an optional short real digest. Auto-scroll that sticks to the bottom unless the user has scrolled up, a "jump to latest" control, an aria-live polite region (throttled), and an empty state.
- Footer line: real connection origin and the backend version from the store (this replaces the old app footer info). No fake PID, latency, or keytar.
## Tests and gates
Tests for starter chips, Enter/Shift+Enter, journal rendering, autoscroll stickiness, and the honest stamp logic. `npm run typecheck && npm test && npm run build` must be green (if a failure is in the P2a cell's files, re-run once before reacting).
Write `REPORT.md` in this cell folder: files changed, gate counts, gaps, and anything not backed by real data.
