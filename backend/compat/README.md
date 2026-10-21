# Compatibility corpus generator

This directory contains the Python reference generator for the Rust backend
migration. It imports the live `kala_agent` implementation and records what
that implementation actually produces. It does not contain Rust product code,
a Cargo workspace, or a packaging cutover.

## Commands

Run from the repository root, using the repository virtual environment:

```bash
# Regenerate the checked-in corpus.
.venv/bin/python backend/compat/generate_corpus.py

# Regenerate into a temporary directory and compare with the checked-in corpus.
# SQLite file bytes and STEP bytes are treated as volatile; every JSON fixture,
# logical database row, and STL digest must match.
.venv/bin/python backend/compat/generate_corpus.py --check

# Structural validation, Python reader verification of the SQLite fixture,
# STL byte checks, and a full regeneration comparison.
.venv/bin/python backend/compat/self_check.py

# Faster structural-only pass (no regeneration).
.venv/bin/python backend/compat/self_check.py --skip-regenerate
```

The corpus is written to `backend/fixtures/corpus/v1`. `--check` never writes
to the checked-in fixture directory. It is independent of the temporary
directory it is handed: the fixture records the normalised `/health` body
length, so the check passes with `TMPDIR=/tmp` and with a much longer path.

## Corpus digest

`manifest.json` carries `corpus_sha256`: the SHA-256 over the sorted
`(path, digest)` list of the twelve indexed fixture files. The SQLite
fixture's file bytes are recorded as `not-asserted` in that list, because the
SQLite library version changes them; its logical rows are asserted separately
in `event_chain.json`, which is covered. `self_check.py` recomputes the
digest from the manifest's own file list and refuses a mismatch.

| Corpus | `corpus_sha256` |
|---|---|
| before this cell's regeneration (2026-10-19 15:58) | `685d10d17c12ecb6b89afa19756d8d98865203f9cb479054a8f143016a015175` |
| after compat-drift's regeneration | `9993939bfea5e01553438e680fe3a9ea4b24335ee9077da5b8dc453b5a987e8b` |
| after corpus-sync (per-case `body_comparison`, `volatile_paths`, `host_dependent`) | `53dcb9da105589b5a92d37b16d63c7fe2d5e60b7630549102127e019c95dff28` |
| after polish (design instruction paragraph, per-pocket warning, concurrent provider rows) | `61328f9143238ca85eeabc99d8ccd9c983ead0f1481ee491573e047ab52367ee` |
| after design-budget (500/64 step budget, exact cut count, 512 KiB tool string) | `e86c7d93157fb844af0e4cf8711824df4e75a782ac9f88b94318d57f21df0f6b` |
| after downloads-cancel (`design_turn_budget_seconds`, artifact download cases) | `c0003bae333f27a079adcf3e86f2618741a6606334a17ffe3fafe727fcd6c330` |
| after corpus-hermetic (normalised `body_length`/`content-length` for `/health`) | `39e0c0a90f9d03704945c79cdb9e777e77549ef208ac2177d3f7bb003dade3fa` |

The corpus-hermetic regeneration changes no fixture content except the core
`/health` length fields and the two metadata tables that explain them: the
corpus digest moved from `c0003bae…` to `39e0c0a9…`, `studio_http.json` is
byte-identical, and the product source pin did not move. Before it, `--check`
failed on a clean run whenever the generator's temporary directory was
shorter than the one that produced the committed fixture: the raw `/health`
body embeds the temporary data-dir path, so `body_length` and the frozen
`content-length` header grew with the path. The recorded length is now the
length of the normalised bytes (the same bytes `body_sha256` covers), and the
`content-length` header is declared in `normalised_headers` for a
self-consistency check instead of being frozen. `--check` therefore passes
under any `TMPDIR` length.

The generator command for both is
`.venv/bin/python backend/compat/generate_corpus.py`.

The corpus was not committed before this cell ran (the whole `backend/` tree
is untracked), so the "before" digest is reconstructed from the per-file
digests the pre-regeneration `manifest.json` recorded. Those digests are
listed in this cell's `REPORT.md`, so the reconstruction can be recomputed.

## Modules

| Module | Freezes |
|---|---|
| `corpuslib/canonical_json_cases.py` | `tools.protocol` canonical dump/load, floats, Unicode, big integers, depth/size limits, duplicate keys, non-finite values |
| `corpuslib/chain_cases.py` | chain preimages and a real schema-v2 SQLite database built by `Database`, `EventJournal`, `SnapshotStore`, and the repositories |
| `corpuslib/decimal_cases.py` | unbounded finite `Decimal` representation, scale preservation, context rounding, microsecond duration grid, ties-to-even, budget ledger |
| `corpuslib/http_cases.py` | all live Studio and core routes, request/header/body gates, valid flows, error envelopes, static serving, STL/STEP responses |
| `corpuslib/geometry_cases.py` | default and maximum tessellation meshes, bounds, normals/indices digests, watertightness, binary STL bytes/digests, structural STEP |
| `corpuslib/provenance_cases.py` | LEGACY Python package/source digests for the three domain packages |
| `corpuslib/cli_cases.py` | command inventory, option contracts, exit codes, representative command output, Studio/core launch targets |

