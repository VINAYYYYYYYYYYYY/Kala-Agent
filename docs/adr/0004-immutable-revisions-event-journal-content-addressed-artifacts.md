# ADR-0004: Single-Writer Immutable Revisions over an Event Journal and Content-Addressed Artifacts

| Field | Value |
|---|---|
| Status | **Proposed** — awaiting stakeholder confirmation. Implementation evidence does not convert a Proposed decision into accepted policy. |
| Date | 2026-09-25 |
| Last reviewed | 2026-09-26 |
| Scope | Persistence, revision history, artifact integrity |
| Requirements addressed | FR-08 one-writer immutable revisions, FR-09 SQLite event journal and content-addressed artifacts, FR-06 pre-commit checkpoints, FR-14 provenance and rollback |
| Related ADRs | ADR-0001, ADR-0002, ADR-0003, ADR-0005, ADR-0006 |
| Implementation status | **Largely implemented, with two named gaps.** `persistence/database.py` holds **schema version 2** with 9 tables, ordered migrations, WAL, foreign keys, and append-only triggers. `persistence/journal.py` maintains a **hash-chained** append-only journal verified before every read and against the tail before every append. `persistence/artifacts.py` is a content-addressed store with `O_NOFOLLOW` confinement, atomic same-directory hard-link publication, and SHA-256 verification on every read. `persistence/repositories.py` provides 6 repositories with a transactional head compare-and-swap. Gaps: **(1)** rollback is **not** implemented — there is no design-level operation that creates a new revision restoring a verified earlier state, and the only `rollback` in the codebase is the SQL transaction rollback; **(2)** projections and projection rebuild are **not** implemented — the journal is queryable but there is no read model to rebuild from it. Retention and garbage collection are also absent, so nothing is ever deleted. **Hardening since the last review:** the data directory is created `0o700` and the SQLite file is set to `0o600`, both with an explicit `os.chmod` after creation because `mkdir(mode=…)` is itself umask-masked and would still yield `0o755` under umask `022`. The serialized writer is exercised by a real-thread concurrency test. A completed workflow can no longer authorize another design's commit, nor can a completed workflow with no stored approval authorize anything. **What is still absent and is not claimed:** an external anchor for the journal head digest, encryption at rest, retention, and any projection or replay layer. |
| Verification | 1,227 tests pass on CPython 3.11.16 and CPython 3.14.7; branch coverage 94.76% / 94.77% against a configured `fail_under = 90`; `tests/security/test_journal_integrity.py` (19), `tests/security/test_artifact_concurrency.py` (7), and `tests/security/test_revision_commit.py` (23) cover this ADR directly. See `docs/verification-report.md`. |

## Context

An engineering agent changes design state over time, and a user needs to answer questions that a mutable
“current document” cannot answer reliably: which values were used, which package and tool versions
produced a result, what the model saw, which approval allowed an action, and whether a later run
can be replayed. Overwriting a design file also makes accidental loss, accidental combination of
incompatible state, and untraceable manual repair difficult to detect.

The local-first constraint makes a database-backed event record more important, not less. A remote
service cannot be the source of truth when the network is unavailable, and binary artifacts such as
geometry, meshes, solver decks, and exchange files can be large, sensitive, and unsuitable for a
remote relational record. The system therefore needs two complementary local storage mechanisms:

1. A transactional, queryable event journal for ordered state transitions and decisions.
2. A content-addressed object store for immutable bytes whose identity is independent of its path.

Several writers are not required for the MVP. A single local user can run one active design session,
and a single-writer rule removes difficult questions about merge order, lock ownership, and the
meaning of a revision sequence. It also gives the system a clear answer when a process crashes:
recover from the last committed event rather than trying to infer which concurrent write was intended.

The design must still leave room for future integrations. A real CAD or CAE tool may need to stream
large files, a second application may need to inspect history, and a future package may add new
events or projections. The storage contract should therefore be explicit and versioned without
making the kernel depend on a particular file format or database server.

## Decision

We adopt the following decisions.

**D1 — The event journal is authoritative.** The authoritative journal is a local SQLite database
at the configured `journal_path` (`journal.sqlite3` in the current `Settings` defaults). It is an
append-only sequence of versioned events. A materialized operational database at the configured
`database_path` may be added later for read models, but it must be rebuildable from the journal and
must never be treated as an independent source of truth. No cloud database, remote synchronization
service, or model response may replace the journal.

