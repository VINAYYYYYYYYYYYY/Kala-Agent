# Cell python-parity: the Python reference learns the three subscription CLI providers

Read first: `../../WAVE.md`, `../../../rust-backend-20261019/cells/CONVENTIONS.md` (the USER DATA rule: in every shell, `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo`, then set a temp `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`; never touch `/home/v9i/.local/share/kala-agent/**`; NEVER read credential, auth or settings files of any CLI (`~/.claude*`, `~/.codex*`, `~/.config/opencode*`, `~/.local/share/opencode*`, …)), and `../cli-providers/REPORT.md` (the Rust contract: provider ids, names, notes, argv, env allowlist, budget, consent, availability probing, error wording).

## Why
The native Rust Studio lists `claude_code`, `codex_cli` and `opencode_cli`. The dev-only Python reference (`src/kala_agent/studio`) still lists only `local`, `openai`, `openrouter` and `codex`. Every differential case that echoes the provider list therefore mismatches: `backend/compat/python_differential.py` has 47 mismatches (33 settings.*, 2 providers.*, 3 read.*, 5 origin.*, 2 host.*, 1 health.*, 1 chat.codex_unavailable), and `backend/crates/kala-studio/tests/worker_differential.py` has 57 (43 settings.*, the rest the same). That noise hides real regressions. The goal is 0 and 0 again.

## Build
Port the three CLI providers into the Python reference so it matches the Rust contract byte for byte where the differentials compare: the provider listing (ids, names, notes, `configured`), the settings and health bodies, availability probing, `allow_remote` consent, the error envelopes and wording, the argv exactly as in the cli-providers report, prompt on stdin, the env allowlist, the 180 s budget, the private temp working dir, and output parsing (text + model). Mirror the Rust structure as `providers/claude_cli.py`, `codex_cli.py` and `opencode_cli.py` plus a shared runtime, registered in `dispatch.py`. Read the Rust sources in `backend/crates/kala-studio/src/providers/{claude_cli,codex_cli,opencode_cli,cli_runtime}.rs` and `settings.rs` as the spec. Also fix `chat.codex_unavailable` (find what differs). Python design turns through a CLI provider should work like Rust's (the remote design-turn path accepts them in Rust), if that is a small extension of the existing Python remote design path. Otherwise report it as a gap.

## Tests
Unit tests with a fake executable on a temp PATH (no real CLI, no network, no credential): argv, stdin, env allowlist, budget/timeout, consent refusal, parse success and failure, availability true/false. Never invoke the real `claude`, `codex` or `opencode` in tests.

## Gates
- `pytest tests/unit -q` (counts); `ruff check` / `ruff format --check` on the touched files.
- `python_differential.py` and `worker_differential.py`: before (expected 47 / 57) and after (target 0 / 0), with the commands from `backend/compat/README.md`. Report any residual mismatch with its id and reason. `python_provider_dto_oracle.py` stays 0.

## Ownership
Yours: `src/kala_agent/studio/providers/**`, `src/kala_agent/studio/{settings,schemas,server}.py` (the provider and settings parts) and their tests. NOT yours: anything Rust, `src/kala_agent/studio/design/**`, `adapters/**` (the polish cell owns some of those now). A live Studio runs on port 8831: do not stop it, and do not use that port. Do not commit.

## REPORT
Write `REPORT.md` here: files, the differential counts before and after with the residual ids, the test counts, and gaps.
