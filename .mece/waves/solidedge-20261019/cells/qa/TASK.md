# Boundary and CI integration

ACTIVE SCOPE OVERRIDE: target integration is Windows only. Both client and bridge run locally on Windows; no Linux runtime adapter/SSH. Linux jobs may run pure code tests but do not claim target support. Include tests/unit/test_portable_imports.py in Windows smoke with root+client+bridge installed. Do not claim strict sandbox ported or live Solid Edge proof from hosted Windows CI. Package test suites run in separate pytest processes/configs to avoid duplicate test-module name collisions. Preserve portable worker owned files; this cell owns only its boundary test and workflow.

Launch only after both new packages exist. Read WAVE.md and reports. Own ONLY tests/unit/test_solidedge_boundaries.py, .github/workflows/solidedge.yml and your cell REPORT.md. Do not edit packages, existing tests/config or docs. No delegation/commit/push/installs.

Add AST boundaries: core src/kala_agent never imports new adapters/shared CAD; bridge never imports kala_agent*; Linux adapter imports core contracts only (no Studio yet); no FreeCAD coupling. Lazy pywin32 imports only inside session.py; importing bridge protocol on Linux doesn't load pythoncom/win32com/pywintypes or core. Do not execute Windows or external network.

Separate CI workflow triggered by new adapter paths/this test/workflow: bridge-only unit/lint/type checks on ubuntu+windows; Windows never installs/imports core. Linux installs root+bridge+adapter and runs new package tests plus root boundary tests and ruff/mypy checks. Python3.12. Skip live solidedge tests by default, no remote native-proof claim. No desktop builds/unrelated changes or invented coverage gate.

Run boundary tests and parse YAML if existing tool available; remote workflows unrun. REPORT actual commands/results and paths.

