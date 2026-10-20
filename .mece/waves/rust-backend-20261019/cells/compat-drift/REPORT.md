# compat-drift — the red `backend/compat/self_check.py`, explained and resolved

**Status: the red self-check is explained, the corpus is regenerated, and both
the self-check and the 268-case live differential are green.**
No unexplained difference was found, so the corpus was regenerated. No commit,
push, deployment, delegation, or credential read was performed, and
`/home/v9i/.local/share/kala-agent` was never touched.

## 1. Scope and ownership

Edited (this cell's own paths):

- `backend/compat/generate_corpus.py`
- `backend/compat/self_check.py`
- `backend/compat/README.md`
- `backend/compat/corpuslib/common.py`
- `backend/compat/corpuslib/geometry_cases.py`
- `backend/compat/corpuslib/http_cases.py`
- `backend/fixtures/corpus/v1/**` (the regenerated corpus — the task's
  "regenerate the corpus" step)
- `.mece/waves/rust-backend-20261019/cells/compat-drift/REPORT.md` (this file)

Read-only: every Python product file under `src/kala_agent/**`, every Rust
crate, `studio/**`, `adapters/**`, `tests/**`, and the other cells' reports.

## 2. Reproduction

```bash
.venv/bin/python backend/compat/self_check.py
```

Before this cell (checked-in corpus of 2026-10-19 15:58 against the product
sources of 2026-10-20):

```
self-check FAILED: regeneration differs from the checked-in corpus:
- cli_launch.json: JSON contents differ
- manifest.json: JSON contents differ
- studio_http.json: JSON contents differ
```

`self_check.py --skip-regenerate` passed at the same time: the checked-in
corpus was internally consistent; it was simply stale against the product
sources.

## 3. Every difference, classified

The task's classes: **(a)** an intentional, accepted change today; **(b)** an
environment artefact; **(c)** unexplained or a regression.

### 3.1 `cli_launch.json` — 4 hunks, all (a)

The Studio CLI gained the worker mode of ADR-0010. The hunks, verbatim:

```diff
     "parser": {
       "commands": [],
-      "description": "Run the Kala Studio standalone HTTP service on a loopback interface. The host defaults to 127.0.0.1 and is refused for anything else; the token is taken from KALA_STUDIO_TOKEN or minted into the studio data directory.",
+      "description": "Run the Kala Studio standalone HTTP service on a loopback interface. The host defaults to 127.0.0.1 and is refused for anything else; the token is taken from KALA_STUDIO_TOKEN or minted into the studio data directory. With --worker the same command runs the internal geometry worker instead (ADR-0010).",
       "epilog": "Exit codes: 0 served; 1 the uvicorn module is not installed; 2 a usage error occurred.",
       "help_first_line": "usage: python -m kala_agent.studio [-h] [--host HOST] [--port PORT]",
-      "help_length": 1095,
-      "help_sha256": "75506d0a9eec56a2d0edede40fffb1f80cb64b70520e576ad0b51df7eeb93fae",
+      "help_length": 1620,
+      "help_sha256": "c136b87c0dc4b7c11b1942e531d27c8a7bdb6bce31299316f60d4104d6b9ef27",
```

```diff
-          "help": "absolute path to the studio data directory; defaults to the standard Kala data directory or the system temporary folder",
+          "help": "absolute path to the studio data directory; with --worker it is required (KALA_STUDIO_DATA_DIR is accepted instead); otherwise it defaults to the standard Kala data directory or the system temporary folder",
```

```diff
+        },
+        {
+          "choices": null,
+          "default": false,
+          "dest": "worker",
+          "help": "run the internal geometry worker (ADR-0010): bind 127.0.0.1:0, require KALA_WORKER_TOKEN (at least 32 characters), and print one KALA_WORKER_READY line; --host, --port, and --static-dist are ignored",
+          "nargs": 0,
+          "option_strings": [
+            "--worker"
+          ],
+          "required": false,
+          "type": null
         }
```

**Class (a).** The change is `src/kala_agent/studio/cli.py`, owned by the
rust-backend wave cells `r1-worker-mode` (ADR-0010 §"Geometry worker (v1)")
and `r2-datadir-fix` (which made `--data-dir` mandatory in worker mode and
introduced the `KALA_STUDIO_DATA_DIR` alternative in the help text). Both
cells are accepted in this wave; `src/kala_agent/studio/cli.py` is listed as
dirty in `git status` and its digest changed in the manifest's source pin.

### 3.2 `studio_http.json` — 3 kinds of difference, all (a) or (b)

**(a) The design-turn fields.** ADR-0009 §4/§6 and ADR-0011 changed two
responses. Verbatim:

```diff
       {
         "expected": {
           "body": {
+            "design_available": true,
             "integrations": [
```

```diff
-          "body_length": 309,
-          "body_sha256": "8203bd35a98b998ec978ba99003d8057b35b4259ddf5b225516601e47b2892b8",
+          "body_length": 333,
+          "body_sha256": "db994c6ec8ed47b4d0f49d2a9b0d5cbfef98989b10a86e78f2233ecff2a1a670",
```

```diff
-          "body_text": "{\"status\":\"ok\",\"service\":\"kala-studio\",\"version\":\"0.1.0\",\"offline\":true,\"step_available\":true,\"integrations\":[...]}",
+          "body_text": "{\"status\":\"ok\",\"service\":\"kala-studio\",\"version\":\"0.1.0\",\"offline\":true,\"step_available\":true,\"design_available\":true,\"integrations\":[...]}",
```

(three occurrences: `studio.health`, `studio.health.online`,
`studio.health.offline_again`, `studio.valid.same_origin`)

```diff
       {
         "expected": {
           "body": {
+            "design": null,
             "model": "deterministic-paraphrase-v1",
```

**Class (a).** `design_available` on `/health` and the optional `design` field
on `/chat` are the B1 API cell of the design-turns wave
(`.mece/waves/design-turns-20261019/cells/b1-api/REPORT.md`: "`POST /design`,
`GET /design/{id}/drawing`, `GET /design/{id}/section`, `GET /journal`,
`GET /journal/verify`, `/export` `design_id`, `/chat` `design`, `/health`
`design_available`"), which is ADR-0009.

**(b) The STEP write timestamp.** Verbatim, from the two STEP cases:

```diff
-              "FILE_NAME('Open CASCADE Shape Model','2026-10-19T15:58:01',('Author'),(",
+              "FILE_NAME('Open CASCADE Shape Model','2026-10-20T04:48:26',('Author'),(",
```

```diff
-          "body_sha256": "07910000da77a80b53cc393ffa3126fef3b113d983b121454663e04f52813552",
+          "body_sha256": "e59fea689584a1f90afca62c8de5ec4a63139ca896c157816149841d6e95b809",
```

**Class (b).** The OCP STEP writer stamps the wall clock into the `FILE_NAME`
header line. The corpus already declared STEP bytes non-contractual (README
rule 6) and already stripped these fields from the comparison, so they never
made the *check* red; they are listed here because they are part of the raw
diff and because they are the reason the corpus was not byte-reproducible
(§4).

### 3.3 `manifest.json` — 4 kinds of difference, all (a) or (b)

**(a) The route table grew by six routes and the corpus did not follow.**
Verbatim:

```diff
         {
           "method": "POST",
+          "path": "/api/studio/design"
+        },
+        {
+          "method": "GET",
+          "path": "/api/studio/design/{design_id}/drawing"
+        },
+        {
+          "method": "GET",
+          "path": "/api/studio/design/{design_id}/section"
+        },
+        {
+          "method": "POST",
           "path": "/api/studio/export"
         },
```

```diff
+        {
+          "method": "GET",
+          "path": "/api/studio/journal"
+        },
+        {
+          "method": "GET",
+          "path": "/api/studio/journal/verify"
+        },
```

```diff
+        {
+          "method": "GET",
+          "path": "/api/studio/providers/cad"
+        },
+        {
+          "method": "GET",
           "path": "/api/studio/settings"
         },
```

```diff
-      "missing": []
+      "missing": [
+        { "method": "POST", "path": "/api/studio/design" },
+        { "method": "GET",  "path": "/api/studio/design/{design_id}/drawing" },
+        { "method": "GET",  "path": "/api/studio/design/{design_id}/section" },
+        { "method": "GET",  "path": "/api/studio/journal" },
+        { "method": "GET",  "path": "/api/studio/journal/verify" },
+        { "method": "GET",  "path": "/api/studio/providers/cad" }
+      ]
```

**Class (a) for the routes, and a corpus-completeness gap this cell had to
close.** The routes exist because of ADR-0009 (`b1-api`) and ADR-0011
(`b2-providers`: "`GET /providers/cad`"). The *gap* — that the corpus had no
request for them — is not a product regression; it is the corpus falling
behind the product. `self_check._check_coverage` would have failed on it
after any regeneration ("uncovered declared routes"), so the corpus was
extended (§5) rather than the check relaxed.

**(b) The working-tree path list.** Verbatim, abridged:

```diff
       "dirty": true,
       "tracked_changes": [
         ".github/workflows/solidedge.yml",
+        ".github/workflows/studio.yml",
+        ".mece/waves/design-turns-20261019/",
...
+        "src/kala_agent/studio/design/",
+        "src/kala_agent/studio/internal_api.py",
+        "src/kala_agent/studio/session_journal.py",
+        "src/kala_agent/studio/worker.py",
```

**Class (b).** `git status --porcelain` names every unrelated file in the
repository. It changes whenever any worker edits anything, so it cannot be
part of a reproducibility contract. It is still recorded, and the parts of the
git state that *are* stable (the commit) are still compared.

**(a) The product source pin grew from 78 to 95 files.** Verbatim:

```diff
     "python_sources": {
-      "file_count": 78,
+      "file_count": 95,
```

with the new entries `src/kala_agent/studio/design/**` (16 files),
`src/kala_agent/studio/internal_api.py`, `src/kala_agent/studio/worker.py`,
`src/kala_agent/studio/session_journal.py`, and changed digests for
`src/kala_agent/api/errors.py`, `src/kala_agent/studio/cli.py`,
`src/kala_agent/studio/schemas.py`, `src/kala_agent/studio/server.py`,
`src/kala_agent/studio/providers/dispatch.py`, and
`src/kala_agent/studio/providers/local.py`.

**Class (a).** Those are the accepted cells of the design-turns wave
(`b1-api`, `b2-providers`, `b3-freecad-adapter`, `b4-solid-count`), the
rust-backend wave (`r1-worker-mode`, `r2-datadir-fix`), and the
`api-errors-fix` cell. The tree digest moved from
`d12f292f69b874068a83aef07d07120f39967c98cc392cef2e5900015d4aa888` to
`754bc70c23dfae525770279c1e70b09fb1f9fd23ee4ee11017b9016962c23f91`.

**(b) The derived file digests.** The `files[]` digests of
`cli_launch.json`, `core_http.json`, `geometry_stl.json` and
`studio_http.json` changed because those files changed. `core_http.json` and
`geometry_stl.json` changed *in bytes only*, through the STEP timestamp of
§3.2(b); their content was already equal.

### 3.4 The (c) verdict

**There is no (c) difference.** Every hunk in the three files is explained by
an accepted cell of this wave or by a host fact. Two independent checks back
this up:

- The corpus's own comparison found no other differing file: the other nine
  indexed fixtures (`canonical_json.json`, `decimal_budget.json`,
  `domain_provenance.json`, `event_chain.json`, `schema_v2.sqlite3`, the
  three STL files) were byte-identical between the checked-in corpus and a
  fresh generation.
- Two consecutive generations into different directories were byte-identical
  **after** the normalisations of §4, which is only possible if no difference
  was hiding behind a stripped field.

## 4. Making the generator deterministic for the (b) class

Before this cell the generator was **not** byte-reproducible: two runs into
two directories produced four different files. Measured:

```
DIFF  ./core_http.json
DIFF  ./geometry_stl.json
DIFF  ./manifest.json
DIFF  ./studio_http.json
```

The causes and the fixes, all recorded in the corpus itself
(`manifest.determinism.normalisation`, `manifest.determinism.normalized_fields`,
`studio_http.json` → `capture` / `normalizations` / `nondeterministic_fields`):

1. **The STEP writer's wall clock (b).** `normalise_step_bytes` replaces the
   `FILE_NAME` timestamp with the fixed clock value `2026-01-01T00:00:00`
   before the digest is taken. The replacement keeps the byte length, so the
   recorded length is still the length of the real response.
2. **The STEP writer's per-process product counter (b).** OCCT numbers the
   translated product per process, so the *second* export of one process says
   `Open CASCADE STEP translator 8.0 2` where the first says `... 8.0 1`.
   The counter is replaced by zeros of the same width. This also makes the
   digest independent of how many STEP exports ran before it.
3. **The temporary data directory in the core `/health` body (b).** The raw
   body embeds `/tmp/kala-corpus-gen-XXXX/core-data`. The recorded
   `body_sha256` is now the digest of the *normalised* bytes, and its role
   says so; the body text is unchanged apart from the existing
   `<DATA_DIR>` substitution.
4. **The Studio journal's wall clock (b).** `StudioSessionJournal.append`
   calls `datetime.now(UTC)`, like the core journal. The fixture harness now
   injects the same kind of fixed clock into
   `kala_agent.studio.session_journal.datetime`. This is a fixture-only
   override, exactly like the existing core-journal override, and it is
   recorded in the fixtures.
5. **The design result's STEP digest and the journal digests that bind it
   (b).** These are values the product itself cannot reproduce: the OCP STEP
   export embeds the wall clock of the write, so its content digest changes on
   every run, and the journal chain digests bind it. They are recorded as
   `<VOLATILE>` with the reason next to them; the reproducible relations are
   frozen instead (the genesis link, the link from every event to its
   predecessor, the digest shape, and the chain verdict).
6. **The working-tree path list (b).** `manifest.determinism.excluded_from_comparison`
   now declares `source_state.git` as recorded-but-not-compared, with the
   reason. The comparison honours the declaration, so the manifest can no
   longer go red because another worker touched another file.

The comparison is *not* weakened: the product source pin, the commit, the
per-file digests, and the corpus digest are all still compared. Two negative
controls prove it:

```
problems after a real content change:   ['studio_http.json: JSON contents differ']
problems after a source-pin change:     ['manifest.json: JSON contents differ']
problems after a git-state change:      []
```

After the fixes, two consecutive generations are byte-identical in **all
thirteen** files, and the corpus carries a single digest that identifies it.

## 5. What changed in the corpus

### 5.1 The six uncovered routes are now covered

The corpus gained eight Studio cases (48 → 56) covering all six declared
routes, and `self_check._check_coverage` now expects 15 Studio routes instead
of 9:

| Case | Method | Route | Notes |
|---|---|---|---|
| `studio.cad_providers` | GET | `/api/studio/providers/cad` | ADR-0011 live probe |
| `studio.design.l_bracket` | POST | `/api/studio/design` | ADR-0009/0011, `provider: opencascade`, L-bracket template |
| `studio.design.drawing` | GET | `/api/studio/design/{id}/drawing` | HLR views of the stored design |
| `studio.design.section` | GET | `/api/studio/design/{id}/section` | section loops of the stored design |
| `studio.design.provider_required` | POST | `/api/studio/design` | 422: the provider is required, not defaulted |
| `studio.design.drawing.unknown` | GET | `/api/studio/design/{id}/drawing` | 404 for an id that was never stored |
| `studio.journal.page` | GET | `/api/studio/journal` | the Studio session journal page |
| `studio.journal.verify` | GET | `/api/studio/journal/verify` | the chain verdict |

The design id used by the two GET cases is not written by hand: it is read
from the `POST /design` response during the same generation run.

The `POST /design` response is recorded in full (4,360 bytes) with two
masked leaves; the measured values, the check rows, the imported view and the
script digest are the product's own numbers. The journal page records the
whole session chain (10 events: the chat turn and the nine design events)
with the three digest fields masked and the chain relations recorded instead:

```json
{"available": true, "chain_is_unbroken": true, "event_count": 10,
 "genesis_previous_digest": "0000000000000000000000000000000000000000000000000000000000000000",
 "head_digest_is_sha256_hex": true}
```

with, per event, the sequence, the event type, `links_to_previous`,
`digest_is_sha256_hex` and `payload_digest_is_sha256_hex`. The verdict case
records that the head digest is the last event's digest, that the counts
agree, and that the chain verifies. `self_check.py` now refuses a recorded
journal chain that is broken, a verdict that is not `ok`, or a head that does
not match the last event — so the relations are asserted, not just recorded.

The journal page is a *session* record: it contains the events the earlier
cases of the same run produced, so adding a Studio case before it changes the
recorded page. That coupling is deterministic and documented, but it is worth
knowing before a case is inserted.

### 5.2 The corpus digest

`manifest.json` now carries `corpus_sha256`: the SHA-256 over the sorted
`(path, digest)` list of the twelve indexed fixture files, with the SQLite
file's entry recorded as `not-asserted` (its bytes are not asserted; its
logical rows are, in `event_chain.json`). `self_check.py` recomputes it from
the manifest's own file list and refuses a mismatch.

| Corpus | `corpus_sha256` |
|---|---|
| before this cell's regeneration | `685d10d17c12ecb6b89afa19756d8d98865203f9cb479054a8f143016a015175` |
| after this cell's regeneration | `9993939bfea5e01553438e680fe3a9ea4b24335ee9077da5b8dc453b5a987e8b` |

Generator command for both:

```bash
.venv/bin/python backend/compat/generate_corpus.py
```

**How the "before" digest was obtained, stated plainly.** The corpus was not
committed — the whole `backend/` tree is untracked (`git status --porcelain`
shows `?? backend/`) — and this cell regenerated it in place, so the previous
bytes are gone. The "before" value is therefore *reconstructed* with the same
rule from the per-file digests the pre-regeneration `manifest.json` recorded,
captured before the regeneration. Those digests are:

| File | Digest before | Status after |
|---|---|---|
| `canonical_json.json` | `d8a2fefcd5d70f05556938896bada27affa2a1a8288d3d1a788d17b1fde4ba07` | unchanged |
| `cli_launch.json` | `d2689e784c0e78a79c0f7b49c5d99601d81d4477d13d857137cd5d9bddb66ae8` | changed |
| `core_http.json` | `ef0df7451001092999b2e47e2fa08e68c5531aa729caec8415db35e947c3d730` | changed |
| `decimal_budget.json` | `7bdce1d559538f9b283414be67bba803a1a00a00e99743d9beef8945935bddaf` | unchanged |
| `domain_provenance.json` | `f7ebb75c2f093e035e5c08a2be7e95a24cb56b4b31e81176e192c51bf473bedb` | unchanged |
| `event_chain.json` | `4cc4dc39e3900ce0b22aa58230ed05e66290351b9be0005f0e0d51ecd93420c7` | unchanged |
| `geometry_stl.json` | `b527a9408870f88e057d050291a3b74297c14c39562eea2aa26cefb19b81ebc2` | changed |
| `schema_v2.sqlite3` | `9d8b6556c99006ce287a52c65e2590ef5d2b485ec73bdd4079fb648191025832` | unchanged |
| `stl/box_default.stl` | `e6c04f2a46a0897aaa157a63b3efc89f0632201728753ce82c7ca0e7d2c7dfd5` | unchanged |
| `stl/cylinder_default.stl` | `0c87786bde66b7cffd9a7c71ff7b501922c4d747eb8e768f4ae7469ca8548243` | unchanged |
| `stl/sphere_default.stl` | `ea50ff2ea4f09bffcee161e017f85565f0ad87434ea8c4743881d40f202419e3` | unchanged |
| `studio_http.json` | `49b51f3a07a9030842d994f778336c7934187258f98a9763ac7aa02d0f269d91` | changed |

The eight "unchanged" files were byte-identical between the old corpus and a
fresh generation (proved by `diff -u`, which reported no hunk for them), so
their digests are the old corpus's own. The four "changed" files are the ones
whose digests the old manifest recorded and the new one replaced. The
reconstruction is therefore exact, and it is recomputable from the table
above.

## 6. Verification

Every command below was run from the repository root with the repository
virtual environment, after the regeneration.

| Command | Result |
|---|---|
| `.venv/bin/python backend/compat/generate_corpus.py` | exit 0; 12 files; 15 Studio routes, 0 uncovered; 17 core routes, 0 uncovered |
| `.venv/bin/python backend/compat/generate_corpus.py --check` | exit 0; `determinism check passed` |
| `.venv/bin/python backend/compat/self_check.py` | exit 0; `self-check OK`; regeneration compared |
| `.venv/bin/python backend/compat/self_check.py --skip-regenerate` | exit 0 |
| two generations into two directories, byte-compared | all 13 files identical |
| `.venv/bin/python -m ruff check backend/compat` | all checks passed |
| `.venv/bin/python -m compileall backend/compat` | passed |
| `.venv/bin/python backend/crates/kala-studio/tests/python_differential.py` | `differential cases: 268`, `mismatches: 0` |

The self-check's own summary:

```json
{
  "canonical_json_cases": 66,
  "chain_events": 5,
  "cli_commands": 12,
  "cli_executions": 9,
  "core_http_cases": 49,
  "core_routes": 17,
  "corpus_sha256": "9993939bfea5e01553438e680fe3a9ea4b24335ee9077da5b8dc453b5a987e8b",
  "decimal_cases": 85,
  "domain_packages": 3,
  "geometry_meshes": 6,
  "regenerated": true,
  "studio_http_cases": 56,
  "studio_routes": 15
}
```

The 268-case live differential was measured **before** this cell's changes
and **after** them, with the same result both times: 268 cases, 0 mismatches.
It is the native `kala-studio` binary against the live Python Studio app, so
it is independent of the corpus generator; it is reported here because the
task asks for it to stay green. Note that the differential's case list does
**not** contain the six new routes — the Rust front door implements nine
route-methods and proxies the rest to the Python worker (ADR-0010) — which
is the same finding the delivery cell recorded. The new routes are frozen by
the corpus, not by the 268-case differential.

## 7. Compatibility and safety notes

- **No product behaviour changed.** The clock substitutions and the STEP
  normalisation are fixture-only overrides in `backend/compat/**`. The hashing,
  serialization, chain and STEP-writer code in `src/kala_agent/**` is
  untouched, and the corpus still records what the product produces.
- **No live credentials.** The synthetic bearer tokens and the synthetic
  provider key are fixture-only; `self_check` asserts that none of them
  appears in a recorded body. The provider key is cleared in the same run.
- **The user's data directory was never touched.** The Studio fixture app is
  built with an explicit temporary `data_dir`, and the design service takes
  that directory, so the design journal and the artifact store live under the
  generator's temporary work root. This was verified, not assumed: the
  modification time of `/home/v9i/.local/share/kala-agent/studio` was
  identical before and after a full generation, a full self-check, the probe
  scripts, and the 268-case differential. No path under
  `/home/v9i/.local/share/kala-agent` was read or written by this cell. (The
  directory's own mtime shows a change at 04:51:20 from a concurrent wave
  worker, which is the same symptom the `r2-datadir-fix` cell recorded; it is
  not from any command in this cell.)
- **The design case writes only into the temporary work root.** The Studio
  journal and the STEP artifact are produced by the product inside the
  generator's temporary directory and are not part of the corpus.
- **The masked values are never written down.** `<VOLATILE>` replaces the
  STEP digest and the chain digests; the reasons are recorded, the values are
  not. `self_check` verifies that every masked case still carries its
  placeholder, its reason, and a body digest that matches the masked text.
- **The comparison is still a same-tree check.** If a product source file
  changes, the manifest's source pin changes and the check fails by design;
  regenerate the corpus. Only the git working-tree path list is excluded.

## 8. Remaining gaps

1. **`backend/fixtures/README.md` is now stale and was deliberately not
   edited** — it is outside this cell's declared scope (`backend/compat/**`
   and this report). Two lines need the corpus cell's attention:
   - line 21: `studio_http.json   9 Studio routes + gates, 48 cases` → should
     read `15 Studio routes + gates, 56 cases`;
   - lines 88-90: the determinism note should also mention the Studio
     session journal's fixed clock and the STEP writer's normalisation, and
     should mention the corpus digest.
   The manifest itself is correct and self-describing.