**D2 — One writer per data directory.** Exactly one process owns the writer lease for a configured
`data_dir`. The lease is held for the duration of a session or service instance and is protected by
a local file lock plus a journal-level writer identity. Other processes may open the database for
read-only projections, but they cannot append events or commit revisions. If a second writer is
detected, the new writer fails closed and reports a recoverable `WRITER_BUSY` condition; it does not
silently merge or retry forever. The implementation may use SQLite locking as defence in depth, but
the application-level lease is the policy boundary.

**D3 — Revisions are immutable, ordered, and explicitly parented.** A `DesignRevision` is never
updated or deleted. A new change creates a new revision with a monotonically increasing sequence
within its design, an optional parent revision, a canonical specification snapshot, the exact
package manifests used, artifact references, and a provenance identifier. The head is a journaled
projection, not a mutable row that can silently move. A commit requires an expected parent digest
and a `PRE_COMMIT` checkpoint decision; a mismatch invalidates the decision and stops the write.

**D4 — Every event has a versioned envelope.** Each event contains at least a globally unique
`event_id`, a monotonically increasing journal sequence, event type, aggregate identifier, schema
version, causation/correlation identifiers, actor, recorded timestamp, payload, and payload digest.
The implementation should also record the previous event digest and the current event digest so
that accidental omission, reordering, and simple corruption are detectable. A hash chain is an
integrity signal, not a replacement for protected backups, filesystem permissions, or an external
signature.

**D5 — Effects are atomic at the logical level.** For a state-changing transaction, the writer
validates the preconditions, obtains the one-writer lease, begins a SQLite transaction, appends
all journal events, updates the materialized head/projection, and commits once. A process crash
before commit leaves the previous committed state intact. A crash after commit leaves a durable
transition; recovery does not infer an unjournaled transition from partial files. Tool calls and
model calls are first recorded as requested/running events and later completed with a result event,
so an interrupted operation is visible rather than silently lost.

**D6 — Artifacts are immutable content-addressed objects.** Every artifact is addressed by the
SHA-256 digest of its bytes and is stored under a digest-derived path such as
`artifacts/sha256/<first-two-hex>/<digest>`. The object is written to a temporary file in the same
filesystem, flushed and fsynced, then atomically renamed. The store rejects a digest mismatch and
never overwrites an existing object. `ArtifactRef` stores the logical artifact identifier, URI,
digest, media type, byte size, kernel-derived classification, optional preview URI, and provenance
reference. Logical filenames and user-facing names are metadata, not object identity.

**D7 — Artifact metadata and bytes have a safe commit order.** Bytes are durably written before the
journal records `ARTIFACT_RECORDED`; a crash may leave an unreferenced object, which is safe to
quarantine or garbage-collect after a journal audit, but it must not expose a partial object as a
valid artifact. The SQLite transaction that records the artifact reference and its consuming event
must commit together. Readers verify digest and size before exposing bytes to a tool or model path.

**D8 — The event catalog is domain-neutral and versioned.** The MVP catalog includes events for
session lifecycle, design/specification changes, workflow and plan lifecycle, tool requests and
results, artifact writes, model transcript completion, checkpoint requests and decisions, budget
accounting, provenance, revision commits, package changes, rollback, and failures. Domain packages
may add named event types only through a reviewed schema registry; arbitrary event names or payloads
are rejected. Every projection declares the event versions it understands and can be rebuilt from
the beginning of the journal.

**D9 — Rollback appends a compensating history.** Rollback never deletes events, artifacts, or a
prior revision. It verifies that the target revision and its package/artifact dependencies remain
available, records a `ROLLBACK_REQUESTED` event and checkpoint decision, creates a new revision
whose state is the target state with explicit rollback provenance, and atomically moves the design
head to that new revision. A package rollback similarly creates a new manifest selection and a new
revision. The old history remains available for audit and replay.

**D10 — Replay has two explicit meanings.** Replay of a recorded run means applying the journal's
committed events and recorded tool/model inputs in order, verifying their digests, and producing
the same revision and artifact references. It does not mean re-querying a hosted model or silently
re-running a changed solver. A fresh workflow is a new run with a new event sequence; reproducibility
is claimed only when the plan, package versions, tool versions, seed, and relevant inputs are pinned.

**D11 — Backup and recovery are local operations.** A backup consists of a consistent SQLite
snapshot or journal export plus the referenced content-addressed object set, with a manifest that
can verify every digest. Restore verifies the manifest before opening the store for writes. The MVP
does not provide a hosted backup service; users can place the verified backup on encrypted local or
user-controlled storage. Backup frequency, retention, and encryption-at-rest are deployment choices
that must be documented separately.

## Consequences

### Positive

