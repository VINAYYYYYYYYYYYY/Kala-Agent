# Entire backend Rust migration — queued after Windows-only milestone

## User authorization and sequencing
User: "after that change the entire back end into rust ... run the test to see the optimization ... deepseek v4.1 at mac [interpreted max], high context use go, otherwise free200k."
Finish and verify current Windows-only Solid Edge code milestone first. Keep live Windows/COM proof explicitly pending if no Windows host. Prepare migration inventory read-only while pending; no Rust implementation ownership overlap with current workers.

## Outcome
Replace the application/backend server and core runtime with standalone Rust, not merely a PyO3 accelerator. Preserve existing frontend/API/CLI/data/artifact contracts. Identify any unavoidable external CAD/solver bindings explicitly; do not call a Python-hosted backend a full Rust rewrite. Product runtime remains Windows-only for Solid Edge; Linux is development/testing only, not a new Linux Solid Edge adapter.

## Baseline facts before implementation
No Cargo.toml or .rs implementation found in current root or user-untracked Rust+buid123 subtree. cargo1.99.1/rustc1.99.1 available. rust-implementation.md is a user-untracked design/reference, NOT implemented Rust and NOT measured performance evidence. Preserve it and Rust+buid123 unchanged.

## Model/delegation policy
Latest explicit user instruction: GPT-6.1-Sol remains the ongoing architecture/code reviewer and final verifier throughout the Windows work and entire Rust migration. OpenCode performs implementation and repairs; Sol does not silently take over routine coding. Review each meaningful worker diff and check evidence before integration; failed checks return to the owning OpenCode worker.
Latest user routing overrides earlier policy: all new implementation and repair workers use ONLY opencode-go/deepseek-v4.1-flash with variant max through T3 delegation. No free-model or alternate-provider fallback. GPT-6.1-Sol conducts, reviews actual changes, and independently verifies. Up to three independent workers with disjoint ownership. Retain T3 task IDs; do not duplicate live work.

## Migration invariants
- Existing HTTP routes, payload/error shapes, bearer/Host/Origin/body gates and desktop backend launch contract preserved.
- Existing canonical JSON/hash-chain/artifact digests and Decimal budget semantics preserved; no JCS change, reduced precision, weaker journal verification or weaker durability just to improve a benchmark.
- SQLite schema/data compatibility or explicit backed-up tested migration, no data reset.
- Existing Python tests become reference/parity corpus; Rust unit/integration/security tests meaningful, no mirrors-only confidence.
- Native CAD proof remains real-host gated. Windows COM via supported native binding can replace bridge when demonstrated, not guessed.
- Preserve user changes; no commit/push/deploy withoutauthorization. Cargo/workspace/schema/lockfile single owner perwave.
- Report actual release-mode benchmark results, environment, repeats and correctness gates; projected speedups in the old design are not results.

## Acceptance and metrics
Full inventory before fileownership split; standalone Rust backend launchable by existing desktop/frontend; compatible critical endpoints and negative cases; Rust tests +existing interface parity; release-build measurements comparing same workloads to Python baseline on same host. Measure API latency/throughput, startup/RSS, geometry/STL and persistence/concurrency where implemented. Separate microbenchmarks from end-to-end numbers. No claim Windows COM or hostedCI execution happened without evidence.

## Next
Read-only audit first when current worker slot frees. Then contract-first Cargo workspace +vertical parity slices, dependent feature domains and persistence, provider/tool runtime, packaging cutover and release benchmarks. Exact ownership/contracts established from inventory, not guessed now.


## 2026-10-19 resumed state
Windows foundation accepted after final integration report and conductor rerun: 16 portable/boundary tests passed. Previous independent bridge186+1skip, client138, lint/format/source-mypy evidence retained. Native Windows/COM executor and real-host proof remain pending. Corpus accepted after conductor self_check.py including regeneration: 66 JSON, 85 Decimal, 48 Studio/49 core cases; live core route count is 17 total (audit18 was incorrect). Rust implementation starts now; no performance claim or packaging cutover.

## Active independent Rust cells
- foundation: backend root Cargo workspace/lock, README and crates/kala-json only. Native canonical JSON/chain compatibility; single shared manifest/lock owner.
- geometry: crates/kala-geometry only; native analytic meshes/STL and parity checks.
- budget: crates/kala-budget only; arbitrary-precision Decimal and ledger parity.
Geometry/budget have no foundation dependency this round and remain explicit workspace exclusions with independent crate locks. They must not edit parent manifests. Integrate into one workspace only after review. Reports/sessions belong to each cell. No unvalidated service or packaging cutover.

