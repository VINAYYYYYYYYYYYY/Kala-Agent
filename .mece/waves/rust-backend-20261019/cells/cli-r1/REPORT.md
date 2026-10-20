# cli-r1 — `kala-cli`, the `kala-agent` command line

Cell: `cli-r1` of the `rust-backend-20261019` wave. Owner: the Rust worker
`cli-r1`. Status: **port complete except the `serve` launch**, the live
differential against the repository Python CLI is green with **zero
unexplained mismatches** (262 steps), and every Rust gate passes except the
Windows cross-check, which cannot compile because the mingw C toolchain is
absent here (evidence in §7).

No commit, no push, no deployment, no delegation and no credential read. The
only paths touched are `backend/crates/kala-cli/**` and this file.

> **Process disclosure (read this first).** During one manual probe I made a
> shell quoting mistake and ran the native binary once with the process HOME,
> i.e. it opened `/home/v9i/.local/share/kala-agent` instead of a temporary
> directory. At `2026-10-20 05:35:30 +0530` that invocation ran `init`; the
> application opened and closed the existing data directory and rewrote
> `kala.sqlite3` (the kernel-state snapshot write `close()` performs). No
> design, revision, approval or journal event was written by that invocation,
> no token was read, and no other file in that directory changed mtime. I did
> not attempt to repair or further modify the directory. All tests and every
> other probe use dedicated temporary HOME/data directories; the test harness
> clears the environment and sets both on every child process.

---

## 1. What was built

`backend/crates/kala-cli` is a NEW standalone crate (its own `[workspace]`,
`Cargo.lock` and `target/`) that ports `src/kala_agent/cli/main.py` (891
lines), `cli/__init__.py` and `cli/__main__.py` into the native command line,
with the console-script name from `pyproject.toml`:

```toml
[[bin]]
name = "kala-agent"
path = "src/main.rs"
```

It composes, read-only by path:

| Accepted crate | What the CLI uses |
|---|---|
| `kala-application` | `KalaApplication`, every operation and DTO, `View`, `ApplicationError`, `LOCAL_ACTOR_ID`, `REVISION_DECISIONS` |
| `kala-api` | `config::Settings`, `settings_from_env`, `SettingsError`, `ENVIRONMENT_FIELDS`; `boundary::{API_TOKEN_FILE_NAME, is_loopback_host, issue_api_token, read_api_token}`; `pytext::trim` |
| `kala-json` | `CanonicalValue`, `CanonicalObject`, `JsonText`, `format_float` |
| `kala-domains` | `NON_AUTHORITATIVE_FINDING` |

The crate is presentation only: it reads validated settings from the
environment (plus the root `--data-dir` override), opens the same local data
directory the API uses, prints one JSON document to stdout, and maps outcomes
onto the Python exit codes. It holds no engineering rule, decides no approval
on its own, never opens an outbound connection, and never mints a credential
except the `serve` launch token (which is not printed; see §6).

### Changed paths (the only ones I touched)

| Path | What |
|---|---|
| `backend/crates/kala-cli/Cargo.toml` | standalone workspace, path dependencies, `Cargo.lock` |
| `backend/crates/kala-cli/src/lib.rs` | crate surface, constants, `ServeTarget`/`ServePlan`, settings, `main`, dispatch, all commands, parameter parsing, output/error writers |
| `backend/crates/kala-cli/src/main.rs` | the `kala-agent` binary |
| `backend/crates/kala-cli/src/parser.rs` | argparse-compatible parser: commands, abbreviations, `--opt=value`, negative-number values, append options, typed ints, choices, required options, exact error wording and exit 2 |
| `backend/crates/kala-cli/src/help.rs` | CPython 3.14 `HelpFormatter` and `textwrap.wrap` reproduction: usage wrapping, two-column actions, `COLUMNS` handling |
| `backend/crates/kala-cli/src/json.rs` | `json.loads` input dialect and `json.dumps(indent=2, sort_keys=True)` output dialect, both iterative |
| `backend/crates/kala-cli/tests/differential.rs` | the live Python differential (262 steps) and data-directory state comparison |
| `backend/crates/kala-cli/tests/security.rs` | 13 security/edge cases |
| `backend/crates/kala-cli/tests/public_api.rs` | 7 public-surface/dialect cases |
| `.mece/waves/rust-backend-20261019/cells/cli-r1/REPORT.md` | this file |

