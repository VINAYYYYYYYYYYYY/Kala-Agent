# Rust backend migration inventory — audit report

Read-only. No tracked file modified. Probes wrote only to `/tmp/opencode` and a temp data dir.
Host: Linux 7.2.5 x86_64, i5-13450HX (16 threads), 15.7 GB, CPython 3.14.8 (`.venv`), rustc/cargo
1.99.1. Windows/COM/NSIS/Windows-CI evidence: **none**. crates.io reachable (HTTP 200).

## 1. Verified facts that change the plan

- **No Rust exists.** No `Cargo.toml`, `Cargo.lock`, or `*.rs` anywhere in the repo, including the
  user-untracked `Rust+buid123/` (it is a Python tree copy differing from `src/` only by the portable
  envelope change). `rust-implementation.md` is design prose; its speedup numbers are unmeasured.
- **Two shippable HTTP services, not one.** `kala_agent.studio` (desktop, 9 routes, `/api/studio`)
  and `kala_agent.api` (core, 17 routes under `/api/v1` plus root `/health`). They share only the
  loopback predicate and (now) the error envelope; they share no database, token, or data dir.
- **Current gates all pass**: root 1460 passed / 21.05 s; Solid Edge bridge 171 passed + 1 live-skip /
  1.79 s; Solid Edge client 103 passed / 10.38 s; FreeCAD adapter 382 tests collect (not run here —
  needs FreeCAD). 1985 test functions total.
- **Frozen backend works**: `dist/kala-studio` answered `/api/studio/health` 200, startup 1.77 s,
  339 MB RSS. Source launch 724 ms / 322 MB. `import OCP` alone is 0.49 s / 282 MB: desktop startup
  and RSS are dominated by a dependency Rust cannot absorb.
- **The Python tests are a usable parity corpus**, but they assert relationships and tolerances, not
  frozen bytes (except STL structure). A golden corpus must be generated before any port, or
  "parity" is unfalsifiable.

## 2. Contract inventory (what must not move)

### Studio service — 9 routes
`GET /api/studio/{health,integrations,providers,settings}`, `POST /api/studio/{settings,preview,export,handoff,chat}`.
- Flat error envelope `{code,message,details}` (`details` ≤16 entries, each ≤256 chars).
- Host guard **421** `invalid_host`; same-origin guard **403** `cross_origin_denied`; bearer on
  POST/PUT/PATCH/DELETE **401** + `WWW-Authenticate: Bearer realm="kala-studio"`; body limit
  256 KiB **413**; unknown route **404** `not_found`; unhandled **500** with no detail.
- Fixed security header set on every response including refusals; `server`/`x-powered-by` stripped.
- `preview` returns `positions/normals/indices/bounds/engine:"analytic"/warnings`; `export`/`handoff`
  return raw bytes with `content-disposition` and, for handoff, `x-studio-handoff`.
- Static middleware: GET/HEAD only, `/api/*` reserved, SPA fallback to `index.html`, no traversal.
- `health` includes `step_available`, which is a live `import OCP` probe.
- Launch contract: `--host 127.0.0.1 --port <n>`, `KALA_STUDIO_TOKEN`, readiness within 15 s,
  SIGTERM then SIGKILL at 3 s, packaged path `resources/backend/kala-studio[.exe]`.
### Core service — 18 routes
`GET /health`; `/api/v1/domains`, `/domains/{d}`, `POST /domains/{d}/evaluations`; `POST /revisions`,
`POST /revision-commits`, `GET /revision-commits/{id}`, `POST .../approve|reject`, `GET /revisions`,
`GET /revisions/{id}`; `POST /workflows/mechanical/reference`, `GET /workflows/{id}`,
`GET /workflows/{id}/checkpoints`, `POST .../approve|reject`; `GET /events`.
- **Different envelope**: `{error:{code,message},details}`. Do not unify.
- `201` vs `200` distinguishes started/resumed; `location` headers are asserted.
- Error→status table: 404 unknown domain/revision/workflow/prepared; 409 checkpoint/fork/head-moved/
  digest-mismatch/finished; 422 invalid identifier/parameters; 503 corrupt kernel state/closed.
- Validation errors expose Pydantic `loc: type: msg` strings in `details`.

### CLI — 4 binaries
`kala-agent` (init, token, serve, domains, evaluate, run-mechanical, workflow-status, approve, reject,
prepare-revision, commit-revision, events) prints one JSON document `{command,data,status}` or
`{error:{code,message}}`, exit 0/1/2/3. `kala-studio` prints a JSON launch banner then serves.
`kala-se-bridge` (token/probe/serve) and `kala-solidedge` (doctor/build/status/cancel) are separate
distributions with their own contracts.

