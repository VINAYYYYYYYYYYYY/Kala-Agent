# python-parity: the Python reference learns the three subscription CLI providers

**Cell:** `python-parity` (wave `agent-providers-20261020`, 2026-10-20)
**Product side:** `src/kala_agent/studio/**` (the dev-only Python Studio reference)
**Reference side:** `backend/crates/kala-studio/src/providers/{cli_runtime,claude_cli,codex_cli,opencode_cli}.rs`
**Status:** implemented, tested with fake CLIs only. No real CLI was ever started.

> **USER DATA rule.** Every shell session in this cell began with the four
> exports the rule demands (`RUSTUP_HOME=/home/v9i/.rustup`,
> `CARGO_HOME=/home/v9i/.cargo`, and `HOME`, `KALA_AGENT_DATA_DIR`,
> `KALA_STUDIO_DATA_DIR`, `XDG_DATA_HOME` pointing at fresh `mktemp -d`
> directories), and every Python entry point ran with those temporary
> directories in place. **`/home/v9i/.local/share/kala-agent/**` was never
> listed, read, written or touched.** No credential, auth or settings file
> of any CLI was read: the availability probe uses only the **exit status**
> of each CLI's own status command, and the CLI's own output never leaves
> the probe. The live Studio on port 8831 was neither stopped nor used; the
> differentials bind their own free port.

---

## 1. What this cell delivers

The Python reference now publishes and runs the same seven provider ids as
the native product, with the same rows, the same notes, the same probe
verdict, the same error envelopes and the same subprocess contract.

| Provider id | CLI | Binary | Path override | Probe |
|---|---|---|---|---|
| `claude_code` | Claude Code | `claude` | `KALA_STUDIO_CLAUDE_CLI_PATH` | `--version`, `auth status` |
| `codex_cli` (+ the historical alias `codex`) | Codex | `codex` | `KALA_STUDIO_CODEX_CLI_PATH` | `--version`, `login status` |
| `opencode_cli` | OpenCode | `opencode` | `KALA_STUDIO_OPENCODE_CLI_PATH` | `--version`, `auth list --format json` |

The provider list is now, in this order:
`local, openai, openrouter, codex, claude_code, codex_cli, opencode_cli`.

### The exact argv (identical to the Rust product)

```text
claude  --print --output-format json --safe-mode --tools "" \
        --strict-mcp-config --permission-mode dontAsk \
        --no-session-persistence [--model <model>]

codex   exec --sandbox read-only --skip-git-repo-check --ephemeral \
        --ignore-user-config --json [--model <model>]

opencode run --standalone --format json --agent kala-chat [--model <provider/model>]
```

The prompt is **never** on the argv: it is written to the child's standard
input, which is closed afterwards. The alias `default` is never passed as
`--model`.

### The safety contract (mirrored from the Rust product)

| Property | How it is enforced in the Python reference |
|---|---|
| no shell | `subprocess.Popen` with an argv list, `shell=False` |
| the prompt never on argv | the payload goes to the child's stdin, then the pipe is closed |
| no tool use | `claude --tools ""`; the OpenCode deny-all agent; the Codex read-only sandbox (honest limit below) |
| private working directory | a fresh `tempfile.TemporaryDirectory()` per turn, deleted with the turn |
| bounded wall time | 180 s by default, `KALA_STUDIO_CLI_TIMEOUT_SECONDS` clamped to 1..=3600; an overrunning child is killed and reaped |
| bounded output | stdout <= 1,000,000 bytes, stderr <= 65,536 bytes; a larger output kills the child and fails the turn |
| sanitized environment | the child starts from an empty environment and receives only `PATH`, `HOME`, the `XDG_*` names, `TMPDIR`/`TMP`/`TEMP`, `LANG`/`LC_ALL`/`LC_CTYPE`, `TERM`, plus `CLAUDE_CONFIG_DIR` (Claude) or `CODEX_HOME` (Codex) |
| no Kala secret reaches the CLI | no `OPENAI_API_KEY`, no `OPENROUTER_API_KEY`, no studio token, no data directory |
| no credential of the operator is read by Kala | the probe reads only the exit status of the status command |
| consent | the four subscription ids require `allow_remote=true`; the refusal is `provider=<id> requires allow_remote=true at request time` and **the CLI is never started** |

---

## 2. Changed paths

### New

