# docs-cutover: document the Rust-only app and the installers
Read `../../WAVE.md` and the REPORTs of cad-r2, cad-r3, native-journal, native-design, packaging (both rounds), and freecad-multi-abi.
Own: `docs/rust-migration-status.md`, `docs/studio-install.md`, `docs/rust-studio.md`, `docs/cad-providers.md`, `backend/README.md`, and `studio/README.md`. ADR-0010: append ONE dated "Phase 2 implemented (2026-10-20)" note to its status/consequences. Do not rewrite the ADR.
- The Python geometry worker is retired from the packaged app (now a dev-only opt-in). kala-cad calls OCCT 7.9 through a narrow C++ shim (it does not rewrite it). Native geometry evidence: 75/118 tests, the differentials, and the env -i smoke.
- Install: the Linux AppImage (path, sha256 `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e`, size 163,642,516). It is unsigned. FreeCAD is detected on the system (not bundled), with cp311–cp314 vendored adapter dependencies.
- Windows: how to run `packaging/windows/build-windows.ps1` (prerequisites, the steps, the post-install checklist), and that it has not been executed yet.
- Still open: a Windows run on the user's machine, the MinGW cross-check (pending the user's install), live Solid Edge proof, code signing, and publishing.
Re-run documented commands or mark them "not run". USER DATA rule in every shell. REPORT.md.