2. **The corpus is not committed.** The whole `backend/` tree is untracked, so
   the regenerated corpus and its digest live only in the working tree. No
   commit was made, as the conventions require.
3. **The six new routes are not covered by the 268-case live differential.**
   The Rust front door exposes nine route-methods and proxies the design and
   journal routes to the Python worker (ADR-0010). The corpus now freezes
   their contract, but the native-vs-Python byte parity of those routes is
   still the delivery cell's open item.
4. **The CAD provider probe is host evidence.** `studio.cad_providers`
   records this host's probe result, including the FreeCAD refusal reason,
   which changes with the host's FreeCAD installation. The same probe is
   recorded in `manifest.oracle_environment.cad_providers`, so a reader can
   tell a host fact from a fixture value.
5. **The STEP digest is a normalised digest.** It is the digest of the STEP
   body with the writer's timestamp and translator counter replaced. The
   length, the ISO prefix, the header lines, the `MANIFOLD_SOLID_BREP` count
   and the byte-equality verdict are unchanged from the previous contract.
6. **Everything the previous corpus cell listed as a gap still stands** —
   Windows/COM/NSIS, the 17-vs-18 core route question, the STEP 503 branch,
   the live-socket capture, `KALA_STUDIO_MESH_LIMIT`, and the timing deferral.
   This cell changed none of them.

## 9. Handoff

`backend/compat/self_check.py` is green, the corpus is reproducible
byte-for-byte, and the corpus digest is recorded in
`backend/compat/README.md` and in `manifest.json`. The next reviewer should:

1. regenerate once more after any product source change and confirm the
   corpus digest moves with the source pin;
2. read `manifest.determinism.excluded_from_comparison` before treating a
   manifest difference as a real drift — only the git working-tree path list
   is excluded;
3. treat `<VOLATILE>` in `studio_http.json` as a deliberate hole, not a
   missing expectation: the product cannot reproduce those digests either.
