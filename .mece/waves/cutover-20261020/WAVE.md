# Wave: Rust cutover + installers (2026-10-20)

## User decisions (2026-10-20)
"move to rust binaries and build the installer for win and linux (AppImage) … check win builds (I will log into my Windows later and test there) … studio journal yes, OpenCascade in rust yes, MinGW also yes, and also build for FreeCAD too if it already exists then ignore."
- The desktop app runs ONLY Rust binaries. The Python geometry worker is replaced by a Rust→OpenCascade C++ adapter (the parked `kala-cad`; it CALLS OCCT and never rewrites it). This is ADR-0010 phase 2.
- Installers: Linux AppImage (built and smoke-tested here); Windows NSIS (cross-checked here with MinGW; final build and test on the user's Windows machine via a script).
- FreeCAD: the addon (`adapters/freecad/mod/Kala`) and the adapter package ALREADY EXIST, so build nothing new. Bundle the adapter files so the FreeCAD provider works from the installed app (FreeCAD itself is not bundled).
- Studio journal: backed up to `~/.local/share/kala-agent/studio/studio-journal.sqlite3.test-pollution-20261019.bak` (done by Opus).
- MinGW: the user installs `mingw-w64-gcc` (sudo); windows-check starts after that.

## Rules
Workers follow `/home/v9i/Projects/Kala-Agent/.mece/waves/rust-backend-20261019/cells/CONVENTIONS.md`, including the USER DATA rule: in EVERY shell, `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo` and then temp `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR`, and `XDG_DATA_HOME`. OpenCode only (DeepSeek and MiniMax), at most 3 per model. No commit, push, or credential reads. No sudo (ask the conductor).

## Parity bar (the tests are the spec, unchanged from ADR-0009/0011)
- L-bracket: 35571.416047 mm³ (rel 1e-6); bounds [-30,30]×[0,45]×[0,40].
- XZ section at y=4: area 2400−25π, exact (rel 1e-9).
- The 28-case construction parity fixture (`tests/fixtures/construction_parity.json`).
- The STEP import view bounds match within 0.001 mm.
- Live differentials vs the Python reference: 268 / 292 / 20 / replay-56 at 0 mismatches, run with the native backend instead of the worker.

## Cells
1. cad-r2 (DeepSeek): the kala-cad OCCT adapter core (compile, measure, tessellate, STEP write and read).
2. native-journal (MiniMax, parallel): the Studio session journal + artifacts in Rust, byte-compatible with the Python file.
3. cad-r3 (after cad-r2): HLR drawing + section in kala-cad.
4. native-design (after cad-r2, native-journal): the native design service in kala-studio, with the default geometry backend = native; the Python worker becomes an opt-in dev fallback.
5. packaging (after native-design): Electron launches only Rust; OCCT libs + the FreeCAD adapter + the UI are staged; the AppImage is built and smoke-tested; Windows build script + NSIS config; the unreviewed delivery-r3 changes are reviewed.
6. windows-check (after MinGW): `cargo check` for x86_64-pc-windows-gnu across all crates, with cfg fixes. OCCT-linked crates are verified on Windows via the script.