## Foundation accepted / persistence starts
Conductor independently verified kala-json23tests, fmt/clippy/release plus adversarial nested-input probe. See cells/foundation/REVIEW-1.md for consumer cautions. Foundation task completed without live child turns. Root workspace/lock ownership now transfers exclusively to persistence cell. Persistence owns schema/migration/journal/snapshots only this round; secure CAS and typed repositories remain later cells. Geometry and budget remain running/excluded with independent locks.

## Geometry accepted / standalone Studio starts
Conductor reviewed native geometry and independently ran32release tests plus fmt/clippy/release, allpass. See cells/geometry/REVIEW-1.md. Geometry worker terminal with no live turns; source now read-only to consumers. Studio cell owns ONLY crates/kala-studio with a local standalone workspace/lock; root manifests remain persistence-owned. Service uses native JSON/geometry dependencies. Native STEP is an explicit later dependency; existing unavailable behavior must be truthful, no Python product fallback or fake full-parity claim. Budget and persistence continue.

## Persistence review requires repair
Full62workspace tests independently passed, but Sol reproduced raw-ID whitespace tamper bypass and failed-COMMIT transaction leak, plus timestamp domain/precision mismatches. See cells/persistence/REVIEW-1.md. Persistence is not accepted. Fresh Go/max repair round2 owns same paths/root manifest; budget and Studio continue. Deep persistence JSON>1024 remains separate foundation compatibility work before full cutover.

## Budget review / resumed repairs
Budget initial55release tests/85corpus cases independently pass; Sol extra probes found Unicode numeric parsing and construction exponent-bound mismatches. Budget not accepted; focused Go/max repair round2 owns same excluded crate. Persistence repair round2 actually launched under SESSION task19093ff3...; Studio status running with pending child work, not duplicated.

## Budget accepted / native STEP starts
Budget round2 independently verified65release tests (85fixture cases plus Unicode760digits and exponent regressions), fmt/clippy/release clean. See budget/REVIEW-2.md. Native STEP cell now owns only crates/kala-step with standalone manifest/lock, no root or Studio edits. Persistence-r2 and Studio-r1 continue; up to3activeGo/max workers. Later integration separate; no Python STEP production sidecar permitted as full Rust implementation.

## Persistence repairs accepted / deep stored-data compatibility active
Conductor reviewed and independently reran76workspace tests plus fmt/clippy/release; prior chain raw-ID/transaction/time defects repaired. Deep_json cell now exclusively owns kala-json, specified persistence canonical/deep tests, root manifests/lock and narrowREADME. Additional explicit ownership exception: journal identifier trim predicates +newidentifier_whitespace_compat test only, to match Pythonstr.stripC0 separators after oracle verification; rawchainpreimages preserved. Studio and nativeSTEP stay disjoint and active. See deep_json/TASK.md and SESSION.md.

## Studio review / repair round2
Original Studio task completed with no live child turns. Sol independently ran65release tests and real corpus (40exact+6STEPdependency+2configcases); additional BOM/UTF16/32WindowsJSON and valid>1MiBsettings probes found regressions. Provider outputDTO/TLS root compatibility also under repair. Studio-r2 owns same standalone crate/docs only; see REVIEW-1/TASK-R2/SESSION-R2. Active independent cells:Studio-r2,STEP-r1,deep_json-r1; rootmanifest/lock remains deep_json-owned.

## Retry46 recovery and next independent cells

Recovered terminal Studio-r2 and deep_json-r1 results; neither has live child turns. Deep JSON independently accepted after 98 locked offline release tests and fmt/clippy/release build; see deep_json/REVIEW-1.md. STEP previous independent acceptance retained (35 release tests, 28 OCP cases, FreeCAD import); no source changes since that verification. Studio-r2 independently passed 78 release tests and real binary corpus with zero unexpected mismatches; see studio/REVIEW-2.md. Cutover stays pending its documented remaining gaps.

