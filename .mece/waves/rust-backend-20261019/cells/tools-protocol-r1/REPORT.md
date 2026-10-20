# Tools protocol cell report — native kala-tools (protocol layer)

Date: 2026-10-20
Cell: `tools-protocol-r1` (wave `rust-backend-20261019`)
Crate: `backend/crates/kala-tools`
Implementer: OpenCode Go **MiniMax M3.1 Flash (Free)**
Status: implemented and validated in isolation; ready for independent review.
No commit, push, publish, deploy, delegation, credential read, runtime cutover,
Windows runtime execution, speed claim or readiness claim.

## Scope and ownership

Owned and changed only:

- `backend/crates/kala-tools/**` (new standalone workspace, local
  `Cargo.lock`)
- this cell's `REPORT.md`

Not touched: `backend/Cargo.toml` and the root `backend/Cargo.lock`, any
accepted crate source or manifest (`kala-json`, `kala-contracts`,
`kala-budget`, `kala-process`, `kala-kernel`, `kala-domains`,
`kala-repositories`, …), the Python product or its tests, `adapters/`,
`studio/**`, shared fixtures, the other waves and `Rust+buid123/`.

The crate is a **new standalone workspace** (`[workspace]` in its own
`Cargo.toml`, its own `Cargo.lock`). It is not a member of the root `backend/`
workspace and the root manifest and lock were not edited. The accepted
`kala-json` and `kala-contracts` crates are consumed **read-only** through path
dependencies; no public API of a dependency was patched.

## Delivered

A complete native port of `src/kala_agent/tools/protocol.py` (1,595 lines) and
of the public exports it contributes to `src/kala_agent/tools/__init__.py`:

| Python member | Rust member |
| --- | --- |
| `PROTOCOL_VERSION`, the 15 `MAX_*` bounds | `kala_tools::protocol::{PROTOCOL_VERSION, MAX_*}` |
| `ToolProtocolError`, `JsonEnvelopeError`, `ToolEnvelopeError`, `ToolWorkspaceError`, `SchemaValidationError` | `ToolProtocolError` with the variants `Protocol`, `JsonEnvelope`, `Envelope`, `Workspace`, `Schema { reason, path }` |
| `ToolErrorCode`, `ToolEnvelopeKind`, `_FAILURE_STATUSES` | `ToolErrorCode`, `ToolEnvelopeKind`, `is_failure_status` |
| `canonical_json_dumps`, `strict_json_loads`, `sha256_hex`, `canonical_json_digest` | the same four functions |
| `_normalize_json`, `_json_equal`, `_object_without_duplicate_keys`, `_reject_json_constant`, `_parse_json_float` | inside the accepted `kala-json` crate; the messages are mapped one for one in `src/protocol/json.rs` |
| `validate_against_schema`, `check_object_schema` and the whole private schema layer (`_schema_keywords`, `_schema_bound`, `_schema_int`, `_schema_float`, `_schema_branch`, `_validate_object`, `_validate_array`, `_validate_string`, `_validate_number`, `_pattern_rejects`) | `validate_against_schema`, `check_object_schema` and the private layer in `src/protocol/schema.rs` |
| `_bounded_pattern`, `_reject_nested_repetition`, `_reject_ambiguous_pattern`, `_quantifier_at`, `_modifier_end`, `_group_body_start`, `_character_class_end`, `_escaped_atom_end`, `_PatternBudget`, `_PatternScope`, `_UNSAFE_PATTERN_CONSTRUCTS`, `_PATTERN_REPEAT_SPEC` | the admission analysis in `src/protocol/pattern/mod.rs` |
| `re.compile(...)` + `compiled.search(value)` | the CPython-compatible matcher in `src/protocol/pattern/engine.rs` |
| `ToolRequestEnvelope` (all fields, `arguments_json`, `arguments_digest`, `envelope_digest`, `to_payload`, `to_json`, `from_payload`, `from_json`, `from_call`) | `ToolRequestEnvelope` with the same members |
| `ToolResponseEnvelope` (all fields, `succeeded`, `result_json`, `result_digest`, `to_payload`, `to_json`, `succeeded_envelope`, `failed_envelope`, `from_payload`, `from_json`, `to_result`) | `ToolResponseEnvelope` with the same members |
| `ToolHandler` | the `ToolHandler` trait |
| `SubprocessToolHandler` | `SubprocessToolHandler` |
| `WORKER_BOOTSTRAP`, `build_worker_command` | `WORKER_BOOTSTRAP` (byte identical), `build_worker_command` |
| `PROTECTED_CHILD_ENVIRONMENT_KEYS`, `build_child_environment` | the same two members |
| `read_request_stream`, `write_response_stream` | the same two functions |
| `resolve_workspace_path` | `resolve_workspace_path` |
| `summarize_error`, `sanitize_child_stderr` | the same two functions |