- Design history is inspectable and resistant to accidental overwrites because every meaningful
  transition is appended and every committed revision is immutable.
- SHA-256 addressing gives artifacts a stable identity, enables deduplication, and makes path
  changes harmless to integrity checks.
- One-writer ordering is easy to explain, test, and recover from, especially for a single-user MVP.
- A crash produces either the previous committed state or a durable, journaled transition; partial
  writes are detectable instead of becoming silent design changes.
- Rollback is auditable and preserves the decisions and artifacts that preceded it.
- Event replay and package-version pinning support review, debugging, and long-lived engineering
  projects.
- A local SQLite and filesystem store have low operational overhead and work without a network.

### Negative

- One writer limits throughput and requires a clear lease/recovery protocol; background tools cannot
  write revisions directly.
- Full event and transcript retention consumes local disk, and content-addressed deduplication does
  not remove the need for retention and garbage-collection policy.
- Canonicalization, digest verification, and event schema migrations add implementation work and
  can make ordinary changes look more complicated than a mutable document model.
- A hash chain detects accidental corruption but cannot protect against an attacker who can rewrite
  the SQLite file and all local backups.
- Rollback is semantically a new revision, not a magical undo; users must understand which state is
  current after a rollback.
- Large CAD/CAE artifacts may require streaming, quotas, and storage planning before integrations
  are added.

### Neutral

- The journal schema will be an internal compatibility surface; event versions allow future readers
  to understand old records.
- A digest-derived object layout is not intended to be human-browsed; user-facing names remain in
  metadata and indexes.
- Read-only readers can coexist with the writer, but they observe committed events only.
- `Settings.database_path` remains available for a future projection store; keeping it non-authoritative
  avoids a second source of truth.

## Alternatives Considered

**A mutable current-state SQLite table with an audit table.**

- Rejected as the sole design: it makes it easy to update a design before its provenance and
  predecessor are durable, and recovery depends on reconstructing missing history from application
  state. A journal plus projections is more work but gives the required audit and replay semantics.

**A conventional relational database with a `revisions` row and files in named directories.**

- Deferred: adequate for a hosted service, but not the best MVP default because it adds a server or
  service dependency and still requires a separate immutable blob strategy. SQLite can provide the
  required transaction boundary locally and can be migrated behind a repository interface later.

**Git as the revision and artifact store.**

- Rejected as the primary store: Git is excellent for source-like text and gives useful history, but
  it does not naturally model typed events, tool-call state, binary artifact metadata, or a
  single-writer local application protocol. A Git-backed export can be a future interoperability
  option.

**Cloud object storage or a hosted event database.**

- Rejected for the MVP: it violates the local-first system-of-record requirement and makes offline
  operation and confidentiality dependent on a service. The repository interface could support a
  later encrypted replica without changing the authoritative model.

**Referencing files by ordinary paths rather than content digests.**

- Rejected: paths are mutable, can escape the data directory, and do not prove which bytes a prior
  revision used. A digest makes every reference verifiable.

**Multiple concurrent writers with automatic merge or last-write-wins behavior.**

- Rejected: merge semantics for typed engineering state, tool side effects, and checkpoint decisions
  are not specified, and implicit conflict resolution would undermine the user's control. A future
  collaborative product can introduce a separate merge protocol.

**Overwrite the design and retain only periodic snapshots.**

- Rejected: snapshots do not explain how a state was produced, cannot represent interrupted steps
  or approvals, and make rollback and provenance incomplete.

## Trade-offs

- We accept one writer and lower write concurrency in exchange for a simple, auditable total order
  and straightforward crash recovery.
- We accept additional local storage and verification work in exchange for immutable history,
  deduplication, and reproducible artifact identity.
- We accept append-only rollback semantics in exchange for preserving evidence and avoiding destructive
  undo operations.
- We accept event and schema migration complexity in exchange for a durable, replayable foundation
  that can grow with real tool integrations.
- We accept that a local journal is not tamper-proof against a user with full filesystem access; the
  MVP prioritizes accidental-integrity protection and auditable controls, with protected backups and
  stronger signing as later hardening.

## Revisit Triggers

- A multi-user or distributed deployment requires concurrent writers or a replicated event log.
- CAD/CAE artifacts exceed local storage or transaction-size limits and need streaming chunking.
- A legal or compliance workflow requires externally signed event ranges or an independent witness.
- A projection store becomes necessary for interactive queries at a scale that rebuilding from the
  journal is too slow.
- A user needs encrypted-at-rest artifacts with a key-management workflow not defined here.

## Verification

Status as of 2026-09-26:

