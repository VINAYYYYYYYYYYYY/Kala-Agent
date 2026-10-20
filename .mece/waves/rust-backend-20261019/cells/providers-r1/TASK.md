# providers-r1: kala-cad-providers (ADR-0011 provider connections in Rust)
Follow `../CONVENTIONS.md`. Read ADR-0011, `/home/v9i/Projects/Kala-Agent/.mece/waves/design-turns-20261019/cells/b2-providers/REPORT.md`, and the Python reference `src/kala_agent/studio/design/providers/{base,freecad,solidedge}.py`.
Port into NEW standalone `backend/crates/kala-cad-providers`:
- the `CadProvider` trait, Availability, and the ProviderBuild result;
- `freecad`: the same FreeCADCmd subprocess protocol as freecad.py (same command, arguments, script hand-off, credential-scrubbed env, bounded wall time, no shell, STEP + FCStd outputs, the same parsed measured values and refusal messages), supervised through the accepted `kala-process` (read-only);
- `solidedge`: probe and build through the accepted `backend/crates/kala-solidedge-client` (read-only path dependency), with the same availability reasons and refusals as solidedge.py.
NO `opencascade` provider here (that stays in the geometry worker) and NO geometry computation: the numbers come from the provider output exactly as the Python parses them.
Differential (REAL FreeCAD 1.1.4 is installed on this host): build every corpus case from `tests/fixtures/construction_parity.json` through both the Rust and the Python freecad providers. The measured values, refusal messages, and artifact presence must match, with STEP files compared by importing bounds via the Python importer, test-only. The Solid Edge probe must give identical availability output on this Linux host, plus mock-bridge build tests reusing the r4 mock patterns.
Full Rust gates. REPORT.md.