### Changed paths

```
backend/crates/kala-tools/Cargo.toml
backend/crates/kala-tools/Cargo.lock
backend/crates/kala-tools/.gitignore
backend/crates/kala-tools/README.md
backend/crates/kala-tools/src/lib.rs
backend/crates/kala-tools/src/protocol/mod.rs
backend/crates/kala-tools/src/protocol/error.rs
backend/crates/kala-tools/src/protocol/enums.rs
backend/crates/kala-tools/src/protocol/json.rs
backend/crates/kala-tools/src/protocol/schema.rs
backend/crates/kala-tools/src/protocol/text.rs
backend/crates/kala-tools/src/protocol/envelope.rs
backend/crates/kala-tools/src/protocol/workspace.rs
backend/crates/kala-tools/src/protocol/handler.rs
backend/crates/kala-tools/src/protocol/diagnostics.rs
backend/crates/kala-tools/src/protocol/pattern/mod.rs
backend/crates/kala-tools/src/protocol/pattern/engine.rs
backend/crates/kala-tools/src/protocol/pattern/unicode.rs
backend/crates/kala-tools/tests/oracle.py
backend/crates/kala-tools/tests/support/mod.rs
backend/crates/kala-tools/tests/protocol_differential.rs
backend/crates/kala-tools/tests/protocol_pattern_differential.rs
backend/crates/kala-tools/tests/protocol_branches.rs
backend/crates/kala-tools/tests/protocol_security.rs
backend/crates/kala-tools/tools/generate_unicode_tables.py
backend/crates/kala-tools/tools/probe_python_regex.py
.mece/waves/rust-backend-20261019/cells/tools-protocol-r1/REPORT.md
```

The module layout is shallow on purpose: later cells add `registry`, `sandbox`,
`simulator` and `executor` to this same crate, and `src/lib.rs` re-exports only
the protocol API today.

The public API is listed exhaustively in
`backend/crates/kala-tools/README.md`.

## Compatibility and security evidence

### The accepted crates are used, not re-implemented

* The canonical bytes, the sorted code-point keys, the ASCII escaping, the
  arbitrary-precision integers and the bounded limits come from the accepted
  `kala-json` crate. `src/protocol/json.rs` only maps the `kala-json` failure
  to the Python message: `NotCanonicalizable` becomes
  `"value cannot be canonicalized as JSON"` on the dump path and
  `"payload is not strict JSON"` on the load path, exactly like the two Python
  paths that share the CPython integer-conversion limit.
* The typed tool call, descriptor, artifact, run status and result models come
  from the accepted `kala-contracts` crate.

### Error classes, messages and precedence

* The five Python classes are the five variants of one Rust enum.
  `python_class_name()` reports the class the Python implementation would have
  raised and `Display` reproduces `str(error)`, including the `"{path}: {reason}"`
  form of a schema failure and its `reason`/`path` attributes.
* The **error precedence** of the envelope validators is preserved. The Python
  implementation reads every member of a payload first and validates them in the
  order of `__post_init__`; the Rust port does the same, which the randomised
  envelope differential (400 payloads) exercises on every field.
* The one place where Python raises *before* it validates is the
  `tuple(granted_capabilities)` conversion in `from_payload`, and the
  attribute error of a payload that is not a mapping; the Rust port raises the
  same failures at the same points, with the same messages.

### Live Python differential

The differential runs the **repository's own** `kala_agent.tools.protocol`
through `/home/v9i/Projects/Kala-Agent/.venv/bin/python` (Python 3.14.8) with
`-I` (isolated mode) and `PYTHONPATH=src`; the oracle script adds the
repository `src` to `sys.path` itself. A missing interpreter fails the test
rather than skipping it. The override is `KALA_TOOLS_PYTHON`.

Every comparison is byte-for-byte on the canonical JSON of the whole result
(the success value or the error class plus the error message). There is no
normalisation of hashes, messages or ordering.

