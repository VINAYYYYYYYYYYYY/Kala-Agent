# Cell runtime-fixes — data-dir precedence, CLI process-tree kill, Windows shims, non-blocking probes, honest offline

Wave `completeness-20261021` · Cell `runtime-fixes` · 2026-10-21

Items: **B2**, **B3 (CLI part)**, **B4**, **B13**, **B16** of
`../audit-backend/AUDIT.md`.

No commit was made. `/tmp` artifacts were removed. `/home/v9i/.local/share/kala-agent/**`
was never read or written: every shell ran with a temporary `HOME`, a temporary
`KALA_AGENT_DATA_DIR` and (where relevant) a temporary `KALA_STUDIO_DATA_DIR`,
`XDG_DATA_HOME`, `TMPDIR`, and `RUSTUP_HOME=/home/v9i/.rustup`,
`CARGO_HOME=/home/v9i/.cargo`.

Never ran a real `claude` / `codex` / `opencode` turn. Two hygiene notes, stated plainly:

* The **unit suite** already probed the host's real CLIs through
  `/settings` and `/providers` before this cell (the `client` fixture inherited
  the developer `PATH`). The fixture now sets `PATH=""`, so the suite cannot
  start a CLI at all; the probe answers "not found" without spawning.
* One early `studio_smoke.py` run (before the smoke was hermeticized, see B2)
  inherited the developer `PATH` and its `/health` call probed the host's real
  `claude` (`--version` and the auth-status command only; no prompt, no turn).
  The smoke now points all three CLI overrides at a nonexistent path and is
  deterministic.

---

## B2 — data-directory precedence

### Change

One precedence, implemented twice (Rust `boundary::default_studio_data_dir`,
Python `kala_agent.studio.settings.resolve_studio_data_dir_from_env`):

