"""The client imports only the contracts it needs and no runtime or CAD module."""

from __future__ import annotations

import os
import subprocess
import sys
import textwrap
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SRC = ROOT / "src"
BRIDGE_SRC = ROOT / "adapters" / "solidedge-bridge" / "src"
OWN_SRC = Path(__file__).resolve().parents[1] / "src"

FORBIDDEN = (
    "FreeCAD",
    "FreeCADGui",
    "Part",
    "kala_agent.application",
    "kala_agent.studio",
    "kala_agent.kernel",
    "kala_agent.tools",
    "kala_agent.runtime",
    "kala_agent.persistence",
    "kala_agent.domains",
    "pythoncom",
    "win32com",
    "pywintypes",
)


def _run(code: str) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment["PYTHONPATH"] = f"{SRC}{os.pathsep}{BRIDGE_SRC}{os.pathsep}{OWN_SRC}"
    source = textwrap.dedent(code).replace("__FORBIDDEN__", repr(FORBIDDEN))
    return subprocess.run(
        [sys.executable, "-c", source],
        capture_output=True,
        text=True,
        timeout=120,
        env=environment,
        check=False,
    )


def test_importing_the_package_loads_no_runtime_or_cad_module() -> None:
    completed = _run(
        """
        import sys
        import kala_agent_solidedge
        from kala_agent_solidedge import client, compile, oracle, journal, paths

        forbidden = __FORBIDDEN__
        loaded = sorted(name for name in sys.modules if any(
            name == item or name.startswith(item + ".") for item in forbidden
        ))
        assert not loaded, loaded
        print("ok")
        """
    )

    assert completed.returncode == 0, completed.stderr
    assert "ok" in completed.stdout


def test_the_compiler_and_oracle_work_without_a_bridge_or_a_socket() -> None:
    completed = _run(
        """
        import json
        from pathlib import Path

        from kala_agent.contracts.construction import load_construction
        from kala_agent_solidedge.compile import build_request
        from kala_agent_solidedge import oracle

        script = load_construction({
            "summary": "plate",
            "components": [{
                "name": "plate",
                "steps": [
                    {"op": "sketch", "profile": "rectangle", "width": 20, "height": 20},
                    {"op": "extrude", "length": 10},
                ],
            }],
            "assembly": [],
        })
        request = build_request(script, request_id="probe-1")
        expectation = oracle.expectation(script)
        assert request.procedure[0].op == "part.new"
        assert expectation.parts[0].volume_mm3 == 4000.0
        print("ok")
        """
    )

    assert completed.returncode == 0, completed.stderr
    assert "ok" in completed.stdout


def test_the_cli_help_and_dry_run_work_on_this_host() -> None:
    environment = dict(os.environ)
    environment["PYTHONPATH"] = f"{SRC}{os.pathsep}{BRIDGE_SRC}{os.pathsep}{OWN_SRC}"
    help_run = subprocess.run(
        [sys.executable, "-m", "kala_agent_solidedge", "--help"],
        capture_output=True,
        text=True,
        timeout=120,
        env=environment,
        check=False,
    )
    assert help_run.returncode == 0, help_run.stderr
    assert "kala-solidedge" in help_run.stdout

    dry_run = subprocess.run(
        [
            sys.executable,
            "-m",
            "kala_agent_solidedge",
            "build",
            "--script",
            str(Path(__file__).resolve().parents[1] / "examples" / "plate_with_hole.json"),
            "--dry-run",
        ],
        capture_output=True,
        text=True,
        timeout=120,
        env=environment,
        check=False,
    )
    assert dry_run.returncode == 0, dry_run.stderr
    assert '"operations": 8' in dry_run.stdout