Nothing else was modified: every accepted crate, the root `backend/Cargo.toml`
and lock, the Python source/tests, shared fixtures, adapters and `studio/**`
are untouched. Other cells were editing `kala-api` concurrently; its transient
compile states were waited out, never edited.

---

## 2. Public API

```rust
use kala_cli::{main, Command, Invocation, Outcome, UsageError};

// One invocation. `argv` excludes the program name; `None` reads the process
// arguments. A supplied application is never closed (the caller owns it);
// `environ` is the settings source (`None` reads the process environment).
pub fn main(
    argv: Option<Vec<String>>,
    application: Option<&KalaApplication>,
    environ: Option<&BTreeMap<String, String>>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32
```

Constants (the Python names): `PROGRAM_NAME`, `EXIT_OK` (0), `EXIT_ERROR`
(1), `EXIT_USAGE` (2), `EXIT_FAILED_OUTCOME` (3),
`DATA_DIR_ENVIRONMENT_KEY`, `MAX_PARAMETER_BYTES` (16,384),
`MAX_REASON_LENGTH` (512), `MAX_ACTOR_LENGTH` (64),
`MAX_PARAMETERS_ENTRIES` (256), `MAX_DIGEST_LENGTH` (64),
`DEFAULT_EVENT_LIMIT` (50), `MAX_EVENT_LIMIT` (1,000),
`MAX_AFTER_SEQUENCE` (1,000,000,000), `REVISION_DECISIONS_LIST`,
`DEFAULT_REVISION_DECISION`, `SERVE_HOST`, `SERVE_PORT`, `MIN_SERVE_PORT`,
`MAX_SERVE_PORT`, `UVICORN_MODULE_NAME`, `SERVE_LOG_LEVEL`,
`SERVE_UNAVAILABLE_MESSAGE`, `NON_LOOPBACK_HOST_MESSAGE`,
`SERVE_ROUTE_LAYER_GAP_MESSAGE`, `DISCLAIMER`, `EPILOG`.

`ServeTarget { host, port }` with `url()` (IPv6 bracketed), and
`ServePlan { target, token_path }`:

```rust
pub fn build_serve_target(host: Option<&str>, port: Option<&BigInt>)
    -> Result<ServeTarget, ApplicationError>;
pub fn build_serve_plan(application: &KalaApplication,
    host: Option<&str>, port: Option<&BigInt>)
    -> Result<ServePlan, ApplicationError>;
pub fn load_uvicorn() -> Result<(), ApplicationError>; // always the gap error
```

`parser`: `parse(&[String]) -> Outcome`, `Outcome::{Run(Invocation),
Help(String), Usage(UsageError)}`, `UsageError::render()`, the `Command`
enum (one variant per subcommand, with the parsed fields), and `COMMANDS`
(the 12 declarations).

`help`: `OptionSpec`, `CommandSpec`, `ROOT_PROG`, `ROOT_DESCRIPTION`,
`EPILOG`, `HELP_OPTION`, `ROOT_OPTIONS`, `formatter_width()`,
`root_help(width)`, `command_help(spec, width)`, `root_usage(width)`,
`command_usage(spec, width)`.

`json`: `loads(&str) -> Result<CanonicalValue, LoadError>` (CPython
`json.loads`: duplicate keys last-wins, `NaN`/`Infinity`/`-Infinity`,
arbitrary-precision integers up to 4,300 digits, lone surrogate escapes,
CPython's 52,099 container-depth limit), `pretty` (`indent=2`,
`sort_keys=True`, ASCII escaping), `default_dumps` (the separators the
parameter-size check measures), `LoadError`, `MAX_DEPTH`,
`MAX_INTEGER_DIGITS`.

