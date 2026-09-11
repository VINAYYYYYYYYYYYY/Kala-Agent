from pathlib import Path

from PyInstaller.building.api import COLLECT, EXE, PYZ
from PyInstaller.building.build_main import Analysis
from PyInstaller.utils.hooks import collect_data_files

block_cipher = None
# SPECPATH is the directory that contains this .spec file.
root = Path(SPECPATH).resolve().parent

datas = [
    (str(root / "kala" / "procedures" / "library"), "kala/procedures/library"),
]
datas += collect_data_files("PySide6")

hiddenimports = [
    "kala",
    "kala.agent",
    "kala.agent.loop",
    "kala.cad",
    "kala.cad.factory",
    "kala.cad.mock",
    "kala.cad.freecad",
    "kala.cad.freecad.backend",
    "kala.cad.registry",
    "kala.llm.stub",
    "kala.ml.stub",
    "kala.parts.catalog",
    "kala.procedures",
    "kala.procedures.gate",  # PartSpec, PartPlan, ClarifyNeeded, assess_goal
    "kala.procedures.schema",
    "kala.session.state",
    "kala.ui.desktop",
    "kala.ui.providers_dialog",
    "kala.llm.providers",
    "kala.llm.model_catalog",
]

a = Analysis(
    [str(root / "kala" / "ui" / "desktop.py")],
    pathex=[str(root)],
    binaries=[],
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[
        "PySide6.QtWebEngineCore",
        "PySide6.QtWebEngineWidgets",
        "PySide6.QtWebEngineQuick",
        "PySide6.Qt3DCore",
        "PySide6.Qt3DRender",
        "PySide6.Qt3DInput",
        "PySide6.Qt3DLogic",
        "PySide6.Qt3DAnimation",
        "PySide6.Qt3DExtras",
        "PySide6.QtCharts",
        "PySide6.QtDataVisualization",
        "PySide6.QtGraphs",
        "matplotlib",
        "numpy",
        "scipy",
    ],
    win_no_prefer_redirects=False,
    win_private_assemblies=False,
    cipher=block_cipher,
    noarchive=False,
)

pyz = PYZ(a.pure, a.zipped_data, cipher=block_cipher)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="kala-gui",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,
)

coll = COLLECT(
    exe,
    a.binaries,
    a.zipfiles,
    a.datas,
    strip=False,
    upx=False,
    name="Kala",
)
