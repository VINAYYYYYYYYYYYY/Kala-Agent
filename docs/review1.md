# Code Review 1 — Implementation vs. Architecture Report

| Field | Value |
|---|---|
| Date | 2026-09-27 |
| Scope | `src/kala_agent/`, `adapters/freecad/src/kala_agent_freecad/` |
| Baseline | `docs/architecture-report.md` (2026-09-26), ADR-0001..0007 |
| Test baseline | 1,227 tests pass on CPython 3.14 |
| Method | Four subsystem reviews (persistence/commit, tool plane, kernel/gateway, domains/API/CLI/adapter); top findings re-verified by hand; many reproduced with scratch scripts |

The suite is green, but several claims in the architecture report do not hold in the code. The most
serious gaps are in the revision commit path (not atomic), approval terminality, the domain
capability check (does not exist), and tool-process supervision.

## High severity

### H1. The revision commit is spread across 5–6 separate transactions
`application.py` ~1438–1494 and 2467–2488. The approval row, `revision.prepared` /
`commit_decided` / `created` events, the revision insert, and the kernel-state save each commit
independently. `revision.created` is appended *after* `commit_revision` returns, and provenance is
read back only from that event. A crash, busy timeout, or journal error between them leaves a
revision row with no event; `get_design_revision` then raises `ProvenanceUnavailableError` forever
and `list_design_revisions` fails for the whole design. Contradicts §9.6.

**Fix:** a unit-of-work that wraps approval, revision, and events in one `Database.transaction()`
(nesting already works as savepoints); store provenance on the revision row.

### H2. "Rejection is terminal" does not hold
- Revision commits: `ApprovalRequestRepository.save` is an unconditional upsert and
  `prepare_design_revision` checks only the in-memory policy engine. After a crash, restore can bring
  back a PENDING policy snapshot, and a re-prepare overwrites the REJECTED row.
- `PolicyEngine`: `invalidate` overwrites REJECTED with INVALIDATED, and eviction removes rejected
  requests first. With `max_tracked_requests=2`, a rejected request became approvable after three
  unrelated decisions (reproduced).

**Fix:** monotone approval status in SQL (`UPDATE … WHERE status='pending'` + trigger); in
`PolicyEngine`, refuse to overwrite REJECTED/ABORTED and evict terminal-negative records last.

### H3. `enforce_capability_request` does not exist
`domains/__init__.py:737`. The only check is private `_require_capability_grants`, which returns
early when nothing is requested. All ten evaluators declare `()`, and `application._evaluate_step`
calls `package.evaluate` without a policy. §13.2, AC-19, and §3.11 row 20 cite a check the product
never runs.

### H4. The evaluator digest misses the code that computes results
`domains/__init__.py:449` hashes only the evaluator class's module. The stress formula
(`mechanical/simulation.py`) and yield strengths (`mechanical/parameters.py`) live elsewhere, so
editing them leaves the pinned package digest unchanged. Contradicts §13.1.

**Fix:** digest every module under the package directory plus `kala_agent.domains`.

### H5. Worker descendants survive a normal exit
`tools/executor.py:1445`. `_terminate_process_group` runs only when `failure is not None`, so a
successful call can leave a child in its group that escapes the timeout and worker cap, rewrites
artifacts during verification, or blocks the reader threads.

**Fix:** always `killpg` after `wait()`.

### H6. Artifact checks can be bypassed or raced
`executor.py:1549–1597`. A FAILED response with artifacts skips confinement, digest, and
classification but still propagates and is charged. On success, each check re-resolves the path
(`is_file` → `stat` → `read_bytes` → `result_sink`), and `read_bytes` is unbounded.

**Fix:** refuse artifacts unless SUCCEEDED; open once with `O_NOFOLLOW`, `fstat`, one bounded read,
then hash/classify/sink the same bytes.

### H7. Budget accounting leaks and undercharges
- The executor catches only specific error types; `OSError`, a `result_sink` exception, or a
  reader-thread start failure after `Popen` leaks the reservation (and can orphan the child).
- The model gateway only *checks* budget; calls that fail identity or schema checks are never
  charged.