Three fresh T3 workers launched, each opencode/opencode-go/deepseek-v4.1-flash variant max: Studio-r3 owns ONLY kala-studio for real native STEP integration and final compatibility repairs; contracts-r1 owns ONLY new kala-contracts; artifacts-r1 owns ONLY new kala-artifacts. All are standalone crates with their own locks, no root/shared ownership overlap. Task/child IDs are in each cell LAUNCH-RETRY46.md. Root manifest/lock has no active writer and remains frozen until explicit integration assignment. Sol continues independent review; no routine coding takeover, commit/push/deploy or packaging cutover.

## Artifact result review / first repair

Artifacts-r1 terminal/no live turns; Sol independently passed40release tests, fmt, Linux all-target clippy, and Windows MSVC all-target check/clippy. Actual sys/windows source review found final writable bucket handle can bypass reparse/type validation in FILE_OPEN_IF success and fallback reopen. Linux slice accepted; Windows/fullcell acceptance withheld pending narrow repair and rename target-contract evidence. Fresh artifacts-r2 Go/max task recorded in cells/artifacts/LAUNCH-R2.md. Other Studio/contracts workers continue on disjoint crates. No Windows execution or performance claim. Firecrawl external docs unavailable due keyless host-IP rejection; no rename bug asserted without evidence.

## Artifact acceptance / workspace consolidation / Studio and contracts review

Artifacts-r2 terminal/no live turns and accepted after Sol reran40release tests/fmt/Linuxclippy/Windowscheck+clippy, reviewed final Windows handle validation and directory-relative rename metadata. See artifacts/REVIEW-2.md; Windows runtime is still pending. Root workspace/lock plus six accepted crate manifests/locks/readmes now exclusively workspace-r1-owned; see workspace/LAUNCH.md. No source ownership overlap with active Studio/contracts.

Studio-r3 terminal/no live turns; production nativeSTEP accepted after independent103release tests, actual HTTP OCP reimports,248livePython comparisons and releasebinary corpus zero unexpected mismatch. Sol extra settings probes confirmed validduplicate+surrogate file loses configuredkey and overwideinteger source500/native200; new narrowStudio-r4 active per REVIEW-3/TASK-R4/LAUNCH-R4. Contractsr1 report ready but task remains running, no duplicate launched. Independent38release tests/434cases pass; extra scratch/livePython probes confirm duplicate-model-input refusal and alias-order construction geometry change. See contracts/REVIEW-1, launch repair only after original terminal/no live turns. Active count3:workspace-r1,Studio-r4,contractsr1.

## Continue49: workspace accepted, native process cell starts

Workspace-r1 terminal/no live turns; Sol independently passed270locked offline release tests, fmt/workspaceclippy-Dwarnings/releasebuild. See workspace/REVIEW-1. Root membership remains six accepted crates with one authoritative lock; root ownership free until future assignment. Studio/contracts still running with pending turns; current contracts source shows last-wins/ordered-profile repair in progress, no duplicate task created. New process-r1 Go/max worker owns only standalone kala-process and reports for bounded native child process/sandbox infrastructure with Windows Job Objects; no dependency on unaccepted contracts. Active count3:Studio-r4,contracts-r1,process-r1. User asked ETA/Pythondeletion; Python preserved as reference until full replacement gates, several-hours estimate only and Windowsliveproofhost-gated.

## Contracts original result reviewed / focused repair round2

Original contracts-r1 completed/no live turns; final result consumed. Sol independently passed43release tests/fmt/clippy/450livecases and confirmed both initialreviewduplicate/order repairs. Further out-of-tree consumer/livePython probes reproduced three concrete failures: DesignSpec nonadjacent duplicate constraints accepted; serde privateNumbertag object misrepresented/rejected; duplicate+surrogate acceptedmodeldata rejected. Contracts not accepted, dependent writers remain gated. New Go/max contracts-r2 on same standalone ownership; exactbrief/findings/ID in cells/contracts/{REVIEW-2.md,TASK-R2.md,LAUNCH-R2.md}. Active3:Studio-r4,contracts-r2,process-r1. Root workspace/lock still no activewriter; no premature Python deletion/cutover.

## Studio accepted / native desktop delivery integration starts

