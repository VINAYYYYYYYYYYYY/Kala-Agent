# Cell design-timeout: a longer remote budget for design turns, plus journaling failed turns

Read first: `../../WAVE.md`, `../../../rust-backend-20261019/cells/CONVENTIONS.md` (the USER DATA rule: in every shell, `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo`, then set a temp `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`; never touch `/home/v9i/.local/share/kala-agent/**`; never read credentials), and `../llm-design/REPORT.md`.

## Opus finding (live, 2026-10-20)
OpenRouter `minimax/minimax-m3.1-flash:free`, cad_provider=freecad, prompt "round steel flange 100 mm OD, 12 mm thick, Ø30 bore, six Ø8 holes on an 80 mm PCD". Run twice, it failed both times with `provider_timeout` ("the chat provider exceeded its bounded timeout"). The second run stopped at exactly 30 s, so the FIRST model call alone exceeded `STUDIO_REQUEST_TIMEOUT_SECONDS = 30` (`kala-studio/src/providers/remote.rs:26`). The simpler 80×40×8 plate prompt succeeded in time. Free and reasoning models need more than 30 s to author a non-template ConstructionScript; the CLI providers already get 180 s. The failed turn also left NO event in the session journal.
The UI's `postChat` (`studio/src/api/client.ts:377`) has no client timeout, so this is backend only.

## Change 1: design-turn remote timeout
- Add `pub const STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS: u64 = 120;` next to the 30 s constant in `remote.rs`, and the Python mirror `STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS: Final[float] = 120.0` in `src/kala_agent/studio/providers/openai_compat.py`. Export it the same way the 30 s constant is exported (`providers/__init__.py`, `dispatch.py` `__all__`).
- Use 120 s ONLY for the remote OpenAI-compatible calls made inside a design turn: the `remote_design_turn` call closure in `server.rs` (~line 1495), which covers both the first call and the single repair call. Plain chat keeps 30 s exactly, so the existing tests and differentials stay unchanged. Thread it as an explicit parameter (for example a `timeout: Duration` argument, or a small options struct on `openai_compat_chat`/`dispatch`), not a global. Keep "never configurable at request time": the client cannot choose it, only the server's code path does.
- reqwest: `build_client` sets a client-level 30 s timeout and the request sets `.timeout(request.timeout)`. Confirm from the reqwest docs or source in `~/.cargo/registry` that the per-request timeout overrides the client one. If it does not, raise the client-level timeout to the maximum (120 s) and keep the per-request value authoritative. Prove it with a test.
- CLI subscription providers (`claude_cli`, `codex_cli`, `opencode_cli`) keep their own 180 s budget. Do not touch them.
- Update the module doc comment in `remote.rs` (it says "the request timeout is 30 seconds") and the matching docstring in `openai_compat.py`.
- Python reference: mirror the same behaviour in `dispatch.py` and `server.py`'s design-turn path so the Python and Rust implementations agree.

## Change 2: journal the failed design turn
When a remote design turn fails with a provider failure (timeout, network, protocol, configuration), append one `chat.turn` event before returning the error envelope. Use the same payload shape the successful remote design turn writes, with `design_requested: true` and `design_id: null`, plus one added field `"failure": "<error code>"` (for example `provider_timeout`, exactly the envelope `code`). No prompt text, no key, no endpoint: the existing payload already stores only `prompt_chars` and `prompt_sha256`. A journal append failure must not mask the provider error. Mirror this in Python. The HTTP response (status, code, message) must stay byte-identical to today.

## Tests (the spec)
- Rust: a FakeTransport test proving a design-turn remote request carries `timeout == 120 s`, and a plain-chat request still carries 30 s, including the repair call. A server-level test (pattern from `tests/chat_and_remote.rs` / `tests/chat_mock_provider_design.rs`): with a timing-out transport and cad_provider set, the response is the same `provider_timeout` envelope AND the journal gains exactly one `chat.turn` with `failure: "provider_timeout"`.
- Python: the same two properties in `tests/unit/studio_design/test_design_api.py` (or the nearest existing provider test).

## Gates
- `cargo fmt --check`, `cargo clippy -p kala-studio --all-targets -- -D warnings`, `cargo test -p kala-studio` (all green; report counts).
- `pytest tests/unit -q` (report counts).
- The live differentials must not regress: `backend/compat/python_differential.py` and `python_provider_dto_oracle.py` with the commands in `backend/compat/README.md`. Report mismatch counts before and after. Any new mismatch must be explained and justified, and none is expected.
- Rebuild the release binary: `cargo build --release -p kala-studio`.

## Ownership
`kala-studio/src/providers/remote.rs`, `providers/mod.rs` (dispatch signature only), `server.rs` (the chat design-turn branch only), `chat_design.rs` (only if the journal append lives there), the matching tests, `src/kala_agent/studio/providers/{openai_compat,dispatch,__init__}.py`, `src/kala_agent/studio/server.py` (chat path only), and the matching Python tests. Re-read each shared file immediately before patching it: other uncommitted work is in the tree. Do not commit.

## REPORT
Write `REPORT.md` here with the diff summary per file, the test and gate counts, the differential counts, the reqwest override evidence, and any gap.
