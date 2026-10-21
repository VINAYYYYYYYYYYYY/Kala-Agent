# ADR-0010: Rust Control Plane; Geometry Stays in the CAD Kernel

| Field | Value |
|---|---|
| Status | **Proposed**. Direction set by the user on 2026-10-19. Later the same day the user widened it: "if rust will be good I want everything in rust". This ADR is now PHASE 1; phase 2 replaces the Python worker with a narrow Rust→OpenCascade C++ adapter, pending the user's explicit OK (it calls OCCT and does not rewrite it). **Phase 2 was implemented on 2026-10-20 — see "Phase 2 implemented (2026-10-20)" at the end of this ADR.** |
| Date | 2026-10-19 |
| Supersedes | The parts of `.mece/waves/rust-backend-20261019` that put geometry in Rust: the `kala-cad` OCCT FFI port (cell cad-r1, cancelled), and the delivery rule "no Python interpreter/OCP in the native distribution". |
| Related | ADR-0005 (isolated tool processes), ADR-0008 (Solid Edge bridge), ADR-0009 (Studio design turns) |

## Decision

1. **Rust owns connections and internal APIs**: the HTTP front door (bearer/Host/Origin/body gates, routing), settings and provider keys, provider calls and chat, persistence/journal/repositories/artifacts, kernel policy/workflow/planning/budget, process supervision, and the client connections to CAD backends.
2. **Geometry is done by a CAD kernel, never re-implemented in Rust:**
   - **Studio (any OS):** OpenCascade through the existing, tested Python/OCP code (ADR-0009 modules), run as a supervised **geometry worker** process.
   - **Windows production:** Solid Edge through the ADR-0008 bridge. Rust is the client that sends `ConstructionScript`-lowered operations over the bridge protocol.
3. The existing Rust geometry crates (`kala-geometry` primitive meshes, `kala-step` primitive STEP writer, partial `kala-cad`) are **parked**: kept in the tree, not deleted, not on any default path. Their removal is a separate user decision.

## Geometry worker (v1)

- **Process:** the Python Studio app started in worker mode: `python -m kala_agent.studio --worker` (dev), or a frozen worker executable (packaged). Rust starts it through the accepted `kala-process` supervisor: one worker per backend, restart with backoff, bounded start time, reaped on shutdown.
- **Transport:** loopback HTTP on `127.0.0.1:<port>`. The worker binds port 0 and prints exactly one readiness line `KALA_WORKER_READY {"port":N,"version":"…","design_available":bool}` to stdout. The token is a fresh per-launch random value passed **only via environment** (`KALA_WORKER_TOKEN`), never argv. The worker enforces the token plus Host=127.0.0.1 and refuses any Origin header (it is never browser-facing).
- **Worker routes** (internal, `/internal/v1/*`): `preview`, `export`, `handoff`, `design`, `design/{id}/drawing`, `design/{id}/section`, `intent/parse` (local strict parser: text → intent | missing values), `intent/validate` (LLM JSON → validated intent | 422 detail), `journal` (page), `journal/verify`, `journal/append` (Rust-originated events such as `chat.turn` and settings changes). These reuse the ADR-0009 code unchanged. Worker mode only adds route wiring and the auth gate.
- **Journal ownership (v1):** the worker is the single writer of the Studio session journal file, which avoids two writers on one SQLite chain. Rust appends its own events via `journal/append`. A later ADR may move the journal into Rust's `kala-persistence`, which already uses byte-identical chain digests.
- **Rust `kala-studio` routing:**
  - Rust-native: `/health` (adds `geometry_worker: {state, version, design_available}`), `/integrations`, `/providers`, `/settings`, and `/chat` (provider calls and consent as today).
  - Proxied to the worker with the same public paths and payloads: `/preview`, `/export`, `/handoff`, `/design*`, `/journal*`. Body and response limits and timeouts are enforced by Rust. If the worker is down, those routes return 503 with a clear message and the other routes keep working.
- **Chat with designs:**
  - Local provider: Rust calls `intent/parse`, then `design`.
  - Remote providers: Rust adds the DesignIntent schema to the prompt (schema text fetched from the worker at startup), extracts the fenced JSON, calls `intent/validate` (one repair round-trip), then `design`, and attaches `design` to the ChatResponse exactly as the Python server does.
- **Parity gate:** the public Studio API served by Rust plus the worker must pass the same live differential as before (the 268-case corpus plus the ADR-0009 criteria 2–7 through the Rust front door). The "30 mismatches / 404 on /design" gap closes by proxying, not by re-implementing.

## Packaging

The desktop package ships: native `kala-studio`, the frozen geometry worker (PyInstaller onedir including OCP), and `studio_dist`. The Electron main process starts only `kala-studio`, which starts the worker. If the worker cannot start, design and geometry routes report 503 and `design_available: false`, and the UI already degrades honestly. Dev override: `KALA_STUDIO_GEOMETRY_WORKER="<python> -m kala_agent.studio --worker"`.

## Solid Edge track

A Rust client crate for bridge protocol v1 (port of `kala_agent_solidedge` compile/client logic, which only lowers `ConstructionScript` into bridge ops and makes connections; no geometry) is the Windows production path: instruction → plan → policy/approval → bridge ops → Solid Edge model → Solid Edge STEP export → journal. Live proof requires a Windows host with Solid Edge. Until then the client is tested against the bridge contract fixtures only, and no live claim is made.