The binary `kala-agent` is `main(None, None, None, ...)` and
`std::process::exit(code)`.

---

## 3. Fidelity

Every subcommand, flag, default, environment variable, help document, exit
code, stdout/stderr document and data-directory side effect was ported:

* **argparse surface**: 12 subcommands in declaration order; `-h/--help`;
  unique long-option abbreviation; `--name=value`; `--`; negative-number
  values (`-1` is a value, `-x` is not); append `--param`; last-wins repeats;
  typed integers (`+5`, `1_0`, surrounding whitespace); `--decision` choices;
  required-option checks before extra-argument reporting; the root parser
  reporting unrecognized extras with the root usage; the pre-dispatch actor
  and reason validation *before* the data directory is opened; usage/error
  exit code 2; help printed to stdout and exit 0 even with trailing extras.
* **help layout**: a faithful port of CPython 3.14's `HelpFormatter`
  (action maximum length across sections, help position, usage wrapping with
  the 0.75·width rule, section blank lines, epilog, `COLUMNS` fallback) and
  `textwrap.wrap` (whitespace munging, hyphen breaks, long-word breaks,
  drop-whitespace). Verified against Python for all 13 documents at 11
  widths (including invalid/zero/negative `COLUMNS`).
* **output bytes**: `json.dumps(payload, indent=2, sort_keys=True)` with
  `ensure_ascii=True`, CPython float repr, integer formatting, empty
  containers and a trailing newline; errors are the same writer over
  `{"error": {"code": ..., "message": message[:512]}}` (512 code points).
* **input bytes**: `json.loads` for `--param` values and
  `--parameters-json`; a parse failure on `--param` falls back to the raw
  string; a parse failure or non-object document on `--parameters-json` is
  the Python `application_error`; the parameter map is merged
  document-then-overrides (last wins), bounded at 256 entries and 16,384
  bytes of `json.dumps` output.
* **validation ordering**: actor/reason before settings; settings before the
  application; digest/decision/actor/reason inside `commit-revision` after
  the application opens; identifier/limit/filter errors from the root with
  the Python codes and messages.
* **commands**: `init` (created flag, settings paths, schema version, tool
  ids, provider/model facts, domains, disclaimer, non-authoritative finding),
  `domains`, `token`, `evaluate`, `run-mechanical`, `workflow-status`,
  `approve`, `reject`, `prepare-revision`, `commit-revision` (two-phase,
  with the Python `status` field and revision record nesting), `events`,
  and `serve`'s pre-bind validation.
* **exit codes**: 0 completed, 1 error, 2 usage, 3 completed-with-failed-
  outcome for evaluations and workflow states; reject always exits 0.

---

## 4. The live differential

The oracle is the repository's installed editable package, run as:

```
<repo>/.venv/bin/python -I -m kala_agent.cli
```

(override the interpreter with `KALA_PYTHON`). `-I` still sees the editable
install in the venv's site-packages; no Python source was edited. Every child
runs with a cleared environment and its own `HOME`, `KALA_AGENT_DATA_DIR`,
`KALA_AGENT_MODEL_PROVIDER=local`, `KALA_AGENT_MODEL_ID=local`, `TZ=UTC`,
`LC_ALL=C.UTF-8` and the scenario's `COLUMNS`. The real data directory is
never an input to the test harness.

### 4.1 Corpus

| Suite | Steps | Scope |
|---|---:|---|
| help documents | 143 | root + 12 commands × `COLUMNS` in `{0, -5, abc, 23, 40, 60, 78, 80, 100, 120, 200}` |
| behaviour | 119 | the whole command surface, stateful, plus errors and configuration |
| **total** | **262** | |

The 119 behaviour steps drive one shared script through both sides in
separate temporary data directories, covering:

