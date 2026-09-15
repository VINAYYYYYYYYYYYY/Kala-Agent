"""Push live geometry into the FreeCAD GUI window."""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import textwrap
import time
from pathlib import Path


def repo_outputs_dir() -> Path:
    root = Path(__file__).resolve().parents[3]
    out = Path(os.environ.get("KALA_OUTPUT_DIR", root / "outputs"))
    if not out.is_absolute():
        out = root / out
    out.mkdir(parents=True, exist_ok=True)
    return out


def default_live_path() -> Path:
    """GUI-facing FreeCAD document (written via freecadcmd for compatibility)."""
    return (repo_outputs_dir() / "kala_live.FCStd").resolve()


def default_step_path() -> Path:
    return (repo_outputs_dir() / "kala_live.step").resolve()


def reload_flag_path() -> Path:
    return (repo_outputs_dir() / "kala_live.reload").resolve()


def mod_heartbeat_path() -> Path:
    return (repo_outputs_dir() / "kala_mod_alive").resolve()


def freecad_bin() -> str:
    for name in ("freecad", "FreeCAD"):
        path = shutil.which(name)
        if path:
            return path
    return "freecad"


def freecadcmd_bin() -> str:
    for name in ("freecadcmd", "FreeCADCmd"):
        path = shutil.which(name)
        if path:
            return path
    return "freecadcmd"


def _freecad_running() -> bool:
    try:
        for name in ("freecad", "FreeCAD"):
            detail = subprocess.run(
                ["pgrep", "-a", "-x", name],
                capture_output=True,
                text=True,
                check=False,
            )
            live = [
                ln
                for ln in detail.stdout.splitlines()
                if ln.strip() and "defunct" not in ln
            ]
            if live:
                return True
        return False
    except OSError:
        return False


def mod_heartbeat_fresh(max_age_sec: float = 5.0) -> bool:
    path = mod_heartbeat_path()
    if not path.is_file():
        return False
    try:
        age = time.time() - path.stat().st_mtime
    except OSError:
        return False
    return age <= max_age_sec


def _close_kala_preview_windows() -> None:
    """No-op: never kill FreeCAD.

    Older versions killed any ``freecad`` process whose cmdline contained
    ``kala_live``. After the first open that matched *every* live preview, so
    each force-open closed the window the user was watching.
    """
    return


def launch_freecad_with_document(fcstd_path: Path) -> str:
    """
    Open geometry in FreeCAD GUI.

    FreeCAD 1.x desktop entry is: ``FreeCAD - --single-instance %F``.
    Passing the file without the leading ``-`` is ignored — you get an empty
    window. Prefer STEP; fall back to FCStd. If FreeCAD is already running,
    only signal a Mod reload — do not kill or restart the process.
    """
    fcstd_path = fcstd_path.resolve()
    step_path = default_step_path()
    open_path = step_path if step_path.is_file() and step_path.stat().st_size > 100 else fcstd_path
    if not open_path.is_file():
        return f"Cannot launch FreeCAD — missing {open_path}"

    # Already running with Kala bridge → soft reload only (no kill / no restart)
    if _freecad_running() and mod_heartbeat_fresh(8.0):
        reload_flag_path().write_text(str(time.time()), encoding="utf-8")
        return f"Reloaded in running FreeCAD ({open_path.name})"

    cmd = [freecad_bin(), "-", "--single-instance", str(open_path)]
    try:
        from kala.session.runlog import freecad_gui_log_path, ensure_freecad_file_logging

        ensure_freecad_file_logging()
        gui_log = freecad_gui_log_path()
        log_fh = open(gui_log, "a", encoding="utf-8")  # noqa: SIM115
        log_fh.write(f"\n--- launch {time.strftime('%Y-%m-%d %H:%M:%S')} {' '.join(cmd)}\n")
        log_fh.flush()
        subprocess.Popen(
            cmd,
            stdout=log_fh,
            stderr=log_fh,
            start_new_session=True,
            env={**os.environ},
        )
    except OSError as exc:
        return f"FreeCAD launch failed: {exc}"
    return f"Opened FreeCAD with {open_path}"

