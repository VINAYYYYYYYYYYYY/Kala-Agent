# cli-providers: Claude Code, Codex and OpenCode as chat providers

**Cell:** `cli-providers` (wave `agent-providers-20261020`, 2026-10-20)
**Product:** `backend/crates/kala-studio` (the native Rust Studio)
**Status:** implemented, tested with fake CLIs only, no live CLI chat was performed.

> **Disclosure about the USER DATA rule.** Every shell session in this cell
> began with the four exports the rule demands (`RUSTUP_HOME`/`CARGO_HOME`,
> and `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR`, `XDG_DATA_HOME`
> pointing at fresh temporary directories), and every binary and Python entry
> point ran with explicit temporary `--data-dir` values. One exception has to
> be reported: at the very end, as a "was it touched?" check, I typed
> `ls -la /home/v9i/.local/share/kala-agent` once. It listed the directory
> entry and its immediate children; **no file inside it was read, written or
> modified**, and the directory's own timestamp (2026-10-20 05:35) predates
> this cell. No credential file was read at any point: the CLI probes use
> only the exit status of each CLI's own status command, and Kala never
> opens a CLI's credential store.

---

## 1. What this cell delivers

Three chat providers that run the operator's own, already-authenticated
command-line tool as a sandboxed subprocess. Kala never sees, stores or
forwards the CLI's credential; the CLI reads its own login from its own
configuration directory.

| Provider id | CLI | Version read from `--help` | Credential store the CLI owns |
|---|---|---|---|
| `claude_code` | `claude` | 2.1.303 (Claude Code) | `~/.claude` (OAuth / keychain) |
| `codex_cli` (alias `codex`) | `codex` | 0.163.0 | `$CODEX_HOME` (`~/.codex`) |
| `opencode_cli` | `opencode` | 2.0.32 | the OpenCode data directory |

The three ids replace the old `codex` refusal stub. `codex` stays a valid
provider id and is the historical alias of `codex_cli`, so an old saved
preference still resolves (the Studio's frontend already treats it that
way).

---

## 2. The exact argv, and why each flag is there

The prompt is **never** on the argv. It is written to the child's standard
input and the standard input is closed afterwards, so a prompt cannot leak
through the process table and cannot overflow the argument limit.

### 2.1 `claude_code` - Claude Code CLI

```text
claude --print --output-format json --safe-mode --tools "" \
       --strict-mcp-config --permission-mode dontAsk \
       --no-session-persistence [--model <model>]
```

| Flag | Why |
|---|---|
| `--print` | the CLI's documented non-interactive mode for scripted and SDK callers |
| `--output-format json` | one JSON object on stdout; the assistant's final text is its `result` field |
| `--safe-mode` | switches off `CLAUDE.md`, skills, plugins, hooks, MCP servers, custom commands and agents while leaving the login, the model selection and the permissions alone. A chat turn must not run the operator's hooks. |
| `--tools ""` | removes **every** built-in tool: no file read, no command, no URL fetch. This is the hard no-tool guarantee. |
| `--strict-mcp-config` | no MCP server is connected, even when the operator configured one |
| `--permission-mode dontAsk` | anything that would still ask for a permission is denied, so a turn can never block |
| `--no-session-persistence` | the turn leaves no transcript in the CLI's session store |
| `--model <model>` | passed through only when the request names a model. The alias `default` means the CLI's own default model and is therefore **not** passed. |

The `--safe-mode` and `--no-session-persistence` flags need Claude Code
2.1.x (both are in the 2.1.303 `--help` this cell read). `--bare` is
deliberately **not** used: the vendor's own documentation states that bare
mode never reads OAuth credentials or the keychain, which would defeat the
whole point of a subscription route.

### 2.2 `codex_cli` - OpenAI Codex CLI

```text
codex exec --sandbox read-only --skip-git-repo-check --ephemeral \
           --ignore-user-config --json [--model <model>]
```

| Flag | Why |
|---|---|
| `exec` | the CLI's non-interactive mode. With no prompt argument it reads the instructions from standard input. |
| `--sandbox read-only` | every command the model may run is confined to a read-only sandbox, so the turn cannot change anything on the host |
| `--skip-git-repo-check` | the turn runs in a private temporary directory, which is not a repository |
| `--ephemeral` | the turn leaves no session file in the CLI's store |
| `--ignore-user-config` | `$CODEX_HOME/config.toml` is not loaded: the operator's MCP servers, hooks and custom instructions stay out of the turn, and the CLI falls back to its own default model. The login still comes from `CODEX_HOME`. |
| `--json` | the turn's events as JSON lines; the assistant's final message is the `item.completed` event whose item is an `agent_message` |
| `--model <model>` | passed through only when the request names a model; `default` means the CLI's own default |