- `duration_seconds` is a float: reserve 0.1 / 0.7 then release in the other order raises
  `BudgetCounterError`; `import_state` can reject a state the ledger itself exported, stopping the
  launch.

**Fix:** one `try/finally` with a `committed` flag; real reserve/commit in the gateway; integer
milliseconds for durations.

### H8. Kernel state is not in the hash-chained journal, and persistence is best-effort
It is stored in `workflow_snapshots` (`application.py:2597`), which has no chain or append-only
triggers; deleting the newest row silently rolls back revocations and budget use.
`_save_kernel_state` swallows every error (`application.py:2537`). A fresh directory that sees one
`evaluate` and is killed before `close()` refuses every later launch with `CorruptKernelStateError`.
Contradicts §9.1, §9.7, §3.11 row 5.

### H9. A declared checkpoint can be skipped
`kernel/workflow.py:586–616`. `record_step_result(state, step, True)` applies a PRE_COMMIT
transition with no approval (reproduced). The product calls `run(state)`, which is safe, but the
public API is not.

### H10. Secrets reach the provider unredacted
`runtime/model_gateway.py:51–69`. The PEM rule replaces only the BEGIN line, so the key body is
sent. `\b` fails after `_`, so `OPENAI_API_KEY=…` and `DB_PASSWORD=…` pass through. The base64 rule
described in §12.2 does not exist.

### H11. Running the CLI and `serve` together splits policy and budget state
Each process holds its own in-memory `PolicyEngine` and `BudgetLedger`; the last snapshot wins. A
CLI revocation is invisible to a running server.

## Medium severity

- **O(n) journal reads.** Every read walks the whole chain: ~90 ms for `read(limit=1)` at 10k events;
  a workflow approve walks it 6+ times, missing NFR-08.
- **`TOOL_CLASS_CHANGE` is per call.** `action_digest` includes the arguments and workspace digests,
  so every call re-approves (contradicts §10.4).
- **Read-only tools fail under an unavailable strict profile** in `_build_sandbox_spec` instead of
  running (contradicts §3.6, §16).
- **The probe differs from the worker launch** (`-I` vs `-B -s` + `PYTHONPATH`) — root cause of
  RR-19. Some namespace controls are assumed rather than observed; "host filesystem invisible" rests
  on one canary in `$TMPDIR`.
- **Registration-time regex admission is shallow** (top two schema levels), and `register_tool()`
  skips it. Runtime still refuses bad patterns, but `oneOf:[bad, good]` silently passes.
- **`preexec_fn` in a multi-threaded parent** (two reader threads per call, up to 8 workers).
- **Invalidated workflow approvals are a dead end**; nothing re-mints an approval.
- **Restore overrides configuration**: grants and `RunBudget` limits come from the snapshot.
- **FreeCAD never runs the validated `ccx`**; `FemToolsCcx` picks its own binary, and relative PATH
  entries are accepted.
- **Unauthenticated GETs** expose the design store to every local user; `/health` returns
  `data_dir`; `/docs` is blank under the CSP and `/openapi.json` is an extra route.
- **uvicorn defaults** re-enable the `server` header and `proxy_headers` (contradicts §14.2).
- **Unexpected errors log full tracebacks** because Starlette re-raises after the 500 handler.
- **A failed second `serve` overwrites the running server's token** (written before bind).

## Architecture suggestions

1. **Unit-of-work + split `application.py` (~3,000 lines)** into `RevisionCommitService`,
   `KernelStateStore`, `WorkflowDriver`, `DomainRegistry`, with `KalaApplication` as thin wiring,
   plus a startup reconciler for half-done commits. Fixes H1, H2, H8.
2. **Kernel state in its own tables**, written in the operation's transaction; configuration stays
   authoritative for grants and limits.
3. **One process owns the data directory** (exclusive `flock`, or the CLI talks to the server).
   Serve over `<data_dir>/api.sock` (0600) and require the token on every method.
4. **Incremental journal verification** from a cached verified head; full walk at startup and in a
   `kala-agent verify` command; expose the head digest for anchoring.
5. **Tool-plane hardening, in order:** exec'd launcher (or `bwrap`) instead of `preexec_fn`, cgroup
   tracking; probe uses the worker's exact launch; strict mode default for side-effecting tools,
   refusing rather than degrading; key `TOOL_CLASS_CHANGE` on a class digest.
