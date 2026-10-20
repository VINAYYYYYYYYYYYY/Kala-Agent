"""CLI entry points and their honest platform limits."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

import kala_solidedge_bridge.__main__ as cli
from kala_solidedge_bridge.auth import read_token, token_path


def test_help_is_available(capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit) as raised:
        cli.main(["--help"])
    assert raised.value.code == 0
    assert "probe" in capsys.readouterr().out


def test_token_command_prints_and_persists_the_token(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert cli.main(["token", "--data-dir", str(tmp_path)]) == 0
    printed = capsys.readouterr().out.strip()

    assert printed == read_token(tmp_path)
    assert token_path(tmp_path).exists()


def test_probe_on_linux_writes_diagnostics_and_exits_2(
    tmp_path: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    # The non-Windows diagnostic branch must be asserted on every CI platform:
    # on an actual Windows runner this monkeypatch emulates a development host
    # so the test does not require Solid Edge.
    monkeypatch.setattr(sys, "platform", "linux")
    output = tmp_path / "probe.json"
    code = cli.main(
        [
            "probe",
            "--data-dir",
            str(tmp_path / "data"),
            "--output",
            str(output),
        ]
    )

    assert code == 2
    report = json.loads(output.read_text(encoding="utf-8"))
    assert report["attach"]["status"] == "unsupported"
    assert report["native_build_capability"] is False
    assert "native build capability: false" in capsys.readouterr().out


def test_invalid_port_is_rejected() -> None:
    with pytest.raises(SystemExit) as raised:
        cli.main(["serve", "--port", "0"])
    assert raised.value.code == 2


def test_serve_refuses_non_windows_before_resolving_the_default_data_dir(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr(sys, "platform", "linux")
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    assert cli.main(["serve", "--port", "8797"]) == 2

    assert "requires Windows" in capsys.readouterr().err
    assert not (tmp_path / "Kala" / "SolidEdgeBridge").exists()


def test_module_entrypoint_exists() -> None:
    source = Path(__file__).resolve().parents[1] / "src"
    environment = dict(os.environ)
    existing = environment.get("PYTHONPATH")
    environment["PYTHONPATH"] = f"{source}{os.pathsep}{existing}" if existing else str(source)
    completed = subprocess.run(
        [sys.executable, "-m", "kala_solidedge_bridge", "--help"],
        check=False,
        capture_output=True,
        text=True,
        env=environment,
        timeout=30,
    )
    assert completed.returncode == 0
    assert "serve" in completed.stdout