### Data formats (highest-risk surface)
- SQLite **schema v2**, 10 tables (`events`, `design_specs`, `design_revisions`, `approval_requests`,
  `tool_calls`, `tool_results`, `memory_entries`, `workflow_snapshots`, `schema_migrations`,
  `sqlite_sequence`), 2 migrations, append-only UPDATE/DELETE triggers, chained-insert trigger.
  Dumped `sqlite_master` digest today: `0b1f499c…61bc1`.
- Journal chain: domain `kala-agent.event-chain.v1.sha256`, 8-byte big-endian length prefixes on every
  part *including the domain*, genesis `"0"×64`, payload digest = SHA-256 of canonical `payload_json`,
  `workflow_id or ""` collapses absent and empty. Sample chain digest: `dea99979…e8291b`.
- Canonical JSON: sorted keys, no whitespace, `ensure_ascii=True`, `allow_nan=False`, duplicate-key and
  non-finite rejection, depth 48, ≤1 MiB.
- Timestamps: `YYYY-MM-DDTHH:MM:SS.ffffffZ`, parsed with `fromisoformat` (accepts space separator and
  truncates 7-digit fractions).
- Artifacts: `objects/<2 hex>/<digest>`, hard-link publication, `0o700`/`0o600`, `O_NOFOLLOW`,
  digest re-verified on read.
- Config: 7 `KALA_AGENT_*` fields, `KALA_STUDIO_{TOKEN,DATA_DIR,DIST,PYTHON,ENABLE_CODEX}`,
  `OPENAI_API_KEY`/`OPENROUTER_API_KEY`; bridge adds `KALA_SOLIDEDGE_*`.
- Solid Edge bridge: bearer on **every** route including `GET /v1/health`; exact loopback authority;
  `Origin` rejected; 256 KiB; states queued/running/succeeded/failed/cancelled/indeterminate/
  interrupted; `request_digest` excludes `request_id`; ≤18 artifacts.

## 3. Measured baseline (this host, today)

| Workload | Result |
|---|---|
| Journal append | 38.7–41.9 µs/event; 20,160 events/s single thread; 11,594/s at 8 threads |
| `verify_chain` | 0.54 ms @100 events, 10.27 ms @2000 |
| `journal.get()` | 2.84 / 5.63 / 11.17 / 22.2 / 44.7 ms @500/1k/2k/4k/8k — **linear O(N)** |
| Artifact CAS | put 1 MiB 1.4 ms; get 0.67 ms; 8-thread 64 KiB 7,359 obj/s |
| Mesh build | box 0.035 ms; sphere 2.56 ms; sphere-max (24,320 tri) 43.3 ms; cyl-max (33,280 tri) 57.7 ms |
| Binary STL | sphere-max 14.8 ms, 1,216,084 B; SHA-256 `b7b27e47…32ea61` |
| Canonical JSON | 759 B payload: dump 0.007 ms, digest 0.008 ms |
| Domain eval | mechanical 0.386 ms |
| Studio HTTP | health 0.90 ms (p95 1.06); preview box 0.84 ms; preview sphere-max 49.6 ms / 1.80 MB; STL export 0.75 ms; local chat 0.90 ms; invalid 0.67 ms |
| Studio gates | 421 bad host, 403 bad origin, 401 no token, 404 unknown — all confirmed |
| STEP export | first 431.6 ms (OCP import), then 2.2–2.7 ms; 15,492 B |
| Core | app startup 2.2 ms; run 3.4 ms; approve 7.4 ms; revision read 0.36 ms; events 1.14 ms |

Interpretation: the Python backend is **already fast at default sizes**. Health/preview/export latency
sits at the HTTP client's measurement floor. Defensible Rust wins are (a) startup/RSS, only if OCP
leaves the process; (b) max-tessellation geometry and STL; (c) O(N) journal reads and writer
contention; (d) concurrency. A "10–100×" API claim is not supported by these numbers.

## 4. Parity risks, ranked

1. **Canonical JSON float/Unicode formatting is not `serde_json`'s.** Measured divergence:
   Python `1e-05`/`1e+16`/`1e-07` vs serde `0.00001`/`1e16`/`1e-7`; Python `"\u03ba…"` and
   `"\ud83d\ude00"` vs serde raw UTF-8. Every payload digest, chain digest, artifact digest, spec/
   state/action digest and package digest changes if this is wrong. Rust needs a custom CPython-`repr`
   float formatter (shortest round-trip, exponent if exp < −4 or ≥ 16, two-digit exponent, explicit
   `+`, `.0` for integral) and an ASCII escaper.
2. **Arbitrary-precision ints.** Python emits `2**70` exactly; `serde_json` needs
   `arbitrary_precision` or a bigint path, or the value silently becomes `f64`.
3. **Decimal scale.** `Decimal("0.10") + Decimal("0.2") = 0.30` and `max_cost` exports as `"1.00"`.
   Fixed-point micro-cents would change stored kernel-state JSON and therefore snapshot digests.
   `rust_decimal` preserves scale; use it, not integers.
