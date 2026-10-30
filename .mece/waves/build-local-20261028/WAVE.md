# Wave: build the local app after pulling 66465c0 (2026-10-28)

## User request
"build the local app". This follows "check the recent changes that happened in the git repo, update the local files": Opus fast-forwarded `main` to `origin/main` `66465c0` ("Fix security audit findings and Windows portability issues"). The uncommitted local backend work (edit turns + usage, ADR-0012) was put back on top and merged cleanly.

## Update 2026-10-30
Re-launched on user request ("pull the repo from github and update the local app"). `main` == `origin/main` == `a31229a` (fetched 2026-10-30; nothing new upstream). The edit-turns/usage work is now committed in `a31229a`, so the tree to package is clean HEAD `a31229a`. Blockers 1 and 2 below are confirmed still present at `a31229a`. `/tmp` had 5.5 GB free at launch.

## Opus gates on the merged tree (before this wave)
- Rust `cargo test --workspace`: 587 passed / 0 failed; `cargo fmt --check` clean.
- Solid Edge bridge pytest: 221 passed, 1 skipped (Windows-only).
- Studio vitest: 794/794 (54 files). `npm ci` gave electron 44.10.0 and happy-dom 21.0.4. The Electron binary was downloaded via `node node_modules/electron/install.js`.
- **BLOCKERS** (from 66465c0, apparently only exercised on Windows):
  1. **`npm run typecheck` fails, so `npm run build` and packaging fail.**
     - `studio/electron/main.cjs` gained `nonce` / `requireLaunchNonce`, but `studio/electron/main.d.cts` was not updated. That breaks `test/electronMain.test.ts` lines 79, 85 and 665; line 646 is a possibly-undefined.
     - `test/freecadVendorHashes.test.ts` lines 75–76 (`digests` possibly undefined) and line 258 (`hashes: undefined` under `exactOptionalPropertyTypes`).
  2. **pytest `tests/unit/test_private_files.py::test_tighten_file_permissions_never_raises_for_an_unrestrictable_file`** fails on Linux. Its `refuse(_path)` stub replaces `os.chmod`, which is called with two arguments on POSIX, so it raises TypeError rather than OSError.

## Rules
- USER DATA rule. In every shell, `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo`, then a temp `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`. Set `npm_config_cache=/home/v9i/.npm`. Never touch `/home/v9i/.local/share/kala-agent/**`.
- Never read credential, auth or settings files. Never run the real `claude`, `codex` or `opencode` CLIs.
- `/tmp` is a 7.7 GB quota tmpfs. Check `df -h /tmp` before packaging. Remove only your own temp dirs. Never touch `/tmp/opencode` or `/tmp/pytest-of-v9i`.
- No commits. Do not touch `~/Applications/**`.