| # | Source | Resolved path | Invalid value |
|---|---|---|---|
| 1 | `--data-dir` | used as-is (CLI resolves it) | CLI refuses |
| 2 | `KALA_STUDIO_DATA_DIR` | used **as-is** (it *is* the studio dir, no `studio` child) | ignored, next row |
| 3 | `KALA_AGENT_DATA_DIR` | `<value>/studio` | ends the search with the temp fallback (the `Settings` validator's documented failure path) |
| 4 | `XDG_DATA_HOME` | `<$XDG_DATA_HOME>/kala-agent/studio` | ignored, next row |
| 5 | `$HOME` | `$HOME/.local/share/kala-agent/studio` (the historical path, unchanged) | next |
| 6 | — | `$TMPDIR/kala-studio` | — |

Validation kept: absolute, not a filesystem root, not an existing
non-directory; a leading `~` is expanded against the home **in the same
environment view**. The env view is the only home source (`HOME`, then
`USERPROFILE`, then `HOMEDRIVE`+`HOMEPATH`), so a request cannot redirect the
data directory and a test can prove every row without mutating the process.
Resolved directories are created and the studio directory is tightened to
`0o700`.

Files: `backend/crates/kala-studio/src/boundary.rs` (+ unit tests),
`src/kala_agent/studio/settings.py` (resolver + docstring),
`src/kala_agent/studio/cli.py` (`_resolve_data_dir` now uses the resolver),
`src/kala_agent/studio/server.py` (`create_studio_app` uses the resolver),
`packaging/native_studio_smoke.py`, `packaging/studio_smoke.py`,
`backend/crates/kala-studio/tests/cli_launch.rs` (+2 launch tests),
`tests/unit/test_studio_data_dir_precedence.py` (new, 12 cases),
`tests/unit/test_data_dir_isolation.py` (one test now passes the mapping).

### Proof

Rust unit tests (in `boundary.rs`): `data_dir_precedence_matches_the_documented_order`,
`an_unusable_studio_override_is_ignored`,
`no_environment_resolves_the_home_path_or_the_temporary_fallback`,
`invalid_data_dir_env_uses_the_temporary_fallback` — all green.

End-to-end, release binary vs `python -m kala_agent.studio`, identical output
for all five rows (banner `data_dir`; the ignored candidates were not created):

```
native-flag            data_dir=/tmp/rf-b2/flag-native
native-studio-env      data_dir=/tmp/rf-b2/studioenv-native
native-agent-env       data_dir=/tmp/rf-b2/agent-native/studio
native-xdg             data_dir=/tmp/rf-b2/xdg-native/kala-agent/studio
native-home            data_dir=/tmp/rf-b2/native-home/home/.local/share/kala-agent/studio
python-flag            data_dir=/tmp/rf-b2/flag-python
python-studio-env      data_dir=/tmp/rf-b2/studioenv-python
python-agent-env       data_dir=/tmp/rf-b2/agent-python/studio
python-xdg             data_dir=/tmp/rf-b2/xdg-python/kala-agent/studio
python-home            data_dir=/tmp/rf-b2/python-home/home/.local/share/kala-agent/studio
```

Smokes: `--data-dir <temp>` **and** a temporary `HOME`, and a new assertion
that the launch banner names that temporary directory (and never
`.local/share/kala-agent`). `packaging/studio_smoke.py` PASSes against both the
native release binary and `python -m kala_agent.studio`, with the temporary
`HOME` left empty afterwards:

```
[smoke] health ok: {..., "offline": true, ...}
[smoke] UI root and assets ok.
[smoke] preview mesh ok.
[smoke] STEP export: 5249 bytes, ISO-10303-21 magic ok.     (native)
[smoke] STEP export: 15518 bytes, ISO-10303-21 magic ok.    (python)
[smoke] PASS
home entries after: 0
```

`packaging/native_studio_smoke.py` cannot be run without a staged
`native-build.json` tree (owned by the packaging cell); its helpers were
exercised directly: `_minimum_environment` returns `HOME=<root>/home`,
`PATH=""`; `_check_banner_data_dir` accepts the temp banner line and raises
`SmokeFailure` for a banner naming `/home/someone/.local/share/kala-agent/studio`.

`studio/test/nativeBackendPackaging.test.ts` (sets only `KALA_STUDIO_DATA_DIR`)
becomes safe automatically; it was not edited.

---

## B3 (CLI part) — process-tree kill

### Change

`backend/crates/kala-studio/src/providers/cli_runtime.rs`:

* Every turn is spawned in its **own process group**: Unix
  `Command::process_group(0)` (pgid == child pid), Windows
  `CREATE_NEW_PROCESS_GROUP`.
* A `ProcessTree` guard is armed right after spawn. It kills the **whole
  group** on timeout, on an error, and — because dropping the request future
  drops the guard before `kill_on_drop` reaps the direct child — on a client
  disconnect or cancel.
* Unix kill: `rustix::process::kill_process_group(pid, SIGKILL)` then
  `kill_process(pid, SIGKILL)` as a backstop for a CLI that creates its own
  session. `rustix` is used instead of `libc` because the crate is
  `#![forbid(unsafe_code)]`; `Cargo.toml` gains a `cfg(unix)` dependency and
  `Cargo.lock` one edge (the crate was already in the lock).
* Windows kill: `taskkill /T /F /PID <pid>` (bounded 1 s), documented in the
  code; a Job Object with `KILL_ON_JOB_CLOSE` would be stronger and is what
  `kala-process` uses for sandboxed tool runs, but this crate has no Windows
  execution proof, so the documented OS tool is used.
* The private working directory is still a `tempfile::TempDir`; the timeout and
  overflow paths still reap the direct child.

`src/kala_agent/studio/providers/cli_runtime.py` mirrors it:
`start_new_session=True` (POSIX) / `CREATE_NEW_PROCESS_GROUP` (Windows),
`_kill_tree` (`killpg` / `taskkill /T /F`) used by `_terminate` and by the
output-overflow path. Honest boundary: the Python route is a synchronous
function in a thread-pool worker, so a client disconnect cannot cancel it (the
event loop cannot drop a thread); the native service is the shipped path and
the Python reference keeps the process-group kill on timeout and overflow.

### Proof

In-tree (both run and assert the grandchildren exist first, then that they are
gone within 2 s):
`a_dropped_turn_kills_the_whole_cli_process_tree`,
`a_timed_out_turn_kills_the_whole_cli_process_tree`
(`backend/crates/kala-studio/tests/cli_providers.rs`), and the Python mirror
`tests/unit/test_studio_cli_process_tree.py::test_a_timeout_kills_the_whole_cli_process_tree`.

The audit's experiment against the release binary — a fake `claude` that
starts two `sleep 300` children, records their pids and waits:

```
case=abort-final http=000 pids='3518525 3518526 ' survivors=0
  cli children while running: | 3518525 3518524 3518524 sleep | 3518526 3518524 3518524 sleep
  server:3518228 3485788 kala-studio
  temp dirs left in TMPDIR: 0
case=timeout-final http=504 pids='3536874 3536875 ' survivors=0
  cli children while running: | 3536874 3536873 3536873 sleep | 3536875 3536873 3536873 sleep
  server:3536561 3485788 kala-studio
  temp dirs left in TMPDIR: 0
```

The children's PGID (`3518524`, `3536873`) is their own group — the fake CLI's
pid — and differs from the server's PGID (`3485788`), so the kill targets a
dedicated group and cannot reach the server. `http=000` is `curl --max-time 0.5`
aborting the client; `http=504` is the bounded timeout.

Scope note: the CAD-build half of B3 (threading a cancellation token through
`native_geometry/mod.rs` + `service.rs` so an aborted `/design` stops building
and journaling) is **not** in this cell — `native_geometry/**` is explicitly
excluded from the ownership list.

---

## B4 — Windows `.cmd`/`.bat` shims

### Change

* `PathSearch` (extensions + `allow_bare`) and a pure
  `resolve_program_with(spec, environ, search, is_exec)`; the real
  `resolve_program` passes `PathSearch::for_current_platform` and
  `is_executable`. Windows: `PATHEXT` (default `.COM;.EXE;.BAT;.CMD`, parsed
  and normalised by `parse_pathext`) searched in order, and a **bare
  extension-less name is never chosen** — npm writes a POSIX helper named
  `codex` beside `codex.cmd`, and `cmd` must run the shim. Unix keeps its
  previous bare-then-`.exe` order. A `KALA_STUDIO_*_CLI_PATH` override also
  gets the extension appended.
* A resolved `.cmd`/`.bat` is run as `cmd.exe /d /s /c "<quoted line>"` with
  an argv array (`cmd_shim_argv`); quoting is the documented `cross-spawn`
  algorithm (backslash runs before quotes doubled, trailing backslash run
  doubled, wrap, `^`-escape cmd metacharacters), unit-tested with spaces,
  quotes, `&`, `|`, `%`, `^`, parentheses, `;`, `,`, `*`, `?`. Any control
  character (newline/CR/NUL) in the argv is refused with a typed failure,
  because it would end the `cmd.exe` line early. The prompt stays on standard
  input only.
* On a non-Windows host a resolved `.cmd`/`.bat` is refused with a clear
  message instead of `Exec format error`.

`src/kala_agent/studio/providers/cli_runtime.py` mirrors the resolver
(`PathSearch`, `parse_pathext`, `resolve_program_with`, `is_batch_shim`,
`cmd_shim_argv`, `_escape_cmd_argument`) and spawns a shim through
`cmd.exe /d /s /c`.

### Proof

Pure-function unit tests that run on Linux (Rust and Python, same vectors):
`pathext_defaults_and_normalises`,
`a_windows_search_prefers_the_cmd_shim_over_the_bare_file`,
`a_windows_search_keeps_pathext_order`,
`a_unix_search_accepts_a_bare_program_and_a_dot_exe`,
`an_override_wins_and_gets_the_extension_appended`,
`a_relative_or_missing_override_falls_through_to_path`,
`batch_shim_detection_is_case_insensitive`,
`cmd_quoting_neutralises_metacharacters`,
`cmd_shim_argv_is_an_argument_vector`,
`cmd_shim_argv_refuses_a_control_character`; Python counterparts in
`tests/unit/test_studio_cli_process_tree.py`.

`#[cfg(windows)]` tests (compiled only on Windows, unexecuted here):
`the_real_pathext_search_finds_cmd_exe` (real `PATH`/`PATHEXT`, expects
`cmd.exe`), and `a_cmd_shim_receives_the_argv_and_the_prompt_on_stdin` (a real
`.cmd` shim writes `%*` and copies stdin to files).

The Windows-only fragments (creation flag, `taskkill` spawn, `Stdio::null`,
`Child::try_wait`, tokio's `creation_flags`) were type-checked against
`x86_64-pc-windows-msvc` in an isolated scratch crate: `cargo check` finished
successfully. The full crate cannot be cross-checked on this host (ring's C
build needs a Windows SDK), so this is the honest limit of the verification.

---

## B13 — non-blocking provider probes

### Change

* `CLI_PROBE_TIMEOUT_SECONDS` 20 → **2** (per probe, whole verdict, and the
  probe's own child run). A CLI that does not answer leaves
  `installed: true, authenticated: false` with
  "installed but its availability check did not finish within 2 seconds" —
  never "unavailable" and never a guessed login.
* New route path `probe_subscription_clis_cached` (Rust and Python):
  serve the cached verdict immediately (even a stale one) and refresh in the
  background; a first probe that has not finished inside
  `PROBE_ROUTE_BUDGET_MILLIS`/`PROBE_ROUTE_BUDGET_SECONDS` (**600 ms**) is
  reported as `checking`: `configured: false`, **`available: null`**,
  `reason: "checking..."` (ASCII, so Rust and Python emit identical bytes).
  The cache and the in-flight set are keyed by `(provider id, resolved
  program)`, so replacing a CLI does not wait for the old refresh.
* `GET /providers`, `GET /settings` and `GET /health` use the cached path.
  The blocking `probe`/`probe_subscription_clis` forms remain for callers that
  want to wait (tests, future diagnostics).
* `CliProbe` gains `checking`; `ProviderRecord.available` is `null` while
  checking (the DTO already had `bool | None`), `reason` carries the honest
  state. Python `CliProbe` defaults `checking=False` so existing constructions
  keep working.

Files: `providers/cli_runtime.rs`, `providers/mod.rs` (record + cached seam),
`server.rs` (the three handlers only), `src/kala_agent/studio/providers/cli_runtime.py`,
`providers/dispatch.py` (`probe_subscription_clis_cached`, `_subscription_record`),
`src/kala_agent/studio/server.py` (the three handlers only).

### Proof

Rust: `a_cached_probe_never_waits_for_a_hanging_cli` (unit),
`a_hanging_cli_never_blocks_the_provider_or_settings_route` (integration:
`/providers`, `/settings`, `/health` each < 1 s, the row is `available: null`
with `reason: "checking..."`). Python: `tests/unit/test_studio_probe_latency.py`
(same assertions plus the background verdict after the cap).

Release binary with a hanging `claude --version` and a `codex` whose
`login status` hangs:

```
GET /providers (cold)        http=200   606 ms
  claude_code: configured=False available=None reason='checking...'
  codex_cli:   configured=False available=None reason='checking...'
GET /settings (same cache)   http=200   605 ms
GET /health (same cache)     http=200   616 ms
GET /providers (warm)        http=200     5 ms
  claude_code: installed but its availability check did not finish within 2 seconds
  codex_cli:   installed but its availability check did not finish within 2 seconds
  opencode_cli: was not found on PATH or at KALA_STUDIO_OPENCODE_CLI_PATH
surviving fake claude/codex probe processes: 0
```

Cold `/settings`-first and `/health`-first runs measure 607-617 ms as well.
Before this change the same fake set made `GET /providers` take 20.003 s
(audit evidence).

---

## B16 — honest `offline`

### Change

* `providers::any_remote_route_available(environ, stored, clis)` (Rust) /
  `_any_remote_route_available(env, clis)` (Python): a remote turn is *known*
  to be possible when an OpenAI-compatible credential is configured **or** a
  subscription CLI is installed and signed in. `any_remote_provider_configured`
  is kept as the API-key half and documents that.
* `/health` and `/settings` now compute `offline` from that function with the
  cached CLI verdicts. Consequently the field is never `true` while a usable
  CLI is on the host, and it stays conservative (`true`) only while a probe is
  still `checking` — at most the 600 ms route budget per cold request.
* Field contract documented in code (Rust `any_remote_route_available` and the
  `/health` handler comment; Python docstrings): `offline` = "no remote route
  is known to be available", not "no API key is configured".

UI (read-only note, `studio/**` not edited): `HealthResponse.offline` and
`StudioSettingsResponse.offline` are parsed in `studio/src/types/studio.ts` and
`studio/src/api/validators.ts` but not rendered anywhere; the only visible
"local · offline" badge comes from `ChatResponse.offline`
(`ChatPanel.tsx:636`), which was already honest for CLI turns. A UI change is
not required for B16, and the boot gate's `/providers` call now cannot fail on
a slow CLI.

### Proof

Rust `a_signed_in_cli_makes_the_studio_online` (no API key; signed-in fake
`claude`: `/health` and `/settings` report `offline: false`; the row is
`available: true`; without the fake the same studio reports `true`). Python
`tests/unit/test_studio_probe_latency.py::test_a_signed_in_cli_makes_the_studio_online`
mirrors it. End-to-end with the release binary and a signed-in fake CLI:
`/health {"offline": false}` in 27 ms.

---

## Gates

All commands ran with the temporary environment described at the top.

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | OK |
| `cargo clippy -p kala-studio --all-targets -- -D warnings` | OK |
| `cargo clippy -p kala-studio --all-targets --all-features -- -D warnings` | OK |
| `cargo test -p kala-studio --no-fail-fast` | **279 passed, 0 failed** (20 test binaries) |
| `pytest tests/unit -q` | **1653 passed, 0 failed** (52 s) |
| `python_differential.py` | **268 cases, 0 mismatches** |
| `worker_differential.py` | **292 cases, 0 mismatches** |
| `python_provider_dto_oracle.py` | **20 cases, 0 failures** |
| `cargo build --release -p kala-studio` | OK |
| `packaging/studio_smoke.py` (native + Python reference) | PASS |

`ruff check` over every touched Python file: clean.

Baseline comparison: before this cell the tree had 4 unrelated failing Python
unit tests (`test_compile_ocp.py::test_the_compile_budget_is_enforced`,
`test_templates.py::test_stepped_shaft_refuses_more_than_the_component_budget`,
two `test_executor_branches.py` cases) and one Rust failure
(`chat_mock_provider_design::a_design_longer_than_the_display_bound_still_builds`)
from the in-flight design-budget work; **all of them are green now** (the other
cell landed while this cell ran). One transient Rust failure was observed once
under full parallel load (`worker_lifecycle::the_worker_is_launched_with_the_resolved_step_import_limit`,
a Python-worker start race) and passed in isolation twice and in the final full
run; it does not touch the code this cell changed.

## Differential id sets

No case id was added, removed or renamed; the expected bodies did not change
(0/0/0 without regenerating anything). Fingerprints over the sorted ids:

| Set | Count | SHA-256 of the sorted ids |
|---|---|---|
| `python_differential.py` | 268 | `1c97fe873e4c82fedfc446cabce84849a7d8e84842355ed15688877ba5aefead` |
| `worker_differential.py` (268 shared + 21 `design.*`) | 289 ids / 292 comparisons | `943f6f5c9c02064a165209098181dd279f2aa86d0c4e8714f1d55b19b700d642` |
| `python_provider_dto_oracle.py` | 20 | `23622c10b4fbb1cbafff63666e67c1787775b812a0880e7c60de08e589822eb7` |

The request ids touched by this cell's change are exactly the ones that read
`offline` or a provider row: `settings.read`, `settings.read_stored`,
`settings.read_both`, `settings.read_cleared`, `settings.store.*`,
`settings.clear.openai`, `providers.configured`, `providers.cleared`,
`health.offline`, `read.health`, `read.providers`, `read.health_query`,
`read.trailing_slash*`, `method.*_health`, `host.*`; the probes find no CLI in
the differential environment (`PATH=""`), so their bodies are unchanged. The
`design.*` set is untouched.

## Digests

* Corpus (unchanged; no regeneration needed, manifest digests match the files):
  * `backend/fixtures/corpus/v1/studio_http.json` `dc4216ba3b6b6c2b8d96acc485a270b6a36794751d6008d5b3563a2edde7a233`
  * `backend/fixtures/corpus/v1/manifest.json` `c2db3bc3beaa0d00c58970b11872e4c60aa540d920e19b84481aedec1c120513` (recorded `corpus_sha256` `e86c7d93157fb844af0e4cf8711824df4e75a782ac9f88b94318d57f21df0f6b`)
  * `backend/fixtures/corpus/v1/canonical_json.json` `0c2a4bd95ce0bd7d0c0951382fc8d56aa46f9417d503bb3deb920daf28d90533`
* `backend/target/release/kala-studio` (built by this cell, default features):
  `be3be975cd5562f6ee373cb6365658b96bfd9acd4212ae5ede7acf162bc0d7c4`
  (11 680 384 bytes; rebuilt again by the packaging cell for the AppImage, so
  the digest is evidence for this build, not the shipped one).

## Files touched

Rust: `backend/crates/kala-studio/Cargo.toml`, `backend/Cargo.lock` (one dep
edge), `src/boundary.rs`, `src/providers/cli_runtime.rs`,
`src/providers/mod.rs`, `src/server.rs` (health/providers/settings handlers),
`tests/cli_launch.rs`, `tests/cli_providers.rs`.

Python/packaging/tests: `src/kala_agent/studio/settings.py`,
`src/kala_agent/studio/cli.py`, `src/kala_agent/studio/server.py`
(health/providers/settings handlers), `src/kala_agent/studio/providers/cli_runtime.py`,
`src/kala_agent/studio/providers/dispatch.py` (probe/record/offline),
`packaging/native_studio_smoke.py`, `packaging/studio_smoke.py`,
`tests/unit/test_studio_data_dir_precedence.py` (new),
`tests/unit/test_studio_probe_latency.py` (new),
`tests/unit/test_studio_cli_process_tree.py` (new),
`tests/unit/test_studio_server.py` (fixture `PATH=""` only; another cell edited
other parts of this file concurrently and its changes are intact),
`tests/unit/test_data_dir_isolation.py` (one test passes the HOME mapping).

Not touched: `providers/remote.rs`, `chat_design.rs`, `native_geometry/**`,
`kala-contracts`, `kala-design-intent`, `adapters/**`, `studio/**`, docs.

## Gaps and honest limits

1. **B3 CAD half is not done here** — the design/build cancellation token
   (`native_geometry/mod.rs`, `service.rs`) is outside this cell's ownership;
   a cancelled `/design` can still build and journal. The CLI half is complete.
2. **Windows is not executed.** The `.cmd` path and `taskkill` tree kill are
   type-checked only (scratch msvc crate); the `#[cfg(windows)]` tests are
   unexecuted. A Job Object with `KILL_ON_JOB_CLOSE` would be stronger than
   `taskkill /T /F`; it needs a Windows host to land safely.
3. **Provider docs are not updated** (`docs/studio-install.md` still shows
   `$XDG_DATA_HOME/kala-studio`, `docs/studio-providers.md` does not mention
   `PATHEXT`/`.cmd`, `docs/studio-limitations.md:519` still lists the
   `KALA_STUDIO_DATA_DIR` gap). Docs belong to the packaging cell / B8; the
   behaviour is documented in code and here.
4. **`offline` is a bool** and stays `true` while a first probe is checking, so
   for up to 600 ms a cold `/health` can still say "offline" with a usable CLI.
   Making that unknown explicit would need a DTO change (`offline: bool | null`)
   and a UI change; not attempted outside the DTO's existing conventions.
5. **The UI loads providers once at boot.** With a hung CLI the first render
   shows the honest `checking...` row and never refreshes until a reload; the
   verdict lands within 2 s in the background. A periodic/refresh-on-open
   provider fetch would close this, but `studio/**` is not this cell's.
6. `PATHEXT` is honoured as the OS defines it; an extension such as `.PS1`
   would be resolved but not runnable through `cmd.exe /c`. No shipped CLI
   needs it.
7. An invalid `KALA_AGENT_DATA_DIR` keeps its existing documented behaviour
   (temp fallback, not the next row), while an invalid `KALA_STUDIO_DATA_DIR`
   is skipped. Both are identical in Rust and Python and covered by tests; the
   asymmetry is intentional and documented in both resolvers.
8. `mypy` could not run in this environment: the installed `numpy` stub uses a
   `type` statement that mypy rejects under the configured `python_version=3.11`
   (`errors prevented further checking`). Pre-existing environment issue,
   unrelated to these files; `ruff` is clean.
9. `/tmp` cleanup: every temporary tree (`/tmp/rf-*`) was removed; the
   experiments' fake CLI processes were killed and verified gone.
