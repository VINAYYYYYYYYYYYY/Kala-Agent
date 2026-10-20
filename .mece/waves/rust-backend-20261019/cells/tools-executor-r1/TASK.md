# tools-executor-r1: executor module in kala-tools (SECURITY-CRITICAL)
Follow `../CONVENTIONS.md`. Extend the accepted `backend/crates/kala-tools` (protocol + registry + simulator; see `../tools-protocol-r1/REPORT.md` and `../tools-registry-r1/REPORT.md`) with module `executor`, porting `src/kala_agent/tools/executor.py` (1,915 lines) completely, plus its exports. Do not change existing modules' behaviour (all 192 tests stay green).
Consume the accepted `kala-sandbox` (including the sandbox-r2 PID-namespace init; see `../sandbox-r2/REPORT.md`) and `kala-process` read-only. Never weaken a sandbox control. If executor.py relies on a control the Rust sandbox lacks, STOP and report it rather than running unsandboxed.
Read ADR-0005 and ADR-0006 and every executor test in tests/unit, tests/security, and tests/integration (including the misbehaving simulator entry points).
Must preserve:
- the request/response envelope flow;
- capability authorization against the registry;
- argument and result validation;
- worker bootstrap and child environment;
- deadlines and cancellation;
- output bounds and stderr sanitization;
- crash, hang, fork-bomb, stray-child, oversize-output, and malformed-response handling;
- provenance and digest records;
- every refusal message and error precedence.
Differential: run the same requests, including EVERY misbehaviour entry point, through the Python executor and the Rust executor on this Linux host. Compare decisions, envelopes, sanitized diagnostics, timing classes (deadline vs crash), and host process-table cleanliness after each run. Zero unexplained mismatches. Full Rust gates. REPORT.md, including a security section listing each control and its test.