6. **Route reference evaluators through `ToolExecutor`** (open decision #12), with deterministic call
   ids recorded in provenance; dual-run for one release.
7. **One orchestrator.** Make `AgentCoordinator` the step executor under `WorkflowEngine`; put
   `ModelPlanner` behind `PlanValidator` and `PLAN_APPROVAL`, with capabilities from the catalog.
8. **Domain packages:** declare first-party-only as permanent for now (§3.1/G1); derive
   `ParameterSpec` from the Pydantic models; collapse the three near-identical package layouts
   behind one builder.
9. **Optional adapters via entry points**, loaded only when configured and only under the strict
   profile (open decision #15).
10. **Split large modules** along existing seams (`executor.py`, `protocol.py`, `model_gateway.py`);
    move `import httpx` to module top; remove `Settings.journal_path`.

## Report corrections needed

- §13.2 / AC-19: `enforce_capability_request` does not exist.
- §9.7: kernel state lives in `workflow_snapshots`, not journal events.
- §9.5: default `max_object_size` is 1,000,000,000, not 1,000,000.
- §9.6: stale digests do not mark the approval invalidated.
- §10.2: workers launch with `-B -s`; `-I` is forbidden.
- §10.4: tool-class approval is per call.
- §12.1: the gateway checks budget rather than reserving it.
- §12.2: no base64 redaction rule exists.
- §14.1: `/openapi.json` and `/docs/oauth2-redirect` are also served.
- §14.2: uvicorn re-adds the `server` header.
- §16: "reservation released on every failure path" and "read-only tools still run" are false.

## Implementation status (2026-09-27)

Suite: 1,250 tests pass (baseline 1,227); `ruff check`, `ruff format --check`, and `mypy --strict`
are clean.

| Finding | Status | Change |
|---|---|---|
| H1 | Fixed | `KalaApplication._unit_of_work()` commits approval rows, revisions, journal events, and kernel state in one transaction for prepare, decide, workflow run, checkpoint decision, and domain evaluation. A lost head CAS is journalled and raised after the unit commits. A failed unit restores the policy engine. |
| H2 | Fixed | `PolicyEngine`: REJECTED/ABORTED survive `invalidate`, evict last. `ApprovalRequestRepository.save` refuses to change a decided row (`ApprovalAlreadyDecidedError`); `prepare_design_revision` checks the store. |
| H3 | Fixed | Public `enforce_capability_request`; used by `run_reference_workflow` and `_evaluate_step`; a surplus grant to a capability-free run is refused. |
| H4 | Fixed | `EvaluatorImplementation.package_digest` covers every file under the evaluator's package plus enclosing package `__init__` files (`evaluator-implementation-digest-v2`). |
| H5 | Fixed | Process group is always killed before reaping; exit detected with `waitid(WNOWAIT)`. |
| H6 | Fixed | Tools may only report SUCCEEDED/FAILED; FAILED with artifacts is refused; kernel decision codes from a tool become `internal_error`; artifacts verified from one `O_NOFOLLOW` fd with a bounded read, sunk only after all verify. |
| H7 | Fixed | Executor releases on every exception; gateway reserves `prompt + max_output_tokens` and commits actuals or releases; durations summed on an integer-microsecond grid. |
| H8 | Partly | Kernel state is written inside each unit and write errors are no longer swallowed, so an unclosed process no longer blocks the next launch. Still open: kernel state is not hash-chained (`workflow_snapshots`). |
| H9 | Fixed | `record_step_result` refuses a successful checkpointed step without its approval. |
| H10 | Fixed | Whole PEM block (or to end of text), `Authorization`/`Proxy-Authorization` headers, prefixed names (`OPENAI_API_KEY`, `db-password`) in assignment and JSON forms; `is_secret_name` matches `authorization`. |
| H11 | Open | Needs architecture suggestion 3 (single owner of the data directory). |

Behaviour changes callers may notice: the gateway now needs token budget for the output cap up
front, and `is_secret_name("Authorization")` is now true. Medium findings, provenance on the
revision row, and architecture suggestions 2–10 are not yet implemented.
