# Cell corpus-hermetic: the corpus --check must not depend on the TMPDIR path length

Read `backend/compat/README.md` and `backend/fixtures/README.md`.
Every shell: `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo`, temp `HOME`, temp `KALA_AGENT_DATA_DIR`. Never touch `/home/v9i/.local/share/kala-agent/**`. Never read credentials. Clean up /tmp.

## Opus finding (2026-10-21)
`.venv/bin/python backend/compat/generate_corpus.py --check` FAILS on a clean run with the default `TMPDIR=/tmp`: `core_http.json` and `manifest.json` differ. The only difference is case `core.health` (`/health`): `expected.body_length` and the `content-length` header are 415 in the committed fixture and 408 when freshly generated. That body embeds the generator's run-specific data-dir path (already normalised to `<DATA_DIR>` in `body_text`; `body_deterministic: false`), so the RAW length depends on the length of the temp path the generator creates (it does not depend on the caller's `KALA_AGENT_DATA_DIR`; Opus tested two lengths → 408 both). The committed fixture came from a run with a longer TMPDIR. CI (a plain `/tmp`) would fail the same way.

## Fix
Make the generated corpus independent of the temp-path length. Preferred: for any case whose body is normalised (`normalizations` non-empty), record the length and `content-length` of the NORMALISED body, or mark them as nondeterministic fields that the consumers skip. Check every consumer (`self_check.py`, `backend/crates/*/tests/corpus_fixture.rs`, the replay tools) so the meaning stays correct and nothing silently stops checking. Pinning a fixed-length temp dir name is acceptable only if you justify it. Regenerate the corpus with the documented command, then prove `--check` passes with at least two different TMPDIR lengths (e.g. `TMPDIR=/tmp` and a much longer one). Run `self_check.py`, the Rust corpus fixture tests, and the three differentials (`python_differential.py` 268, `worker_differential.py` 295, `python_provider_dto_oracle.py` 20 → 0/0/0). Report the old and new corpus digests and update `backend/compat/README.md` / `backend/fixtures/README.md` if they cite the digest.

## Ownership
`backend/compat/**`, `backend/fixtures/**`, the corpus fixture test(s) under `backend/crates/*/tests/` that read these fields, and the two READMEs. Nothing else. Do not commit.

## REPORT
`REPORT.md` here: the change, the consumers checked, the two-TMPDIR proof, the digests, the test counts.
