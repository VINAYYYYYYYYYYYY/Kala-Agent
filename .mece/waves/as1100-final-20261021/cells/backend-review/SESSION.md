# Worker session — backend-review

Transport: T3 app-owned delegation (OpenCode worker, shared checkout)
Model: opencode-go/minimax-m3.1-flash-free
Brief: `TASK.md` in this folder
Status: complete

## Scope observed

- Read `.mece/waves/as1100-final-20261021/WAVE.md`,
  `.mece/waves/as1100-sheet-20261021/WAVE.md` (the "Backend chat additions"
  contract) and `cells/edit-turns/REPORT.md`, then the actual `git diff` of
  `backend/**`, `src/kala_agent/studio/schemas.py`,
  `backend/fixtures/corpus/v1/**` and the studio client/validators.
- Reviewed, with file/line evidence: the request validation, the base-design
  context, the cumulative usage accounting, the client/server protocol and
  the journal semantics — the five areas the brief names.
- Wrote only this cell's `REPORT.md` and `SESSION.md`. Product source, tests,
  fixtures and packages were read-only.
- No commit, reset, clean, push, publication, delegation, release build,
  credential read, remote API call or real CLI invocation.
- Every test ran with `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR`
  and `XDG_DATA_HOME` pointed at `/tmp/opencode/kala-review/**`; the real
  `CARGO_HOME=/home/v9i/.cargo` and `RUSTUP_HOME=/home/v9i/.rustup` were kept
  so the toolchain resolved. The live differentials were pointed at a
  **debug** binary I built myself, so the parent's release packaging was
  never touched.

## Result

- The backend edit-turn/usage diff is **sound**. No functional defect that
  breaks the ADR-0012 contract was found; every gate I could run is green.
- Four findings, all in the report/document layer, not in the product code:
  - **F1** the edit-turns REPORT's stated reason for the 404 on a malformed
    `base_design_id` is false — the design routes answer **422**
    (`server.rs:1381-1388`, `:1400-1408`), not 404.
  - **F2** the corpus digest moved to `5914b484…` in this wave; five files
    still declare `39e0c0a9…` as the value `corpus_fixture.rs` "pins now",
    and three of them also claim `studio_http.json` was byte-identical
    across the regeneration (it was not).
  - **F3** the REPORT's error table omits the 501 and 504 the base-design
    read can also answer.
  - **F4** a failed *validation-repair* call drops the first call's already
    reported tokens, while the *build-repair* failure path keeps them.
- Gates: `cargo test --workspace` 583/583; `cargo test -p kala-studio`
  309/309 (lib 169, `edit_turns` 8, `chat_and_remote` 7, `cli_providers` 24);
  `cargo fmt --check` clean; `cargo clippy -p kala-studio --all-targets -D
  warnings` clean; `cargo check --no-default-features --lib` clean; pytest
  unit 1667, integration 12, security 189; `generate_corpus.py --check` and
  `self_check.py` pass; `python_differential.py` 268 cases / 0 mismatches;
  `worker_differential.py` 295 cases / 0 mismatches / 0 gaps;
  `replay_binary_corpus.py` 59 cases / 0 mismatches (a gate the cell's own
  table omits, and the corpus fixture changed);
  `python_provider_dto_oracle.py` 20 cases / 0 failures.
- No blocker for the parent. F2 needs an owner outside every cell's file
  list; F1, F3 and F4 are documentation/honesty wording.
