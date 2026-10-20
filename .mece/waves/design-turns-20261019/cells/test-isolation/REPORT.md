# test-isolation — report

## Outcome

No test in the Python or adapter suites resolved or opened the user's real data
directory `/home/v9i/.local/share/kala-agent` during the recorded runs. Both
real SQLite files kept identical size, mtime, and inode, and no `-shm`/`-wal`
sibling appeared at any of 357 half-second polls across the full test run.

All suites are green: **2473 passed, 1 skipped**.

## What changed

| File | Change |
| --- | --- |
| `tests/conftest.py` | New. Autouse, function-scoped `isolate_user_data_dirs` fixture plus `isolated_home` / `isolated_xdg_data_home` / `isolated_kala_data_dir` / `isolated_studio_data_dir` helpers. |
| `tests/unit/test_data_dir_isolation.py` | New. Pins the fixture contract and proves default resolution lands under the temporary `HOME`. |
| `tests/unit/test_studio_server.py` | `test_token_is_never_echoed_in_any_response` no longer builds the app with `data_dir=None` through an `if False` expression; it uses the test's temporary `data_dir` fixture. |

No product code was edited.

### Fixture contract

For every test, before the test body runs, `monkeypatch.setenv` redirects:

- `HOME` → `<tmp_path>/isolated-home`
- `XDG_DATA_HOME` → `<tmp_path>/isolated-xdg-data`
- `KALA_AGENT_DATA_DIR` → `<tmp_path>/isolated-xdg-data/kala-agent`
- `KALA_STUDIO_DATA_DIR` → `<tmp_path>/isolated-xdg-data/kala-agent/studio`

The fixture then asserts `Path.home() == isolated_home`, so a platform that
ignored the `HOME` override would fail loudly instead of silently falling back
to the real home. The studio path mirrors production layout (studio inside the
core data directory). This covers every resolution order the suite can reach:

1. `KALA_STUDIO_DATA_DIR` (studio's first preference),
2. `KALA_AGENT_DATA_DIR` (core `Settings`),
3. `Path.home() / ".local/share/kala-agent"` (`Settings.from_env({})` and
   `create_studio_app(data_dir=None)` fallbacks).

`tests/unit/test_data_dir_isolation.py` asserts (a) all four environment
values point into `tmp_path`, (b) `Settings.from_env({})` resolves to
`<isolated-home>/.local/share/kala-agent`, and (c) `create_studio_app` with
`data_dir=None` and an empty environment view resolves to the same tree under
`isolated-home` — never the real home.

One collision was caught while iterating: the first fixture version used
`tmp_path/"home"`, which `test_worker_process.py` also creates. Renaming the
fixture directories to `isolated-home` / `isolated-xdg-data` fixed it; the
recorded runs below are from the fixed version.

## Proof method (stat / ls only)

The user's files were never opened, read, modified, or deleted. Only `stat`
and `ls` were used.

Baseline and final stats (`stat -c '%n|%s|%Y|%i'`, size | mtime epoch | inode):

| File | Baseline | Final |
| --- | --- | --- |
| `studio/studio-journal.sqlite3` | `237568 \| 1792449777 \| 3714183` | `237568 \| 1792449777 \| 3714183` |
| `kala.sqlite3` | `131072 \| 1792402275 \| 3457516` | `131072 \| 1792402275 \| 3457516` |

`diff baseline-stats.txt final-stats.txt` is empty. Directory listings
(`ls -1A`) of `/home/v9i/.local/share/kala-agent` and its `studio/` child are
also identical before and after:

```
root:   artifacts, kala.sqlite3, runs, studio
studio: artifacts, exports, imports, studio-journal.sqlite3, studio-token
```

A background watcher polled both directories every 0.5 s, logging `ls -1A`
listings and `stat` lines, and grepping the listings for `-shm`/`-wal`:

- **357 polls**, first `2026-10-20T04:53:29+05:30`, last
  `2026-10-20T04:56:29+05:30`
- Watcher log contains exactly two distinct `stat` lines, both identical to
  the baseline above (no mtime change even momentarily).
- `watch-hits.log` is empty — **0 occurrences** of any `-shm` or `-wal` name,
  including the historical `studio-journal.sqlite3-shm` / `-wal`.
- Watcher window covers the full `pytest tests` run and all three adapter
  suites (plus an earlier, non-green attempt; it also produced zero hits).

Watcher loop (abridged):

```bash
while [ ! -f "$OUT/stop-watch" ]; do
    ts=$(date -Is)
    printf '== %s ==\n' "$ts" >> "$LOG"
    ls -1A "$ROOT/studio" >> "$LOG" 2>&1
    ls -1A "$ROOT" >> "$LOG" 2>&1
    stat -c 'stat %n|%s|%Y|%i' "$ROOT/studio/studio-journal.sqlite3" "$ROOT/kala.sqlite3" >> "$LOG" 2>&1
    ls -1A "$ROOT/studio" | grep -E -- '-(shm|wal)$' >> "$HITS" || true
    sleep 0.5
done
```

## Test results (all green)

| Suite | Command | Result |
| --- | --- | --- |
| Main (unit, security, integration) | `pytest tests -q` | **1761 passed**, 18 warnings, 53.22 s |
| FreeCAD adapter | `pytest tests -q` in `adapters/freecad` | **388 passed**, 4.19 s |
| Solid Edge bridge | `pytest tests -q` in `adapters/solidedge-bridge` | **186 passed, 1 skipped** (live Windows test), 1.88 s |
| Solid Edge adapter | `pytest tests -q` in `adapters/solidedge` | **138 passed**, 24.77 s |

Total: **2473 passed, 1 skipped**.

`ruff format --check` and `ruff check` pass on all touched files.

## Scope notes

- `tests/conftest.py` is discovered by `pytest tests` (rootdir is the repo).
  The adapter suites have their own rootdirs and their own conftests, so they
  do not load it; no adapter code references the `kala-agent` data directory
  (grep: adapters use `~/.local/share/kala`, `~/.local/share/FreeCAD`, and
  `Kala/SolidEdgeBridge`), and the watcher confirms they never touched it.
  Adapter test trees were deliberately left untouched per the task's edit
  boundary.
- `mypy` on the touched files reports only pre-existing environment issues
  (the installed `kala_agent` distribution lacks a `py.typed` marker, and the
  installed numpy stubs use Python 3.12 `type` statements under the project's
  3.11 target). No error originates in the new fixture or tests.
- No commits, no delegation, no credential reads. Raw logs remain in
  `/tmp/opencode/test-isolation/`.