def step_to_fcstd(step_path: Path, fcstd_path: Path) -> str:
    """Convert STEP → FCStd using freecadcmd (same writer as the GUI)."""
    step_path = step_path.resolve()
    fcstd_path = fcstd_path.resolve()
    if not step_path.exists():
        return f"STEP missing: {step_path}"

    script = textwrap.dedent(
        f"""\
        import FreeCAD as App
        import Import
        step = r"{step_path}"
        out = r"{fcstd_path}"
        for name in list(App.listDocuments().keys()):
            App.closeDocument(name)
        doc = App.newDocument("kala_live")
        Import.insert(step, "kala_live")
        doc.recompute()
        # Rename imported shape so it is obvious in the tree
        for obj in doc.Objects:
            if hasattr(obj, "Shape") and not obj.Shape.isNull():
                try:
                    obj.Label = "KalaSolid"
                except Exception:
                    pass
        doc.recompute()
        doc.saveAs(out)
        App.closeDocument("kala_live")
        print("KALA_FCSTD_OK", out)
        """
    )
    with tempfile.NamedTemporaryFile("w", suffix=".py", delete=False) as fh:
        fh.write(script)
        script_path = fh.name
    try:
        proc = subprocess.run(
            [freecadcmd_bin(), script_path],
            capture_output=True,
            text=True,
            timeout=120,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return f"freecadcmd failed: {exc}"
    finally:
        Path(script_path).unlink(missing_ok=True)

    if "KALA_FCSTD_OK" not in (proc.stdout + proc.stderr):
        err = (proc.stderr or proc.stdout or "").strip()[-400:]
        return f"freecadcmd convert failed: {err or proc.returncode}"
    return f"Wrote GUI document {fcstd_path.name}"


def freecad_user_mod_roots() -> list[Path]:
    """FreeCAD 1.x uses …/FreeCAD/v1-1/Mod; older layouts used …/FreeCAD/Mod."""
    roots: list[Path] = []
    base = Path.home() / ".local/share/FreeCAD"
    # Prefer versioned app-data Mod (FreeCAD 1.1+)
    for child in sorted(base.glob("v*/Mod"), reverse=True):
        roots.append(child)
    roots.append(base / "Mod")
    roots.append(Path.home() / ".FreeCAD" / "Mod")
    # De-dupe while preserving order
    seen: set[Path] = set()
    out: list[Path] = []
    for r in roots:
        rp = r.resolve() if r.exists() else r
        if rp in seen:
            continue
        seen.add(rp)
        out.append(r)
    return out


def install_kala_freecad_mod(live_path: Path | None = None) -> Path | None:
    """Install FreeCAD Mod that auto-reloads kala_live and writes a heartbeat."""
    live = (live_path or default_live_path()).resolve()
    step = default_step_path()
    roots = freecad_user_mod_roots()
    # Always install into versioned path first when present/creatable
    preferred = Path.home() / ".local/share/FreeCAD/v1-1/Mod/Kala"
    candidates = [preferred] + [r / "Kala" for r in roots]
    mod_dir: Path | None = None
    for cand in candidates:
        try:
            cand.mkdir(parents=True, exist_ok=True)
            mod_dir = cand
            break
        except OSError:
            continue
    if mod_dir is None:
        return None

    # Mirror into legacy Mod path too (harmless if unused)
    legacy = Path.home() / ".local/share/FreeCAD/Mod/Kala"
    try:
        legacy.mkdir(parents=True, exist_ok=True)
    except OSError:
        legacy = None

    # Remove broken package.xml that referenced a non-existent workbench class
    for d in {mod_dir, legacy}:
        if d is None:
            continue
        pkg = d / "package.xml"
        if pkg.exists():
            pkg.unlink()

    init_py = 'print("[Kala] FreeCAD bridge ready")\n'
    (mod_dir / "Init.py").write_text(init_py, encoding="utf-8")
    if legacy is not None:
        (legacy / "Init.py").write_text(init_py, encoding="utf-8")

    flag = reload_flag_path()
    heartbeat = mod_heartbeat_path()
    from kala.cad.freecad.mod_template import render_init_gui

    init_gui = render_init_gui(
        live=str(live),
        step=str(step),
        flag=str(flag),
        heartbeat=str(heartbeat),
    )
    (mod_dir / "InitGui.py").write_text(init_gui, encoding="utf-8")
    if legacy is not None:
        (legacy / "InitGui.py").write_text(init_gui, encoding="utf-8")

    for macro_dir in (
        Path.home() / ".local/share/FreeCAD/v1-1/Macro",
        Path.home() / ".local/share/FreeCAD/Macro",
    ):
        try:
            macro_dir.mkdir(parents=True, exist_ok=True)
            (macro_dir / "KalaAutoReload.FCMacro").write_text(
                f'exec(open(r"{mod_dir / "InitGui.py"}", encoding="utf-8").read())\n',
                encoding="utf-8",
            )
        except OSError:
            pass
    return mod_dir


def publish_to_gui(
    *,
    step_path: Path,
    fcstd_path: Path,
    force_open: bool = False,
) -> str:
    """Build a GUI-compatible FCStd and open/reload it in FreeCAD."""
    install_kala_freecad_mod(fcstd_path)
    convert_note = step_to_fcstd(step_path, fcstd_path)
    if convert_note.startswith("freecadcmd") or convert_note.startswith("STEP"):
        return convert_note

    reload_flag_path().write_text(str(time.time()), encoding="utf-8")

    running = _freecad_running()
    heartbeat_ok = mod_heartbeat_fresh(8.0)

    if force_open:
        # Prefer soft reload when FreeCAD is already up — never kill it.
        if running and heartbeat_ok:
            return (
                f"{convert_note}. Reloaded running FreeCAD "
                f"(heartbeat ok). Live: {fcstd_path}"
            )
        launch_note = launch_freecad_with_document(fcstd_path)
        return f"{convert_note}. {launch_note} (force_open)"

    if running and heartbeat_ok:
        return (
            f"{convert_note}. Signaled Kala Mod reload "
            f"(heartbeat ok). Live: {fcstd_path}"
        )

    return (
        f"{convert_note}. Live file updated ({fcstd_path.name}); "
        f"GUI open deferred to end of run"
        + (" (Mod heartbeat missing)" if running and not heartbeat_ok else "")
    )


def ensure_live_shown() -> str:
    """Final handoff after a run: convert if needed and force-open FreeCAD."""
    step = default_step_path()
    live = default_live_path()
    install_kala_freecad_mod(live)
    if step.is_file():
        return publish_to_gui(step_path=step, fcstd_path=live, force_open=True)
    if live.is_file():
        reload_flag_path().write_text(str(time.time()), encoding="utf-8")
        return launch_freecad_with_document(live) + " (existing FCStd)"
    return "No live STEP/FCStd to show in FreeCAD"