| Corpus | Cases | Compared against |
| --- | --- | --- |
| strict decoding and canonical encoding | 48 raw payloads (valid, malformed, duplicate keys, JSON constants, non-finite floats, invalid UTF-8, a BOM, trailing data, a string over the bound, a key over the bound, an array over the bound, 49 and 48 levels of nesting, the exact byte budget and one byte over it) | `strict_json_loads` + `canonical_json_dumps` |
| canonical encoding | 141 values (21 curated + 120 randomised) | `canonical_json_dumps` |
| schema validation | 288 (value, schema) pairs (88 curated over every keyword, every type, every error branch + 200 randomised schemas and values) | `validate_against_schema` |
| object schema admission | 14 schemas | `check_object_schema` |
| request envelopes | 28 curated (including the CPython whitespace padding of an identifier) + 200 randomised payloads | `from_payload`, `to_payload`, `to_json`, `arguments_json`, `arguments_digest`, `envelope_digest` |
| response envelopes | 20 curated + 200 randomised payloads | `from_payload`, `to_payload`, `to_json`, `result_json`, `result_digest`, `succeeded` |
| workspace paths | 20 relative paths against a real tree with a symlink | `resolve_workspace_path` |
| child diagnostics | 22 byte payloads with and without a redaction | `sanitize_child_stderr` |
| error summaries | 80 cases (5 error classes × 8 messages × with and without a redaction) | `summarize_error` |
| pattern admission and matching | 117 curated + 300 randomised patterns, each against 46 sample values (19,182 value comparisons) | the full `validate_against_schema` path |

The corpus is canonicalised before it is sent to the oracle, so both sides see
the object members in the same order and the error precedence is comparable.
The corpus sizes are asserted by a test so this report cannot drift away from
the corpus that runs.

**Result: 20,243 comparisons, zero unexplained mismatches.**

### The pattern layer

The pattern layer is the only part of the protocol that is not delegated to an
accepted crate, so it carries the most evidence:

* The **admission analysis** is a line-by-line port of the Python
  `_bounded_pattern`, `_reject_nested_repetition` and
  `_reject_ambiguous_pattern`, including the order of the six unsafe-construct
  checks, the repeat specification of one to nine digits, the possessive
  modifier, the backtracking budget (2,048 states), the unbounded-repeat budget
  (one), the consumed-width budget (1,024) and the repetition bound (1,000).
  A refused pattern is never compiled and never executed.
* The **matcher** reproduces CPython's `re` semantics: the `str` based `\w`,
  `\d` and `\s` (the Unicode tables are generated from the live CPython
  predicates by `tools/generate_unicode_tables.py` and committed as
  `src/protocol/pattern/unicode.rs`), `$` at the end of the subject *or* just
  before a final newline, `\Z` only at the very end, leftmost-first alternation,
  the CPython escape, character-class and repetition rules, and matching over
  Unicode code points so a lone surrogate is an ordinary character.
* The matcher is an **explicit-stack backtracking VM**: it never recurses, so a
  hostile pattern cannot overflow the thread stack. A step counter is a
  backstop; the admission analysis bounds the real work far below it.
* The **security suite** is the Rust port of `tests/security/test_schema_redos.py`:
  the 22 ambiguous repetitions, the 18 unsupported constructs, the 7
  over-budget repetitions, the registration refusal, the admitted subset that
  must still match and still bind, the bounded-work assertion and the
  `maxLength` requirement. All of them pass.

### Bounds and limits

Every bound of the Python module is present and enforced: 1 MiB payload, 256 KiB
arguments, 32,768 code points per string, 2,048 items per array, 256 code points
per key, 48 levels of JSON nesting, 32 levels of schema nesting, 32 path
segments, 1,024 path characters, 256 pattern characters, 1,024 pattern input
characters, 1,000 repetitions, one unbounded repetition, 2,048 backtracking
states and 512 characters of sanitised diagnostic.

## Commands and results actually run

From `backend/crates/kala-tools/` with `cargo 1.99.1` / `rustc 1.99.1`:

```
cargo fmt --check                                              -> clean
cargo clippy --all-targets --all-features -- -D warnings       -> clean
cargo test --locked                                            -> 71 passed, 0 failed
  (45 branch + 13 differential + 3 pattern differential + 10 security + 0 doc)
cargo test --locked --release                                  -> 71 passed, 0 failed
cargo build --locked --release                                 -> success
cargo check --locked --target x86_64-pc-windows-gnu             -> success
cargo check --locked --target x86_64-pc-windows-gnu --all-targets -> success
cargo clippy --locked --target x86_64-pc-windows-gnu --all-targets -- -D warnings -> clean
```

The `x86_64-pc-windows-gnu` target **is installed** on this machine, so the
Windows cross-check was run. It is a compile check only: no Windows runtime
execution is claimed and no Windows behaviour is claimed.

Debug and release totals are identical. No ignored test, no benchmark, no
performance number and no readiness statement is claimed.

## Intentional differences (complete list)

