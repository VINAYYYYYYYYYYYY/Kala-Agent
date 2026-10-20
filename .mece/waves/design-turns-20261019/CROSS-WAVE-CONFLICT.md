# Cross-wave conflict: design turns (Python/OCP) vs. Rust native backend cutover

Found by Opus on 2026-10-19 while reviewing C1a.
- `.mece/waves/rust-backend-20261019/cells/delivery/TASK.md` packages ONLY the native Rust `kala-studio` binary ("No Python interpreter/OCP/vendor Python trees in native distribution") and edits `studio/electron/**` and `studio/scripts/**` (native-backend.cjs, native-readiness.cjs, electron-builder.json). Those edits are theirs, not this wave's.
- `backend/crates/**` implements none of ADR-0009's routes (`/design`, `/design/{id}/drawing|section`, `/journal`, `/journal/verify`, `/export` by design_id, chat `design`).
- Result: design turns work with the Python Studio backend (dev, or KALA_STUDIO_PYTHON) but are absent from the packaged native app.

Options (the user decides):
A. Port design turns into Rust with an OCCT binding (opencascade-rs/occt-sys). Large; it must still match ADR-0009 criteria 1–7 numerically.
B. Native Rust backend plus an optional Python/OCP geometry sidecar process for `/design*` only. Contradicts the delivery cell's "no Python in native distribution" rule unless that rule is relaxed for an optional sidecar.
C. Keep design turns Python-only for now. The native backend reports `design_available: false` and the UI degrades honestly. Decide A or B later.

Meanwhile (true under every option): the UI must handle `design_available: false` gracefully. Added to C1b.

## User decision (2026-10-19)
User: "just the connections and internal apis into rust ... why is opencascade written into rust". Rust is for connections and internal APIs ONLY. OpenCascade stays C++, reached through the existing tested Python/OCP code. Option A is rejected. The direction is B: Rust front door (HTTP, auth, routing, journal, process control) → internal loopback API → Python+OCP geometry worker. The Rust wave's "no Python/OCP in native distribution" rule must change to allow this geometry worker. Pending: whether Opus writes this into rust-backend-20261019/WAVE.md or the user tells that conductor.
