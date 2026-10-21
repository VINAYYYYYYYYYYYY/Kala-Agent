# design-timeout — a 120 s design-turn budget and journaling failed turns

Date: 2026-10-20
Cell: `design-timeout` (agent-providers wave)

## USER DATA

Every shell exported `RUSTUP_HOME=/home/v9i/.rustup` and
`CARGO_HOME=/home/v9i/.cargo`, then set fresh temporary `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME` before any
binary or Python entry point. No credential file and no settings file was read.
The live Studio on port 8831 was not stopped, not restarted and not used by any
test; every test used a mock transport and an ephemeral port. The user's real
data directory was never opened or written.

## Change 1: the design-turn remote timeout

`STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS = 120` was added next to the 30 s
constant in `remote.rs` and mirrored as
`STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS: Final[float] = 120.0` in
`openai_compat.py`. Both are re-exported the same way the 30 s constant is
(`providers/mod.rs` `pub use`, `dispatch.py` and `providers/__init__.py`
`__all__`).

The timeout is an explicit parameter, never request-controlled:

- Rust: `dispatch(..., timeout: Duration)` forwards to
  `openai_compat_chat(..., timeout: Duration)`, which sets
  `RemoteRequest.timeout`.
- `server.rs` `chat_route` passes `Duration::from_secs(120)` only inside the
  remote design branch (the `remote_design_turn` call closure), so the first
  call and the single repair call both get 120 s. The plain-chat branch passes
  30 s explicitly. The request body cannot choose either.
- Python: `openai_compat_chat(..., timeout: float = 30.0)`; `dispatch._remote_turn`
  passes `STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS` to the design prompt and the
  repair prompt, and leaves the plain path on the provider default.

CLI providers (`claude_cli`, `codex_cli`, `opencode_cli`) were not touched and
keep their own 180 s budget; `timeout` is unused on those arms.

The module doc in `remote.rs` and the `openai_compat.py` docstring now state
30 s for a plain chat and 120 s for a design turn.

### Reqwest override evidence

The dependency is reqwest 0.12.28 (`Cargo.toml` `reqwest = "0.12"`).
`build_client` keeps a 30 s client-level timeout and each POST sets
`.timeout(request.timeout)`.

- `~/.cargo/registry/src/*/reqwest-0.12.28/src/async_impl/request.rs:286-295`
  documents `RequestBuilder::timeout`: "It affects only this request and
  overrides the timeout configured using `ClientBuilder::timeout()`."
- `.../reqwest-0.12.28/src/config.rs:66-79` (`RequestConfig::fetch`) returns the
  request-scoped value when present, otherwise the client value.
- `.../reqwest-0.12.28/src/async_impl/client.rs:2629` builds the total timeout
  with `self.inner.total_timeout.fetch(&extensions)`, i.e. the same
  request-first lookup.

The override is also proven at runtime by
`providers::remote::tests::per_request_timeout_overrides_the_client_level_timeout`
(a loopback server replies after 150 ms; the client default is 50 ms and the
request budget is 5 s; the response succeeds).

## Change 2: journal the failed design turn

When a remote design turn fails with `DesignTurnError::Provider`, `server.rs`
appends one `chat.turn` event before returning the unchanged
`provider_failure(failure)` envelope. The payload is the successful remote
design payload (`provider`, `model`, `offline`, `prompt_sha256`, `prompt_chars`,
`design_id: null`, `design_requested: true`, `cad_provider`, `missing: []`)
plus `"failure": "<envelope code>"`. No prompt text, key or endpoint is stored,
and the successful payload is unchanged because `failure` is only inserted
when it is `Some`. A journal append failure is logged and never masks the
provider error.

`chat_design.rs` `append_chat_turn` gained `failure: Option<&str>` and now
takes `model: Option<&str>` so a missing requested model journals as `null`,
matching Python.

Python `server.py` mirrors it in the chat path with
`_journal_failed_design_turn`: it journals only a non-local remote design turn
(`provider in {openai, openrouter}` and `cad_provider` set or a template family
detected), using the same payload plus `failure`, and swallows a broken journal
with a log line. The HTTP status, code, message and details are produced by the
same `_provider_*_response` functions as before.

## Files changed (this cell)

Rust product:

- `backend/crates/kala-studio/src/providers/remote.rs` — doc comment, new
  constant, `timeout: Duration` parameter, doc, client-relative timeout
  comments, 2 new unit tests.
- `backend/crates/kala-studio/src/providers/mod.rs` — re-export of the new
  constant and `timeout: Duration` on `dispatch` (`#[allow(too_many_arguments)]`).
- `backend/crates/kala-studio/src/server.rs` — imports, 120 s in the design
  closure, 30 s in the plain branch, failed-turn journal append,
  `provider_failure_code`, success append now passes `Some(model)`.
