# application-review: independent adversarial review of kala-application (REVIEWER)
You do NOT edit `backend/crates/kala-application/src/**`. You may add only `backend/crates/kala-application/tests/review_*.rs` and this cell's REVIEW.md. Follow `../CONVENTIONS.md` gates. Read `../application-r1/REPORT.md` (especially the normalisations, §8–§9 gaps, and the identity boundary), `src/kala_agent/application.py`, and the application tests.
Probe and pin with tests that fail if the code is broken:
1. The differential normalised "digest values". Re-run key operations WITHOUT digest normalisation, except the declared native-identity digests. Prove that every other digest (event chain, spec/state/action, request ids, artifact sha256) is byte-equal to Python.
2. Revision commit two-phase: prepare/decide/commit races (threads), finality after approve and after reject, replay of a decided request, a stale state digest, a forged or tampered pending commit restored from disk.
3. Workflow checkpoints: approve/reject/decide on wrong, stale, or foreign ids; resume after restart from a Python-written data dir.
4. Cross-read safety: Rust opening a Python data dir that contains a tampered journal row or a truncated artifact must refuse (as Python does), never silently continue.
5. Close semantics: operations after close, double close, and an open file handle leak (count fds before and after).
6. Error precedence and messages vs Python on 30+ invalid inputs.
REVIEW.md: a verdict per probe (PASS / FAIL with a minimal repro), counts, and ACCEPT or REPAIR with exact defects.