## Oracle rules

1. **No hand-written expected values.** Every digest, serialized body, status,
   error type, and byte length is produced by calling the current Python code.
2. **Inputs keep their type.** A JSON number is stored as JSON text or as a
   tagged typed source (repeat string, nested arrays, float literal, and so
   on), so integers, floats, strings, and Decimal values cannot collapse.
3. **The source tree is pinned.** `manifest.json` records the git commit and
   SHA-256 of all 95 product Python files at generation time. Regenerate after
   any source change; the corpus is not a cross-revision comparison. The
   working-tree path list is recorded for provenance but is excluded from the
   comparison (see `determinism.excluded_from_comparison`), because it names
   every unrelated file in the repository and changes whenever another worker
   edits anything.
4. **Timestamps are deterministic.** The application clock is injected.
   `EventJournal.append` and `StudioSessionJournal.append` use
   `datetime.now(UTC)` in production, so HTTP and CLI generation temporarily
   substitute a fixed clock for `kala_agent.persistence.journal.datetime` and
   for `kala_agent.studio.session_journal.datetime`. The hashing and
   serialization code is unchanged. This override is recorded in the fixtures.
5. **SQLite bytes are not the contract.** The database fixture is a real
   SQLite file, but its bytes can vary with the SQLite library. Logical rows,
   `sqlite_master` normalized SQL, triggers, migrations, event digests, and
   the verified chain head are the contract.
6. **STEP bytes are not the contract, and the writer's own run-specific
   values are normalised.** The header embeds a write timestamp, and OCCT
   numbers the translated product with a per-process counter, so the same
   export inside one process differs from the first one. The corpus replaces
   the timestamp with the fixed clock value and the counter with zeros of the
   same width before it takes a digest, and records both replacements in
   `manifest.determinism.normalisation`. STEP is frozen structurally: ISO
   prefix, header lines, timestamp presence, `MANIFOLD_SOLID_BREP` count, and
   length. The length is the length of the real bytes; only the digest is
   taken over the normalised bytes.
7. **No external model calls.** Studio provider cases use the deterministic
   local provider or refused/unconfigured paths. The synthetic provider key in
   the fixture is not a live credential and is cleared in the same run.
8. **Temporary paths are normalized, together with everything derived from
   them.** Only the core `/health` `data_dir` and CLI temporary paths are
   replaced with `<DATA_DIR>`. Every other byte is preserved; meaningful
   differences are not normalized away. The core `/health` body digest **and
   `body_length`** are taken over the normalised bytes, because the raw bytes
   embed the temporary path. The raw `content-length` header is not frozen:
   the case lists it in `normalised_headers`, and a consumer checks it against
   the body the service actually sent. Freezing the raw length made `--check`
   depend on the length of the temporary directory the generator was given,
   so a CI `TMPDIR=/tmp` failed against a fixture generated under a longer
   one. `self_check.py` refuses a normalised case whose `body_length` is not
   the length of its recorded (normalised) body or that freezes
   `content-length`.
9. **OCP output is not fixture data.** The STEP writer prints transfer
   statistics from native code. The generator silences those descriptors and
   leaves them silenced only after the final report is flushed.
10. **Values the product cannot reproduce are masked, not guessed.** The
    design result's STEP digest and the journal chain digests that bind it
    change on every run, because the STEP export embeds the wall clock of the
    write. They are recorded as `<VOLATILE>` with the reason next to them.
    The reproducible relations are frozen instead: the genesis link, the link
    from every journal event to its predecessor, the digest shape, and the
    chain verdict.

## Adding a case

Add the case in the relevant module, building the expected value by calling
the product function. Do not type a digest or a serialized body by hand. Run:

```bash
.venv/bin/python backend/compat/generate_corpus.py
.venv/bin/python backend/compat/self_check.py
```

The route coverage check fails if a declared Studio or core route has no
request. Case ids must be unique within a fixture.

## Scope guard

This cell owns `backend/compat/**`, `backend/fixtures/**`, and the corpus cell
report. It does not edit the Python product, existing tests, the Windows
workers, `Rust+buid123/`, `rust-implementation.md`, packaging, or any Cargo
workspace. No commit, push, install, or delegation is performed here.
