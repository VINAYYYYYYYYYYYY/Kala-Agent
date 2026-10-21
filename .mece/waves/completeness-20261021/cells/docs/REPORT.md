# Cell docs — REPORT

**Cell:** `docs` · **Wave:** `completeness-20261021` · **Date:** 2026-10-21
**Brief:** `cells/docs/TASK.md` · **Rules:** `../../WAVE.md` · **Audit:** `../audit-backend/AUDIT.md` (B1, B2, B5, B8, B9, B12, B17, B18, B19)

No commit was made. No file outside the ownership list was edited
(`docs/**` except `docs/adr/**`, `README.md`, `packaging/linux/README.md`,
and the three comment headers named by the brief). **No code cell's files
were touched** — `server.rs`, `native_geometry/**`, `studio/src/**` and
`studio/scripts/**` were read only.

**USER DATA.** Every shell ran with `RUSTUP_HOME=/home/v9i/.rustup`,
`CARGO_HOME=/home/v9i/.cargo` and a temp `HOME`, `KALA_AGENT_DATA_DIR`,
`KALA_STUDIO_DATA_DIR`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME` and `TMPDIR`
under `/tmp/opencode/kala-docs-cell` (removed again at the end). No
credential or settings file was read. No `claude`, `codex` or `opencode`
CLI was run. `/home/v9i/.local/share/kala-agent/**` was never opened,
named in a command or written.

---

## 1. Files changed, and the verification behind every non-trivial claim

### 1.1 `docs/studio-providers.md` (rewritten)

| what | verification |
|---|---|
| **Seven** provider ids, published in the order `local, openai, openrouter, codex, claude_code, codex_cli, opencode_cli` | `backend/crates/kala-studio/src/providers/mod.rs:28-36` (`PROVIDER_IDS`); the rows themselves in `list_provider_records`, `providers/mod.rs:254-335` |
| `codex` is the historical alias of `codex_cli` (same CLI, same name, same notes); the picker folds the alias row into the canonical one | `providers/mod.rs:25-27,294-323`; `studio/src/providerPolicy.ts` (`PROVIDER_ALIASES`, `canonicalProviderId`); `studio/src/components/chat/ProviderPicker.tsx:67-72` |
| The three CLI argv tables (exact flags, in order) | `providers/claude_cli.rs:72-89`, `providers/codex_cli.rs:74-88`, `providers/opencode_cli.rs:119-130` |
| Prompt on stdin, never on argv; argv array, no shell | `providers/cli_runtime.rs:8-14` (module contract), `:653-660` (the spawn) |
| Env allowlist (`PATH`, `HOME`, `XDG_*`, `TMPDIR`/`TMP`/`TEMP`, `LANG`/`LC_ALL`/`LC_CTYPE`, `TERM`) + `CLAUDE_CONFIG_DIR` / `CODEX_HOME` | `providers/cli_runtime.rs:91-106` (`ENV_ALLOWLIST`), `:208` (the child env), the CLI-specific name in each `CliSpec` |
| Private temporary working directory per turn | `providers/cli_runtime.rs:21-23`, `:1115` |
| Output caps 1 000 000 / 65 536 bytes | `providers/cli_runtime.rs:56-58` |
| 180 s CLI budget, `KALA_STUDIO_CLI_TIMEOUT_SECONDS`, clamped 1…3600 | `providers/cli_runtime.rs:48-54`, `:187-197` |
| Own process group; the **whole group** killed on timeout, failure, cancel/disconnect; Unix `kill_process_group(SIGKILL)` + child backstop; Windows `taskkill /T /F /PID` | `providers/cli_runtime.rs:691,695` (group creation), `:598-611` (both kills), `:640-642` (`CREATE_NEW_PROCESS_GROUP`) |
| `PATHEXT` search (default `.COM;.EXE;.BAT;.CMD`), bare names never chosen over a shim, `cmd.exe /d /s /c` with an argv array, control characters refused, non-Windows refusal | `providers/cli_runtime.rs:231` (`DEFAULT_PATHEXT`), `:295` (`parse_pathext`), `:322-330` (candidates), `:479-502` (`cmd_shim_argv` → `["/d","/s","/c", …]`, control-char check at `:489-492`), `:653-660` (the shim spawn) |
| Consent: all four subscription ids require `allow_remote=true`; refusal `provider=<id> requires allow_remote=true at request time`; the CLI is never started | `providers/mod.rs:397-428`; `providers/cli_runtime.rs:1226-1228` |
| Optional model; `default` is never passed as `--model`; the reply carries `default` when the CLI names none | `providers/claude_cli.rs:68,85,129-135`, `providers/codex_cli.rs:70,84,136-142`, `providers/opencode_cli.rs:67,128,182-188` |
| The honest "CLI default model" label and the model-picker note | `studio/src/providerPolicy.ts` (`defaultModelLabel`, `optionalModelNote`); the reply badge at `studio/src/components/chat/ChatPanel.tsx:635-642` |
| Probe: 2 s per CLI, 120 s cache, 600 ms route budget, `checking...`; `/providers`, `/settings`, `/health` answer from the cache | `providers/cli_runtime.rs:64,66,72,76`; `providers/mod.rs:240-246`; `server.rs:1095-1107` |
| Probe commands (version + own status command, exit status only) | `providers/claude_cli.rs:62-63`, `providers/codex_cli.rs:64-65`, `providers/opencode_cli.rs:61-62` |
| 30 s plain chat, 180 s design call (first call **and** the one repair), 180 s CLI turn | `providers/remote.rs:27,33`; `providers/cli_runtime.rs:48`; the 180 s reaches both design calls through `chat_design.rs` (`STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS`) |
| Model catalog route, envelope, field rules, status codes, per-provider source, the 500-row / 600 s / 15 s / 10 s / 32-entry bounds | `server.rs:236-242` (classifier), `:206-216` (route bearer), `:1231-1271` (handler); `providers/catalog.rs:66,68,72,74,84,86` (bounds), `:389-440` (per-provider behaviour), `:446-456` (`canonical_provider`), `:97-114` (the static CLI alias lists), `:901-935` (the OpenRouter mapping and the `free` rule), `:988` (the id charset that filters `~` ids), `:1045-1060` (`opencode models --standalone`) |
| The OpenRouter list is live and carries the free models | **I ran it**: `curl -fsS https://openrouter.ai/api/v1/models` (unauthenticated, public data) on 2026-10-21 → **468 models, 20 free** (ids ending `:free`), max context 2 000 000, max name 56 chars, **19 ids containing `~`** (filtered by the chat request schema). The response file lived under the temp dir and was deleted. |
| `offline` now means "no remote route is **known** to be available" in `/health` and `/settings`, and "this turn did not leave the machine" in a chat reply | `server.rs:1095-1107`; `providers/mod.rs:432-461` (`any_remote_provider_configured` vs `any_remote_route_available`); the reply value: `providers/local.rs:99` (`true`) against `providers/claude_cli.rs:134`, `providers/codex_cli.rs:141`, `providers/opencode_cli.rs:187`, `providers/remote.rs:511` (`false`) |
| The honest Codex limit (the CLI keeps its shell tool) | `providers/codex_cli.rs:16-27` (module doc: `--sandbox read-only` is the strongest posture the CLI offers) |

### 1.2 `docs/studio.md`

* **Route table and count** — rewritten from *16* to **17** route-methods
  and the model-catalog row added.
  Verification: `server.rs:184-201` (16 `RouteKind` variants),
  `server.rs:220-259` (the classifier), `server.rs:262-281` (the method
  table) → 17 route-methods; `studio/release/native-readiness.json`
  (`python.routeMethodCount: 15`, `native.routeMethodCount: 17`,
  `rustNativeRouteMethodCount: 7`, `proxiedRouteMethodCount: 10`,
  `ready: true`, `releaseEligible: true`);
  `studio/test/nativeBackendPackaging.test.ts:965-990` (the same snapshot
  in a test). "All 17 are answered natively" is the built report's own
  wording (`warning`: *"the default build answers them from the native
  kala-studio backend (the Python worker is opt-in)"*).
* **New section "Design checks and limits"** —
  * budget 64 components / 500 steps per component / **500 steps in the
    whole script** / 16 assembly steps and the exact refusal text:
    `backend/crates/kala-contracts/src/construction.rs:22,24,31,36` and
    `:1309-1315`.
  * Solid Edge refuses > 8 parts (`the script has 9 components; the wire
    cap is 8 parts`), the 600-operation cap is a second line of defence
    and a contract-valid script cannot reach it (max 543 ops):
    `backend/crates/kala-solidedge-client/src/compile.rs:127-136`; the
    lowering ratio and the 543 maximum are the design-budget cell's
    **measurement** (`.mece/waves/agent-providers-20261020/cells/design-budget/REPORT.md` §2).
  * FreeCAD 1 800 s (max 3 600 s) and the OpenCascade 60 s compile budget:
    `backend/crates/kala-cad-providers/src/freecad.rs:96,100`;
    `adapters/freecad/src/kala_agent_freecad/contracts.py:57`;
    `backend/crates/kala-cad/src/lib.rs:79`.
  * the measured timings (100-hole plate 58.8 s in the provider / 59.8 s
    through the Studio; 500-step plate 1 009.5 s idle and 1 250 s loaded;
    OpenCascade 3.2 s / 16.9 s): the design-budget REPORT §4 and its
    `freecad_budget` output. **Not re-measured here** (≈ 23 min of
    FreeCAD time); quoted from the landed cell's evidence.
  * the check ids `brep_valid`, `single_solid`, `solid_count`,
    `envelope_x/y/z`, `cuts_effective` and the rule that
    `cuts_effective` is appended **only** for the failure:
    `backend/crates/kala-studio/src/native_geometry/service.rs:1799-1887`
    (rows at `:1809,1820,1830,1844,1858,1877`; the `cuts_effective`
    branch at `:1873-1885`; `CUTS_EFFECTIVE_TOLERANCE = 0.0` at `:55`).
  * the same-plane pocket rule and its message:
    `backend/crates/kala-design-intent/src/script.rs:538-560` (message at
    `:557`), called at `:937`.
  * the per-pocket warning:
    `backend/crates/kala-cad/src/native/compile.rs:304`;
    `adapters/freecad/src/kala_agent_freecad/cad/design.py:59,615-619`.
  * the 16-entry warning bound (14 individual + 2 summaries) and the exact
    ineffective-cut count: `backend/crates/kala-cad-providers/src/base.rs:31,43,99+`
    (`MAX_WARNINGS`, `MAX_INDIVIDUAL_WARNINGS`, `bound_warnings`).
  * one repair round per turn, shared by validation and build, with the
    trigger table: `backend/crates/kala-studio/src/chat_design.rs:1431`
    (`repair_spent`), `:1486` (`if !repair_spent`), `:289-305` (the
    validation prompt: instruction + full schema + the model's own
    previous JSON + error ≤ 512 + schema excerpt ≤ 700),
    `:574-582` (the build prompt: instruction + feedback + script),
    `:275,278,480,486` (the bounds).
  * the 4 096-character display bound while the design is read from the
    full reply: `providers/remote.rs:36`; `providers/mod.rs:62-93`
    (`ChatOutcome.text` vs `ChatOutcome.reply`); the design-budget REPORT §3.
* **New section "What the workspace shows you"** — the user-visible ui-fixes
  behaviour, each cited to the shipped source:
  * live elapsed clock: `studio/src/components/chat/ChatPanel.tsx:106-109,180-185,471-481,649`
  * 30-minute client deadline: `studio/src/api/client.ts:138,408-410`
  * viewport keys (arrows / `+` `-` / `0`): `studio/src/viewer/viewportKeys.ts:41-70`
  * honest view/source labels: `studio/src/viewer/designSource.ts:26-48`
  * honest default-model wording: `studio/src/providerPolicy.ts`
  * failed-turn code in the ledger: `studio/src/state/journal.ts`
  * real preview / journal-poll errors: `studio/src/components/inspector/Inspector.tsx`,
    `studio/src/components/journal/JournalPanel.tsx`,
    `studio/src/components/bottombar/BottomBar.tsx`
  * one Codex row: `studio/src/providerPolicy.ts` + `ProviderPicker.tsx:67-72`
  * source-aware "Clear key": `studio/src/components/panels/ProviderPanel.tsx`
* **Desktop shell** — the bounded backend log and the startup dialog added:
  `studio/electron/main.cjs:81-118` (log directory + logger),
  `studio/electron/backend-log.cjs:47-64` (secret scrubbing), `:66-99`
  (rotating log, 5 files × 1 MiB, mode 0600), and the
  `reportStartupFailure` calls in `main.cjs`.
* **False statements removed** — see §2.

### 1.3 `docs/studio-install.md`

* **Data-directory precedence** (six rows) —
  `backend/crates/kala-studio/src/boundary.rs:346-412` (the doc comment
  *and* the implementation: `--data-dir` → `KALA_STUDIO_DATA_DIR` (as-is)
  → `KALA_AGENT_DATA_DIR/studio` → `$XDG_DATA_HOME/kala-agent/studio` →
  `$HOME/.local/share/kala-agent/studio` → `$TMPDIR/kala-studio`).
* **The token** — the false "writes it to a launcher file with restricted
  permissions (0600)" claim is gone. The packaged flow passes the token
  **in the child environment** and writes **no token file**:
  `studio/electron/main.cjs:36` ("No temporary token files are written from
  the Electron flow itself"), `:121-135` (`buildBackendInvocation` puts
  `KALA_STUDIO_TOKEN` in the child env).
* **The backend log location** — `studio/electron/main.cjs:81-101` uses
  Electron's `app.getPath('logs')`. Resolved for this app:
  * the packaged app's name is **`kala-studio`**: I read the packaged
    `studio/release/linux-unpacked/resources/app.asar` and found
    `"name": "kala-studio"` with **no `productName`** (so `app.getName()`
    is `kala-studio`);
  * Electron's `logs` path is *inside the `userData` directory* on Linux
    and Windows (Electron `app.setAppLogsPath` docs), `userData` is
    `appData` + the app name, and `appData` is `%APPDATA%` on Windows and
    `$XDG_CONFIG_HOME`/`~/.config` on Linux
    (<https://www.electronjs.org/docs/latest/api/app>);
  * ⇒ Linux `~/.config/kala-studio/logs/backend.log`, Windows
    `%APPDATA%\kala-studio\logs\backend.log`.
  * The packaging cell's live evidence agrees:
    `Log file: /tmp/kala-packaging-cell/xdg/config/kala-studio/logs/backend.log`
    (`cells/packaging/REPORT.md` §B1).
  * The rotation bound (current file + 4 archives of 1 MiB, mode 0600) and
    the token scrubbing: `studio/electron/backend-log.cjs:31,66-99,164`
    and `:47-64`.
* **The startup error dialog** — `studio/electron/backend-log.cjs`
  (`formatStartupFailure` / `reportStartupFailure`) and the three call
  sites in `studio/electron/main.cjs`; the packaging cell's three headless
  runs show the body it produces (`cells/packaging/REPORT.md` §B1).
* **The FreeCAD add-on install from the AppImage** — the packaging cell's
  **one** method, verbatim: `packaging/linux/README.md` §"Installing the
  FreeCAD add-on"; the resolver it relies on:
  `adapters/freecad/mod/Kala/kala_paths.py`; the four FreeCADCmd
  scenarios A–D in `cells/packaging/REPORT.md` §B5.
* **Windows CLI notes** — the `.cmd`/`PATHEXT` resolution and the
  `taskkill /T /F` fallback (and the Job Object that has **not** landed):
  `providers/cli_runtime.rs:226-231,289-330,388-502,587-611`; the
  runtime-fixes REPORT §B3/B4 and its gap list. The doc states plainly
  that **neither Windows path has been executed on a Windows host**.
* **The AppImage size/SHA-256** — the stale digest is replaced by a
  pointer to `packaging/linux/README.md`; the digest table itself now
  lives only there (see §1.8).
* Two stale test-count rows updated and **attributed** to the cell that
  ran them (`53 files / 758 tests`, packaging cell; `554` workspace tests,
  design-budget cell) instead of being presented as my own verification.

### 1.4 `docs/cad-providers.md`

* the language-model provider list in the header corrected to the seven ids
  (`providers/mod.rs:28-36`);
* the viewer-label bullet corrected to the honest labels
  (`studio/src/viewer/designSource.ts:26-48`);
* the FreeCAD build timeout corrected from **120 s** to **1 800 s**
  (`backend/crates/kala-cad-providers/src/freecad.rs:96,100`;
  `adapters/freecad/src/kala_agent_freecad/contracts.py:57`);
* the unsupported-Python message corrected to the staged ABI set
  `cp311, cp312, cp313, cp314`
  (`backend/crates/kala-cad-providers/src/freecad.rs:582-591`);
* the add-on install: the **one** supported method (copy the staged add-on
  and its payload into FreeCAD's user `Mod` directory), with the ABI
  probe — `packaging/linux/README.md`, `adapters/freecad/mod/Kala/kala_paths.py`;
* **new section "Cuts: the plane rule, the check and the per-pocket
  warnings"** — the same-plane rule and its exact message
  (`backend/crates/kala-design-intent/src/script.rs:538-560`), the
  per-pocket warning (`backend/crates/kala-cad/src/native/compile.rs:304`;
  `adapters/freecad/src/kala_agent_freecad/cad/design.py:615-619`), the
  `cuts_effective` row and its exact arithmetic
  (`backend/crates/kala-studio/src/native_geometry/service.rs:1873-1885`),
  and the live off-plate vs four-hole numbers (25600.0 mm³ / 24695.221316
  mm³, `verified: false` / `true`) from the cut-guard cell's live
  evidence (`.mece/waves/agent-providers-20261020/cells/cut-guard/REPORT.md`);
* **new section "The limits"** — the script budget, the Solid Edge 8-part
  cap, the 1 800 s deadline, the 60 s compile budget, the 8 000-call
  adapter bound and the measured build cost
  (`backend/crates/kala-contracts/src/construction.rs:22,24,31`;
  `backend/crates/kala-solidedge-client/src/compile.rs:127-136`;
  `backend/crates/kala-cad-providers/src/freecad.rs:96,100`;
  `backend/crates/kala-cad/src/lib.rs:79`; the design-budget REPORT §4/§"Other hidden bounds");
* two new items in "Honest limitations" (the 8-part cap, the slow build).

### 1.5 `docs/studio-limitations.md`

* **the fixed gap removed** — the "the normal Studio CLI still ignores
  `KALA_STUDIO_DATA_DIR`" bullet and the "the packaging smokes … are not
  fixed" bullet are gone; the documented six-row precedence replaces them
  (`backend/crates/kala-studio/src/boundary.rs:346-412`) and the smokes
  now launch with an explicit temp `--data-dir` **and** a temp `HOME`
  (runtime-fixes REPORT §B2).
* **"Provider honesty" rewritten** — the seven ids, the probe verdict
  (`providers/mod.rs:180-215`), the consent refusal
  (`providers/cli_runtime.rs:1226-1228`), the cached probe and its
  `checking...` state (`providers/cli_runtime.rs:64,72,76`), the unknown
  live model name (`providers/claude_cli.rs:133`,
  `providers/codex_cli.rs:141`, `providers/opencode_cli.rs:187`;
  `studio/src/providerPolicy.ts`), and the fact that the Studio loads the
  provider list once at boot (runtime-fixes REPORT gap 5).
* **new section "What is still slow, capped or unbounded"** — the five
  remaining gaps the brief names:
  1. a 500-step FreeCAD build is slow (17–21 min, measured — design-budget
     REPORT §4);
  2. the Solid Edge 8-part cap is far below the 64-component budget
     (`backend/crates/kala-solidedge-client/src/compile.rs:127-131`);
  3. the Windows tree kill uses `taskkill /T /F`, **not** a Job Object,
     and no Windows path has been executed
     (`providers/cli_runtime.rs:587-611`; runtime-fixes REPORT gap 2);
  4. **the journal and the artifact store grow without bound** — the files
     live at `<studio data dir>/studio-journal.sqlite3` and
     `<studio data dir>/artifacts/objects/<xx>/<digest>`
     (`backend/crates/kala-studio-journal/src/journal.rs:37,39,320`;
     `backend/crates/kala-artifacts/src/lib.rs:540`), and there is **no
     retention, prune route or size budget** (a grep for
     `DELETE|prune|retention|garbage` over `kala-studio-journal/src` and
     `kala-artifacts/src` finds nothing that deletes; the in-memory design
     cache is 32 entries);
  5. the live CLI model name is unknown → the honest `default` wording.
* the stale AppImage digest → pointer to `packaging/linux/README.md` (two
  places, including the "Provenance" bullet that called the digest "the
  provenance");
* the corpus digest updated from `53dcb9da…` to **`e86c7d93…`**, the
  value the fixture itself pins
  (`backend/crates/kala-studio/tests/corpus_fixture.rs:46`; the raw
  `studio_http.json` sha256 is `dc4216ba…`, as the runtime-fixes REPORT
  lists);
* the readiness row updated to **17** native route-methods (the built
  report) and the test-count rows attributed to the cells that ran them;
* "What this wave did not test" gained three bullets: no real subscription
  CLI was run, the Windows-only CLI paths were not executed, and the
  subscription CLIs are chat providers, not orchestration.

### 1.6 `README.md` (B19)

* **new "## Architecture" section** — the shipped app is a **native Rust**
  application (Electron shell → `kala-studio` binary → real OpenCascade
  7.9.3 through `backend/crates/kala-cad/src/ffi/adapter.cpp`); the
  AppImage contains the Rust binary, the staged OCCT closure, the compiled
  UI and the FreeCAD adapter and **no Python interpreter, no PyInstaller
  payload and no `cadquery-ocp` binding**; the Python tree is the
  **development-only** reference/parity oracle.
  Verification: `studio/electron-builder.json:23-52` (the packaged
  `files`/`extraResources` list — `electron/backend/`, no Python);
  `studio/electron/main.cjs:3-11` (packaged mode launches only the native
  Rust `kala-studio`); `backend/crates/kala-cad/src/ffi/adapter.cpp` (the
  one C++ adapter); `docs/studio.md` §"The native Studio service and the
  worker fallback" (the Python worker is opt-in).
* the repository-layout table corrected (`backend/crates/`, the
  development-only Python reference, `packaging/` = AppImage + NSIS
  script — the "PyInstaller spec and entry script" row was false for the
  shipped artefact);
* the quick start rewritten: the **native Rust service** is the developer
  path, the Python reference server is labelled *development only*, and
  the desktop build no longer instructs the reader to run PyInstaller
  (the old `pyinstaller packaging/studio-backend.spec` recipe described
  the **retired** staging — `studio/electron-builder.json:1-6` says the
  legacy `studio/backend/` directory is never copied);
* the honesty disclaimer rewritten: seven provider ids, the three
  subscription CLIs as sandboxed chat providers, the honest
  default-model wording, the commercial-CAD clarification (the only
  geometry the studio builds itself is the native OpenCascade path), and
  the `cadquery-ocp` → native-OCCT correction.

### 1.7 `packaging/linux/README.md` (prose only)

* **new section "The built artefact (size and SHA-256)"** — the file, the
  size and the digest of the artefact in the tree, with the explicit note
  that the digest changes with every rebuild and is recorded **only**
  here, plus the `sha256sum`/`stat` commands to verify it.
  Verification: I ran them myself —
  `sha256sum studio/release/kala-studio-0.1.0-x86_64.AppImage` →
  `eed6ef7b780fe1a3372e9e86e3e3f8d6b74b74395c2f5469618125970ea0ff98`,
  `stat -c %s` → `163868354`. The same numbers are in the packaging
  cell's REPORT §"AppImage", so the artefact in the tree and the cell's
  build agree.

### 1.8 `docs/rust-studio.md`, `docs/rust-migration-status.md`

Not in the brief's update list, but inside my ownership (`docs/**`) and
carrying the **same stale digest** the brief asked to remove (audit B9).
The digest tables and the `sha256sum` rows now point at
`packaging/linux/README.md` instead of quoting a digest that no longer
matches the artefact. **Nothing else** in those two files was changed.
`docs/adr/**` was **not** touched (the brief forbids rewriting ADRs) —
see §4.

### 1.9 The three comment headers (B18) — comments only

| file | what the stale comment claimed | what the code does | comment now says |
|---|---|---|---|
| `.github/workflows/studio-release.yml` | the vendored set "is coupled to the FreeCAD build used at build time" | the staging vendors **all four** ABIs (`studio/scripts/native-backend.cjs:83` `FREECAD_PYTHON_ABIS = ['cp311','cp312','cp313','cp314']`; `studio/scripts/freecad-python-deps.json` lists the same four); the build host's FreeCAD is only used for the in-FreeCAD verification | all four ABIs are vendored; the detected interpreter is used for the verification only |
| `.github/workflows/studio.yml` | "development defaults to the Python reference"; "the accepted native slice currently covers the original nine Studio route-methods" | the native backend is the default (`backend/crates/kala-studio/src/cli.rs:165-181`; the worker starts only when `KALA_STUDIO_GEOMETRY_WORKER` names one) and the readiness report counts **17** route-methods (`studio/release/native-readiness.json`) | native by default, the Python reference is the explicit dev-only oracle, 17 route-methods |
| `packaging/linux/smoke-appimage.mjs` | the geometry/FreeCAD checks are "PENDING **until native-design lands**" | native-design has landed; the flags `--require-geometry` / `--require-freecad` make the checks mandatory, and the release workflow passes both (`.github/workflows/studio-release.yml:128,131`) while the local build script does not (`packaging/linux/build-appimage.sh:98`) | the flags decide; native-design has landed |

`git diff` on those three files shows **only comment lines** changed.

---

## 2. False statements removed (the point of this cell)

| where | the false statement | why it was false |
|---|---|---|
| `docs/studio-providers.md` | "The studio chat surface accepts **one of four** provider identifiers"; `codex` is "**Not implemented in this build**: the route fails closed with a sanitised `503`" | seven ids are published (`providers/mod.rs:28-36`); the Codex CLI route runs (`providers/codex_cli.rs`) |
| `docs/studio-providers.md` | the whole "Codex CLI — disabled by design" section, including "the studio does not launch the Codex CLI", "the `KALA_STUDIO_ENABLE_CODEX` flag is inert", and the quoted 503 refusal | the route launches the CLI; there is no such flag in the code and the refusal is now `provider=codex_cli requires allow_remote=true at request time` (`providers/mod.rs:397-412`, `cli_runtime.rs:1226-1228`) |
| `docs/studio-providers.md` | "Only the `openai` and `openrouter` routes make an outbound request, and only when `allow_remote=true`" | the three CLI routes also leave the machine (the CLI talks to its vendor) and need the same consent |
| `docs/studio-providers.md` | the trust-model recap "the `codex` route never leaves the process" | the same |
| `docs/studio.md` | "The `codex` route is **not implemented** in this build; it fails closed with a sanitised `503` and sends no request." | same |
| `docs/studio.md` | "No app or OpenCode orchestration. The studio does not drive other applications, editors or agent runtimes." (as written, it also denied the OpenCode route) | reworded: the CLIs are **chat providers with every tool denied**, and the studio still drives no other application |
| `docs/studio.md` | "the viewport says … `view: OpenCascade (opt-in) build` only when the selected provider **is** `opencascade`" | that label was removed: the picture is always the OpenCascade **import** (`studio/src/viewer/designSource.ts:26-34`) |
| `docs/studio.md` | the AuxView label `SOURCE: OCP HLR · VIEW: IMPORT` | the real strings are `SOURCE: OCP HLR OF THE IMPORTED MODEL` and `VIEW: OCC IMPORT OF FREECAD STEP` (`designSource.ts:36-48`) |
| `docs/studio.md`, `docs/studio-install.md`, `docs/studio-limitations.md`, `docs/rust-studio.md`, `docs/rust-migration-status.md` | the AppImage "163 642 516 bytes … sha256 `b069b7d4…`" and "**The SHA-256 above is the provenance**" | the artefact in the tree is 163 868 354 bytes / `eed6ef7b…`; the old digest matched no build in the tree |
| `docs/studio.md` | "The **Python reference surface is 15 route-methods**; the native service answers **16** … All 16 are answered natively" | 17 (`server.rs:184-281`, the built readiness report) |
| `docs/studio-install.md` | "generates a random `KALA_STUDIO_TOKEN`, **writes it to a launcher file** with restricted permissions (0600 on POSIX)" and "The bearer token file is created with mode 0600." | the Electron flow writes **no** token file (`studio/electron/main.cjs:36,121-135`) |
| `docs/studio-install.md` | "**The app writes its data directory** under `$XDG_DATA_HOME/kala-studio/` (default `~/.local/share/kala-studio/`)" | the studio data directory lives under the kala-agent data directory (`boundary.rs:346-412`), and the env precedence is documented |
| `docs/studio-install.md` | the troubleshooting row "Check the launcher log file in `~/.local/share/kala-studio/log/` or `%APPDATA%\kala-studio\log\`" | no such directory is created; the log is `<Electron logs>/backend.log` (`studio/electron/main.cjs:81-101`) |
| `docs/studio-install.md` | "npm test # 43 test files, 628 tests"; "cargo test --workspace … (449 tests)" | 53 files / 758 tests and 554 workspace tests (attributed to the cells that ran them) |
| `docs/studio-limitations.md` | "The `codex` (ChatGPT subscription) route is **not implemented** … The `KALA_STUDIO_ENABLE_CODEX` flag is inert." | same as above |
| `docs/studio-limitations.md` | "only `openai` and `openrouter` perform a real remote call" | the three CLI routes are real remote turns too |
| `docs/studio-limitations.md` | the open gap "The **normal** (non-worker) Studio CLI still ignores `KALA_STUDIO_DATA_DIR` … This is pre-existing and deliberate." | fixed by the runtime-fixes cell (`boundary.rs:346-412`) |
| `docs/studio-limitations.md` | the open gap "The **packaging smokes** … start the backend without `--data-dir` … it is not fixed." | fixed by the runtime-fixes cell |
| `docs/studio-limitations.md` | the corpus digest `53dcb9da…` | the fixture pins `e86c7d93…` (`corpus_fixture.rs:46`) |
| `docs/cad-providers.md` | the FreeCAD build "timeout = 120 s" | 1 800 s (`freecad.rs:96,100`, `contracts.py:57`) |
| `docs/cad-providers.md` | "the bundled adapter dependencies support cp310, cp311, cp312, cp313 only" | the staged set is cp311–cp314 (`freecad.rs:582-591`; `native-backend.cjs:83`) |
| `docs/cad-providers.md` | the viewer-label bullet "`view: OpenCascade (opt-in) build` only when the selected provider *is* `opencascade`" | the same removed label |
| `docs/cad-providers.md` | "the `mod/Kala` addon is **staged, not installed**: the user copies it into FreeCAD's user `Mod` directory by hand" (and the docs it pointed at) | the add-on is now self-contained and the one install method is documented in full (`packaging/linux/README.md`, `adapters/freecad/mod/Kala/kala_paths.py`) |
| `README.md` | "the studio backend, the desktop shell, the **PyInstaller pipeline**, and the GitHub Actions matrices were produced by this wave" | the shipped pipeline is native Rust + OCCT; the PyInstaller spec is retired |
| `README.md` | "A local build needs the PyInstaller backend binary first … `pyinstaller packaging/studio-backend.spec`" | the AppImage is built from the Rust runtime (`packaging/linux/build-appimage.sh`) |
| `README.md` | "The ChatGPT subscription route (`codex`) is **not implemented** in this build … The only real API providers are `openai` and `openrouter`" | seven ids, three CLI providers |
| `README.md` | "There is no native CAD or solver automation" | ambiguous and, read next to "native vendor connectors … are not implemented", it denied the native OpenCascade path that does exist; reworded to "no **commercial** CAD automation" |
| `README.md` | "exact STEP export today uses the native `cadquery-ocp` bindings" | the shipped app uses the native OpenCascade 7.9.3; `cadquery-ocp` belongs to the Python reference |
| `README.md` | the layout row "`packaging/` — PyInstaller spec and entry script for the studio desktop backend" | the AppImage + NSIS build |
| `README.md` | "The studio backend reuses the project's `.venv`" as the developer path | the native Rust service is the developer path; the Python reference is dev-only |
| `.github/workflows/studio-release.yml` | the vendored FreeCAD set "is coupled to the FreeCAD build used at build time" | all four ABIs are vendored |
| `.github/workflows/studio.yml` | "development defaults to the Python reference"; "the original nine Studio route-methods" | native by default; 17 route-methods |
| `packaging/linux/smoke-appimage.mjs` | the geometry checks are "PENDING **until native-design lands**" | native-design has landed; the two flags decide |

---

## 3. Could not verify / honest limits

1. **Nothing was executed against a Windows host.** The `.cmd`/`PATHEXT`
   resolution and the `taskkill /T /F` tree kill are documented from the
   code (`providers/cli_runtime.rs`) and from the runtime-fixes cell's
   type-checked-against-msvc evidence; the docs say so in three places
   (`docs/studio-providers.md`, `docs/studio-install.md`,
   `docs/studio-limitations.md`). No Windows runtime claim is made.
2. **The Electron startup dialog was never seen on a desktop.** The
   packaging cell verified the body through the same code path in a
   headless Electron run and through `studio/test/backendLog.test.ts`; the
   docs describe the dialog from that evidence and do not claim a rendered
   window.
3. **The FreeCAD timings were not re-measured.** The 58.8 s / 1 009.5 s /
   1 250 s figures are the design-budget cell's live measurements on
   FreeCAD 1.1.4; re-running them costs ≈ 23 minutes of FreeCAD time.
   They are quoted with their provenance.
4. **The "543 operations maximum" is the design-budget cell's measured
   lowering ratio**, not something I recomputed from the compiler.
5. **The OpenRouter numbers are a single observation.** One unauthenticated
   `curl` on 2026-10-21: 468 models, 20 free, 19 `~` ids. The list moves;
   the doc dates the observation.
6. **No test suite was run.** The brief limited me to read-only commands
   apart from the doc edits, and `studio/**` + the Rust backend are being
   edited by concurrent cells. The UI claims are cited to the shipped
   **source** (and to the ui-fixes cell's tests), not to a run of mine.
   The test counts quoted in the docs are attributed to the cells that
   produced them.
7. **B11 (an artifact download route) and the CAD-build cancel are not
   documented** — the brief assigns them to round 2, and the concurrent
   `downloads-cancel` cell owns the code.
8. **`docs/adr/0010-rust-control-plane-cad-kernel-geometry-worker.md:61`
   still carries the stale AppImage digest** (`163 642 516 bytes`,
   `b069b7d4…`). The brief forbids rewriting ADRs, so I left it; every
   user-facing document now points at `packaging/linux/README.md` instead.
   This is the **one false statement left in the tree that I could have
   removed** — flagging it for the conductor.
9. `studio/README.md:54-55` also carries the stale digest. That file is
   **not** in my ownership list (`docs/**`, the root `README.md`,
   `packaging/linux/README.md`), so I left it untouched.
10. `docs/adr/**` was not touched at all, as instructed.

---

## 4. Cleanliness

* `/tmp/opencode/kala-docs-cell` (the temp `HOME`, data dirs, the fetched
  OpenRouter list) was **removed** at the end; `df -h /tmp` shows 5.2 GB
  free. No other cell's temp directory was touched.
* `/home/v9i/.local/share/kala-agent/**` was never listed, opened or
  written.
* No credential, auth or settings file was read; no `claude`, `codex` or
  `opencode` CLI was run.
* **No commit was made.**

---

# Round 2 (after `downloads-cancel` landed)

**Cell:** `docs` · **Wave:** `completeness-20261021` · **Round:** 2 ·
**Date:** 2026-10-21
**Input:** `../downloads-cancel/REPORT.md` · **Prior round:** §1–§4 above.

Same rules as round 1: every statement below is verified against the code
or against a command I ran (cited here, not in the docs). **No code was
touched.** The `packaging/linux/README.md` **digest row
(`packaging/linux/README.md:191-195`) was left exactly as it was** for
Opus, as instructed. Every shell ran with a temp `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR`, `XDG_DATA_HOME`,
`XDG_CONFIG_HOME` and `TMPDIR` under `/tmp/opencode/kala-docs-r2`
(removed again at the end). No credential or settings file was read; no
`claude`, `codex` or `opencode` CLI was run;
`/home/v9i/.local/share/kala-agent/**` was never listed, opened or
written. **No commit was made.**

## R2.1 Files changed in round 2

| file | what changed |
|---|---|
| `docs/studio.md` | two new sections (**"Downloading what a design stored"**, **"Cancelling a turn"**); the route table and counts; the readiness wording; the `/health` highlight; the client-deadline bullet; the worker's internal route list |
| `docs/cad-providers.md` | new **"A cancelled build is killed, not asked to stop"**; the new artifact-download route; the `499 design_cancelled` error row; the journal's cancelled-turn event; a new honest limitation |
| `docs/studio-limitations.md` | two new limits (the 64 MiB download cap, the cancel that cannot interrupt the in-process build); the readiness row; the test-suite rows; three new "what this wave did not test" bullets |
| `docs/rust-studio.md` | the route surface section (16 → **18** native, 15 → **16** Python, the two missing routes, the seam wording); the worker-differential count; the corpus digest |
| `docs/rust-migration-status.md` | the worker-differential count (3 places) and the corpus digest (2 places) |
| `docs/studio-install.md` | the `npm test` count (758 → **769**) |
| `studio/README.md` | the stale AppImage digest → a pointer to `packaging/linux/README.md` (**the one line the brief allowed**) |
| `docs/adr/0010-rust-control-plane-cad-kernel-geometry-worker.md` | **one** dated line appended under "Phase 2 implemented" (**the only ADR edit allowed**) |

Untouched by round 2: `README.md`, `packaging/linux/README.md`,
`docs/studio-providers.md`, the three comment headers, and every code
file.

## R2.2 The artifact download route

Documented in `docs/studio.md` §"Downloading what a design stored" and
`docs/cad-providers.md` §"`GET /api/studio/design/{design_id}/artifact/{kind}`".

| what the docs say | verification |
|---|---|
| The route is `GET /api/studio/design/{design_id}/artifact/{kind}` | `backend/crates/kala-studio/src/server.rs:246-268` (the `split_once("/artifact/")` arm of `classify`), `:277-297` (`RouteKind::Artifact => &[Method::GET]`) |
| The kinds are exactly `script`, `step`, `fcstd`, `stl` | `backend/crates/kala-studio/src/artifacts.rs:15` (`ARTIFACT_KINDS`); the Python mirror `src/kala_agent/studio/design/artifacts.py:19` |
| **Only the kinds the design stored** answer; the others are `404 artifact_not_found` with `details: ["design_id=<id>; kind=<kind>"]` | `native_geometry/service.rs:1239-1256` (`find_artifact_record` → `NotFound`), `native_geometry/mod.rs:1155-1160` (the envelope) |
| The `design_id` must be a 64-character lowercase hex sha256, else `422 invalid_request`; an unknown kind is `422` and the detail names the four kinds | `server.rs:312-317` (`valid_design_id`), `:1401-1427` (both refusals, before any store access) |
| The route carries its own bearer gate | `server.rs:208-219` (`requires_route_bearer` lists `Artifact`), the same gate as `/drawing` and `/journal` |
| **The exact stored bytes**, re-hashed against the journalled sha256; a mismatch is `500 artifact_digest_mismatch`, not a download | `native_geometry/service.rs:1239-1301` (the store read at `:1257`, the re-hash at `:1287-1292`), the refusal at `native_geometry/mod.rs:1149-1154`; the digest comes from the journal's `provider.build.completed` / `artifact.stored` payload (`service.rs:1869-1914`) |
| The hard cap is **64 MiB**; larger is `413 artifact_too_large` | `artifacts.rs:21` (`MAX_ARTIFACT_DOWNLOAD_BYTES = 64 * 1024 * 1024`), the check at `service.rs:1280-1286`, the refusal at `native_geometry/mod.rs:1143-1148` |
| The headers: `content-type` (`application/json` / `application/step` / `model/stl` / `application/octet-stream`), `content-disposition`, `x-studio-design-id`, `x-studio-digest`, `x-studio-artifact-kind` | `artifacts.rs:31-38` (the content types), `native_geometry/mod.rs:1113-1130` (the response) |
| The filename is `<label-slug>-<digest8>.<ext>`; `FCStd` for the native document | `artifacts.rs:45-55` |
| The slug: ASCII alphanumerics lowercased, other runs → one `-`, bounded to **40** characters, `kala-studio` when the label has no usable character | `artifacts.rs:24,27,64-81`; the same algorithm in `src/kala_agent/studio/design/artifacts.py:50-…` |
| The **download button** sits next to every digest chip in the **design card** and in the **export panel**; the bytes are saved under the backend's `content-disposition` filename and a receipt (`Saved <filename>` + byte count + digest8) goes into the local ledger; a refusal is shown as `code: message` | `studio/src/components/chat/DesignAgentCard.tsx:61-94` (the handler, the receipt at `:73-79`, the refusal at `:81-88`) and `:203-215` (the button); `studio/src/components/export/ExportPanel.tsx:224-250` and `:270-328`; the client `studio/src/api/client.ts:717-777`; the chip→kind mapping `studio/src/components/chat/designCard.ts:45-61,210-238` |
| The `stl` kind answers only after the design was exported as one, and no UI chip offers it | `native_geometry/service.rs:1157-1204` (the export stores the STL and writes `artifact.stored` with `kind: "stl"`); the chips are script / step / native only (`designCard.ts:210-238`) |
| The Python reference mirrors the rules, and the worker proxy answers the route | `src/kala_agent/studio/design/artifacts.py:19-42`, `src/kala_agent/studio/server.py:917-961`, `src/kala_agent/studio/internal_api.py:475-484`, `backend/crates/kala-studio/src/geometry_backend.rs:190-200,517-529` |

## R2.3 Cancel

Documented in `docs/studio.md` §"Cancelling a turn",
`docs/cad-providers.md` §"A cancelled build is killed, not asked to stop"
and `docs/studio-limitations.md`.

| what the docs say | verification |
|---|---|
| A client abort drops the request future; a **cancel-on-drop guard** sets the turn's token; the blocking design task observes it; a finished turn disarms the guard | `native_geometry/mod.rs:82-111` (`CancelOnDrop`), `:214-225` (the wiring in `GeometryBackend::design`) |
| The same seam covers `POST /design` **and** the design turn inside `POST /chat` | `chat_design.rs:1249-1265` (`compile_design` → `backend.design(...)`, the same trait method) |
| The token is checked at every phase boundary: validation → intent, probe → build request, build → import, import → measure/journal, and before each post-build event | `native_geometry/service.rs:451-476` (the doc comment), `:514-517,548,588,634,642,715,736,766,789,822,854` (the checks) |
| The **FreeCAD process group is killed**, and the killed build is a *cancellation*, not a "build refused" | `backend/crates/kala-cad-providers/src/freecad.rs:599-620` (`build_cancellable`), `:677-694` (`run_bounded_with_cancel` → `supervise(…, cancel)`; the reason `the FreeCAD build was cancelled; the FreeCADCmd process group was stopped`), `backend/crates/kala-cad-providers/src/base.rs:689-700` (`CadProviderError::Cancelled`), `native_geometry/service.rs:608-612` |
| The **in-process OCCT build is not interruptible mid-operation**; the token is honoured at the next phase boundary, worst case one **60 s** compile | `native_geometry/service.rs:455-460` (the doc comment), `backend/crates/kala-cad/src/lib.rs:79` (the 60 s compile budget) |
| The **CLI process tree** dies with the request future (its own group: `SIGKILL` on Unix, `taskkill /T /F` on Windows) | `backend/crates/kala-studio/src/providers/cli_runtime.rs:542-581` (the guard), `:592-611` (the kill), `:732` (the attach) |
| The journal records **exactly one** `design.cancelled` with the design id and a bounded reason — no prompt, no credential, no provider output — and **nothing after it** | `native_geometry/service.rs:89` (`DESIGN_CANCELLED`), `:92` (`CLIENT_DISCONNECT_REASON`), `:1776-1803` (`stop_if_cancelled` / `cancel`); "nothing after it" is the phase-boundary check above the post-build record |
| The service's own refusal is `499 design_cancelled` | `native_geometry/mod.rs:1261-1266` |
| The UI reads `Cancelled — the backend stopped the turn.` and the ledger `Turn cancelled — the backend stopped it` | `studio/src/components/chat/ChatPanel.tsx:285-296`, `:305-307` (the abort), `:494` (the line), `:581-582` (the Cancel button) |
| **The Python reference cannot cancel a running synchronous build** — its `/design` route is synchronous in the FastAPI thread pool and a thread cannot be dropped by the event loop | `src/kala_agent/studio/design/service.py:547-559` (the docstring says exactly this) |
| The download cap (64 MiB) is independent of the STEP-import ceiling (`max(64 MiB, KALA_STEP_IMPORT_MAX_BYTES)`) | `native_geometry/service.rs:379-382`, `backend/crates/kala-studio-journal/src/journal.rs:45` (`DEFAULT_MAX_ARTIFACT_BYTES = 64 MiB`) |

**What I did not re-run:** the cancellation experiments themselves (the
FreeCAD process-group kill and the 0.05 s survival measurement) — those are
the `downloads-cancel` cell's live evidence, quoted in the docs as
behaviour, and this round ran **no** FreeCAD, no server and no test suite.

## R2.4 The budget and the client deadline

Documented in `docs/studio.md` (`/health` highlight + the client-deadline
bullet).

| what the docs say | verification |
|---|---|
| `/health` publishes **`design_turn_budget_seconds`** | `backend/crates/kala-studio/src/server.rs:1125-1132`; the Python mirror `src/kala_agent/studio/providers/dispatch.py:75` and `src/kala_agent/studio/schemas.py:427` |
| The value is `2 × (180 + 1 800) = 3 960 s` (66 min) | `backend/crates/kala-studio/src/providers/remote.rs:33` (the 180 s model call), `:43` (the 1 800 s build), `:45` (two attempts), `:53-56` (the formula) |
| The client deadline is **budget + 5 min** = **71 min**, with a **70-minute** fallback when the budget is missing or unusable | `studio/src/api/client.ts:140-141` (the margin, the fallback), `:152-161` (`chatTurnTimeoutMs`), `:441-446` (the chat turn uses it), `:327-328` + `studio/src/state/store.ts:153,517-520` (the health field is stored and read) |
| **The old "30 minutes" claim was false** and is gone | there is no 30-minute constant left in `studio/src` (`CHAT_TURN_TIMEOUT_MS` is now the fallback alias, `client.ts:143`) |

## R2.5 The route table, the counts and the readiness wording

| what the docs say | verification |
|---|---|
| The **Python reference surface is 16** route-methods; the **native service answers 18** | **I ran the scanner**: `node studio/scripts/native-readiness.cjs --report-only --json` in the current tree → `python.routeMethodCount: 16`, `native.routeMethodCount: 18`, `rustNativeRouteMethodCount: 7`, `geometrySeamRouteMethodCount: 11`, `missingRouteMethods: []`, `ready: true` |
| The 18 = the 16 plus the ADR-0011 view-only `POST /import` and the provider model catalog `GET /providers/{id}/models` (the two "native-only" rows) | the same JSON run → `extraNativeRouteMethods: ["GET /api/studio/providers/{}/models", "POST /api/studio/import"]`; the route table itself `server.rs:185-297` |
| The readiness wording is now **"geometry-backend seam (answered natively by default; the Python worker is a development opt-in)"**, and the old "proxied" label is gone from the report | `studio/scripts/native-readiness.cjs:6-14` (the header), `:664-670` (the JSON fields `geometrySeamRouteMethodCount` / `geometrySeamRouteMethods`, with `proxiedRoute*` kept as a compatible alias), `:707` (the printed line), `:677-683` (the summary); `studio/scripts/desktop-pack.cjs:191-202` (the reworded warning) |
| The **built** stage report in the tree still carries the **earlier** split (15 Python / 17 native, ten "proxied") because it is a snapshot of the sources at packaging time — the docs say so explicitly | `studio/release/native-readiness.json` (`generatedAt: 2026-10-20T20:11:14`, `python.routeMethodCount: 15`, `native.routeMethodCount: 17`, `proxiedRouteMethodCount: 10`, `warning: "10 route-method(s) are delegated through the geometry-backend seam…"`), read from the tree |
| The worker differential is **295** cases now (three new artifact ids) | `backend/crates/kala-studio/tests/worker_differential.py` (the `design.artifact.{script,fcstd,bogus}` cases at `:887-918`); the **295 / 0 mismatches / 0 known gaps** result is the `downloads-cancel` cell's run, and the docs attribute it to that cell |
| The corpus in the tree carries the digest **`c0003bae…`** | **I ran it**: `sha256` of `backend/fixtures/corpus/v1/{studio_http.json,manifest.json,canonical_json.json}` → `3eebaedd…`, `2dbf37ea…`, `0c2a4bd9…`; `manifest.corpus_sha256 = c0003bae333f27a079adcf3e86f2618741a6606334a17ffe3fafe727fcd6c330`, which is what `backend/crates/kala-studio/tests/corpus_fixture.rs:46` pins |

## R2.6 The two one-line edits the brief allowed

* `studio/README.md:53-58` — the stale digest
  (`163 642 516 bytes`, `b069b7d4…`) is replaced by a pointer to
  `packaging/linux/README.md`.
* `docs/adr/0010-rust-control-plane-cad-kernel-geometry-worker.md` — one
  dated line appended at the end of the "Phase 2 implemented" section:
  *"**2026-10-21.** Later builds supersede the AppImage digest above; the
  current artefact is recorded in packaging/linux/README.md."* Nothing
  else in the ADR changed (`git diff` on the file is that one line).
* **`packaging/linux/README.md` was not touched at all** — the digest row
  at `:191-195` is byte-identical to what it was.

## R2.7 False statements removed in round 2

| where | the false statement | why it was false |
|---|---|---|
| `docs/studio.md` | "A turn that has not answered within **30 minutes** is abandoned by the browser" | the client deadline is now **derived from `/health`**: `design_turn_budget_seconds` (3 960 s) + 5 min = **71 min**, with a **70-minute** fallback (`studio/src/api/client.ts:140-161`) |
| `docs/studio.md`, `docs/rust-studio.md`, `docs/studio-limitations.md` | "The **Python reference surface is 15** route-methods; the native service answers **17**" / "**16 native**" / "15 Python route-methods covered, **17 in the native service**" | 16 / 18 (my own scanner run, §R2.5) |
| `docs/studio.md`, `docs/rust-studio.md` | the readiness report "still *labels* ten route-methods **proxied**" | the report now names the **geometry-backend seam** (11 route-methods) and never prints "proxied" (`studio/scripts/native-readiness.cjs:707`, `desktop-pack.cjs:198`) |
| `docs/studio.md` | the route table had no row for the artifact download | the route exists (`server.rs:246-268`) and the table now lists it |
| `docs/studio.md`, `docs/rust-studio.md` | the worker's internal route set had no artifact route | `src/kala_agent/studio/internal_api.py:475-484` |
| `docs/studio.md`, `docs/cad-providers.md`, `docs/studio-limitations.md` | nothing anywhere said that a cancel really stops the work, or that the Python reference cannot cancel | the whole behaviour was undocumented although it landed; it is now documented with its limits |
| `docs/rust-studio.md`, `docs/rust-migration-status.md`, `docs/studio-limitations.md`, `docs/cad-providers.md` | "**292 cases**, 0 mismatches, 0 known gaps" (the worker differential) | the harness now has 295 cases (`worker_differential.py:887-918`); the docs attribute the 295/0 result to the `downloads-cancel` cell |
| `docs/rust-studio.md`, `docs/rust-migration-status.md`, `docs/studio-limitations.md` | the corpus digest `53dcb9da…` / `e86c7d93…` | the corpus in the tree carries `c0003bae…` (my own sha256 run, §R2.5) |
| `docs/rust-studio.md`, `docs/rust-migration-status.md` | "the corpus digest is **unchanged**" | the corpus was regenerated when the artifact route landed; both texts now say so |
| `docs/studio-install.md`, `docs/studio-limitations.md` | "**53 test files, 758 tests**" (the studio suite) | the latest run is **769** (`downloads-cancel` cell); the `pytest tests/unit` row (1 555) is now **1 667**, and a `cargo test -p kala-studio` row (**289 passed**) was added |

## R2.8 Could not verify / honest limits

1. **No server, no CAD and no test suite was run in round 2.** The
   cancellation timings (the FreeCAD tree gone 0.05 s after the abort),
   the live FCStd download and the 295-case differential are the
   `downloads-cancel` cell's evidence; the docs describe the behaviour and
   attribute the numbers, exactly as round 1 did with the design-budget
   timings. **The only command I ran** was the readiness scanner
   (`node studio/scripts/native-readiness.cjs --report-only [--json]`) and
   three `sha256sum`-style hashes of the corpus files.
2. **The built stage report in the tree is stale by design**
   (`studio/release/native-readiness.json`: 15/17, "proxied"). It is a
   build artefact tied to the sources of the packaging build; the docs now
   say so instead of quoting it as the current surface. **It was not
   regenerated** — that belongs to the next packaging build.
3. **The 64 MiB download cap was not exercised** against a >64 MiB
   artefact. The cap and its `413` come from the code
   (`artifacts.rs:21`, `service.rs:1280`); no oversized object was
   created.
4. **The `stl` download path was not exercised.** The kind is in the
   allow-list and the export route stores one, but no STL artefact was
   downloaded here.
5. **Nothing on Windows was executed** — the `taskkill /T /F` CLI kill and
   the Windows FreeCAD kill remain type-checked-only. The docs say so.
6. **`docs/adr/0009-…md` shows as modified in `git status`** — that is
   **another cell's** change, not mine: I edited only
   `docs/adr/0010-…md`, and only by the one line the brief allowed.
7. **The AppImage digest row is still the packaging cell's value**
   (`163 868 354` / `eed6ef7b…`), not a fresh build. Opus writes the
   final row after the last build, as instructed.

## R2.9 Cleanliness (round 2)

* `/tmp/opencode/kala-docs-r2` (temp `HOME`, data dirs, `XDG_*`, `TMPDIR`)
  was **removed** at the end; `df -h /tmp` checked.
* `/home/v9i/.local/share/kala-agent/**` was never listed, opened, named
  in a command or written.
* No credential, auth or settings file was read; no `claude`, `codex` or
  `opencode` CLI was run; no FreeCAD was run.
* **No commit was made.**
