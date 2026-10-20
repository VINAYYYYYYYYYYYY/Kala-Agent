# schemas-r1 — the core API DTOs of `kala-api`

**Cell:** `schemas-r1` · **Worker:** Rust worker `schemas-r1` (Opus
conductor, wave `rust-backend-20261019`) · **Date:** 2026-10-20

## 1. What was ported

`src/kala_agent/api/schemas.py` (the **current working tree**, which carries
the user's uncommitted changes) is ported completely into the accepted crate
`backend/crates/kala-api`, in the new modules `schemas`, `views`,
`validation`, `datetime` and `patterns`.

The Python file was **not** edited. It was read as it is in the working tree,
including the uncommitted change that moved `ErrorDetail`, `ErrorResponse`,
`error_response`, `MAX_ERROR_DETAILS` and `MAX_ERROR_MESSAGE_LENGTH` into
`kala_agent.api.envelope`; the native port re-exports the same names from the
crate's `envelope` module, so the public surface of the Python module is
unchanged.

Every model of the Python module is here, with its fields, its constraints,
its defaults, its failure messages and its JSON shapes:

| Python model | Native type | module |
| --- | --- | --- |
| `DomainName` | `schemas::DomainName` | `schemas` |
| `EvaluateDomainRequest` | `schemas::EvaluateDomainRequest` | `schemas` |
| `CreateRevisionRequest` | `schemas::CreateRevisionRequest` | `schemas` |
| `PrepareRevisionRequest` | `schemas::PrepareRevisionRequest` | `schemas` |
| `RunMechanicalWorkflowRequest` | `schemas::RunMechanicalWorkflowRequest` | `schemas` |
| `CheckpointDecisionRequest` | `schemas::CheckpointDecisionRequest` | `schemas` |
| `HealthResponse` | `schemas::HealthResponse` | `schemas` |
| `DomainListResponse` | `schemas::DomainListResponse` | `schemas` |
| `EvaluationResponse` | `schemas::EvaluationResponse` | `schemas` |
| `RevisionListResponse` | `schemas::RevisionListResponse` | `schemas` |
| `CheckpointListResponse` | `schemas::CheckpointListResponse` | `schemas` |
| `RunResponse` | `schemas::RunResponse` | `schemas` |
| `ErrorDetail`, `ErrorResponse`, `error_response` | `envelope::{ErrorDetail, ErrorResponse, error_response}` | `envelope` (re-exported by the Python module too) |

The response models embed the application views of `kala_agent.application`.
Those views are not in `schemas.py`, but the models are meaningless without
them, so they are ported as well, in the new module `views`:

| Python view | Native type |
| --- | --- |
| `DomainPackageView` | `views::DomainPackageView` |
| `CatalogStepView` | `views::CatalogStepView` |
| `WorkflowDefinitionView` | `views::WorkflowDefinitionView` |
| `WorkflowTransitionView` | `views::WorkflowTransitionView` |
| `DomainEvaluationReport` | `views::DomainEvaluationReport` |
| `StepAttemptView` | `views::StepAttemptView` |
| `WorkflowTransitionRecordView` | `views::WorkflowTransitionRecordView` |
| `CheckpointView` | `views::CheckpointView` |
| `WorkflowStateView` | `views::WorkflowStateView` |
| `BudgetUsageView` | `views::BudgetUsageView` |
| `RevisionRecord` | `views::RevisionRecord` |
| `MechanicalRunResult` | `views::MechanicalRunResult` |
| `ApprovalKind`, `ApprovalStatus`, `WorkflowStatus`, `PolicyDecisionStatus` | `views::{ApprovalKind, ApprovalStatus, WorkflowStatus, PolicyDecisionStatus}` |

The contract models the views embed — `DesignRevision`, `Provenance`,
`EvaluationResult` and `ApprovalRequest` — are the **accepted**
`kala-contracts` types, consumed read-only as a path dependency. They are not
duplicated here.

## 2. The public API

```rust
use kala_api::schemas::{
    ApiModel, CheckpointDecisionRequest, CheckpointListResponse, CreateRevisionRequest,
    DomainListResponse, DomainName, EvaluateDomainRequest, EvaluationResponse, HealthResponse,
    PrepareRevisionRequest, RevisionListResponse, RunMechanicalWorkflowRequest, RunResponse,
    request_body_limits, transport_failures,
};
use kala_api::validation::{Context, Failures, Issue, Location, Reading, kind};
```

* `ApiModel::from_value` reads a model and answers the failures the Python
  source reports: the kind, the location, the message, the input and the
  context. It is the native form of `model_validate`.
* `ApiModel::from_body` reads a **request body** the way the body layer of
  FastAPI reads it: a body that is not an object is refused as a
  `model_attributes_type`, and a body that is not there is a missing field.
* `ApiModel::to_wire_value` and `ApiModel::to_wire_json` write the document
  the Python source writes: the declaration order of the fields, the compact
  separators and the UTF-8 text Starlette writes.
* `ApiModel::to_canonical_json` writes the sorted, ASCII escaped document.
* `schemas::transport_failures` turns the failures of a rejected body into
  the sanitized detail lines the error handlers write, so the answer is the
  same body the Python source answers with:
  `errors::handle_request_validation(Some(&schemas::transport_failures(&failures)))`.
* `schemas::request_body_limits` are the limits a body is parsed with.
* `validation::{Issue, Location, Context, Failures, kind, Reading}` are the
  vocabulary of the failures: `kind` names the Pydantic error kinds
  (`missing`, `extra_forbidden`, `model_type`, `model_attributes_type`,
  `string_type`, `string_unicode`, `string_too_short`, `string_too_long`,
  `string_pattern_mismatch`, `int_type`, `int_parsing`, `int_from_float`,
  `float_type`, `float_parsing`, `bool_type`, `bool_parsing`, `enum`,
  `tuple_type`, `dict_type`, `value_error`, `datetime_type`,
  `datetime_from_date_parsing`, `timezone_aware`).

### Every model, for the later `api-routes` cell

**`EvaluateDomainRequest`** (`extra="forbid"`, `frozen=True`,
`str_strip_whitespace=True`)

| field | type | default | constraint |
| --- | --- | --- | --- |
| `design_id` | `Option<String>` | `None` | at most 64 code points, `^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$` |
| `parameters` | `Option<CanonicalObject>` | `None` | at most 256 entries, a name of 1..64 code points, a canonical JSON of at most 16 KiB, and the bounded JSON domain of the tool protocol |

**`CreateRevisionRequest`** (`extra="forbid"`, `frozen=True`,
`str_strip_whitespace=True`)

| field | type | default | constraint |
| --- | --- | --- | --- |
| `design_id` | `String` | — | 1..64 code points, `^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$` |
| `domain` | `DomainName` | — | `building`, `mechanical` or `wing` |
| `summary` | `Option<String>` | `None` | at most 512 code points |
| `parent_revision_id` | `Option<String>` | `None` | at most 128 code points, `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$` |
| `parameters` | `Option<CanonicalObject>` | `None` | as above |
| `state_digest` | `Option<String>` | `None` | `^[0-9a-f]{64}$` |
| `action_digest` | `Option<String>` | `None` | `^[0-9a-f]{64}$` |

**`PrepareRevisionRequest`**: `design_id`, `domain`, `summary`,
`parent_revision_id` and `parameters`, with the constraints above.

**`RunMechanicalWorkflowRequest`**

| field | type | default | constraint |
| --- | --- | --- | --- |
| `design_id` | `String` | `mechanical-reference` | 1..64 code points, the design pattern |
| `workflow_id` | `Option<String>` | `None` | at most 128 code points, the identifier pattern |
| `actor_id` | `String` | `local-user` | 1..64 code points, the identifier pattern |

**`CheckpointDecisionRequest`**

| field | type | default | constraint |
| --- | --- | --- | --- |
| `actor_id` | `String` | `local-user` | 1..64 code points, the identifier pattern |
| `state_digest` | `Option<String>` | `None` | `^[0-9a-f]{64}$` |
| `action_digest` | `Option<String>` | `None` | `^[0-9a-f]{64}$` |
| `reason` | `Option<String>` | `None` | at most 512 code points |

**`HealthResponse`** (`extra="forbid"`, `frozen=True`): `status`, `service`,
`version`, `provider`, `model` (`String`), `offline`,
`egress_configured`, `provider_endpoint_declared` (`bool`), `data_dir`
(`String`), `schema_version` (`BigInt`), `domains` (`Vec<String>`),
`disclaimer` (`String`).

**`DomainListResponse`**: `count` (`BigInt`), `packages`
(`Vec<DomainPackageView>`), `workflows` (`Vec<WorkflowDefinitionView>`),
`domains` (`Vec<String>`).

**`EvaluationResponse`**: `evaluation` (`DomainEvaluationReport`),
`evaluations` (`Vec<DomainEvaluationReport>`, default empty).

**`RevisionListResponse`**: `design_id` (`String`), `count` (`BigInt`),
`revisions` (`Vec<RevisionRecord>`).

**`CheckpointListResponse`**: `workflow_id` (`String`), `count` (`BigInt`),
`pending` (`Option<CheckpointView>`, **required** — a null is a value, not an
absent field), `approvals` (`Vec<ApprovalRequest>`).

**`RunResponse`**: `run` (`MechanicalRunResult`), `state`
(`WorkflowStateView`), `revision` (`Option<RevisionRecord>`, default `None`),
`checkpoint` (`Option<CheckpointView>`, default `None`).

The bounds of the module (`MAX_PARAMETERS_BYTES`, `MAX_PARAMETERS_ENTRIES`,
`MAX_PARAMETER_KEY_LENGTH`, `MAX_SUMMARY_LENGTH`, `MAX_REASON_LENGTH`,
`MAX_ACTOR_LENGTH`, `MAX_DESIGN_ID_LENGTH`, `MAX_IDENTIFIER_LENGTH`,
`MAX_LIST_LIMIT`, `DEFAULT_LIST_LIMIT`, `MAX_AFTER_SEQUENCE`,
`IDENTIFIER_PATTERN`, `DESIGN_ID_PATTERN`, `DIGEST_PATTERN`, `SERVICE_NAME`)
are public constants of `schemas` and `patterns`.

## 3. Changed paths

```
backend/crates/kala-api/Cargo.toml                  (kala-contracts and num-bigint)
backend/crates/kala-api/Cargo.lock                  (the two new dependencies)
backend/crates/kala-api/README.md                   (the modules, the models, the counts)
backend/crates/kala-api/src/lib.rs                  (the new modules and re-exports)
backend/crates/kala-api/src/schemas.rs              (new: the port of schemas.py)
backend/crates/kala-api/src/views.rs                (new: the application views)
backend/crates/kala-api/src/validation.rs           (new: the failure vocabulary)
backend/crates/kala-api/src/datetime.rs             (new: the aware timestamps)
backend/crates/kala-api/src/patterns.rs             (new: the declared patterns)
backend/crates/kala-api/tests/oracle_schemas.py     (new: the live Python oracle)
backend/crates/kala-api/tests/schemas_differential.rs (new: the live differential)
backend/crates/kala-api/tests/schemas_security.rs   (new: the security and edge cases)
.mece/waves/rust-backend-20261019/cells/schemas-r1/REPORT.md   (this file)
```

Nothing else was touched: no Python source, no Python test, no other crate,
no root manifest or lock, no `studio/**`, no shared fixture.

## 4. The live differential

`tests/oracle_schemas.py` builds the **live** Pydantic models of
`kala_agent.api.schemas` and a FastAPI application that carries the **real**
error handlers of `kala_agent.api.errors`, replays a deterministic corpus of
**560** payloads through them, and writes both the corpus and the answers to
disk. `tests/schemas_differential.rs` replays the very same payloads through
the native models and compares:

* the canonical JSON of every accepted object,
* the wire JSON of every accepted object (the declaration order of the
  fields, the compact separators, the UTF-8 text),
* every rejection: the kind, the location, the message, the input and the
  context Pydantic reports,
* the body of every 422 the live error handlers answer.

| model | accepted | refused | total |
| --- | --- | --- | --- |
| `EvaluateDomainRequest` | 36 | 82 | 118 |
| `CreateRevisionRequest` | 15 | 105 | 120 |
| `PrepareRevisionRequest` | 18 | 55 | 73 |
| `RunMechanicalWorkflowRequest` | 17 | 74 | 91 |
| `CheckpointDecisionRequest` | 20 | 63 | 83 |
| `HealthResponse` | 4 | 18 | 22 |
| `DomainListResponse` | 5 | 12 | 17 |
| `EvaluationResponse` | 4 | 7 | 11 |
| `RevisionListResponse` | 2 | 6 | 8 |
| `CheckpointListResponse` | 4 | 6 | 10 |
| `RunResponse` | 3 | 4 | 7 |
| **total** | **128** | **432** | **560** |

**Result: 0 mismatches out of 560 cases.** Eight of the cases differ only in
the order of the undeclared fields, which is the intentional difference of
§8.1; the differential counts them and does not report them as mismatches.

The corpus covers, for every model: the defaults and the minimal bodies,
every field with a valid value, the whitespace that is stripped, the unicode
text, the boundaries of every bound (one under, the exact length, one over),
the wrong type of every field (a number, a float, a boolean, a list, an
object, a null), a missing required field, an undeclared field, the enum with
an accepted and with a refused value, a body that is not an object at all
(a number, a text, a list, a boolean, a null), the parameters with an empty
name, with a long name, with 256 and with 257 entries, with a value that is
larger than the size bound, with a string that is longer than the protocol
bound, with a collection that is longer than the protocol bound, with a key
that is longer than the protocol bound, with 48, 49 and 64 levels of nesting,
with a huge integer, with a negative zero and with a float in the exponent
range, and the nested models with a missing field, with an undeclared field,
with a refused enum value, with a naive timestamp, with a timestamp with an
offset, with a timestamp that cannot be read and with a nested model that is
not an object at all. On top of that, **300 randomized deterministic cases**
(the seed is `20261020`) mutate a valid body of every request model: a field
is replaced by one of 31 values, a field is removed, two fields are replaced
or an undeclared field is added.

**The differential is sensitive.** Breaking one bound of the native port
(`MAX_SUMMARY_LENGTH` from 512 to 511) produces **3 mismatches**; breaking the
first character rule of the identifier pattern produces **1 mismatch**. Both
were reverted, and the differential is green again.

## 5. The gates

### Rust

| Gate | Command | Result |
| --- | --- | --- |
| format | `cargo fmt --check` | clean |
| lint | `cargo clippy --all-targets --all-features -- -D warnings` | clean, no warnings |
| tests (debug) | `cargo test --locked` | **100 passed**, 0 failed |
| tests (release) | `cargo test --locked --release` | **100 passed**, 0 failed |
| release build | `cargo build --release --locked` | success |
| Windows compile | `cargo check --locked --target x86_64-pc-windows-gnu` | success (the target **is** installed) |

Test breakdown: 61 unit tests in `src/`, 2 in `tests/config_differential.rs`,
2 in `tests/differential.rs`, 1 in `tests/schemas_differential.rs`, 19 in
`tests/schemas_security.rs`, 14 in `tests/security_boundaries.rs`, 1 in
`tests/socket_differential.rs`, 0 doc tests — **100**. The 58 tests the crate
had before this cell are still green.

Toolchain: `cargo 1.99.1`, `rustc 1.99.1`. The Windows target **compiles**
here; nothing was executed on Windows, and no Windows runtime claim is made.

### Python

| Gate | Command | Result |
| --- | --- | --- |
| the API tests | `pytest tests/unit/test_api.py tests/security/test_api_boundary.py tests/unit/test_portable_imports.py -q` | **53 passed**, 0 failed |

The Python source was not edited, so the wider suite is unchanged; the three
files above are the ones that cover the API surface this port belongs to.

## 6. Compatibility evidence

* **The failure vocabulary is the Pydantic one**, and it was taken from the
  live Python source and not from memory: `missing` ("Field required"),
  `extra_forbidden` ("Extra inputs are not permitted"), `model_type`,
  `model_attributes_type`, `string_type`, `string_unicode`, `string_too_short`,
  `string_too_long`, `string_pattern_mismatch`, `int_type`, `int_parsing`,
  `int_from_float`, `float_type`, `float_parsing`, `bool_type`, `bool_parsing`,
  `enum`, `tuple_type`, `dict_type`, `value_error`, `datetime_type`,
  `datetime_from_date_parsing` and `timezone_aware`.
* **No model declares an alias.** `schemas.py` and the application views name
  every field with the name it is read and written under
  (`grep -n "alias\|populate_by_name" src/kala_agent/api/schemas.py
  src/kala_agent/application.py` is empty), so there is no alias to port and
  no case of the corpus that could exercise one.
* **The order of the failures is the order Pydantic reports**: the fields of
  a model in the order the model declares them, and the undeclared fields
  after them.
* **The context is the context Pydantic reports**: `min_length`,
  `max_length`, `pattern`, `expected`, `error` and `class_name`.
* **The bounds are the bounds of the Python source**: the identifiers, the
  digests, the summaries, the reasons, the actors, the parameter names, the
  parameter count and the parameter size.
* **The `str_strip_whitespace` of the request models is reproduced**, and the
  response models do not strip, exactly as the Python source configures them.
* **The aware timestamps are written the way Pydantic writes them**: a `Z`
  suffix for UTC, six fractional digits when the microseconds are not zero,
  and the offset of a value that is not in UTC. The reasons `speedate`
  reports (`invalid character in year`, `month value is outside expected
  range of 1-12`, `day value is outside expected range`, `invalid date
  separator, expected \`-\``, `input is too short`, `unexpected extra
  characters at the end of the input`) are reproduced.
* **The integers keep their precision**: an `int` field is a `BigInt`, and a
  30-digit integer is written as it is.
* **The canonical JSON is the canonical JSON of the accepted
  `kala-json` crate**, which reproduces the bytes of CPython's
  `json.dumps(..., sort_keys=True)`.
* **A rejected body answers with the same bytes**: the 422 of the live
  application is compared with the native
  `errors::handle_request_validation(Some(&schemas::transport_failures(&failures)))`
  in every one of the 432 refused cases.

## 7. Security evidence

`tests/schemas_security.rs` (19 tests) covers the hostile inputs:

* the parameters are bounded in every way: the entry count (256), the name
  (1..64 code points), the canonical size (16 KiB) and the bounded JSON
  domain of the tool protocol;
* a parameters document of 200 levels is refused with the depth failure of
  the protocol, and a body of 20 000 levels and a parameters document of
  2 000 entries are refused without exhausting the stack: the parser, the
  validator, the encoder and the value lifecycle of the accepted `kala-json`
  crate are iterative;
* an undeclared field is refused;
* a hostile identifier (`a design`, `a-design;rm -rf /`, `../etc/passwd`) and
  a hostile digest (`A`, `../../etc/passwd`) are refused by the pattern;
* a string that carries a lone surrogate is refused as a `string_unicode`,
  which is what Pydantic reports;
* the failures of a rejected body never carry the submitted value in the
  detail lines the error handlers write;
* the failures are reported in the order the models declare their fields;
* the models are values: a copy can be changed without touching the original,
  and no model carries interior mutability.

## 8. Intentional differences

1. **The undeclared fields are reported in the order the value model keeps
   them.** The Python source reports them in the order the input carries
   them; the native layer reports them in the order of the code points of
   their names, because the JSON value model of the transport keeps the keys
   of an object sorted. The failures themselves are the same. This is the
   only difference the differential reports, and it is reported for 8 of the
   560 cases. A transport that parses a body with an order-preserving parser
   can restore the order of the input; the schemas module cannot, because it
   reads a value and not a text.
2. **The keys of a parameters object are sorted in the wire document.** The
   Python source keeps the order of the input. The canonical JSON of the
   object is the same, and the digest of the parameters is taken over the
   canonical form, so nothing that is digested changes.
3. **The context of a `value_error` carries the raised text.** The Python
   context carries the raised exception itself, which the API never renders:
   it answers a sanitized envelope and not the raw failure.
4. **The nested contract models are validated by the accepted
   `kala-contracts` crate.** Its vocabulary is the contract's own: a value
   that the Python contract refuses with a `ValueError` is refused here with
   the same text (`Value error, identifier must be nonempty and contain only
   safe characters`, `Value error, approval options cannot be empty`, …),
   and a value that Pydantic would refuse with a constraint kind is refused
   here with the kind `value_error`. The one case of the corpus that reaches
   a nested contract model with a refused value matches exactly; the
   difference is visible for a nested timestamp without an offset, which
   Pydantic refuses as `timezone_aware` and the contracts crate refuses as
   `value_error` with the text `Input should have timezone info`.
5. **The models are plain Rust values.** The Python models are frozen, so a
   field cannot be assigned; the native models expose their fields, so the
   owner of a model may change its copy. No model carries interior
   mutability, so a model that is shared cannot be changed behind the back of
   another owner.
6. **A body is parsed with the limits of `schemas::request_body_limits`.**
   The Python transport parses a body with `json.loads`, which applies no
   protocol limit: the bounded JSON domain of the tool protocol is applied by
   the model that reads the body, and not by the parser. A native transport
   that parsed a body with the protocol limits would refuse a payload the
   Python source refuses later, and with another failure. The native
   parser refuses a body that is deeper than 512 levels; the Python parser
   refuses one with a `RecursionError`, which the API answers with a 500.
7. **The canonical JSON of a model is written with the limits of a body**,
   and not with the protocol limits of 48 levels: the Python source writes the
   dump of a model with `model_dump(mode="json")`, which has no depth limit
   at all.

## 9. Remaining gaps

* **The application views are duplicated.** `views` is the native form of the
  views of `kala_agent.application`, which the `application-r1` cell ports
  into `kala-application`. The two will carry the same shapes; the later
  `api-routes` cell must pick one. This report lists the shapes, so the
  conversion is mechanical.
* **The routes, the dependencies and the composition root are not here.**
  They belong to the later `api-routes` cell. This cell adds no route, no
  server wiring and no composition.
* **The body layer of the framework is modelled, not implemented.**
  `ApiModel::from_body` reproduces the failures the body layer of FastAPI
  reports (a body that is not an object, a body that is not there), but the
  transport that reads the bytes of a body is not here.
* **The `mypy` gate of the repository is blocked by the environment** (the
  numpy stubs against `python_version = "3.11"`), independently of this
  change, and no Python file was edited in any case.
* **No Windows runtime was exercised**, no cutover was performed, and no
  speed, readiness or production claim is made. The Python reference remains
  in place; the Rust crate is not wired into any running service.
* **The response models are not exercised against a running application.**
  The corpus builds the nested documents by hand, from the shape of the
  Python models, and not from a live `KalaApplication`; the values the
  composition root produces are the concern of the `application-r1` and
  `api-routes` cells.
