# Sol process repair round 2 — accepted within explicit platform limits

Repair task terminal/no live child turns. Accepted for native process-layer consumers after source review and independent checks; no root workspace membership or full-backend/Windows-runtime acceptance implied.

Independent evidence on final code:

- All 46 offline locked release tests passed: 2 unit, 33 process, 1 live Python oracle, 10 sandbox. Windows tests do not execute on Linux.
- fmt, Linux all-target clippy-Dwarnings, MSVC all-target clippy-Dwarnings and GNU all-target clippy-Dwarnings passed. Target clippy includes compile checking all test targets.
- Opt-in KALA_STRICT_SANDBOX=1 release sandbox suite: all 10 tests passed, actual helper verified the same ten namespace/mount/network/workspace controls and exercised real confinement. PID namespace remains unsupported and procfs is not advertised on this host.
- Independent /tmp/kala-sol-process-review consumer reprobes: malformed aggregate environment now rejected; signal status -9; pending 1 MiB stdin with 50 ms bootstrap/600 ms wall now TimedOut(Bootstrap) after 52.72 ms; continuous 32-writer stderr flood returns TimedOut(WallClock) after 52.26 ms for a 50 ms wall limit.

Reviewed every prior finding: Windows pipe constructor now consistently returns parent/child endpoints with correct directions/inheritance and a meaningful ignored Windows round-trip test; aggregate and launch/environment-block validation reject malformed entries/NUL argv; Python and shell exit views are separate; Unix stdin bootstrap deadline influences decision and poll timeout; output/stdin operations have 256 KiB budgets; parent outputs are nonblocking before handshake cleanup and pending stdin has RAII closure.

No unresolved REVIEW-1 repair objections. Remaining original boundaries are explicit: real Windows execution and Job Object/handle/cleanup runtime proof pending; Unix process groups do not contain escaped sessions and report incomplete cleanup; cwd confinement alone is not a filesystem jail; PID namespace/procfs limits above. Python remains read-only reference and is not invoked by product process code. Root manifest/lock remains delivery-owned; this crate stays standalone pending explicit integration.
