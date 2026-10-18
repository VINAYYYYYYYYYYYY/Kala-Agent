# New native Electron boot regression introduced by test gating

Independent actual launch after desktop fixes: `env -u ELECTRON_RUN_AS_NODE KALA_STUDIO_PYTHON=/home/v9i/Projects/Kala-Agent/.venv/bin/python npm run desktop`. Electron stays alive with NO backend and NO window. Mock75tests do not detect it.

Root cause PROVEN native Electron44.9: `require.main === module` is FALSE for main `.cjs` entry. require.main.filename is `electron`, not module path. Your new bottom guard prevents all app startup.

Conductor actual diagnostic `/tmp/kala-electron-entry-probe.cjs` prints `{entryEqualsModule:false,mainFilename:'electron',moduleFilename:'/tmp/kala-electron-entry-probe.cjs',electron:'44.9.0',appDefined:true}`. Native log `/tmp/kala-desktop-native.log` has only npm preamble, process had zero backend children. Do not change backend to workaround.

Own studio/**. Narrow fix real entry startup while keeping helper import side effects out of Vitest (e.g. separate main bootstrap entry and tested helpers, or a correct Electron browser-process guard with process.type; do not invent require.main semantics). Actual native app startup must invoke main, own one backend child with real --port and show renderer. Add regression test against actual startup gate path. Conductor will rerun actual native launch without sandbox disabling.

Also BackendProcess currently only records unexpected exit after readiness; main must close app/error visibly if owned backend exits unexpectedly, not leave dead window. Ensure process shutdown reaps child and no orphans. Launcher auto-build must use installed Vite JS or fail clearly (manual-regression brief already requests).

Preserve manual-connect fix and all other passing renderer/desktop tests. No commit/delegation. Update ABSOLUTE root report path, not studio/.mece. Do not start another PyInstaller/build: conductor has successfully built dist/kala-studio (131s actual; worker shell default120s was killing build).
