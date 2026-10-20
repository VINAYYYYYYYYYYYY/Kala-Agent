# compat-drift: explain and resolve the red `backend/compat/self_check.py`
docs-r2 found that `backend/compat/self_check.py` exits 1, with differences in `cli_launch.json`, `manifest.json`, and `studio_http.json` between the checked-in corpus and a fresh generation. Read `backend/compat/**` (the generator, self_check, README), the studio cell's REPORT/REVIEWs in this wave, and today's accepted reports (design-turns B1–B4, r1, r2*, api-errors-fix).
Own ONLY `backend/compat/**` and this cell's REPORT.md. Do not edit Python product code, Rust crates, or studio.
1. Run self_check. For EVERY difference, classify it as: (a) an intentional, accepted change today (name the cell and ADR, e.g. ADR-0009/0011 routes, provider field, token rule); (b) an environment artefact (paths, versions, timestamps, host FreeCAD availability); or (c) an unexplained or regression. Quote the exact diff hunks.
2. If ALL differences are (a)/(b): make the generator deterministic for (b) (normalise host-specific values, recorded explicitly), regenerate the corpus, and record the old and new corpus sha256s plus the generator command in the README and REPORT. Then re-run the 268-case differential and the self_check (both green).
3. If ANY difference is (c): do NOT regenerate. Report it with a minimal reproduction.
Follow `../CONVENTIONS.md` honesty rules. REPORT.md.