1. Concurrent writer tests prove that a second writer cannot append or commit a revision.
   **Partially met.** Two real threads racing the same commit produce exactly one revision, and the
   loser is told the head moved with no orphan row
   (`tests/security/test_revision_commit.py:371`); a separate real-thread test asserts that concurrent
   transactions lose no acknowledged event (`tests/unit/test_persistence.py:218`). Artifact publication is additionally proven safe
   under **real multi-process** stress, because the store requires no inter-process coordination at
   all (`tests/security/test_artifact_concurrency.py:207`). The journal, by contrast, relies on
   SQLite's own locking plus the head CAS; the CAS is what makes it correct, not a writer lease.
   There is no OS-level writer lease.
2. Crash-injection tests at each transaction boundary leave either the previous revision or a fully
   journaled new revision, never a partially committed one. **Not met.** No `SIGKILL` fault
   injection exists. The property is argued from `BEGIN IMMEDIATE`, the savepoint for nested
   transactions, and the head CAS, and the CAS rollback is verified directly
   (`test_revision_commit.py:296,325`).
3. Artifact tests prove byte-to-digest verification, atomic promotion, deduplication, and safe
   handling of orphaned temporary files. **Met.** `tests/security/test_artifact_concurrency.py`
   covers publication races, a reader that races publication, symlinked buckets and objects, and
   inode-identity re-verification. Temporary files are created with `O_CREAT|O_EXCL` and a random
   name, and are unlinked on both the success and the failure path.
4. Replay tests apply a recorded event sequence twice and compare revision identifiers, event
   digests, and artifact references. **Not met.** No replay driver exists.
5. Rollback tests create a new head and prove that the original events and revisions remain intact.
   **Not applicable, because rollback is not implemented.** What *is* enforced is the adjacent
   property: a commit that does not extend the current head is refused outright
   (`RevisionForkRefusedError`), so a design history cannot be **forked**
   (`test_revision_commit.py:252`).
6. Projection rebuild tests delete a derived database and reconstruct it from the journal.
   **Not applicable, because there is no projection.** The single local database is authoritative, and
   there is nothing derived to delete.
7. Tamper evidence for the journal, from the position of an attacker who already has direct write
   access to the database file. **Met, within an honest limit.** `tests/security/test_journal_integrity.py`
   drops the append-only triggers the way a file-level attacker would and requires detection anyway:
   an omitted event, a reordered history, an edited payload, and a tampered-while-closed journal are
   all found. A wholesale rewrite yields an internally consistent chain with a **different head
   digest** (`test_journal_integrity.py:256`), which is exactly what that test pins.
8. A test asserts that the data directory and the database are not readable by another local account,
   independently of the process umask. **Met.** `Database.__init__` creates the parent with
   `mode=0o700` and then `os.chmod`s it, and `os.chmod`s the connection target to `0o600`. The explicit
   `chmod` is the part that matters: `mkdir`'s mode argument is masked by the umask, so `mode=0o700`
   alone would still yield `0o755` under umask `022`. Observed on the verification host for both a
   `python -m` invocation and an installed `kala-agent` console-script launch on CPython 3.11.16:
   `700` for the directory, `600` for the database. (`tests/unit/test_persistence.py:190`.) The
   artifact store is `0o700`/`0o600` and the tool run directory is `0o700`
   (`tests/unit/test_artifacts_branches.py:298`, `tests/unit/test_executor_branches.py:1895`).
   **What this does not do:** the contents are still plaintext. The modes decide *who on the host* can
   read the files; they do not protect the bytes. There is no encryption at rest.
9. A test asserts that a real-thread concurrency run loses no acknowledged write. **Met.**
   `tests/unit/test_persistence.py:218` drives concurrent transactions and asserts that a single
   writer serializes them with no acknowledged event lost. This complements the two-commit race test
   in item 1; the CAS remains what makes the *revision* write correct, since the journal still relies
   on SQLite's own locking plus that CAS for multi-process coordination.
10. A test asserts that an empty journal verifies rather than raising. **Met.** `verify_chain` on
    genesis returns a verification with no head rather than failing
    (`test_journal_integrity.py:146`).

## Implementation notes (2026-09-26)

Schema version 2, 9 tables, all created by ordered migrations applied atomically:

```text
schema_migrations   applied versions; a database newer than the code is refused
events              sequence PK AUTOINCREMENT; 4 indexes; UPDATE and DELETE abort;
                    INSERT aborts without a 64-char lowercase chain digest
design_specs        id PK
design_revisions    revision_id PK; UNIQUE(design_id, sequence)
approval_requests   request_id PK
tool_calls          call_id PK
tool_results        call_id PK; FK to tool_calls ON UPDATE/DELETE RESTRICT
memory_entries      memory_id PK; indexed on source and expires_at
workflow_snapshots  sequence PK; snapshot_id UNIQUE; digest-verified canonical JSON
```