4. **Byte-canonical rows.** Journal/snapshot readers reject non-canonical stored JSON
   (`payload_json != row["payload_json"]` → error). A Rust writer and the Python reader must agree
   byte-for-byte during any coexistence window. This is the cutover gate, not a detail.
5. **Domain package digests are source-file digests.** `package_source_digest()` hashes the Python
   source tree; Rust cannot reproduce `a6d1936…`/`97a2eba…`/`108cd68…` from Rust sources. These values
   are returned by `GET /domains`, stored in revisions, and committed by state digests. Either pin them
   as versioned constants (one-time provenance note) or bump `implementation_digest_version` and accept
   a visible digest change. Silent drift is unacceptable.
6. **Pydantic error text.** 422 `details` include `loc: type: msg`. Reimplementing exact strings is
   expensive and brittle; freeze a request→(status, body) corpus instead.
7. **Banker's rounding.** `round()` is half-to-even; Rust `f64::round` is half-away-from-zero. Use
   `round_ties_even`. Affects `clamp_unit_interval`, simulator metrics, durations.
8. **POSIX-only file security.** `O_NOFOLLOW`/`openat`/`dir_fd` has no direct Windows equivalent. The
   product runtime is Windows, so the Rust CAS needs handle-relative opens with
   `FILE_FLAG_OPEN_REPARSE_POINT` + final-path containment, or the symlink property degrades silently.
9. **Sandbox.** `sandbox.py` is Linux-namespace-only and already unavailable on Windows; strict
   isolation is not a product feature today. Port the portable profile (process-group kill, wall clock,
   output caps, minimal env, cwd confinement) and keep strict-Linux as dev-only or document its removal.
   Do not claim namespace isolation on Windows.
10. **OCP STEP is unavoidable and non-deterministic.** `cadquery-ocp 8.0.1.1.0` is a C++/Python
    binding; STEP files carry a timestamp header, so parity must be structural (prefix, valid B-rep,
    volume/area tolerance), not byte equality. Keep it as a supervised sidecar and say so.
11. **pywin32/COM.** `pywin32` is absent on this host and no production executor exists (builds fail
    `unavailable`). Rust can own the bridge wire/job ledger now; COM remains a separate Windows-gated
    deliverable. Do not call a Rust bridge with a Python COM worker a "full Rust rewrite".
12. **Docs drift found:** `KALA_STUDIO_MESH_LIMIT` is documented in `docs/studio-limitations.md` but is
    not implemented anywhere. Fix the doc or implement the knob before freezing contracts.

## 5. Target architecture

Standalone Rust binaries, one workspace, no PyO3 in the product path:

```
backend/            (single Cargo.lock owner)
  crates/kala-json          canonical JSON, floats, bigint, SHA-256, chain preimage
  crates/kala-contracts     serde models + validators mirroring the frozen Pydantic surface
  crates/kala-persistence   rusqlite (bundled), schema v2, journal, CAS, repositories
  crates/kala-kernel        budget (Decimal), policy, workflow, memory, planning
  crates/kala-domains       3 packages + evaluators + pinned digests
  crates/kala-tools         protocol, registry, executor, simulator
  crates/kala-runtime       gateway, planner, agent
  crates/kala-core-api      axum core routes (18)
  crates/kala-studio-api    axum studio routes (9), geometry, STL, providers, static
  crates/kala-cli           clap, exact JSON/exit codes
  bins/kala-studio, kala-agent
```

Minimal stack, versions on crates.io today (all MSRV ≤ 1.99): `axum 0.8.9`, `tokio 1.53.3`,
`serde 1.0.229`, `serde_json 1.0.154` (+`arbitrary_precision`), `rusqlite 0.40.2` +
`libsqlite3-sys 0.38.2` (`bundled`), `sha2 0.11.1`, `clap 4.6.7`, `chrono 0.4.45`,
`rust_decimal 1.43.0`, `subtle 2.6.1`, `regex 1.13.1`, `tower-http 0.7.1`, `mime_guess 2.0.5`,
`getrandom 0.4.3`, `tempfile 3.27.0`, `thiserror 2.0.21`; dev: `criterion 0.8.2`/`divan 0.1.21`,
`proptest 1.12.0`, `insta 1.49.0`.

Local cargo cache: 228 crates, including tokio 1.46.1, serde 1.0.219, serde_json 1.0.140,
hyper 0.14.32, regex 1.11.1, chrono 0.4.41, reqwest 0.11.27. **Not cached**: axum, rusqlite,
clap, sha2, rust_decimal — first build requires network (reachable now). Vendor a lockfile + `cargo
vendor` once, then build offline. `cc`/`gcc`/`pkg-config`/`sqlite3.h` are present, so `bundled` is safe.
No Windows target/linker installed; Windows binaries must be built on Windows.

