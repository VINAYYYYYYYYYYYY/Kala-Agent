# Python compatibility corpus v1

Frozen reference output of the current Python backend, generated for the
planned standalone Rust backend. This is **compatibility-fixture preparation
only**. It is not a Rust implementation, not a packaging cutover, and not
evidence that Windows/COM or a Windows milestone has passed.

## Layout

```
backend/fixtures/corpus/v1/
  manifest.json              source/oracle pin, coverage, file hashes, corpus digest, gaps
  canonical_json.json        66 dump/load cases and rejection boundaries
  event_chain.json           5 chain preimages + schema-v2 logical dump
  schema_v2.sqlite3          real Python-created schema-v2 SQLite database
  decimal_budget.json        85 Decimal/budget/duration/clamp cases
  geometry_stl.json          6 meshes, bounds, array digests, STL/STEP records
  stl/                       byte-exact default box/cylinder/sphere binary STL
  domain_provenance.json     LEGACY Python package/source digests
  cli_launch.json            command/option/exit-code/launch contract
  studio_http.json          16 Studio routes + gates, 59 cases
  core_http.json             17 live core routes + gates, 49 cases
```

`manifest.json` is the index. It records the exact oracle environment
(CPython 3.14.8, fastapi 0.141.1, pydantic 2.13.5, starlette 1.7.0, OCP
8.0.1.1.0, SQLite 3.54.0), the git commit, the dirty path list, and a
SHA-256 for every product Python source file.

It also carries **`corpus_sha256`**: the SHA-256 over the sorted
`(path, digest)` list of the twelve indexed fixture files, with the SQLite
file's entry recorded as `not-asserted` (its bytes are not asserted; its
logical rows are, in `event_chain.json`). `self_check.py` recomputes it from
the manifest's own file list and refuses a mismatch. The current corpus
digest is

```
39e0c0a90f9d03704945c79cdb9e777e77549ef208ac2177d3f7bb003dade3fa
```

The corpus was regenerated on 2026-10-20 by the `compat-drift` cell. That
regeneration moved the Studio corpus from **9 routes / 48 cases** to
**15 routes / 56 cases** (eight new cases covering the six routes the
design-turn wave added) and refreshed the core corpus, the CLI launch
contract and the source pin. Every hunk was classified: the route table
growth, the `design_available`/`design` response fields, the Studio CLI
`--worker` flag and the source-pin growth are **accepted changes**; the
STEP writer's timestamp, the OCCT translator's per-process counter, the
temporary data directory in the core `/health` body and the Studio journal's
wall clock are **environment artefacts**; there was **no unexplained
difference**.

The `polish` cell regenerated it again on 2026-10-20 after the design
instruction grew a sketch-coordinate/pocket/step-limit paragraph in
`src/kala_agent/studio/design/intent.py` and the OCCT compiler began naming
the pocket step index in its no-material warning (both product-source
changes move the source pin). The same run absorbed the concurrent
`python-parity` cell's provider-list rows and Codex wording, which had
already been visible as differential drift. The digest moved from
`53dcb9da…` to `61328f91…`; the Studio corpus is still 56 cases / 15 routes.

The `corpus-hermetic` cell regenerated it on 2026-10-21 without a product
change: `studio_http.json` is byte-identical and the source pin did not move.
The one functional change is the core `/health` case, whose raw length embeds
the temporary data-directory path. `body_length` and the body digest are now
taken over the normalised bytes, and the `content-length` header is declared
in `normalised_headers` instead of being frozen, so `--check` no longer
depends on the length of the `TMPDIR` the generator is given. The digest
moved from `c0003bae…` to `39e0c0a9…`; the Studio corpus is 59 cases over 16
routes and the core corpus 49 cases over 17 routes.

## Consumption rules for a Rust implementation

1. **Canonical JSON is not JCS and not plain `serde_json`.** Use
   `canonical_json.json` to validate the CPython formatter: sorted keys,
   `,`/`:` separators, `ensure_ascii=True`, `allow_nan=False`, CPython float
   exponent thresholds/sign/padding, `-0.0`, integral floats as `1.0`,
   arbitrary-precision integers, escaped lone surrogates, duplicate-key
   rejection, depth 48, string 32,768, collection 2,048, key 256, and the
   1 MiB byte limit.
