# GPT-6.1-Sol delivery review — integration useful, cutover withheld

Consumed terminal delivery-r1 result; no live child turns. Reviewed actual root/member manifests, Electron resolver, native staging/pack helper, workflow and source drift. Independent focused desktop tests37/37 and typecheck pass. Independent native_studio_smoke passes on staged native runtime/UI from temp packaged layout. Original9-route build works; full current-product default cutover is not accepted for reasons below.

## P1 — custom stage can recursively delete project/user data

`native-backend.cjs::stageNativeBackend` calls `fs.rmSync(stageDir,{recursive:true,force:true})` before writing/validating the replacement. `assertSafeStageDir` only protects a short list of paths. Independent read-only guard calls accepted all of `<repo>/.git`, `<repo>/studio/src`, and `<repo>/docs`; executing the advertised custom stage option for any would erase those directories. No destructive invocation was made. An unrelated existing nonempty custom directory is also not protected. Fix stage ownership/allowlist and transactional replacement: never clear arbitrary project/custom directories; allow only dedicated stage paths/new-empty customdirs or previously helper-owned stage with validated ownership metadata and no source/control paths. Validate sourcebinary/UI and replacement in a temporary sibling before replacing an existing managed stage, preserve old good stage on validation/I/O failure. Protect all project sources/control/userfiles, not merely more blacklist names. Missing CLI path values must error before mutation.

## P2 — input symlinks dereferenced despite advertised refusal

`fs.cpSync(distDir,...,{dereference:true})` follows UI-source symlinks before `validateNativeTree` walks the copied stage. The later walk sees ordinary copied files and cannot establish the claimed no-symlink input guarantee. Reject symlinked input roots/components/files before copying, and don't traverse them during copy. Test only temporary innocent external files; verify no existing managed stage or source is changed on rejection.

## Current-source compatibility drift blocks default cutover

Concurrent design-turn work is visible in `.mece/waves/design-turns-20261019/WAVE.md`, ADR0009, Python Studio sources and new tests. It deliberately owns no backend/Rust paths. Current Python router has14route-methods, adding design/drawing/section/journal/verify, health.design_available, chat.design and stored-design export. Rust covers original9 only.

Independent actual-source differential against backend/target/release/kala-studio:268cases,30mismatches (health extra design flag and chat.design changes among them). Additional real HTTP probes: GETjournal and GETjournal/verify Python200/native404; POSTdesign{} Python422/native404. This is intentional concurrent feature addition, not authorization to delete/normalize away it or a reason to edit the other wave. Previous accepted Studio evidence applies to the original frozen slice only.

Do not make the incomplete native slice the silent default for the current desktop, and do not let packaging/CI smokes limited to oldroutes count as full compatibility. Keep staged native build and explicit dev opt-in reviewable; preserve a working sourcePython reference default until expanded native compatibility gates pass. Add a concrete drift/readiness gate so packaging/release artifacts cannot claim currentfeature support while14-route/design/journal semantics are missing. No fake design availability or empty-journal stubs; no Python sidecar inside native runtime. Native port of ADR0009 is a subsequent Rust geometry/session/API cell. Preserve all other-wave source/UI/tests.

Also verify metadata hash/feature checks meaningfully validate what is packaged and always force `--publish never`; do not accept an overriding caller publish flag. Existing explicit-reference mode must use known interpreter path, never arbitrary PATH fallback. Root7-crate build integration can be retained; no accepted Rust source changes needed in this repair.

FullWindows/MSVC/NSIS/COM proof remains pending. Independent GUI execution is not yet repeated by Sol; worker's GUI evidence is reported as worker evidence. No approval/publication requested or performed.