Deliberately rejected for now: lock-free/warm-worker/merkle-incremental designs from
`rust-implementation.md`. They are unmeasured and add risk. Start with one serialized writer, WAL, and
add a read pool only if a measured benchmark demands it.

## 6. Phased plan, ownership, first vertical

**W0 — contract freeze (1 cell, read-only).** Generate a golden corpus from the current Python code:
canonical JSON vectors (floats, Unicode, bigint, depth, duplicate keys), chain digests, STL SHA-256 for
five meshes, `sqlite_master` dump + migration rows, HTTP route→status→body corpus for both services
(including every negative gate), CLI outputs, and the three package digests. Store as fixtures, not
code. This cell also owns `backend/Cargo.toml` + `Cargo.lock`.

**W1 — `kala-json` + `kala-contracts` (bounded, ≤200k).** Gate: byte-identical corpus. Highest risk,
so it must land before anything that hashes.

**W2 — `kala-persistence` (Go/high-context).** Open a **Python-created** database, verify the chain,
append, close, re-open with Python and verify. Gate: schema dump equality, cross-reader acceptance,
CAS symlink/concurrency tests.

**W3 — `kala-studio-api` (bounded, ≤200k).** 9 routes, middleware, geometry, STL, providers, settings,
static. Gate: corpus + Electron launch + golden STL hashes. **This is the first vertical deliverable**:
it is self-contained (no SQLite, no kernel), proves the desktop contract, the security gates, and the
geometry parity, and can ship behind the existing Electron shell without touching user data.

**W4 — `kala-kernel` + `kala-domains` (Go).** Budget/policy/workflow + 3 packages; pin or version the
package digests per §4.5.

**W5 — `kala-core-api` + `kala-cli` (Go).** 18 routes + 12 CLI commands, exact envelopes/exit codes.

**W6 — `kala-tools` + `kala-runtime` (Go).** Protocol, registry, executor, simulator, gateway. Keep the
portable profile only; strict-Linux stays out of the product.

**W7 — packaging + cutover + benchmarks (1 cell).** `kala-studio` replaces the PyInstaller onefile at
`resources/backend/`; core binary replaces the Python entry point. STEP sidecar remains Python+OCP.

Second vertical (immediately after W3): **core read-only over a Python-created DB** — `/health`,
`/domains`, `/domains/{d}`, `/events`. It is the only test that proves canonical JSON + chain + SQLite
compatibility against real data, and it is small.

Cells that fit 200k: W1, W3, CLI, single-domain port, benchmark harness. Cells needing Go context:
persistence, kernel, domains, core API, tools/runtime.

## 7. Benchmark plan (fair, reproducible)

Baseline commands that work today (all executed in this audit):
```bash
.venv/bin/python -m pytest -q                     # 1460 passed, 21.05 s
.venv/bin/python /tmp/opencode/probe_baseline.py  # mesh/STL/JSON/CAS/journal
.venv/bin/python /tmp/opencode/probe_concurrency.py
.venv/bin/python /tmp/opencode/probe_http.py      # startup, RSS, p50/p95, gates
```
Comparison rules: same host, same payloads, same HTTP client, release build only, ≥5 repeats,
interleaved A/B, report median + p95 + min/max, `nproc` and load average. Separate microbenchmarks
(criterion/divan) from end-to-end. Correctness gates run **with** every timing: STL SHA-256, canonical
JSON digest, chain head digest, HTTP body equality, and the negative-gate status codes. Do not compare
the 1.77 s frozen Python binary to a debug Rust build, and do not present health-endpoint deltas as a
speedup — both are at the client floor. The honest headline metrics are startup/RSS (with and without
OCP in-process), max-tessellation mesh/STL time, `journal.get()` vs N, and multi-writer throughput.

## 8. Prerequisites and gaps

Present: rustc/cargo 1.99.1, crates.io reachability, gcc/cc/pkg-config/sqlite3 headers, Node 26,
PyInstaller 6.22.3, OCP 8.0.1.1.0. Missing: any Windows host or MSVC target (COM, NSIS, Windows CI
unproven), `pywin32`, a vendored Cargo registry. Not done here: installs, commits, pushes, delegation,
edits to Python sources or to `Rust+buid123`/`rust-implementation.md`.

**Bottom line.** A standalone Rust backend for both services is feasible and the contracts are
enumerable, but the migration is gated by four things that are not "write more Rust": byte-exact
canonical JSON (CPython float/Unicode formatting, bigints), scale-preserving Decimal, domain-package
digest provenance, and two unavoidable native sidecars (OCP STEP, Solid Edge COM). Start with the
studio vertical behind the existing Electron launch contract, then prove the core over a
Python-created database before any write path is cut over.