| Path | What it is |
|---|---|
| `src/kala_agent/studio/providers/cli_runtime.py` | the shared subprocess machinery: discovery, the sanitized environment, the bounded runner, the ETXTBSY start retry, the probe and its 120 s cache |
| `src/kala_agent/studio/providers/claude_cli.py` | the Claude Code route: argv, the JSON result parser, the failure wording |
| `src/kala_agent/studio/providers/codex_cli.py` | the Codex route: argv, the JSONL event parser, the failure wording |
| `src/kala_agent/studio/providers/opencode_cli.py` | the OpenCode route: argv, the deny-all sandbox document, the JSONL event parser |
| `tests/unit/test_studio_cli_providers.py` | 40 tests with a fake executable on a temporary `PATH` |
| `tests/unit/conftest.py` | the unit-suite guard that keeps the real subscription CLIs from ever being started |

### Modified

| Path | Edit |
|---|---|
| `src/kala_agent/studio/providers/dispatch.py` | the seven published ids, the three probe specs, the four subscription rows (with `available`/`reason`/`source`), the consent gate, the generalised remote turn and the new `provider_call` seam |
| `src/kala_agent/studio/providers/__init__.py` | the new exports and the honest module docstring |
| `src/kala_agent/studio/schemas.py` | `ProviderRecord` gained `available`/`reason`/`source`; `CredentialSource` gained `subscription`; `ProviderCredentialStatus.provider` is now a bounded string (the API-key rows are unchanged, so the settings route still refuses `provider=codex` with the same 422) |
| `src/kala_agent/studio/settings.py` | `describe_provider_credentials` publishes the four subscription rows (`env_var: ""`, `source: "subscription"`) after the two API-key rows, so the stored-key indices do not move |
| `src/kala_agent/studio/server.py` | the health route publishes the constant id list (it probes nothing, like the Rust); the provider and settings routes probe once and pass the verdicts to both bodies; the build-repair call goes through the same provider seam |
| `tests/unit/test_studio_providers.py` | the fail-closed codex stub tests are replaced by the subscription-route contract |
| `tests/unit/test_studio_server.py` | the provider-list and settings-body expectations follow the seven rows |

### Deleted

| Path | Why |
|---|---|
| `src/kala_agent/studio/providers/codex.py` | the fail-closed stub contradicts the new contract; the Rust product removed the same stub and its message. `codex_cli.py` replaces it. |

### Not touched (other cells' ownership)

No Rust file, no `src/kala_agent/studio/design/**`, no `adapters/**`, no
`studio/src/**`, no `backend/**` was edited by this cell. The live Studio
on port 8831 was neither stopped nor used. Nothing was committed.

---

## 3. The differential results

### Before (this cell's starting point)

| Oracle | Cases | Mismatches |
|---|---|---|
| `backend/crates/kala-studio/tests/python_differential.py` | 268 | **47** |
| `backend/crates/kala-studio/tests/worker_differential.py` | 292 | **57** (0 known gaps) |

The 47 residual ids (the same 47 in both oracles, plus the ten
`settings.invalid_utf8_*` / `settings.integer_limit_*` ids that only the
worker oracle replays):

```text
chat.codex_unavailable
health.offline, read.health, read.health_query
host.ipv6, host.uppercase
origin.empty_port, origin.ipv6, origin.same, origin.same_port, origin.tab_port
providers.cleared, providers.configured, read.providers
settings.read, settings.read_both, settings.read_cleared, settings.read_stored,
settings.store.openai, settings.store.openrouter, settings.clear.openai,
settings.key_1c_spaces, settings.large_read, settings.large_clear,
settings.large_read_cleared,
settings.surrogate_read, settings.surrogate_health, settings.surrogate_providers,
settings.surrogate_write, settings.surrogate_read_after_write,
settings.surrogate_clear, settings.surrogate_read_cleared,
settings.dup_surrogate_read, settings.dup_surrogate_health,
settings.dup_surrogate_providers, settings.dup_surrogate_write,
settings.dup_surrogate_read_after_write, settings.dup_surrogate_clear,
settings.dup_surrogate_read_cleared,
settings.mixed_valid_read, settings.mixed_valid_write,
settings.mixed_valid_read_after_write, settings.mixed_valid_clear,
settings.mixed_valid_read_cleared,
settings.at_limit_read, settings.at_limit_clear, settings.at_limit_read_cleared
settings.invalid_utf8_read, settings.invalid_utf8_health,
settings.invalid_utf8_providers, settings.invalid_utf8_write,
settings.invalid_utf8_read_again,
settings.integer_limit_read, settings.integer_limit_health,
settings.integer_limit_providers, settings.integer_limit_write,
settings.integer_limit_read_again        (worker oracle only)
```