1. **Exception classes.** Python raises `AttributeError` for a payload that is
   not a mapping and `TypeError` for a `granted_capabilities` member that is
   not iterable, where Rust has no equivalent class. The Rust port reports the
   *same message text* (`"'list' object has no attribute 'get'"`,
   `"'int' object is not iterable"`) through `ToolProtocolError::Protocol`. The
   live differential compares the message and the failure and compares the
   class only for the five protocol classes.
2. **Object member order.** The canonical value model sorts object keys, so a
   schema or a value whose members were declared in another order is iterated
   in code-point order. When **more than one** member can fail, the *reported*
   member (and therefore the reported path) can differ from the Python
   implementation, while the accepted set, the message text and the pass/fail
   decision are identical. The differential corpus is canonicalised before it
   is sent to the oracle, so the two sides iterate in the same order and this
   difference is not exercised by the corpus; it is a property of the value
   model, not of the validator.
3. **`\N{...}`.** CPython resolves a Unicode character name through the
   Unicode database. The Rust port refuses the escape, so a schema that uses
   it is refused instead of matched. The refusal fails closed and is reported
   as `"pattern is not a valid regular expression"`.
4. **A repetition count of 2^32-1 or more.** CPython raises `OverflowError`
   from `re.compile`, which is *not* a `SchemaValidationError`; the Rust port
   refuses the pattern with `"pattern is not a valid regular expression"`.
5. **A number that does not fit in a binary64.** CPython raises `OverflowError`
   from `float(value)` when a schema bound or a validated number does not fit;
   the Rust port saturates the value and reports the ordinary bound failure.
6. **The interpreter of a worker command.** CPython falls back to
   `sys.executable`; the Rust port has no such interpreter, so the caller names
   one and a missing or empty interpreter is refused with
   `"no python interpreter is available for the tool worker"`.
7. **Timestamps.** The accepted contract timestamps always carry an offset, so
   the "must include a timezone" rule of `to_result` is structural in Rust and
   only the ordering rule (`"finished_at cannot precede started_at"`) remains.
8. **The exception hierarchy.** The five Python classes are one Rust enum, so
   a caller that catches `JsonEnvelopeError` in Python catches the
   `JsonEnvelope` variant in Rust; there is no shared `ValueError` base.

## Remaining gaps and limits

1. **Not a runtime cutover.** Nothing in the product calls this crate yet.
   The registry, sandbox, simulator and executor cells still own their own
   modules and are not touched by this cell.
2. **The Unicode tables are a snapshot.** `src/protocol/pattern/unicode.rs` was
   generated from CPython 3.14.8 (Unicode 16.0.0) and is committed. A future
   Unicode release would need the table regenerated with
   `tools/generate_unicode_tables.py`; the crate never runs Python.
3. **`repr()` of an exotic string.** The Python `repr()` of a non-printable
   code point uses the Unicode "printable" property. The Rust port approximates
   it with the control, separator and format ranges. The approximation only
   affects the rendering of a non-printable code point inside a schema
   validation message (a property name or a declared type name).
4. **Group names.** CPython requires a group name to be a Python identifier.
   The Rust port accepts the Unicode alphabetic and alphanumeric properties,
   which is the same rule for every character that matters in practice, but it
   is not the full XID rule.
5. **The workspace resolution is an approximation of `Path.resolve(strict=False)`.**
   The longest existing prefix is resolved with the operating system and the
   remaining components are appended, with `..` folded away. A path with a
   symlink in a component that does not exist yet is resolved differently from
   CPython on a platform with different symlink semantics.
6. **`Path.resolve` on Windows.** The port uses `std::path::canonicalize` for
   the existing prefix, which resolves junctions differently from CPython's
   `realpath`. The `PYTHONPATH` import roots keep the platform separator, like
   the Python `str(Path(root).resolve(strict=False))`, while the workspace uses
   the POSIX separator, like the Python `as_posix()`. The differential only ran
   on Linux.
7. **The `ToolHandler` trait is object safe only for the two methods.** Python's
   `ToolHandler` is a `runtime_checkable` `Protocol`, so any object with the
   right attributes satisfies it; the Rust trait must be implemented.
8. **No performance claim.** The bounded-work assertion in the security suite
   is a regression guard, not a throughput measurement.

## Reproduction

```bash
cd backend/crates/kala-tools
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --locked --release
cargo check --locked --target x86_64-pc-windows-gnu
```

The differential needs the repository virtual environment
(`/home/v9i/Projects/Kala-Agent/.venv/bin/python`); `KALA_TOOLS_PYTHON`
overrides it.