Studio-r4 terminal/no live turns; independent115release tests/fmt/clippy/268livePythoncases passed. Sol actualreleasebinary GET/POST+finalfilebytes probes matchPython for prior duplicate+surrogate,4301digitfailure and privateNumbermarker cases; failedwrite leaves originalbytes intact. See studio/REVIEW-4.md; native9routes and realSTEP acceptedforintegration within explicitWindows/diagnosticlimits. Fresh delivery-r1 Go/max now sole rootworkspace/lock+specifiedStudio/desktopbuildmanifest/CI owner, no acceptedRustsource edits. It adds nativeStudio as seventhmember and stages native desktop separately while preserving Python sources/originalfrozenartifacts. Exactownership/ID in delivery/TASK.md/LAUNCH.md. Other activecells contracts-r2/process-r1 remain standalone/disjoint. No publish/commit/push/deploy, fullrewrite/Windowsruntime/performanceclaim.

## Contracts accepted / domain migration starts

Contracts-r2 terminal/no live turns. Sol independently passed50release tests/fmt/allfeatureclippy-Dwarnings and486livePythoncases; /tmpconsumercases now preserve privateNumberobject and duplicate+surrogate data and reject nonadjacentduplicates. Iterative ordered parser/canonicalconversion/Drop, fixedscopeprofilehints and fulluniqueness reviewed; accepted in contracts/REVIEW-3.md. Source remains read-only/standalone; root membership not reassigned fromdelivery. New domains-r1 Go/max ports all3purepackages/sharedsupport with explicit nativeprovenance+legacyboundary and live numerical/schema/capability tests. See domains/TASK.md/LAUNCH.md. Active3:delivery-r1,process-r1,domains-r1. Next independent queue on free slots:typedrepositories and policy/workflow/planning kernel, then tools/executor/application/coreAPI/CLI. Python remains reference; no prematuredeletion.

## Delivery review / source-drift guard repair

Delivery-r1 terminal/no live turns; Sol independently passed root385release tests,37focused desktoptests+typecheck and stage native-smoke. Code-review pureguard probes accepted .git/studio/src/docs as customstage while stageNative recursively removes destination: P1 source-data deletion risk, never destructively executed. UI copy dereferences symlinks despite advertisedrefusal. Same-review detected concurrent ADR0009/design-turns Pythonwave: current14routes vs Rust9, actual268case differential30mismatches and missingnative journal/designroutes. Preserveotherwave; originalStudioacceptedonlyfrozen9slice. Currentdefault/fullcutover withheld. Fresh delivery-r2Go/max on sameownedpaths fixes managed transactionalstaging/symlink/integrity and reference-default/native-explicit+current-feature readinessgate. See delivery/REVIEW-1/TASK-R2/LAUNCH-R2. ExpandednativeStudio port now required beforefullcutover; installedOCCT7.9headers/libs offer nativeRust/C++kernel route withoutPython. No sourcecode/scope takeover ofotherwave.

## Review54: process repair, domains findings, repositories starts

Process-r1 terminal/no live turns. Sol independently passed 38 locked offline release tests, fmt/clippy and all 10 opt-in strict sandbox tests; actual host verifies ten controls. Independent review found reversed Windows output pipe directions, absent Unix stdin bootstrap deadline, malformed environment constructor bypass, wrong Python signal status and unbounded I/O-loop guarantee gaps. Acceptance withheld, fresh process-r2 Go/max launched; see process/REVIEW-1, TASK-R2 and LAUNCH-R2.

Domains-r1 terminal/no live turns. Independent 25 release tests/fmt/clippy pass, but scratch/live Python comparison reproduces workflow mean-score tie mismatch and duplicate-model-JSON refusal. Shared behavioral dependency source is omitted from native provenance. Domains not accepted; focused domains-r2 brief queued in domains/TASK-R2 pending a free slot, not launched.

