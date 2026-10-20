# rust-e2e-qa: the real Studio UI through the Rust front door (READ-ONLY QA)
Read `../r2-studio-proxy/REPORT.md`, `../r2-followup/REPORT.md`, ADR-0010/0011, and `/home/v9i/Projects/Kala-Agent/.mece/waves/design-turns-20261019/cells/d1-qa/REPORT.md` (how D1 drove headless Chromium; reuse its harness approach WITHOUT the provider/mesh shims, which C2 made unnecessary).
Do NOT edit any source (Rust, Python, or studio). You may write scripts and logs under /tmp and your own `shots/` and REPORT.md. A UI worker (C3) is editing studio/src concurrently: build `studio/dist` once at your start, from current sources, into a temp copy, and test that.
1. Build the release `kala-studio` (`cargo build --release -p kala-studio` in backend/). Run it as the front door with the Python geometry worker (dev default or `KALA_STUDIO_GEOMETRY_WORKER`), serving your dist copy. Note the exact command line.
2. In headless Chromium at 1440x900, connect and select **FreeCAD** in the CAD target selector, then send the Stitch L-bracket sentence. Verify on screen: "measured by freecad 1.1.4", volume 35571.42 mm³, B-Rep Verified, the HLR drawing with hidden lines, Section 2,321.46 mm², the ledger with server rows + HASH-CHAIN OK (note if server rows are missing; C3 is fixing that), and STEP export with sha256.
   Repeat with OpenCascade (opt-in), and confirm that Solid Edge is disabled with its reason.
   Then a two-part place-only assembly via chat or the API: solid_count 2, verified.
3. Kill the worker process mid-session: the geometry routes show a clear 503-style state in the UI, chat/settings keep working, the worker restarts, and design works again.
4. Run the same flows against the Python public server, and diff the visible results and network payloads (status codes and JSON) between the two front doors.
REPORT.md: a pass/fail checklist, any Rust-vs-Python differences, console/network errors, and screenshots in `shots/`.
