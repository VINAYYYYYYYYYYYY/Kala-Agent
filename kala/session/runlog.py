"""Persistent Kala + FreeCAD logs (readable by agents and `kala logs`)."""

from __future__ import annotations

import json
import os
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def logs_dir() -> Path:
    root = Path(__file__).resolve().parents[2]
    out = Path(os.environ.get("KALA_OUTPUT_DIR", root / "outputs"))
    if not out.is_absolute():
        out = root / out
    path = out / "logs"
    path.mkdir(parents=True, exist_ok=True)
    return path


def agent_log_path() -> Path:
    return logs_dir() / "agent.jsonl"


def agent_latest_path() -> Path:
    return logs_dir() / "latest_run.json"


def freecad_gui_log_path() -> Path:
    return logs_dir() / "freecad_gui.log"


def freecad_native_log_candidates() -> list[Path]:
    base = Path.home() / ".local/share/FreeCAD"
    return [
        base / "v1-1" / "FreeCAD.log",
        base / "FreeCAD.log",
        Path.home() / ".FreeCAD" / "FreeCAD.log",
    ]


def freecad_native_log_path() -> Path | None:
    for path in freecad_native_log_candidates():
        if path.is_file():
            return path
    return None


def ensure_freecad_file_logging() -> Path | None:
    """Turn on FreeCAD's on-disk log if FreeCAD Python is importable."""
    try:
        import FreeCAD  # type: ignore
    except ImportError:
        lib = Path("/usr/lib/freecad/lib")
        if lib.is_dir():
            import sys

            sys.path.insert(0, str(lib))
            try:
                import FreeCAD  # type: ignore
            except ImportError:
                return None
        else:
            return None

    log_path = Path.home() / ".local/share/FreeCAD/v1-1/FreeCAD.log"
    try:
        log_path.parent.mkdir(parents=True, exist_ok=True)
        grp = FreeCAD.ParamGet("User parameter:BaseApp/Preferences/General")
        grp.SetBool("RedirectPythonOutput", True)
        grp.SetBool("RedirectPythonErrors", True)
        # File logging preferences vary by version — always touch a marker path
        FreeCAD.ParamGet("User parameter:BaseApp").SetASCII(
            "KalaLogHint", str(log_path)
        )
        try:
            FreeCAD.saveParameter()
        except Exception:
            pass
    except Exception:
        return freecad_native_log_path()
    return freecad_native_log_path() or log_path


def write_run_log(payload: dict[str, Any]) -> Path:
    """Append one run to agent.jsonl and overwrite latest_run.json."""
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    record = {
        "ts": stamp,
        "unix": time.time(),
        **payload,
    }
    # Never persist API keys if nested somehow
    text = json.dumps(record, indent=None, default=str)
    if "sk-or-" in text or "api_key" in text.lower():
        # scrub common key shapes from accidental dumps
        import re

        text = re.sub(r"sk-or-[A-Za-z0-9_-]+", "sk-or-***", text)
        text = re.sub(r'("api_key"\s*:\s*")[^"]*(")', r"\1***\2", text)

    path = agent_log_path()
    with path.open("a", encoding="utf-8") as fh:
        fh.write(text + "\n")
    latest = agent_latest_path()
    latest.write_text(json.dumps(record, indent=2, default=str) + "\n", encoding="utf-8")
    return latest


def read_tail(path: Path, *, max_bytes: int = 32_000) -> str:
    if not path.is_file():
        return f"(missing) {path}"
    data = path.read_bytes()
    if len(data) > max_bytes:
        data = data[-max_bytes:]
        return "…\n" + data.decode("utf-8", errors="replace")
    return data.decode("utf-8", errors="replace")


def summarize_logs(*, agent_lines: int = 3, freecad_bytes: int = 8_000) -> str:
    parts: list[str] = []
    agent = agent_log_path()
    parts.append(f"=== agent log: {agent} ===")
    if agent.is_file():
        lines = agent.read_text(encoding="utf-8").splitlines()
        for line in lines[-agent_lines:]:
            try:
                obj = json.loads(line)
                state = obj.get("state") or {}
                parts.append(
                    f"- {obj.get('ts')} status={state.get('status')} "
                    f"goal={state.get('goal')!r} tools={len(state.get('history') or [])}"
                )
            except json.JSONDecodeError:
                parts.append(line[:200])
    else:
        parts.append("(no runs logged yet)")

    parts.append(f"\n=== latest run: {agent_latest_path()} ===")
    if agent_latest_path().is_file():
        parts.append(read_tail(agent_latest_path(), max_bytes=12_000))
    else:
        parts.append("(none)")

    gui = freecad_gui_log_path()
    parts.append(f"\n=== freecad GUI redirect: {gui} ===")
    parts.append(read_tail(gui, max_bytes=freecad_bytes) if gui.is_file() else "(none yet)")

    native = freecad_native_log_path()
    parts.append(
        f"\n=== freecad native: {native or freecad_native_log_candidates()[0]} ==="
    )
    if native:
        parts.append(read_tail(native, max_bytes=freecad_bytes))
    else:
        parts.append("(missing — open FreeCAD once after logging is enabled)")
    return "\n".join(parts)