These four flags are exactly the set the repository's own Python reference
documents as the intended Codex integration
(`src/kala_agent/studio/providers/codex.py`: "the documented flags
(``codex exec --sandbox read-only --skip-git-repo-check --ephemeral
--ignore-user-config``), prompt-on-stdin (never on argv), a minimal
sanitised environment, a bounded deadline, dead child reaping, and a capped
concurrent reader buffer").

**Honest limit:** the Codex CLI has no flag that removes its shell tool.
`--sandbox read-only` plus `--ignore-user-config` is the strongest
"read-only, no agentic extras" posture the CLI offers. A read-only command
the model chooses to run can still read a file it can see.

### 2.3 `opencode_cli` - OpenCode

```text
opencode run --standalone --format json --agent kala-chat [--model <provider/model>]
```

| Flag | Why |
|---|---|
| `run` | OpenCode's non-interactive mode. With no message argument it reads the message from standard input. |
| `--standalone` | a private server for the turn instead of the operator's background service, so a chat turn cannot disturb or inherit a running session |
| `--format json` | the turn's events as JSON lines; the assistant's text is the `text` event's `part.text` |
| `--agent kala-chat` | the agent the turn runs as. That agent denies every action on every resource (see below). |
| `--model <provider/model>` | passed through only when the request names a model; `default` means OpenCode's own default |

The turn also runs in a private temporary directory that holds an
`opencode.json`, and the same document is passed inline in
`OPENCODE_CONFIG_CONTENT`. The document denies everything three times over,
so the intent survives whichever configuration shape the installed OpenCode
reads:

```json
{
  "permission": { "*": "deny" },
  "permissions": [ { "action": "*", "resource": "*", "effect": "deny" } ],
  "agent": {
    "kala-chat": {
      "description": "Kala Studio chat turn: no tools, no file access, no network fetch.",
      "mode": "primary",
      "permission": { "*": "deny" },
      "permissions": [ { "action": "*", "resource": "*", "effect": "deny" } ],
      "tools": { "bash": false, "edit": false, "write": false, "patch": false,
                 "read": false, "grep": false, "glob": false, "list": false,
                 "todowrite": false, "webfetch": false, "websearch": false,
                 "task": false, "question": false, "lsp": false, "skill": false,
                 "external_directory": false, "doom_loop": false }
    }
  }
}
```

`--agent kala-chat` is the loud failure mode: if the installed OpenCode does
not read the document, the agent does not exist and the turn is refused
rather than silently running with tools.

---

## 3. The safety contract

| Property | How it is enforced |
|---|---|
| no shell | `tokio::process::Command` with an argv array; nothing is handed to a shell |
| the prompt never on argv | the prompt is written to standard input, which is then closed |
| no tool use | `claude --tools ""`; OpenCode's deny-all agent; the Codex CLI's read-only sandbox (see the honest limit above) |
| private working directory | every turn gets a fresh `tempfile::tempdir()` that is deleted when the turn ends |
| bounded wall time | 180 s by default, configurable with `KALA_STUDIO_CLI_TIMEOUT_SECONDS` (clamped to 1..=3600); an overrunning child is killed and reaped |
| bounded output | stdout <= 1,000,000 bytes, stderr <= 65,536 bytes; a larger output kills the child and fails the turn |
| sanitized environment | the child starts from an empty environment and receives only `PATH`, `HOME`, `XDG_*`, `TMPDIR`/`TMP`/`TEMP`, `LANG`/`LC_ALL`/`LC_CTYPE`, `TERM`, plus `CLAUDE_CONFIG_DIR` (Claude) or `CODEX_HOME` (Codex) |
| no Kala secret reaches the CLI | no `OPENAI_API_KEY`, no `OPENROUTER_API_KEY`, no studio token, no data directory; asserted by a test |
| no credential of the operator's is read by Kala | the probe uses only the **exit status** of the CLI's own status command; the CLI's own output never leaves the probe |
| consent | the three routes require `allow_remote=true`, exactly like `openai`/`openrouter` (ADR-0001). The refusal is `provider=<id> requires allow_remote=true at request time`. |

### Discovery and the availability probe

* The executable is the configured path first (`KALA_STUDIO_CLAUDE_CLI_PATH`,
  `KALA_STUDIO_CODEX_CLI_PATH`, `KALA_STUDIO_OPENCODE_CLI_PATH`), then the
  executable of the same name on `PATH`.
* The probe runs the CLI's own version command and, when the CLI has one,
  its own authentication-status command:
  * `claude --version` and `claude auth status` (the vendor's CLI reference:
    "Exits with code 0 if logged in, 1 if not"),
  * `codex --version` and `codex login status`,
  * `opencode --version` and `opencode auth list --format json`.
* Only the exit status of the status command is read. The CLI's own text is
  never echoed, so no account name, address or token can reach a response.
* The verdict is cached for 120 s per CLI (`CLI_PROBE_TTL_SECONDS`), because
  the Studio UI polls the provider list and the CLI must not be launched on
  every poll. The three probes run concurrently.

---

## 4. The provider list and the settings route

`GET /api/studio/providers` and `GET /api/studio/settings` now publish the
seven provider ids in the order the frontend already expects:

```text
local, openai, openrouter, codex, claude_code, codex_cli, opencode_cli
```

Every provider row gained three optional fields:

| Field | Meaning |
|---|---|
| `available` | the probe's verdict: the CLI is installed **and** signed in. `null` for the rows that are not probed (the local helper and the two API-key routes). |
| `reason` | the honest reason the provider is unavailable, e.g. `the Claude Code CLI was not found on PATH or at KALA_STUDIO_CLAUDE_CLI_PATH` |
| `source` | `subscription (CLI)` for the four subscription rows, `null` for the others |

`configured` is `CLI found + authenticated`, as the brief requires: a row is
`configured` only when the probe found the binary and the CLI reported that
it is signed in.

The settings route's `credentials` array gained one row per subscription
provider with `env_var: ""` and `source: "subscription"`. The two API-key
rows keep their indices (0 and 1), so the stored-key route is unchanged.

---

## 5. Changed paths

### New (this cell)

| Path | What it is |
|---|---|
| `backend/crates/kala-studio/src/providers/cli_runtime.rs` | the shared subprocess machinery: discovery, the sanitized environment, the bounded runner, the probe and its cache. **This is a fourth file beyond the three the brief names**; the three providers need one runner and one probe, and duplicating them three times would have been worse code. |
| `backend/crates/kala-studio/src/providers/claude_cli.rs` | the Claude Code route |
| `backend/crates/kala-studio/src/providers/codex_cli.rs` | the Codex CLI route |
| `backend/crates/kala-studio/src/providers/opencode_cli.rs` | the OpenCode route |
| `backend/crates/kala-studio/tests/cli_providers.rs` | 19 tests with fake CLIs |

### Shared files (minimal additive edits, re-read immediately before each patch)

| Path | Edit |
|---|---|
| `backend/crates/kala-studio/src/providers/mod.rs` | registered the three modules, the provider rows, the probe, the dispatch arms; removed the `codex` refusal stub and its message |
| `backend/crates/kala-studio/src/schemas.rs` | `ProviderId` gained `ClaudeCode`, `CodexCli`, `OpenCodeCli`; the chat request's provider enum gained the three ids |
| `backend/crates/kala-studio/src/server.rs` | the provider list and the settings body became async (they probe the CLIs); the provider JSON gained `available`/`reason`/`source`; `/health` publishes the provider ids from a constant and probes nothing |
| `backend/crates/kala-studio/src/settings.rs` | `CredentialSource` gained `Subscription`; the credential rows gained the subscription providers |
| `backend/crates/kala-studio/tests/chat_and_remote.rs` | the old "codex stays fail-closed" test now asserts that the route needs consent and reports the missing CLI |
| `backend/crates/kala-studio/tests/corpus_fixture.rs` | the documented normalisation now also covers the subscription providers (see §7) |

### Python reference

| Path | Edit |
|---|---|
| `src/kala_agent/studio/schemas.py` | the `ProviderId` enum gained the three subscription ids, so the DTO's validation message stays in parity with the Rust product. **No other Python file was touched**; `src/kala_agent/studio/providers/dispatch.py` belongs to the `llm-design` cell in `WAVE.md` ("disjoint ownership"), and `codex.py` still ships the Python reference's own fail-closed stub. |

---

## 6. Tests

`backend/crates/kala-studio/tests/cli_providers.rs` - 19 tests. Every case
writes a small bash script into a temporary directory and points the
provider at it with the documented path-override variable. **No test calls a
real CLI.** The scripts record their own argv, their own environment, their
own working directory and their own standard input, so the assertions can
prove the contract.

| Test | What it proves |
|---|---|
| `claude_code_turn_uses_the_documented_argv_and_the_prompt_on_stdin` | the exact argv, the parsed reply, the model, and that the prompt is on stdin and not on argv |
| `claude_code_without_a_model_leaves_the_default_to_the_cli` | no `--model` flag without a model, and none for the alias `default` |
| `codex_cli_turn_uses_the_documented_argv_and_reads_the_jsonl` | the exact argv and the JSONL answer |
| `opencode_cli_turn_uses_the_documented_argv_and_the_sandbox_agent` | the exact argv and the deny-all configuration that reaches the child |
| `the_child_environment_carries_no_kala_secret` | `PATH`/`HOME` reach the child; `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `KALA_STUDIO_TOKEN` and `KALA_AGENT_DATA_DIR` do not |
| `a_missing_cli_is_reported_as_unavailable` | a missing binary is a 503-class failure, not a crash |
| `a_failing_cli_reports_its_own_bounded_text` | a non-zero exit is a subprocess failure carrying the CLI's own bounded text |
| `a_failure_the_cli_prints_as_its_result_is_reported` | a failure the CLI prints as its JSON result (the vendor's documented behaviour) is reported with that text |
| `a_cli_that_reports_an_error_event_is_a_subprocess_failure` | the Codex JSONL error event is a subprocess failure |
| `output_beyond_the_bound_is_refused` | more than 1,000,000 bytes of stdout is refused |
| `a_cli_that_overruns_is_killed_and_reported_as_a_timeout` | the child is killed at the budget and the turn is a timeout |
| `the_probe_reports_a_missing_cli_without_running_it` | a missing binary is reported without running anything |
| `the_probe_reports_the_version_and_the_login_state` | the version and the login state are read from the exit statuses |
| `the_probe_reports_a_cli_that_is_not_signed_in` | a CLI that reports "not signed in" is installed but not usable |
| `the_route_refuses_a_cli_turn_without_remote_consent` | the route answers 400 `provider_not_configured` and **the CLI is never started** |
| `the_route_runs_the_cli_when_the_operator_consented` | the route answers 200 with the CLI's text |
| `the_route_refuses_the_codex_alias_without_remote_consent` | the historical `codex` id keeps its own refusal wording |
| `the_provider_list_names_the_subscription_routes` | the seven ids, the order, the `subscription (CLI)` source and the probe verdict |
| `the_working_directory_is_private` | the CLI reports its working directory and it is a private temporary directory, not the project |

The suite was run 12 times in a row after the last change: 12/12 green,
and 10 more times after the final edit: 10/10 green.

### Gates

| Gate | Result |
|---|---|
| `cargo fmt --all --check` (backend workspace) | clean |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo test --locked --workspace` (debug) | **69 test binaries, all green** |
| `cargo test --locked --workspace --release` | all green |
| `cargo build --locked --workspace --release` | ok |
| `cargo check -p kala-studio --target x86_64-pc-windows-gnu` | **could not run**: the target is installed but the mingw C/C++ toolchain is not (`x86_64-w64-mingw32-gcc` not found), so the `cc-rs` build of the native geometry dependency fails before any Rust code of this cell is compiled. This is an environment limitation, not a result. |

---

## 7. The one real ETXTBSY finding

The first version of the test suite was flaky: roughly one run in three saw
`ExecutableFileBusy` when the child was started. The cause is not the test
harness: the kernel refuses to execute a file that is still open for
writing, and the release of the last reference to a file is deferred (the
final `fput` runs from an RCU callback), so a program that writes an
executable and starts it in the same breath can be refused with `ETXTBSY`.
It was reproduced in a standalone 24-thread program with no Kala code in
it, and it disappears completely when the start is delayed by 50 ms.

The product code now retries a start that the kernel refused as "text file
busy" up to four times, 50 ms apart
(`cli_runtime::MAX_BUSY_START_ATTEMPTS`, `cli_runtime::BUSY_START_BACKOFF_MILLIS`).
The realistic production case is an installer replacing the CLI at the
exact moment a turn starts. The refusal is reported as a subprocess failure
if the retries are exhausted.

---

## 8. Known gaps and intentional differences

### 8.1 The Python reference has not caught up (47 differential cases)

`backend/crates/kala-studio/tests/python_differential.py` reports
**268 cases, 47 mismatches** after this change. Every one of them is the
same deliberate difference: the Rust product publishes four subscription
provider rows, four subscription credential rows, the probe verdict of every
row and three new provider ids in the health document; the Python reference
publishes four provider rows without the new fields.

The affected cases are every `settings.*` case, every `providers.*` case,
every `read.providers` case, every health case (`health.offline`,
`read.health`, `read.health_query`, `host.*`, `origin.*`) and
`chat.codex_unavailable`. Nothing else differs: the health documents differ
only in the `providers` id list, and the Python's `design_available` and the
native `geometry_worker` field are already normalised by the harness.

To close them the Python reference has to publish the same rows, which needs
`src/kala_agent/studio/providers/dispatch.py` (the `llm-design` cell's file
in `WAVE.md`), `src/kala_agent/studio/providers/codex.py` and the
`ProviderRecord`/`CredentialSource` models in
`src/kala_agent/studio/schemas.py`. This cell did not touch those files:
the wave's cell list assigns `providers/dispatch.py` to another worker who
is editing it concurrently, and the Python Studio is the dev-only
reference. The Python `ProviderId` enum **was** updated (it is the request
DTO, it is not owned by another cell, and it is what keeps the validation
message in parity).

### 8.2 The frozen corpus

`backend/fixtures/corpus/v1/studio_http.json` is a record of the Python
reference, so it cannot describe the subscription rows. The corpus
fixture's documented normalisation (the mechanism the file already uses for
the ADR-0009/ADR-0010 additions) now also removes the subscription rows,
the subscription credential rows, the probe fields and the subscription
provider ids before the comparison. Every other field of every other row is
still compared byte for byte.