New repositories-r1 Go/max owns only standalone kala-repositories/** and cell reports, using accepted persistence/contracts/JSON read-only. Scope all typed repositories, immutable revision CAS, approval finality, memory expiry and bidirectional Python/SQLite readers; see repositories/TASK.md/LAUNCH.md. Root manifest/lock remains exclusively delivery-owned. No Python deletion/default cutover/full migration/Windows runtime/performance claim.

## Review55: process accepted, domain repair starts

Process-r2 terminal/no live turns. Independently all 46 offline locked release tests/fmt/Linux+MSVC+GNU all-target clippy-Dwarnings pass; opt-in strict sandbox 10 tests and actual helper pass. Prior scratch reprobes now reject malformed env, report -9, enforce 50 ms stdin bootstrap in 52.72 ms and bound continuous flood. Windows stdio direction/inheritance and new round-trip tests reviewed. Accepted for native process consumers in process/REVIEW-2.md within explicit Windows execution, process-group escape, PID namespace/procfs limits; no root integration yet.

Freed slot assigned to fresh domains-r2 Go/max with full original brief/report/findings and repair scope, recorded in domains/LAUNCH-R2.md. Current active three: delivery-r2, repositories-r1, domains-r2. Domains remains unaccepted pending repair review. Next disjoint queue: kernel policy/workflow/planning or native expanded CAD, then tools/runtime/application/API composition. Python remains intact and full cutover/Windows runtime/performance gates remain pending.

## Review56: repositories accepted, pure kernel starts

Repositories-r1 terminal/no live turns. Conductor reviewed all six repositories, immutable MemoryRecord, SQL/CAS/finality/expiry and private typed-error rollback adapter against Python/accepted Database. Independent 33 offline locked release tests/fmt/allfeature clippy passed, including actual bidirectional Python/SQLite/raw-wire checks and concurrent CAS/approval finality. Extra /tmp consumer compared 51 Pydantic model cases with zero mismatches. Accepted in repositories/REVIEW-1.md; Windows SQLite SDK/C toolchain/runtime and typed i64/API representation boundaries remain explicit. Standalone crate only, root ownership unchanged.

New kernel-r1 Go/max owns ONLY standalone kala-kernel/** plus REPORT/SESSION for entire pure policy/workflow/planning; accepted contracts/JSON/budget dependencies read-only, no dependency on domains or repository composition under review at launch, memory excluded. Exact brief/ID in kernel/TASK.md/LAUNCH.md. Active three: delivery-r2, domains-r2, kernel-r1. Next independent queue: memory over accepted repositories or native expanded CAD layer; then tools/executor/runtime/application/API/CLI composition. No premature Python deletion, cutover or Windows/performance claim.

## Review57: delivery repair still needs two concrete fixes

Delivery-r2 terminal/no live turns. Sol independently passed 57 focused packaging/Electron tests and typecheck. Actual helper source shows initial project protection/symlink/integrity/reference-default improvements, but controlled /tmp probe created an unmanaged destination during awaited hashes and observed successful staging delete its sentinel. Separate read-only installed electron-builder parser probe confirms --dir then --dir=false or --no-dir yields dir=false + win=nsis while decidePackaging allows development mode. No source/user deletion or installer execution occurred in probes. Acceptance withheld in delivery/REVIEW-2.md.

Fresh delivery-r3 Go/max launched with complete original briefs/prior report/findings, same ownership and no new memberships; see TASK-R3.md/LAUNCH-R3.md. This is the second focused staging repair attempt. Active three: delivery-r3, domains-r2, kernel-r1. Root manifest/lock remains delivery-owned. Current14/9 runtime parity remains pending expanded native port, no fake readiness or premature cutover. Python remains working reference.

## Continue recovery: domains accepted; existing workers preserved

Recovered all four task IDs through T3. Domains-r2 is completed with no pending child runs; delivery-r3, kernel-r1 and cad-r1 remain active and were not duplicated. Kernel's first run was cancelled, but its second run is active under the same task, so the original cancellation is not a fresh retry trigger.

Sol independently re-ran all 29 locked offline domain release tests (including actual live Python differential and dependency mutation gates), fmt, Linux/MSVC/GNU all-target/all-feature clippy-Dwarnings and the release build. The out-of-tree duplicate-key/score probes now return width40 and score0.299. Ordered parser/drop/conversion, compensated sum, dependency source binding and native/legacy identity boundary reviewed; accepted for read-only consumers in domains/REVIEW-2.md within its stated diagnostic/source-identity/Windows-runtime limits.

Current three implementation workers remain Go DeepSeek V4.1 Flash max: delivery-r3 owns guarded native desktop staging/build/root manifests; kernel-r1 owns standalone pure policy/workflow/planning; cad-r1 owns NEW standalone kala-cad with real native OCCT compiler/measure/mesh/STEP. CAD launch was recorded before the previous interruption in cells/cad/LAUNCH.md. No fourth worker, routine conductor implementation, Python deletion or default cutover. Next independent slot: memory over accepted repositories, followed by reviewed native tool/runtime/application composition; expanded Studio drawing/session/API follows accepted CAD.