2. **Event digests cover a length-prefixed domain and eight parts.**
   `event_chain.json` gives the domain, 8-byte big-endian length prefixes,
   genesis all-zero digest, covered fields, and five exact preimages. Payload
   digest is SHA-256 of the canonical `payload_json` bytes.
3. **The SQLite fixture is a real database, not a schema sketch.**
   `schema_v2.sqlite3` contains 10 tables including `sqlite_sequence`, 2
   migrations, 3 triggers, 5 linked events, 1 snapshot, and 1 design spec.
   Open it with the Python reader to verify the chain head
   `143fed75…99fc7` before comparing. Do not compare the file bytes across
   SQLite library versions; compare logical rows and `sqlite_master`
   normalized SQL.
4. **Decimal is arbitrary-precision and scale-preserving.** `rust_decimal`'s
   96-bit/28-scale domain is insufficient. The corpus includes `1.00`,
   `0.10 + 0.2 = 0.30`, `1e-999999`, `1e+999999`, 38-digit coefficients,
   context-rounded sums, and ties-to-even microsecond durations.
5. **The two HTTP envelopes are different.** Studio uses flat
   `{code,message,details}`; core uses nested `{error:{code,message},details}`.
   Do not unify them. Every recorded response includes the security header
   set and asserts `server`/`x-powered-by`/`set-cookie` are absent.
6. **HTTP request records are replayable.** Each case has the exact method,
   path, explicit headers (including the synthetic bearer token when used),
   and a tagged body (`json`, `raw_bytes`, or `chunked`). The default Host is
   `127.0.0.1`. Bodies up to 64 KiB are stored exactly with `body_text`;
   larger bodies keep the exact SHA-256 and a structural summary. A case with
   `normalizations` (the core `/health` `data_dir`) is frozen by its
   **normalised** bytes: `body_text`, `body_sha256` and `body_length` all
   describe the same normalised document, and `normalised_headers` names the
   headers (today `content-length`) that a consumer must check against its own
   response body instead of against a recorded number.
7. **STL defaults are byte-exact.** The three small default files are stored.
   Maximum-tessellation STLs (box 2,457,684 B; cylinder 1,664,084 B; sphere
   1,216,084 B) are represented by SHA-256 and length; regenerate the mesh and
   compare. The sphere-max digest begins `b7b27e47…`, matching the migration
   audit's measurement.
8. **STEP is structural only.** The header carries a write timestamp. Check
   ISO prefix, header/data sections, `MANIFOLD_SOLID_BREP` count, and length.
   Never compare STEP bytes for equality.
9. **Domain package digests are LEGACY Python source identity.**
   `domain_provenance.json` records `building`, `mechanical`, and `wing`
   package digests plus source-file digests. A Rust implementation cannot
   reproduce these from Rust sources. Pin them as versioned constants or bump
   `implementation_digest_version` and accept a visible change. Do not present
   them as Rust source identity.
10. **CLI contract is command/exit-code first.** `cli_launch.json` records 12
    core commands, option contracts, exit codes 0/1/2/3, representative
    output, Studio/Electron `--host`/`--port`/`KALA_STUDIO_TOKEN` promises,
    and launch target validation. The serve command is not started.
11. **The Studio corpus covers 16 routes and 59 cases; the core corpus 17
    routes and 49 cases.** A Studio case is a **record of a run**, not
    always a contract: read `body_deterministic` and `volatile_fields` on
    every case before you compare it. Three of the four cases the 2026-10-20
    regeneration added are `body_deterministic: false`, and the fourth
    (`studio.cad_providers`) is **host evidence** — it records the CAD
    provider probe of the generating machine. The `journal.page` case is a
    **session** record: it holds the events the earlier cases of the same
    generation run produced, so inserting a Studio case before it changes
    the recorded page. That coupling is deterministic and documented, but
    it is worth knowing before a case is inserted.

## Determinism

- JSON fixtures are byte-identical on the same source tree. Two consecutive
  generations into two different directories are byte-identical in **all
  thirteen** files.
- The SQLite file bytes are informational; logical rows are asserted.
- STEP bytes are informational; structural fields are asserted.
- Core `/health` `data_dir` and CLI temporary paths are `<DATA_DIR>`.
- **A normalised body's length follows the normalised bytes.** For the core
  `/health` case, `body_text`, `body_sha256` and `body_length` describe the
  same `<DATA_DIR>` document, and the `content-length` header is listed in
  `normalised_headers` rather than frozen, because the raw response length
  embeds the length of the temporary data-directory path. The generator is
  therefore independent of the `TMPDIR` it is given: `--check` passes with
  `TMPDIR=/tmp` and with a path more than twice as long.
