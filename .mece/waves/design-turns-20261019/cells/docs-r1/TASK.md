# docs-r1: bring user-facing docs in line with today's work (NO overclaims)
Read ADR-0009, ADR-0010, ADR-0011, and the REPORTs in `cells/*` of both waves (`.mece/waves/design-turns-20261019/cells/` and `.mece/waves/rust-backend-20261019/cells/`), plus `.mece/waves/rust-backend-20261019/TAKEOVER.md`.
Own: `docs/studio.md`, `docs/studio-providers.md`, `docs/studio-limitations.md`, `docs/studio-install.md`, `studio/README.md`, `backend/README.md` (create if missing), and a NEW `docs/cad-providers.md`. Do not edit ADRs (their status stays Proposed), code, or the other docs.
Content:
- **The UI:** the Vellum drafting board, its zones, the Options drawer, themes, the measure tool, the CAD target selector, journal and ledger stamps, and when "B-Rep Verified" and "HASH-CHAIN OK" appear.
- **CAD providers (ADR-0011):**
  - Solid Edge (Windows + bridge; live proof pending).
  - FreeCAD (how it is found, version 1.1.4 tested, the 28/28 corpus, assembly export).
  - OpenCascade (opt-in builder; also the view-only importer).
  - There is no default provider. Say who measures what.
- **Rust front door (ADR-0010):** what is Rust-native vs proxied to the Python geometry worker, how to run it in dev (exact commands from the r2 and rust-e2e-qa reports if available), `KALA_STUDIO_GEOMETRY_WORKER` and `KALA_STEP_IMPORT_MAX_BYTES`, and the token/lifeline security model.
- **Rust migration status:** list the accepted crates with their test/differential counts, what is still Python, and what is pending (tools executor, agent runtime, application/API/CLI, packaging decision, Windows/Solid Edge proof, MinGW for Windows checks).
- **Limitations, stated honestly:** no FEA, tolerances are declared not analysed, the perspective camera in clay mode, release packaging blocked pending the decision, and the fixed core API bug (`errors.py`, 4xx statuses).
Verify every command you document by running it (or mark it "not run"). Write REPORT.md listing the changed files.