Every one of them had the **same single cause**: the Python reference
published four provider rows without the probe fields and four
subscription credential rows, and the health document carried four
provider ids instead of seven.

### After

| Oracle | Cases | Mismatches | Known gaps |
|---|---|---|---|
| `python_differential.py` | 268 | **0** | - |
| `worker_differential.py` | 292 | **0** | **0** |
| `python_provider_dto_oracle.py` | 20 | **0 failures** | - |

**Residual mismatch ids: none.** The provider list, the settings body and
the health document are byte-for-byte equal to the native product's
(verified separately by comparing the two bodies directly: 3,455 bytes for
`/api/studio/providers`, 3,987 bytes for `/api/studio/settings`).

### What `chat.codex_unavailable` was

The Python reference answered with its own fail-closed stub:

```text
503 {"code":"provider_unavailable","details":["provider=codex is not available
in this studio build; the verified Codex CLI subprocess integration is not
shipped yet, so no request is sent. Use provider=openai or provider=op..."]}
```

The native product answers with the honest reason the CLI is missing:

```text
503 {"code":"provider_unavailable","details":["the Codex CLI was not found on
PATH or at KALA_STUDIO_CODEX_CLI_PATH"],"message":"the chat provider is not
available in this studio build"}
```

The Python reference now answers the same 503 with the same wording,
because the route really tries the CLI and reports that it is not
installed.

---

## 4. The tests

| Suite | Result |
|---|---|
| `pytest tests/unit -q` | **1619 passed**, 18 warnings, 0 failed |
| `tests/unit/test_studio_cli_providers.py` (new) | **40 tests** |
| `tests/unit/test_studio_providers.py` | **27 tests** |
| `tests/unit/test_studio_server.py` | **31 tests** |
| `ruff check` (13 touched files) | clean |
| `ruff format --check` (13 touched files) | 13 files already formatted |
| `mypy` (the new provider modules and `settings.py`) | no issues |

Every CLI test drives a **fake executable** written into a temporary
directory: a small Python program with the repository interpreter in its
shebang that records its own argv, its standard input, its environment and
its working directory before it answers. It is reached through the
documented path-override variable or a temporary `PATH`. **No test ever
starts the real `claude`, `codex` or `opencode`, opens a network
connection, or reads a credential, an auth file or a settings file.**

What the 40 new tests cover:

* the exact argv of the three routes, with and without a model, and with
  the `default` alias;
* the prompt on standard input and **not** on the argv;
* the sanitized environment: the allowlist reaches the child, no Kala
  secret does, and the one CLI-specific login variable does;
* the budget: the default, the clamp, the fallback for an unreadable
  value, and a child that overruns (killed, reported as a timeout);
* the consent refusal for all four ids, with the CLI never started, and
  the historical `codex` alias keeping its own wording;
* the failure paths: a missing binary, a non-zero exit with the CLI's own
  bounded text, a failure the CLI prints as its JSON result, an error
  event in the JSONL stream, a reply that is not JSON, a reply with no
  event, and output beyond the bound;
* the probe: a missing CLI (nothing is run), the version and the login
  state, a CLI that is not signed in, the verdict cache, and the proof
  that the probe never echoes the CLI's own text (an account name never
  reaches the response);
* the provider list: the seven ids, the order, the `subscription (CLI)`
  source, the probe verdict, and the proof that the rows never disclose
  the local CLI path;
* the private working directory (the CLI's cwd is a temporary directory
  that is gone with the turn);
* a design turn through a subscription CLI: the DesignIntent instruction
  and the user's request travel to the CLI on stdin, exactly like the
  API-key routes.

`tests/unit/conftest.py` adds one autouse guard: in the whole unit suite
the availability probe is replaced by an honest "not installed" verdict, so
a polled provider list on a machine that happens to have `claude` or
`codex` installed can never launch it. The dedicated CLI module opts out
with `ALLOW_FAKE_CLI_PROBE = True` because it drives a fake executable.

---

## 5. The design turn through a subscription CLI