* `init` (create and verify), `domains`, `token` (absent, stored, invalid),
  every domain evaluation (passed, failed constraints, JSON overrides,
  duplicate keys, `null`, `NaN`/`Infinity`, booleans, arrays, unknown
  domain, unsafe/empty identifiers), the mechanical run (first and resumed),
  workflow status, approve (success and finished), reject, the two-phase
  revision prepare/commit (commit with digests, commit again, stale head,
  rejection), and journal pages/filters/limits;
* error paths: empty/over-long actor, over-long reason, bad and uppercase
  digests, unknown requests, unsafe/reserved identifiers, unknown workflow,
  `--limit` 0/negative, `--after-sequence` negative, parameter limits,
  deeply nested JSON (CPython `RecursionError`), long-message truncation,
  non-ASCII size accounting, data directory `/`;
* argparse errors: no command, unknown command, unknown flag, extra
  positional, missing value, missing required (one and several), invalid int,
  invalid choice, ambiguous option, unknown root option/short option, root
  option after the command, negative and option-like values, `--`, help
  abbreviation, short help suffix, `--help=x`, integer spellings, repeated
  options, `--data-dir` and its abbreviation;
* configuration: relative data dir, worker count 0/9/non-integer, negative
  and non-numeric timeout, bad provider, non-HTTP and credentialed endpoint,
  two invalid fields, and a data directory that is a file;
* environment: a declared endpoint and a credential-shaped value are reported
  without being printed;
* the HOME-derived default data directory (both HOMEs temporary), compared
  structurally.

Deterministic values a later command needs (`request_id`, `state_digest`,
`action_digest`) are read from each side's own output, because the declared
native package identity makes them different by design.

### 4.2 Comparison and normalization

Every step compares exit code, stdout and stderr **byte for byte** after
normalizing only:

1. that side's data-directory, HOME and `--data-dir` override paths;
2. any 64-character lowercase SHA-256 digest;
3. ISO-8601 UTC timestamps (`Z` and `+00:00`, with or without fraction);
4. digest-derived `commit-<design>-<16 hex>-<16 hex>` request ids.

This is exactly the declared identity/timestamp boundary of
`application-r1` (§8); no formatting, ordering, wording or numeric
difference is masked.

At the end of the main sequence the two data directories are compared:

* the complete relative file tree (digest filenames and the
  `artifacts/objects/<2 hex>` prefix normalized);
* every SQLite table's row count;
* the full journal sequence (`sequence`, `event_type`, `aggregate_id`,
  `workflow_id`, `run_id`);
* every revision id/design/sequence;
* every approval status/workflow;
* every design spec id; and
* the schema migrations.

Result: **equal**. The only normalized state values are the same identity
digests and commit request ids.

### 4.3 Results

| Run | Result |
|---|---|
| main behaviour corpus | 119/119 steps equal |
| help corpus | 143/143 documents byte-identical |
| final data-directory state | equal after the declared normalization |
| debug run | 2 differential tests pass |
| release run | 2 differential tests pass |

Two consecutive debug runs and two release runs produced the same result.

---

## 5. Compatibility and security evidence

* **Same data directory.** Both implementations produce the same tree,
  schema, table row counts, journal sequence, revision ids and approval
  statuses for the whole corpus; cross-opening is already proven by the
  accepted `kala-application` cross-reader.
* **Same output bytes.** Help, usage errors, JSON documents and error
  documents are byte-identical under the declared normalization.
* **Token discipline** (`tests/security.rs`):
  * `token` prints a stored token; before a launch it is the Python message
    and exit 1;
  * `init`, `domains`, `events`, `evaluate`, `run-mechanical`,
    `workflow-status`, `approve` and `serve` never print the token, and a
    byte scan of every file in the data directory finds it only in
    `api-token`;
  * an invalid stored token is treated as no token;
  * the `ServePlan` value never renders the token.