- `backend/crates/kala-studio/src/chat_design.rs` — `append_chat_turn` accepts
  `failure` and an optional model (only this function; the rest of the file is
  the concurrent llm-design cell's work).
- `backend/crates/kala-studio/tests/chat_mock_provider_design.rs` — scripted
  transport can now fail with a timeout; 2 new server-level tests.

Python dev reference:

- `src/kala_agent/studio/providers/openai_compat.py` — constant, docstring,
  `timeout` parameter, `__all__`.
- `src/kala_agent/studio/providers/dispatch.py` — import/export the constant and
  pass it on both design calls (`__all__`).
- `src/kala_agent/studio/providers/__init__.py` — import/export the constant.
- `src/kala_agent/studio/server.py` — `hashlib`, `JournalEventType`,
  `detect_design_family`, failed-turn journal helper, calls in the six provider
  exception handlers (chat path only).
- `tests/unit/studio_design/test_design_api.py` — fake providers accept the
  `timeout` keyword; 3 new tests.

No `studio/**`, adapters, another cell's source, or a credential/settings file
was edited. No commit was made.

## Tests and gates

- `cargo fmt --check`: **clean**.
- `cargo clippy -p kala-studio --all-targets --all-features -- -D warnings`:
  **clean**.
  The brief's literal command without `--all-features` fails on a **pre-existing**
  issue outside this cell: `tests/geometry_exports.rs:18` imports
  `build_registry`, whose only use is under `#[cfg(feature = "native-step")]`,
  while the default features stop at `native-primitives`. The import is
  present at `HEAD`; `--all-features` makes it used and is the gate the
  repository README documents.
- `cargo test -p kala-studio`: **226 passed, 0 failed, 0 ignored** (125 lib +
  101 integration across 19 binaries). Four of those are this cell's:
  - `providers::remote::tests::design_turn_requests_carry_120_seconds_plain_chat_carries_30`
  - `providers::remote::tests::per_request_timeout_overrides_the_client_level_timeout`
  - `chat_mock_provider_design::design_turns_and_their_repair_carry_the_120_second_budget`
  - `chat_mock_provider_design::a_timed_out_design_turn_is_journalled_once_with_its_code`
  The server-level test asserts the exact 504 envelope
  `{"code":"provider_timeout","details":["the provider request exceeded its
  bounded timeout"],"message":"the chat provider exceeded its bounded timeout"}`
  and exactly one journal `chat.turn` with `design_requested: true`,
  `design_id: null`, `cad_provider: "opencascade"`, `missing: []`,
  `failure: "provider_timeout"`, a 64-char `prompt_sha256`, the prompt length,
  and no `prompt` key.
- `pytest tests/unit -q`: **1570 passed, 18 warnings** (was 1567 before this
  cell; the 3 new tests are the design timeout, the failed-turn journal, and
  the broken-journal no-mask case). `ruff check` and `ruff format --check` are
  clean on every changed Python file.
- `cargo build --release -p kala-studio`: **rebuilt** (11565152 bytes,
  2026-10-20 17:33). The live process on 8831 kept running.

## Differential counts

Commands from `backend/crates/kala-studio/README.md` (the files are under
`backend/crates/kala-studio/tests/`, not `backend/compat/`):

| Differential | Before | After |
|---|---|---|
| `tests/python_differential.py` | 268 cases, **47 mismatches** | 268 cases, **47 mismatches** |
| `tests/python_provider_dto_oracle.py` | 20 cases, **0 failures** | 20 cases, **0 failures** |

The mismatch **set is byte-for-byte identical** before and after (verified by
diffing the extracted case ids). All 47 are pre-existing and unrelated to this
cell: the Python reference still publishes the four-row provider list and the
old Codex wording while the native binary already carries the wave's
`claude_code`/`codex_cli`/`opencode_cli` rows and the `codex_cli` refusal text.
Categories: 33 `settings.*`, 2 `providers.*`, 3 `read.*`, 5 `origin.*`,
2 `host.*`, 1 `health.*`, 1 `chat.codex_unavailable`. That is the
`cli-providers` cell's Python-parity gap, not a regression here: no case in
either differential exercises the design-turn timeout or a provider-failure
journal, and the design/chat contracts this cell touches compare equal.

## Gaps / notes for the conductor

- The Python dev reference has no CLI provider implementations, so a failed
  design turn there is only possible through `openai`/`openrouter` (the codes
  this cell journals). The Rust design branch also journals a failed CLI design
  turn (for example `provider_subprocess_error`); Python cannot reach that path
  because its `codex` route has no design hook. This is an existing dispatch
  difference, not a Rust/Python contract divergence in the OpenAI-compatible
  route.
- The literal `cargo clippy --all-targets` (without `--all-features`) gate
  cannot be green until the pre-existing `geometry_exports.rs` feature-gated
  import is fixed by its owner; `--all-features` is clean.
- The design budget stays server-chosen: there is no request field, environment
  variable or stored setting for it. A future operator knob would need a new
  ADR-level decision.
- No live model call was made by this worker. The transport and server tests
  use a scripted/timeout transport only; the conductor should rerun the
  OpenRouter `minimax/minimax-m3.1-flash:free` flange prompt against the
  rebuilt binary to confirm the 120 s budget in the field.
- Linux only: the Windows target was not rebuilt or re-tested in this cell.