The brief asked for this "if that is a small extension of the existing
Python remote design path". It is, and it is done: the remote design turn
now takes a **provider call** instead of a hard-wired `openai_compat_chat`
call, and the four subscription routes pass their own CLI call. The design
instruction, the DesignIntent JSON Schema and the user's request travel to
the CLI on stdin; the reply is parsed and validated exactly like an
API-key reply, with the same one repair call and the same honest "no part
was built" wording. The build-repair call (a build that did not verify, or
a builder that refused the script) goes through the same seam, so a CLI
provider can repair its own design too.

The one difference from the Rust product is the *source* of the design
prompt: the Rust front door fetches the schema from the worker
(`intent/schema`), while the Python reference builds it in-process from
`design_intent_schema_text()`. That difference is the llm-design cell's
domain and predates this cell.

---

## 6. Gaps and honest limits

1. **The corpus manifest's source pin is stale.**
   `backend/compat/generate_corpus.py --check` now fails on **one** file:
   `manifest.json`, and only in `source_state.python_sources` and
   `source_state.git` — the digests of the 95 product Python files, which
   this cell changed. The corpus **content** fixture
   (`backend/fixtures/corpus/v1/studio_http.json`) already matches: the
   concurrent `polish` cell regenerated the corpus at 18:45 and its
   `backend/fixtures/README.md` records that the run "absorbed the
   concurrent `python-parity` cell's provider-list rows and Codex wording".
   The regeneration of the manifest belongs to the `compat-drift` /
   `polish` cell, which owns `backend/fixtures/**`; this cell did not
   write there.

2. **The Codex CLI keeps its shell tool.** `--sandbox read-only` plus
   `--ignore-user-config` is the strongest posture the CLI offers; a
   read-only command the model chooses to run can still read a file it can
   see. Identical to the native product's honest limit.

3. **Environment-var authentication is not forwarded.** The child receives
   only the allowlist, so a CLI the operator authenticated through an
   environment variable reports itself as signed out. The subscription
   logins (OAuth in `~/.claude`, the keychain, `$CODEX_HOME/auth.json`,
   the OpenCode credential store) are found normally, because `HOME` and
   `XDG_*` are forwarded. Forwarding an API key would have meant Kala
   handling the operator's credential, which the brief forbids. Identical
   to the native product.

4. **The probe reports "signed in" from an exit status.** The CLI's own
   account text is never read, so the provider row can say *that* the CLI
   is signed in but never *as whom*.

5. **A CLI that exits non-zero is a 502** (`provider_subprocess_error`),
   even when the reason is "not logged in"; the CLI's own bounded text is
   in the response's `details`. A CLI that produces more output than the
   bound is the native product's *internal* failure (a sanitized 500),
   because the Python reference raises `TooLargeResponseError`, which the
   route does not catch.

6. **Windows.** The discovery code looks for `<name>` and `<name>.exe` on
   `PATH`, and the executable check is POSIX-mode based. Nothing in this
   cell was exercised on Windows, and the 40 new CLI tests are skipped
   there (`pytestmark`): their fake executable is a POSIX program.

7. **The reference is synchronous.** The Rust front door is async; the
   Python reference blocks the request thread while the CLI runs. The three
   probes do run concurrently (a `ThreadPoolExecutor`), exactly like the
   Rust, and the verdict is cached for 120 s, so a polled provider list
   does not launch the CLI on every poll.

8. **The `codex` id is an alias, not a separate row.** It publishes its
   own provider row and its own credential row (both naming the Codex CLI),
   and its chat route runs the same CLI, but it never appears in the
   settings route's *key* rows: `POST /api/studio/settings` still refuses
   `provider=codex` with the same 422 as before, because a subscription
   credential is never stored in Kala.

9. **Two pre-existing `mypy` errors** in
   `src/kala_agent/studio/server.py` (`turn.design_intent` is
   `DesignIntent | None` at the two call sites in
   `_build_and_maybe_repair`) come from the concurrent `llm-design` cell's
   code in a region this cell does not own. They are not caused by this
   cell and were left alone.

10. **No live CLI chat was performed.** The three CLIs were never started
    on this host: the reference was exercised only with fake executables,
    and the differentials run with an empty `PATH` on both sides, so both
    report the CLIs as not installed. A live turn still needs the operator's
    signed-in CLI and `allow_remote: true`.
