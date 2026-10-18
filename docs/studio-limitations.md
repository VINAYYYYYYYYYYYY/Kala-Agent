# Kala Studio limitations, engineering safety, packaging trust, testing

> Scope: what this wave explicitly does **not** ship, the scope of the
> chat-first agent and the OpenCascade geometry/estimation workbench,
> what an engineer must still do by hand, the trust posture of the
> unsigned installer, and the procedure to test the desktop bundle
> honestly.
>
> This document is the source of truth for honesty claims. The README
> links to it; the studio backend and the desktop bundle must
> continue to honour every claim here.

## Roadmap of limitations

The studio is a **preview build**. The following items are open and
will not be silently patched over:

### Native vendor integrations

- **SOLIDWORKS** — Windows COM automation. Requires a licensed
  installation of SOLIDWORKS on the host. The studio backend does not
  ship a COM bridge. The integration row reports
  `status: native_unavailable` with `platforms: ["windows"]`,
  `host_sdk_required: ["SOLIDWORKS"]`, and `notes: "Requires licensed
  SOLIDWORKS installation; COM automation not implemented in this
  wave"`.
- **Solid Edge** — Windows COM automation. Same posture as SOLIDWORKS.
- **AutoCAD** — `.NET` plugin / ObjectARX / automation. Requires
  licensed AutoCAD and a registered plugin assembly. The studio
  backend does not ship one. Integration row reports
  `native_unavailable` with `host_sdk_required: ["AutoCAD",
  "AutoCAD .NET plugin"]`.
- **Fusion 360** — Add-in. Requires a Fusion installation and an
  installed add-in from this repository. The studio backend does not
  ship arbitrary external Python; the Fusion add-in lives in a
  separate, future distribution.

A future wave may publish a vendor bridge; until then the studio
correctly refuses any "launch / connect / drive" claim.

### Analysis solver integrations

The integration registry lists five solver names as **representative
target integrations**: ANSYS Mechanical, Abaqus, COMSOL, Simcenter
Nastran, and Altair OptiStruct. They are **not** a ranked top-five
and **not** evidence-backed market leaders. The five-name set is a
request artefact from the wave brief; the studio asserts no
market-share claim about any of them.

The studio does **not**:

- Launch any solver executable.
- Read any solver result file (`.frd`, `.odb`, `.csv`, `.txt`, etc.)
- Fabricate a maximum displacement, a maximum von Mises stress, a
  factor of safety, or a pass/fail verdict.
- Provide a placeholder stub function that returns plausible numbers
  when the solver is absent.

Every analysis handoff returns either a neutral geometry file (STL or
STEP) or an honest `analysis_solver_not_available` error. The studio
never substitutes a stand-in computation for a real one.

### Open-source tools

- **FreeCAD** is reported by the registry as `discovered` /
  `not_found` from the host's `freecadcmd`. The studio does not
  import FreeCAD; FreeCAD integration is the responsibility of the
  existing `adapters/freecad` distribution, which uses the kernel's
  tool-execution seam.
- **OpenSCAD** is reported separately with status `discovered` /
  `not_found`. The studio does not embed an OpenSCAD interpreter
  and does not auto-execute user-supplied `.scad` scripts. There is
  no CSG-only route.
- **OpenCascade.js (WASM)** is a port of OpenCascade compiled to
  WebAssembly that could, in a future wave, give the studio a
  browser-side B-rep kernel. It is **not** used in this wave; the
  preview path is server-side analytic triangulation, not a
  browser-side B-rep. See
  <https://github.com/donalffons/opencascade.js>.

### Geometry limits

- WebGL renders **triangle meshes**, not exact B-rep. A user looking
  at the viewport sees a tessellation, not a parametric solid.
- STEP export requires the `[geometry]` extra
  (`cadquery-ocp>=7.7`). Without it, the export route returns
  `503` with an actionable message; the studio never ships a
  placeholder STEP.
- The preview mesh generators are covered by regression tests
  (`tests/unit/test_studio_geometry.py`). An independent check this
  wave computed positive (outward) signed volumes for a box, a
  cylinder, and a sphere, so triangles now wind consistently outward
  and the earlier winding-defect warning has been removed.
- Primitive counts are bounded. The studio backend rejects requests
  whose parameters would produce more triangles than the
  configuration allows; the limit is operator-tunable through
  `KALA_STUDIO_MESH_LIMIT`.

### Nominal geometry estimates

The workbench presents a nominal bounding envelope and nominal
analytical volume and surface-area estimates for the active
primitive. Their exact scope:

- **Nominal, closed-form, ideal-primitive values.** Volume is shown in
  `mm³` and `cm³`, surface area in `mm²` and `cm²`, and the bounding
  envelope (with its envelope volume) in `mm` / `mm³`. The values are
  the closed-form formulas for the ideal primitive (box `xyz` /
  `2(xy + yz + zx)`; cylinder `πr²h` / `2πr(r + h)`; sphere
  `(4/3)πr³` / `4πr²`) computed from the same validated parameters the
  preview mesh uses. Unit conversions are nominal: `1 cm³ = 1000 mm³`
  and `1 cm² = 100 mm²`.
