# sandbox-r2: restore the PID namespace in the production worker jail (security repair)
Follow `../CONVENTIONS.md`. Read `../sandbox-r1/REPORT.md` (the intentional-differences list) and `src/kala_agent/tools/sandbox.py`.
Finding (Opus review): the Rust production worker jail has NO PID namespace, because the accepted `kala-process` launcher has no PID-1 transition. Python's jail isolates PIDs. That is a security regression and must not ship.
Fix WITHOUT editing the accepted `kala-process` crate. Own only `backend/crates/kala-sandbox/**` and this cell's REPORT.md. Suggested approach: a tiny native init shim binary in kala-sandbox (for example `kala-sandbox-init`), launched via `kala_process::supervise`. It `unshare(CLONE_NEWPID)` (and the other namespaces in Python's order), forks so that the child becomes PID 1 of the new namespace, that PID 1 performs the jail setup and then runs or reaps the real worker (forwarding signals, propagating exit status, killing the namespace on parent death), and it preserves `kala-process`'s deadline, cancellation, and output-bound semantics end to end. Any equivalent design that meets the tests is acceptable.
Tests:
- Inside the jail, `getpid()==1` for the init and the worker cannot see host PIDs (`/proc` contains only namespace PIDs).
- Kill/cancel/deadline still terminate the whole namespace, with no orphans left (check the host process table).
- Exit status and output bounds are identical to the pre-repair behaviour.
- The 182-case live differential and all 29 security tests stay green, and the live probe reports the `pid_namespace` control as verified for the production path.
- Full Rust gates.
REPORT.md: the design, evidence, and any remaining limit.