| Decision | Implementation | Reference |
|---|---|---|
| D1 authoritative journal | The `events` table in `<data_dir>/kala.sqlite3`. Note `Settings.journal_path` (`journal.sqlite3`) is **vestigial and unused**; the product opens `database_path` and the journal is a table inside it. The containing directory is `0o700` and the file is `0o600`, both set with an explicit `os.chmod` after creation because `mkdir(mode=…)` is umask-masked. | `persistence/database.py:111,312,329` |
| D2 one writer per data directory | One shared connection with a per-database `RLock`; `_SerializedConnection` takes the same lock on every statement, so a caller holding only the connection cannot interleave with another thread's transaction. `BEGIN IMMEDIATE` at the top level, a savepoint when nested. | `persistence/database.py:59,285` |
| D3 immutable, ordered, parented revisions | `commit_revision` re-reads the head inside the transaction and raises `HeadMovedError`; `UNIQUE(design_id, sequence)`; a commit must extend the head | `persistence/repositories.py:136` |
| D4 versioned event envelope | The **chain** version is in the digest domain string `kala-agent.event-chain.v1.sha256`; there is no explicit event-`schema_version` column. | `persistence/chain.py:24` |
| D5 logical atomicity | Two-phase commit: prepare writes `revision.prepared` and nothing to the design; commit appends `revision.commit_decided`, then writes under the CAS, then appends `revision.created`. A completed workflow cannot authorize another design's commit, and a completed workflow with no stored approval authorizes nothing. Durable kernel state is stored under the reserved `kernel-state:` namespace, which no user-supplied identifier can name. | `application.py`, `application.py:671,675,678` |
| D6 immutable content-addressed artifacts | `objects/<ab>/<digest>`, directories `0o700`, objects `0o600`, digest verified on every read | `persistence/artifacts.py:159` |
| D7 safe commit order | Bytes are written to a temporary file, fsynced, and published by a same-directory hard link; the directory is then fsynced and the final name re-checked | `persistence/artifacts.py:261` |
| D8 domain-neutral event catalog | Event types are identifiers, not an enum; the observed set is `domain.evaluated`, `plan.created`, `workflow.*`, `policy.*`, `revision.*` | journal payloads |
| D9 rollback appends compensating history | **Not implemented.** | — |
| D10 replay has two explicit meanings | **Not implemented** as a capability; the determinism preconditions are in place | `docs/architecture-report.md` §3.8 |
| D11 local backup and recovery | **Not implemented.** No backup, no manifest, and no external anchor for `head_event_digest` | — |

The hash chain deserves one sentence of its own, because it is the strongest integrity control in the
system and its limit should be stated rather than implied. Every event digest commits to the event's
own immutable content **and** to its predecessor's digest, over a domain-separated, length-prefixed
preimage so no field boundary can be forged by moving characters between adjacent fields. The chain
is verified in full at construction, again before every read, and against the tail before every
append. It is tamper-**evident**, not tamper-**proof**: anyone who can write the database can rewrite
a whole suffix consistently. `ChainVerification.head_event_digest` exists so a caller can anchor that
value outside the database, and no such anchor is implemented. That is residual risk RR-2.

## References

- `docs/architecture-report.md` — the as-built schema and tables (§9.2), serialized single-writer
  access (§9.3), the hash chain (§9.4), the artifact store (§9.5), the two-phase commit (§9.6),
  restart restore (§9.7), and the rollback, retention, and projection gaps (§3.7–§3.9).
- `docs/verification-report.md` — the per-file coverage of `persistence/*`, the security-control
  tables for the journal, artifacts, and revision commit, and residual risks RR-2, RR-3, RR-6, RR-9.
- `docs/threat-model.md` — TB-5, the persistence boundary: T5.1–T5.11.
- ADR-0001 — Local-First, No-Egress-By-Default Operation with a Provider-Neutral LLM Gateway.
- ADR-0002 — Field-Independent Kernel with Declarative Domain Packages.
- ADR-0003 — Deterministic Workflow Engine with Bounded LLM Planning and Policy Checkpoints.
- ADR-0005 — Capability-Based Typed Tool Contracts Executed in Isolated Processes.
- ADR-0006 — Package Trust, Sandboxing, Provenance, Rollback, and Agent Budgets.