- **Not solver output.** These numbers are a sizing aid. They are not
  measured from the tessellation, not a stress, displacement, factor
  of safety, fatigue life, mass, or certification, and never a
  fabricated FEA/CFD result.
- **Bounding envelope is server-verified.** The preview route returns
  the mesh and its `bounds`; the bounding envelope is re-derived from
  that server mesh, so a local approximation can be compared against
  the server result.
- **Exact B-rep confirmation needs OCP.** Exact B-rep volume and area
  mass properties are confirmed with OpenCascade's `BRepGProp` only
  when the `[geometry]` extra (`cadquery-ocp>=7.7`) is installed. The
  exported STEP shape round-trips to within `1e-7` relative of the
  closed-form volume. Without the extra, STEP export returns `503` and
  no exact mass property is claimed.
- **Tessellation is not exact.** The analytic preview mesh approximates
  curved surfaces. The sphere tessellated volume is `1.066 %` below the
  analytic sphere volume at the default resolution; the estimate table
  reports the analytic ideal value, not the tessellated one.

### Provider honesty

- The studio never claims to use a model the user has not configured.
  The `/api/studio/providers` route is the source of truth.
- The `codex` (ChatGPT subscription) route is **not implemented** in
  this build. It fails closed with a sanitised `503` and sends no
  request. A `codex` binary merely present on `PATH` does **not**
  make the provider usable, and the studio does not report the CLI's
  location or claim the CLI is signed in. The
  `KALA_STUDIO_ENABLE_CODEX` flag is inert.
- Subscription access and API access are distinct plans billed
  separately by the provider. In this build only the `openai` and
  `openrouter` routes make a real API call; they call fixed endpoints
  with the operator's key. The studio never converts a `codex`
  request into a paid API call and never reuses an inherited
  `OPENAI_API_KEY` for the subscription path.
- The studio exposes no arbitrary-provider escape hatch: the accepted
  provider identifiers are fixed in the request schema, and only
  `openai` and `openrouter` perform a real remote call.
- The studio does not assert legal or terms-of-service claims about
  third-party products. Provider-side eligibility is the provider's
  decision, not the studio's.

## Engineering safety

Kala Studio is a chat-first Mechanical Engineering Agent with an
integrated OpenCascade geometry, estimation, and neutral-file handoff
workbench. It captures intent in chat, builds parametric geometry,
reports nominal estimates, and exports a neutral file. It is **not** an
engineering analysis tool. The following items must remain outside the
studio's scope until a future wave decides otherwise with explicit
evidence:

- **Nominal estimates are not analysis.** The volume, surface-area,
  and bounding-envelope figures are closed-form values for the ideal
  primitive. They do not model material, load, constraint, mesh, or
  environment, and they carry no safety margin or certification.
- **STEP geometry is not ready-to-solve analysis.** STEP is a
  boundary representation. To run analysis an engineer must still
  choose a meshing strategy, mesh the body, assign materials,
  apply loads, apply boundary conditions, choose a solver, run the
  solver, and interpret the result. The studio's role ends at
  exporting a file the next tool can read.
- **No material database.** The studio does not store a material
  catalogue and does not look up Young's modulus, Poisson's ratio,
  yield strength, density, or fatigue data. The user supplies
  material data to the next tool.
- **No unit system beyond length.** The studio accepts dimensions in
  millimetres and treats them as nominal. It does not enforce a
  unit system on mass, force, pressure, temperature, or any other
  quantity. Exporting a STEP file does not commit the user to any
  particular unit interpretation downstream.
- **No tolerance stack-up, no GD&T.** These are engineering
  decisions, not parametric primitives.
- **No certification, no regulatory claim, no signed-off design.**
  Files produced by the studio carry no provenance beyond the
  operator's local environment. They are not engineering deliverables
  in any regulatory sense.
- **No persistent state.** The studio does not save revisions, mark
  approvals, or maintain a journal of changes. The kernel's
  revisioned, journaled, content-addressed artifact store is in the
  core and remains the system of record.

## Packaging trust

### Signing posture

- **No code-signing certificate** is configured in this wave. The
  Windows `.exe` and `.nsis.7z` artefacts are **unsigned**. SmartScreen
  will warn; the user is expected to right-click -> Properties ->
  Unblock and accept the publisher warning.
- **No Linux signing key** is configured. The AppImage has no GPG
  signature. A user who wants to verify the build must rely on the
  CI commit SHA and the artefact's SHA-256 digest.
- The CI matrix uploads the **unsigned artefact** with its **SHA-256
  digest** in the GitHub Actions summary. The digest is **not** a
  substitute for a signature; it only proves that the artefact
  uploaded to the run is the artefact the user downloaded.

