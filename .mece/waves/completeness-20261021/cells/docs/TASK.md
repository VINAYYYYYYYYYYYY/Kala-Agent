# Cell docs: make the docs true again (round 1: everything that has landed)

Read `../../WAVE.md`, `../audit-backend/AUDIT.md` (rows B1, B2, B5, B8, B9, B12, B17, B18, B19), and the REPORTs of every landed cell: `../ui-fixes`, `../packaging`, `../runtime-fixes`, and in `../../../agent-providers-20261020/cells/`: `llm-design`, `cli-providers`, `model-catalog`, `model-ui`, `design-timeout`, `cut-guard`, `build-repair`, `polish`, `python-parity`, `repair-context`, `design-budget`.
**Rule: every statement you write must be verified against the code or a command you ran (cite file:line in your REPORT, not in the docs). No invented behaviour, no aspirational claims.** If code and REPORT disagree, the code wins: note it.

## Update
- `docs/studio-providers.md`: the 7 provider ids. For the 3 subscription CLIs (Claude Code, Codex, OpenCode): what they run (argv), the sandboxing (env allowlist, private workdir, tools denied), `allow_remote` consent, the optional model and the honest "CLI default model" display (the CLIs do not report it), the 180 s CLI budget, process-tree kill on cancel and timeout, Windows `.cmd` shims, the fast cached availability probe ("checking…"). The model catalog route `GET /providers/{id}/models` (OpenRouter live list incl. free models). The 30 s plain-chat / 180 s design-call budgets. What `offline` means now.
- `docs/studio.md`: the route table and count (verify against the router), a **"Design checks and limits"** section (500 total steps / 64 components; Solid Edge refuses > 8 parts; the FreeCAD build deadline of 1 800 s and the measured timings: 100 holes ≈ 1 min, 500 steps ≈ 17–21 min; the checks `brep_valid`, `single_solid`/`solid_count`, `envelope_*`, and `cuts_effective`, which is shown only when a cut removed nothing; the same-plane pocket rule; the per-pocket `removed no material` warnings; one repair round per turn, shared by validation and build, carrying the full instruction and the previous JSON; the reply display bounded to 4 096 chars while the design is read from the full reply). The user-visible UI behaviour from ui-fixes (elapsed timer, client deadline, viewport keys, honest view/source labels, default-model wording).
- `docs/studio-install.md`: the data-dir precedence (`--data-dir` > `KALA_STUDIO_DATA_DIR` > `KALA_AGENT_DATA_DIR/studio` > `$XDG_DATA_HOME/kala-agent/studio` > `~/.local/share/kala-agent/studio`); the token (the packaged flow passes it in the child environment and writes NO token file; fix the false claim); the backend log location (resolve Electron's `app.getPath('logs')` for this app name on Linux and Windows, and verify it from the packaging cell's evidence or the code); the startup error dialog; the FreeCAD add-on install from the AppImage (the packaging cell's single method); the Windows CLI notes. Leave the AppImage size/sha256 as "see packaging/linux/README.md", where round 2 will put the final digest.
- `docs/cad-providers.md`: the add-on install, the cut check, the plane rule, the per-pocket warnings, the limits.
- `docs/studio-limitations.md`: remove the gaps that are now fixed (e.g. `KALA_STUDIO_DATA_DIR`) and add the real remaining ones (500-step builds are slow; the Solid Edge 8-part cap; Windows Job Object not landed (`taskkill /T` fallback); unbounded journal and artifact growth (B12: document where the files live and that there is no retention yet); the live CLI model name unknown with "default").
- `README.md` architecture paragraph (B19: native Rust + OCCT, not PyInstaller/cadquery-ocp in the shipped app; the Python reference is dev-only).
- Stale header comments (B18) in `.github/workflows/*.yml` and `packaging/linux/smoke-appimage.mjs`: COMMENTS ONLY, no logic changes.

## Rules
Own `docs/**` (except `docs/adr/**`: do not rewrite ADRs), `README.md`, `packaging/linux/README.md` (prose only), and the comment headers named above. A code cell (downloads-cancel) is running concurrently on `server.rs`, `native_geometry/**`, `studio/src/**` and `studio/scripts/**`: do not document the artifact download route or the build cancel yet (round 2 does). Read-only commands only, apart from your doc edits. Temp HOME + `KALA_AGENT_DATA_DIR` if you run anything; never touch `/home/v9i/.local/share/kala-agent/**`. No commits. Clean up /tmp.

## REPORT
`REPORT.md` here: per file, what changed, and the verification citation for each non-trivial claim; plus the statements you removed as false.

## Round 2 (Opus, after downloads-cancel landed)
Read `../downloads-cancel/REPORT.md`, then document, under the same verify-against-code rule:
- the artifact download route `GET /api/studio/design/{design_id}/artifact/{kind}` (kinds `script|step|fcstd|stl`, only the kinds the design stored; exact provider bytes re-verified against the journalled sha256; 64 MiB cap; the filename form; the download button in the design card and export panel);
- Cancel: a client abort stops the work (the CLI process tree, the FreeCAD process group, the OCCT build at the next phase boundary), and one `design.cancelled` journal event with nothing after it. The Python reference cannot cancel a running sync build (dev-only, documented);
- `/health` `design_turn_budget_seconds` (3 960 = 2 × (180 + 1 800)) and the client deadline = budget + 5 min (70 min fallback);
- the route table/count now (re-count from the router: the REPORT says 16 Python / 18 native route-methods; verify);
- the readiness wording change ("geometry-backend seam");
- `studio/README.md:54` stale AppImage digest → pointer to `packaging/linux/README.md` (you may edit that one line);
- `docs/adr/0010-…md`: do NOT rewrite. Append ONE dated line under its "Phase 2 implemented" section: "Later builds supersede the AppImage digest above; the current artefact is recorded in packaging/linux/README.md." (the only ADR edit allowed).
Leave the `packaging/linux/README.md` digest row for Opus, who writes it after the final build. Append a "Round 2" section to REPORT.md.