- Journal timestamps are wall-clock in production. The HTTP and CLI fixtures
  inject a fixed clock into the journal module so event timestamps and digests
  are reproducible. This is a fixture override, recorded in the files.
- **The Studio session journal gets the same fixed clock.** The Studio
  fixture harness injects it into `kala_agent.studio.session_journal.datetime`,
  exactly like the core-journal override, so the Studio events and their
  digests are reproducible. This is a fixture-only override; the product
  code is untouched.
- **The STEP writer's bytes are normalised before they are digested.** The
  `FILE_NAME` header timestamp is replaced with the fixed clock value
  `2026-01-01T00:00:00` (the replacement keeps the byte length, so the
  recorded length is still the length of the real response) and the OCCT
  translator's per-process product counter is replaced by zeros of the same
  width, because OCCT numbers the translated product per process. The
  recorded `body_sha256` is therefore the digest of the **normalised** bytes,
  and the manifest says so in the role of that field.
- **The design result's STEP digest and the journal digests that bind it
  are recorded as `<VOLATILE>`**, with the reason next to them: the OCP STEP
  export embeds the wall clock of the write, so its content digest changes
  on every run. The reproducible relations are frozen instead — the genesis
  link, the link from every event to its predecessor, the digest shape and
  the chain verdict — and `self_check.py` refuses a recorded journal chain
  that is broken, a verdict that is not `ok`, or a head that does not match
  the last event.
- **The Studio provider key is synthetic** and is cleared in the same run.
- **The working-tree path list is recorded but not compared.** It names every
  unrelated file in the repository, so it changes whenever any other worker
  edits anything. The commit, the product source pin, the per-file digests
  and the corpus digest are all still compared.
- No timing or RSS is recorded. Benchmarks are deferred until a release Rust
  build exists and the host is quiescent.

## Verify

```bash
.venv/bin/python backend/compat/generate_corpus.py --check
.venv/bin/python backend/compat/self_check.py
```

`self_check.py` regenerates the whole corpus **in a temporary directory** and
compares it with the checked-in files, so it never writes into
`backend/fixtures/`. Observed on 2026-10-21: **exit 0, `self-check OK`**, with
`corpus_sha256 39e0c0a9…` (after the `corpus-hermetic` cell's normalised
`/health` length; previously `c0003bae…`), 59 Studio cases / 16 Studio routes
and 49 core cases / 17 core routes. `generate_corpus.py --check` passed with
`TMPDIR=/tmp` and with an 82-character `TMPDIR` (a 126-character generator
data directory), which previously differed by 78 bytes in the `/health`
response.

## Known gaps

See `manifest.json` `known_gaps`. The headline ones:

- The live core route table has 17 routes including root `/health` (16 under
  `/api/v1`), not the 18 stated in the audit/task text. All 17 are covered;
  the extra route is not in the current code.
- Windows/COM/NSIS/CI is not exercised and remains a gate.
- STEP is captured for the default box only; the 503 branch is not captured
  because OCP is installed.
- HTTP uses Starlette `TestClient`, not a live uvicorn socket.
- Static traversal/symlink edge cases remain in the Python security tests.
- `KALA_STUDIO_MESH_LIMIT` is documented but not implemented.
- **The CAD-provider probe is host evidence.** `studio.cad_providers` records
  the probe result of the machine that generated the corpus, including the
  FreeCAD refusal reason, which changes with the host's FreeCAD
  installation. The same probe is recorded in
  `manifest.oracle_environment.cad_providers`, so a reader can tell a host
  fact from a fixture value. The corpus still marks the case
  `body_deterministic: true`, which is **wrong** — a replay on a host where
  FreeCAD works answers differently (542 bytes instead of the recorded 783),
  and `crates/kala-studio/tests/replay_binary_corpus.py` reports that as a
  mismatch.
- **The six routes the design-turn wave added are frozen by the corpus, not
  by the 268-case live differential.** The Rust front door implements nine
  route-methods natively and proxies the design and journal routes to the
  Python worker (ADR-0010), so the native-vs-Python byte parity of those
  routes is still an open item of the delivery cell.
