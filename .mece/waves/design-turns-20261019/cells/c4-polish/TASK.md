# C4 polish: ledger column overlap (Opus visual review of c3-visual/shots/after-1440-pencil.png)
Own `studio/src/components/journal/**`, `studio/src/styles/journal.css`, and the journal tests.
1. In the 6-column ledger at 1440, the digest copy button overlaps the CHAIN cell (it renders as "⧉k" instead of the copy icon next to the digest plus "ok" in its own column). Keep the copy button inside the DIGEST cell; CHAIN shows "ok" / "local" / "broken" fully.
2. The "SEQ" header is clipped to "SE(". Give SEQ a minimum width that fits the header and 3-digit sequences.
3. Reduce the MESSAGE wrapping: server rows wrap to 4–5 lines. Show a single-line summary (type-specific, e.g. "freecad 1.1.4 · build completed · step d8b89af5") with the full message in an expandable detail or title. Keep the a11y semantics.
Add a DOM/layout test asserting no cell overlap (bounding boxes) at 1440 and 1280. Gates: `npm run typecheck && npm test && npm run build`. Save one after-screenshot to `shots/` using the C3 harness, with the FreeCAD provider. REPORT.md.