### Provenance

- The CI matrix is `ubuntu-latest` and `windows-latest`, GitHub-hosted
  runners. The PyInstaller and electron-builder steps run in
  separate jobs; the backend binary is uploaded as one artefact, the
  desktop bundle as another.
- The CI matrix **does not** authenticate against any LLM provider,
  does not push to a registry, does not sign artefacts, and does not
  modify the host system.
- Source provenance comes from the GitHub commit SHA in the run
  header. A user who wants to rebuild must check out the same
  commit.

### Why this matters

The studio is unsigned because code-signing requires a paid
certificate (Windows) and a GPG key with a published trust chain
(Linux). This wave did **not** budget for either. Until signing is
in place:

- A user should only run the studio on a host they trust.
- An enterprise should treat the studio as developer-mode software
  and run it inside its own sandbox.
- An installer that is asked to elevate, install a service, or write
  to `/usr/` is **not** this build; this build does none of those
  things.

## Testing procedure

The wave's contract is **not** a tested end-to-end installer. It is
a build pipeline and a backend that together produce a working
desktop. The honest testing procedure:

1. **Local backend smoke test.** From a clean checkout, in a venv:

   ```bash
   pip install -e ".[studio]"
   # Optional: pip install -e ".[geometry]" for real STEP export.
   KALA_STUDIO_PYTHON="$PWD/.venv/bin/python" \
     kala-studio --host 127.0.0.1 --port 8765
   curl -fsS http://127.0.0.1:8765/api/studio/health
   ```

   Expect `{"status":"ok"}`.

2. **Local UI smoke test.** In a second terminal:

   ```bash
   cd studio
   npm ci
   npm run build
   # the backend above now serves the UI on http://127.0.0.1:8765/
   ```

   Open the URL in a browser; paste the printed token; create a
   box; export STL; observe the download.

3. **Frozen backend smoke test.** The PyInstaller backend bundle
   ships as `dist/kala-studio`. The repository ships a stdlib-only
   validator that launches the frozen binary on a free loopback
   port, waits for a bounded health deadline, checks the UI root and
   its relative assets, requests a preview mesh, and exports a real
   STEP file when the health probe advertises `step_available`:

   ```bash
   python packaging/studio_smoke.py --backend dist/kala-studio
   # --allow-missing-step relaxes the STEP requirement only if the
   # [geometry] writer is genuinely absent; CI never passes it.
   ```

   The probe never calls a live provider or solver; it uses a
   synthetic token and a temporary working directory, and reaps the
   child process in a `finally` block.

4. **Desktop bundle smoke test.** On the target OS (Windows or
   Linux), run the unsigned artefact:

   ```bash
   # Linux (kala-studio-<version>-<arch>.AppImage)
   chmod +x kala-studio-*-*.AppImage
   ./kala-studio-*-*.AppImage --appimage-extract-and-run
   ```

   On Windows the user-facing artefact is the NSIS installer
   `kala-studio-<version>-setup.exe`; right-click it, accept the
   publisher warning, and observe the window. The `.nsis.7z` payload
   is an internal archive, not an installer.

5. **CI artefact verification.** Download the backend artefact and
   the desktop artefact from the GitHub Actions run page. The job
   summary prints the **backend** binary's SHA-256; compare it to the
   downloaded backend binary. If they differ, do **not** run the
   artefact.

6. **Adversarial posture.** From a second host, attempt
   cross-origin requests to `http://127.0.0.1:8765/` and confirm the
   backend rejects them. Attempt a request with a wrong token and
   confirm a 401. Attempt a body larger than the configured cap and
   confirm a 413. Attempt a request with `provider: "openai"` and
   no `OPENAI_API_KEY` set; confirm the backend refuses. Attempt a
   request with `allow_remote: false` and `provider: "openai"`; the
   backend confirms the consent field is required.

### What this wave did not test

- The wave did **not** test the installer end-to-end on a clean
  Windows host. The Windows build path is configured in CI, but no
  Windows runtime has been exercised for this wave.
- The Linux AppImage configuration exists, but a generated AppImage
  is **not** proof that a desktop session works; no desktop shell
  has been validated on a clean Linux host.
- The wave did **not** test the AppImage on every supported Linux
  distro (GNOME, KDE, Hyprland, Sway, etc.).
- The wave did **not** run the studio against a real CAD or analysis
  product. The integration rows remain honest placeholders, and the
  studio holds no native CAD or solver automation.
- The wave did **not** implement app/OpenCode orchestration; the
  studio produces a neutral handoff only.
- The wave did **not** submit the artefact to VirusTotal, Microsoft,
  or any other reputation service. A user who needs that should
  arrange it separately.

## What changes would close these gaps

The next wave that addresses one of these items must update this
document with the new evidence and reference the test command. Do
not silently change a "no" into a "yes" in this file.
