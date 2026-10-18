# Kala Studio desktop backend PyInstaller spec.
#
# Owned by the delivery cell. The studio backend is owned by the
# backend cell and lives in src/kala_agent/studio/. This spec is the
# seam between the source distribution and the unsigned desktop
# bundle that `.github/workflows/studio.yml` uploads.
#
# Build:
#     pyinstaller packaging/studio-backend.spec --noconfirm
#
# Output:
#     dist/kala-studio          (POSIX onefile)
#     dist/kala-studio.exe      (Windows onefile)
#
# Contract with the backend cell:
#   The frozen process imports ``kala_agent.studio.__main__:main``
#   (re-exported by ``packaging/studio_entry.py``). The backend
#   resolves its UI root relative to ``sys._MEIPASS`` / ``studio_dist``
#   first, falling back to the repo-relative ``studio/dist`` path
#   during development. PyInstaller collects the UI files into the
#   bundle's ``studio_dist/`` resource directory; the backend looks
#   up ``index.html`` under that directory. Any change to either
#   side is a contract change and must be coordinated.
#
# Notes:
# - ``--onefile`` keeps the desktop bundle to a single binary; the
#   electron-builder config picks it up at
#   ``studio/backend/kala-studio[.exe]``.
# - PyInstaller evaluates the spec at build time. It exposes the
#   spec's own directory as the global ``SPECPATH`` (a ``str``),
#   which is the right anchor regardless of the caller's CWD. We do
#   not rely on ``__file__`` because the spec is not a regular Python
#   module.
# - ``module_collection_mode = {"kala_agent": "pyz+py"}`` instructs
#   PyInstaller to keep the source ``.py`` files on disk under
#   ``sys._MEIPASS`` (in addition to the bytecode inside ``PYZ.pyz``).
#   The hardened core's startup code calls
#   ``module.__file__.stat()`` to compute module digests; without the
#   source-on-disk mode that synthetic ``__file__`` resolves to a
#   path that does not exist and the frozen process cannot start.
#   PyInstaller's own ``module_collection_mode`` is the supported API
#   for this and avoids editing the core.
# - ``hiddenimports`` forces the loader to pick up the studio
#   backend's modules that are discovered dynamically (FastAPI route
#   handlers, provider adapters). The backend cell publishes the
#   canonical list in ``src/kala_agent/studio/__init__.py``.
# - ``excludes`` removes heavy modules the desktop bundle does not
#   need (pytest, mypy, ipython, jupyter). This trims the binary;
#   the backend never imports them.

from __future__ import annotations

import sys
from pathlib import Path

from PyInstaller.utils.hooks import collect_submodules, copy_metadata

# PyInstaller injects ``SPECPATH`` (a str) into the spec's globals,
# and ``SPECPATH`` is the directory that *contains* the spec file --
# not the file itself. Older revisions of this spec wrapped the
# result in ``.parent`` unconditionally, which stripped the spec's
# own directory and made ``STUDIO_DIST`` resolve to a sibling of
# ``packaging/`` instead of ``<repo>/studio/dist``. The probe
# harness in ``/tmp/opencode/probe.spec`` confirmed
# ``Path(SPECPATH).is_dir() == True`` and ``Path(SPECPATH).resolve()
# == packaging``. We therefore use ``SPECPATH`` directly when it is
# a directory and only ``.parent`` the file-path fallback.
_SPECPATH_RAW = globals().get("SPECPATH")
if _SPECPATH_RAW and Path(_SPECPATH_RAW).is_dir():
    SPEC_DIR = Path(_SPECPATH_RAW).resolve()
elif _SPECPATH_RAW:
    SPEC_DIR = Path(_SPECPATH_RAW).resolve().parent
elif sys.argv and sys.argv[0]:
    SPEC_DIR = Path(sys.argv[0]).resolve().parent
else:
    raise SystemExit("could not resolve spec directory; SPECPATH unset and sys.argv empty")
REPO_ROOT = SPEC_DIR.parent
STUDIO_DIST = REPO_ROOT / "studio" / "dist"

# Studio backend dynamic modules. The studio package's `__init__`
# transitively imports the parent `kala_agent` package, which in
# turn imports `kala_agent.application`. That module is a
# composition root and pulls in every domain, the kernel, the
# persistence layer, the runtime, and the API. We therefore bundle
# the entire `kala_agent` namespace via `collect_submodules`; the
# frozen process must reflect the actual dependency graph, not a
# hand-curated subset. The conductor's review notes flagged this
# directly: avoiding studio → middleware → core import paths
# does not bypass Python parent package initialisation.
HIDDENIMPORTS = [
    "uvicorn.logging",
    "uvicorn.loops",
    "uvicorn.loops.auto",
    "uvicorn.protocols",
    "uvicorn.protocols.http",
    "uvicorn.protocols.http.auto",
    "uvicorn.protocols.websockets",
    "uvicorn.protocols.websockets.auto",
    "uvicorn.lifespan",
    "uvicorn.lifespan.on",
]
HIDDENIMPORTS.extend(collect_submodules("kala_agent"))

EXCLUDES = [
    "pytest",
    "pytest_asyncio",
    "mypy",
    "mypy_extensions",
    "ipython",
    "jupyter",
    "notebook",
    "tkinter",
    "matplotlib",
    "numpy.tests",
    "pandas.tests",
]

datas = []
# Bundle the studio UI build. The desktop bundle MUST include the UI;
# failing the build when ``studio/dist`` is absent is the contract.
# CI runs ``npm run build`` before invoking PyInstaller; the developer
# workflow must do the same.
if STUDIO_DIST.is_dir():
    datas.append((str(STUDIO_DIST), "studio_dist"))
else:
    raise SystemExit(
        "studio/dist not found; the desktop bundle MUST include the UI. "
        "Run `npm run build` inside studio/ first, then re-run "
        "PyInstaller. CI does this in the same job; local builds must "
        "match.",
    )

# Include distribution metadata for ``importlib.metadata`` look-ups
# (Pydantic plugin discovery, FastAPI version detection).
datas.extend(copy_metadata("fastapi"))
datas.extend(copy_metadata("pydantic"))
datas.extend(copy_metadata("anyio"))
datas.extend(copy_metadata("starlette"))


# ``Analysis``, ``PYZ``, ``EXE`` are injected into the spec's globals by
# PyInstaller at build time; ruff sees them as undefined names which is
# expected for a PyInstaller spec.
a = Analysis(  # noqa: F821
    [str(SPEC_DIR / "studio_entry.py")],
    pathex=[str(REPO_ROOT / "src")],
    binaries=[],
    datas=datas,
    hiddenimports=HIDDENIMPORTS,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=EXCLUDES,
    noarchive=False,
    # Keep ``kala_agent`` source ``.py`` files on disk under
    # ``sys._MEIPASS`` so the core's startup ``module.__file__.stat()``
    # works. See the spec's Notes section.
    module_collection_mode={"kala_agent": "pyz+py"},
)
pyz = PYZ(a.pure)  # noqa: F821

exe = EXE(  # noqa: F821
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="kala-studio",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=True,
    disable_windowed_redirector=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