## Consequences
- Removes the largest remaining Rust effort (a native CAD port) and the risk of geometry parity drift. The tested Python/OCP results stay the single source of measured numbers.
- The package grows by the size of the frozen OCP worker (a few hundred MB).
- Two runtimes ship. The boundary between them is one authenticated loopback API with a fixed route list.

## Phase 2 implemented (2026-10-20)

One dated note. The decision above is unchanged; this records what phase 2 did to it, what it did to the package, and what is still open.

**Status.** Phase 2 is **implemented and shipped on Linux**; it is **not** claimed on Windows, on Solid Edge, for code signing, or for publication.

- The Python geometry worker is **retired from the packaged app**. The desktop package now launches **only** the native Rust `kala-studio`; the packaged child environment has `PYTHONPATH`, `PYTHONHOME`, `KALA_STUDIO_PYTHON`, `KALA_STUDIO_DEV_BACKEND` and `KALA_STUDIO_GEOMETRY_WORKER` removed, and no Python interpreter is bundled or started. The worker survives only as a **development-only opt-in**: it starts when `KALA_STUDIO_GEOMETRY_WORKER` names one, and the packaged mode never consults a PATH Python or a repository virtualenv.
- The phase-2 adapter is the accepted `kala-cad` crate: a **narrow C++ shim over the real OpenCascade 7.9.3 SDK** (`src/ffi/adapter.cpp`). It **calls OCCT and does not rewrite it**. `Shape` owns exactly one kernel handle and is deliberately `!Send + !Sync`; every C++ entry point catches `Standard_Failure`, `std::exception` and `...`, so no exception crosses the C ABI.
- The `kala-studio` default feature is now `native-geometry`, which links `kala-cad`, `kala-cad-providers`, `kala-design-intent`, `kala-studio-journal`, `kala-contracts` and `kala-persistence`. The front door answers the design, drawing, section, import, journal, intent and CAD-provider routes **itself**; `POST preview`, `POST export` and `POST handoff` use the reviewed analytic crates (`kala-geometry`, `kala-step`), which the `native-geometry` feature enables.
- The session journal and the artifact store moved into Rust as well (`kala-studio-journal`), byte-compatible with the Python journal: the same DDL, the same rows, the same chain digests, the same tamper verdicts, and one file that both implementations can extend.
- **Parity evidence.** The four live differentials — `worker_differential.py` 292, `python_differential.py` 268, `python_provider_dto_oracle.py` 20, `replay_binary_corpus.py` 56 — all report **0 mismatches against the live Python reference with the native backend**. The `kala-cad` suite is 75 tests after the core and **118** after the HLR/section work, with a live OCP differential (60 cases) and a live drawing differential (13 shapes, 55 views, 55 sections, 15 refusals) at 0 mismatches. The parity bar of ADR-0009/0011 — L-bracket 35 571.416047 mm³, the exact XZ section area `2400 − 25π`, the 28-case construction fixture and the STEP import bounds within 0.001 mm — is met from the extracted AppImage under a literal `env -i`.
- **Two declared differences** (both written down, neither hidden): the native build links the **system OCCT 7.9.3** while the Python oracle imports the `cadquery-ocp` binding 8.0.1.1, so the reported `opencascade` provider version names a different kernel binding; and the two OpenCascade versions assemble the same STEP file with its faces in a different order, so the display mesh is compared by an **order-independent fingerprint** (the same vertex set, the same triangle set, the same counts and the same bounds). The geometry, the measurements, the checks, the drawing and the section are identical.

**Consequences.**

- The package **shrinks instead of growing**: the frozen OCP worker (a few hundred MB of Python) is gone from the deliverable, replaced by the OCCT shared libraries (23 `libTK*.so*` staged from the exact `ldd` resolution on Linux, the `TK*.dll` closure next to `kala-studio.exe` on Windows). One runtime ships in the package; the FreeCAD adapter is the only Python left in it, and it runs inside the user's own FreeCAD, which is not bundled.
- The Linux deliverable exists: an **unsigned** AppImage, `studio/release/kala-studio-0.1.0-x86_64.AppImage`, **163 642 516 bytes**, sha256 `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e`, smoke-tested end to end from the extracted image with no Python on `PATH`.
- The Windows deliverable is **a definition, not a run**: `packaging/windows/build-windows.ps1` (MSVC + vcpkg `opencascade:x64-windows` + NSIS) is written and reviewed, and **has not been executed** — this host has no Windows and no `pwsh`. The user's Windows machine runs the build and the post-install checklist.
- The two-runtime boundary of phase 1 is now a **development** boundary, not a product one. The Python reference server and the Python worker remain in the tree and remain the parity oracle; they are deleted only at the very end and only with the user's explicit approval.
- **Still open:** the Windows build and run; the MinGW cross-check (pending the user's `mingw-w64-gcc` install); a live Solid Edge proof (Windows only, and the bridge still has no verified native executor); code signing; publication; the Electron window-rendering path (no `xvfb-run` on the build host, so the packaged **backend** is verified by extraction, not the Chromium shell); and a real FreeCAD on Python 3.11–3.13 (the official FreeCAD 1.1.4 AppImage ships `py311`; the `cp311`–`cp313` adapter imports were proven with matching CPython builds, not with a real FreeCAD).
- **No performance, readiness or speed claim** is made, and no credential was read.
- **2026-10-21.** Later builds supersede the AppImage digest above; the current artefact is recorded in packaging/linux/README.md.
