# Sol independent domains repair review — accepted

Domains-r2 is terminal with no live child turns. Recovered its final task result and reviewed the actual repair source, regression tests and explicit provenance boundary. Accepted for read-only native consumers; this does not approve application cutover or legacy approval reuse.

Independent verification on 2026-10-19:
- `cargo test --offline --locked --release`: all 29 tests pass (5 library, 15 domain contracts, 4 differential, 5 source-mutation). The live oracle is mandatory and actually ran against the repository Python environment.
- `cargo fmt --all --check`, Linux all-target/all-feature clippy with `-D warnings`, both `x86_64-pc-windows-msvc` and `x86_64-pc-windows-gnu` all-target/all-feature clippy with `-D warnings`, and locked offline release build: pass. Windows checks are compilation/lint evidence only.
- Re-ran the independent out-of-tree `/tmp/kala-sol-domain-review` consumer: duplicate `width_mm` input now accepts its last value, 40; the four-score reproduction now returns `Some(0.299)`.

Review findings resolved:
1. Workflow mean calculation mirrors CPython's compensated float sum before the existing reviewed clamp/rounding path. The original rounding failure and adversarial order/length/None cases have live bit-level comparisons.
2. Ordinary model JSON uses an iterative ordered parser with last-wins duplicate assignment and first insertion position. Unicode escape pairing, non-finite literals, integer bounds, container cleanup and private serde-marker-shaped data are handled without a lossy serde value intermediary. Strict canonical/hash parsing remains distinct. Both text and byte paths are exercised across all three domains.
3. Build-time package identities now bind accepted local dependency source trees and manifests for kala-budget/kala-json/kala-contracts. Mutation tests exercise each dependency and a dependency manifest; embedded values equal fresh source recomputation. The native Rust scheme, historical Python scheme and required downstream re-preparation/re-approval remain explicit.

Consumer limits remain: first-error-only validation and generic JSON parse diagnostics are not complete Pydantic error aggregation/text parity. This source identity does not bind external crates, compiler/target/profile, linker or executable bytes. Downstream application/repository composition must enforce the native-versus-legacy identity boundary. No Windows runtime, application migration, packaging cutover or performance claim follows from this acceptance. The crate remains standalone; root membership requires a separately owned integration task.
