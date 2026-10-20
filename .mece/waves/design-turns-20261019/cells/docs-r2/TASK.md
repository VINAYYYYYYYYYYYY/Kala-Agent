# docs-r2: refresh the docs after r2-chat-providers, gateway, and executor
Read `../docs-r1/REPORT.md` and the REPORTs of `.mece/waves/rust-backend-20261019/cells/{r2-chat-providers,gateway-r1,tools-executor-r1,tools-registry-r1,rust-e2e-qa}`.
Own: `docs/cad-providers.md`, `docs/studio-limitations.md`, `backend/README.md`, and `docs/rust-studio.md` (now stale: it still describes the old 9-route surface).
- Remove the now-false limitation "the Rust front door cannot build with FreeCAD or Solid Edge yet". State what IS true: chat and design through the Rust front door pass the chosen provider to the worker; FreeCAD builds there (with the live differential evidence); Solid Edge refuses honestly on Linux; there is no fallback.
- Add the cold-reload area fix to the fixed-bugs notes.
- `backend/README.md`: add the accepted kala-runtime gateway (55 tests) and the kala-tools registry, simulator, and executor (229 tests) with their differential counts. Note that agent-r1 is in progress (not accepted).
- Rewrite `docs/rust-studio.md` for the current front door: Rust-native vs proxied routes (6/10), the worker supervision, the shared STEP limit, the token and lifeline, and the live differentials (292/268/48/20).
Re-run every command you document, or mark it "not run". No code or ADR edits. REPORT.md.