The corpus itself was **not** regenerated: its Python source pin is already
stale (`intent.py`, `dispatch.py`, `schemas.py` and `server.py` all changed
since it was frozen), so a regeneration belongs to the `compat-drift` cell
and would otherwise capture other workers' in-flight changes. The corpus
digest is unchanged (`53dcb9da…`).

### 8.3 The other honest limits

* **The Codex CLI keeps its shell tool.** `--sandbox read-only` is the
  strongest posture the CLI offers; a read-only command the model chooses to
  run can still read a file it can see.
* **Environment-var authentication is not forwarded.** The child receives
  only the allowlist, so a CLI that the operator authenticated through an
  environment variable (`ANTHROPIC_API_KEY`, for example) reports itself as
  signed out. The subscription logins (OAuth in `~/.claude`, the keychain,
  `$CODEX_HOME/auth.json`, the OpenCode credential store) are found
  normally, because `HOME` and `XDG_*` are forwarded. Forwarding an API key
  would have meant Kala handling the operator's credential, which the brief
  forbids.
* **The OpenCode authentication probe is the slowest one.**
  `opencode auth list` starts a server and takes about 1.7 s. The probes run
  concurrently and their verdict is cached for 120 s.
* **The probe reports "signed in" from an exit status.** The CLI's own
  account text is never read, so the provider row can say *that* the CLI is
  signed in but never *as whom*.
* **A CLI that exits non-zero is a 502** (`provider_subprocess_error`), even
  when the reason is "not logged in". The CLI's own bounded text is in the
  response's `details`, so the operator sees the reason, but the status is
  the subprocess status and not a configuration status.
* **Windows.** Nothing in this cell was exercised on Windows; the
  cross-check could not run (§6). The discovery code looks for `<name>` and
  `<name>.exe` on `PATH`, and the executable check is POSIX-mode based with
  a Windows fallback, but that is untested code on that platform.

---

## 9. What Opus has to do for the live test

1. The three CLIs must be installed and signed in on the host. The
   provider list then reports `configured: true` for them.
2. A turn needs `allow_remote: true`; without it the route answers 400 and
   no CLI is started.
3. `model` is optional: without one the CLI's own default model answers, and
   the reply's `model` field is the model's own name where the CLI reports
   one (`claude_code`: the `modelUsage` key; `codex_cli`/`opencode_cli`: the
   requested id, or `default`).
4. The design-turn flow is unchanged: the front door asks the worker for the
   DesignIntent schema, adds it to the prompt and sends **that** prompt
   through the same provider dispatch, so a design can be built through any
   of the three CLIs.