* **Secret discipline**: `KALA_AGENT_API_KEY`, `OPENAI_API_KEY` and a declared
  endpoint never appear in stdout/stderr; `offline`/`egress_configured` facts
  come from the root, not from settings.
* **Configuration refusals** report only field names and failure kinds
  (`data_dir:value_error`, `worker_count:greater_than_equal`, …), never the
  rejected value, and never a traceback.
* **Fail-before-create**: a usage error, an invalid actor/reason, and an
  invalid configuration leave the data directory absent.
* **Loopback only**: wildcard, routable, blank and hostname addresses are
  refused with the Python message; ports 0, negative, 65,536 and
  beyond-i64 are refused; IPv6 URL bracketing is preserved.
* **Limits**: 256 parameter entries, 16,384 serialized bytes, 512-code-point
  messages, 64-hex digest shape, reserved `kernel-…` identifiers, journal
  page bounds and identifier rules all keep the Python code/message.
* **Injected application ownership**: a caller-supplied root is never closed.
* **Tampered journal**: the CLI restores the Python
  `local_failure`/`JournalIntegrityError` document for a failed chain
  verification (the native root keeps the message and one error type; the
  mapping is by the deterministic message prefix).
* **No sockets**: the library has no networking dependency and `serve` refuses
  before binding; the only `serve` networking code in the accepted tree is
  `kala-api`'s server, which this CLI does not call (§9).

---

## 6. Tests

From `backend/crates/kala-cli`:

| Suite | Tests |
|---|---:|
| `tests/differential.rs` | 2 (262 live steps) |
| `tests/public_api.rs` | 7 |
| `tests/security.rs` | 13 |
| unit tests in `src/` | 0 |
| **total** | **22** |

All 22 pass in debug and release, 0 failed, 0 ignored. The live differential
needs `/home/v9i/Projects/Kala-Agent/.venv/bin/python` (override with
`KALA_PYTHON`); it skips with a printed message if that interpreter cannot
import `kala_agent`. The library and the binary have no Python dependency.

---

## 7. Gates

Run in `backend/crates/kala-cli`:

| Gate | Result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked` | 22 passed, 0 failed, 0 ignored |
| `cargo test --locked --release` | 22 passed, 0 failed, 0 ignored |
| `cargo build --locked --release` | pass |
| `cargo check --locked --target x86_64-pc-windows-gnu` | **cannot run**: the target is installed, but the mingw C compiler is not (`libsqlite3-sys` build script fails with `failed to find tool "x86_64-w64-mingw32-gcc"`). This is a missing cross toolchain, not a code error; no Windows runtime claim is made. |

The gates were re-run after the last source edit. `kala-api` was under
concurrent editing during the run; a transiently broken state was waited out
and never edited, and the final dependency state compiles and passes the
gates above.

---

## 8. Intentional differences

1. **`serve` starts nothing.** The Python command imports uvicorn, mints a
   launch token, builds the FastAPI application and binds the loopback
   interface. This port validates the target with the Python messages and
   then refuses with `application_error` and
   `SERVE_ROUTE_LAYER_GAP_MESSAGE` (exit 1). It does not mint a token and
   opens no socket. Every `serve` path that fails before a socket exists
   (host, port, argparse) matches Python exactly. See §9 for the precise
   upstream state.
2. **Constructor failure typing.** The accepted root returns one
   `ApplicationError` where Python raises concrete classes. Journal chain
   verification is mapped back to `local_failure`/`JournalIntegrityError`
   because its message is deterministic; other constructor classes are not
   recoverable. Example: an unwritable data directory gives Python
   `local_failure`/`PermissionError` and Rust `application_error` with
   `the local database cannot be opened: Permission denied (os error 13)`.
3. **Extra-field parameter errors (accepted `kala-domains`).** Python's
   Pydantic model reports every unknown key in input order, joined by `; `:
   `evaluate --domain mechanical --param z=1 --param a=2` →
   `z: Extra inputs are not permitted; a: Extra inputs are not permitted`.
   The native path reports only the first key in `CanonicalObject` order:
   `a: Extra inputs are not permitted`. Cause:
   `kala-domains/src/input.rs::reject_extra` returns on the first key of the
   sorted map; the CLI cannot preserve object insertion order through
   `kala_application::DomainBinding::parameters(&CanonicalValue)`. Not patched
   (accepted crate).
4. **Lone surrogate in a parameter value (accepted `kala-domains`).** Python
   reports `thickness_mm: Input should be a valid string, unable to parse raw
   data as a unicode string`; Rust reports `thickness_mm: Input should be a
   valid number, unable to parse string as a number`. The CLI preserves the
   lone surrogate code point exactly; the coercion wording differs in the
   accepted crate.
5. **Oversized integers / huge bounds.**
   * `--parameters-json` with a >4,300-digit integer: Python's
     `json.loads` raises `ValueError`, which the Python CLI does not catch and
     which produces a traceback; Rust reports the documented
     `--parameters-json must be a JSON object` error (exit 1). `--param` with
     the same integer matches exactly (Python falls back to the raw string).
   * `events --after-sequence` above `i64::MAX`: Python's SQLite binding
     raises an uncaught `OverflowError` (traceback, exit 1); Rust reports
     `local_failure`/`OverflowError` (exit 1) instead of silently returning an
     empty page. Negative out-of-range values keep Python's
     `invalid_parameters` message.
6. **`int()` spellings.** Rust accepts ASCII digits, signs, whitespace and
   underscore grouping, which covers the declared CLI. Python's `int()` also
   accepts non-ASCII Unicode decimal digits; those are usage errors here.
7. **Deep JSON.** The 52,099-container limit is the measured CPython 3.14
   threshold on this machine; a document past it is
   `local_failure`/`RecursionError` on both sides, but the exact threshold is
   platform-dependent.
8. **Non-UTF-8 argv/environment.** Rust `String` arguments are lossy-decoded;
   Python uses `surrogateescape`. No valid UTF-8 invocation differs.
9. **Terminal width.** For a redirected stream both sides use `COLUMNS`
   (fallback 80); on a real TTY Python queries the terminal while the native
   line keeps `COLUMNS`/80.
10. **Injected application + `serve`.** The native port cannot share a
    borrowed root with a server factory; `serve` is not supported for an
    injected application. All other injected-application commands work and
    never close it.

---

## 9. Remaining gaps

* **`serve` launch.** The Python command and the native command diverge on
  the one path that starts a server. At the time of this port:
  * `kala-api` publishes `routes::{Application, Routes, Lifespan}` and the
    connection layer;
  * a standalone `backend/crates/kala-api/server` crate exists with
    `LocalApplication` (the `Application` implementation over
    `KalaApplication`) and a `kala-api-server` binary, but the
    `api-routes-r1` cell has published **no REPORT**, and the crate was
    transiently uncompilable while this cell ran (`cargo check --locked` in
    that directory failed inside `kala-api` while the cell was editing).
  * Per the task, this cell calls a `kala-api` server entry point only when
    `api-routes-r1` has landed it. No accepted entry point exists yet, so
    `serve` refuses with the exact gap message instead of launching the
    unreported server. `build_serve_target` and `build_serve_plan` (target
    validation and token minting) are complete and tested; wiring the
    command is the only remaining step once the entry point is accepted.
* **`serve` live behavior** (binding, health, token gate, shutdown summary)
  is therefore not claimed or differentially tested; the api-routes cell owns
  the route/transport differential.
* **Constructor error classes** beyond the journal case (§8.2).
* **Accepted-crate parameter-validation wording and aggregation** (§8.3,
  §8.4) needs a change in `kala-domains`/`kala-application`; this cell did not
  patch it.
* **Windows cross-check** needs the mingw C toolchain; no Windows execution
  is claimed.
* **Runtime cutover, packaging, speed and readiness** are not claimed. This
  crate is the native CLI surface for the wave; the Python console script
  remains the installed entry point.
