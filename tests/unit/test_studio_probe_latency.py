"""The provider probes never hold a route open, and ``offline`` tells the truth.

Two behaviours are pinned here, both shared with the native product
(``providers/cli_runtime.rs``):

* ``GET /providers``, ``GET /settings`` and ``GET /health`` answer inside the
  route budget even when a subscription CLI hangs on ``--version``. The row
  is the honest "checking" state (``available: null``), and the background
  refresh fills the real verdict afterwards.
* ``offline`` is false when a subscription CLI is installed and signed in,
  even with no API key stored, because such a CLI can make a real remote
  turn.

Every CLI in these tests is a fake executable in a temporary directory; no
real ``claude``/``codex``/``opencode`` is ever started.
"""

from __future__ import annotations

import os
import time
from collections.abc import Iterator
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from kala_agent.studio.providers import cli_runtime
from kala_agent.studio.providers.codex_cli import SPEC as CODEX_SPEC
from kala_agent.studio.providers.opencode_cli import SPEC as OPENCODE_SPEC
from kala_agent.studio.server import create_studio_app

TOKEN = "probe-latency-token-0123456789abcdef"
AUTH = {"Authorization": f"Bearer {TOKEN}"}
BASE_URL = "http://127.0.0.1"
ROUTE_BUDGET_SECONDS = 1.0


@pytest.fixture(autouse=True)
def _clear_probe_cache() -> Iterator[None]:
    cli_runtime.clear_probe_cache()
    yield
    cli_runtime.clear_probe_cache()


def _fake_cli(directory: Path, name: str, body: str) -> Path:
    program = directory / name
    program.write_text(f"#!/bin/sh\n{body}\n", encoding="utf-8")
    program.chmod(0o755)
    return program


def _app(data_dir: Path, environ: dict[str, str]) -> TestClient:
    app = create_studio_app(
        data_dir=data_dir,
        api_token=TOKEN,
        max_body_bytes=64 * 1024,
        static_root=None,
        environ=environ,
    )
    return TestClient(app, base_url=BASE_URL, headers=AUTH, raise_server_exceptions=False)


def _hermetic_environment(tmp_path: Path, **values: str) -> dict[str, str]:
    """An environment view with no real CLI reachable.

    ``PATH`` holds only the system directories; the three provider rows are
    pointed at explicit fakes (or at an absent path). ``HOME`` is temporary,
    so a CLI's own login file can never be the operator's.
    """
    home = tmp_path / "home"
    home.mkdir(parents=True, exist_ok=True)
    environment = {
        "PATH": "/usr/bin:/bin",
        "HOME": str(home),
        "KALA_STUDIO_CODEX_CLI_PATH": "/nonexistent/kala/codex",
        "KALA_STUDIO_OPENCODE_CLI_PATH": "/nonexistent/kala/opencode",
        "KALA_STUDIO_CLAUDE_CLI_PATH": "/nonexistent/kala/claude",
    }
    environment.update(values)
    return environment


def test_a_hanging_cli_never_blocks_the_provider_routes(tmp_path: Path) -> None:
    fake = _fake_cli(tmp_path, "claude", "sleep 300")
    environ = _hermetic_environment(
        tmp_path, KALA_STUDIO_CLAUDE_CLI_PATH=str(fake)
    )
    with _app(tmp_path / "data", environ) as client:
        started = time.monotonic()
        response = client.get("/api/studio/providers")
        elapsed = time.monotonic() - started
        assert response.status_code == 200
        assert elapsed < ROUTE_BUDGET_SECONDS, f"/providers took {elapsed:.3f}s"

        rows = {row["id"]: row for row in response.json()["providers"]}
        claude = rows["claude_code"]
        assert claude["configured"] is False
        assert claude["available"] is None
        assert claude["reason"] == cli_runtime.PROBE_CHECKING_REASON

        # The settings route is the same decision with more rows.
        started = time.monotonic()
        settings = client.get("/api/studio/settings")
        elapsed = time.monotonic() - started
        assert settings.status_code == 200
        assert elapsed < ROUTE_BUDGET_SECONDS, f"/settings took {elapsed:.3f}s"
        assert settings.json()["providers"][4]["available"] is None

        # The liveness route must not wait for the CLI either.
        started = time.monotonic()
        health = client.get("/api/studio/health")
        elapsed = time.monotonic() - started
        assert health.status_code == 200
        assert elapsed < ROUTE_BUDGET_SECONDS, f"/health took {elapsed:.3f}s"
        assert isinstance(health.json()["offline"], bool)

        # The background refresh finishes inside the per-probe cap and the
        # next call carries the honest measured verdict, not "checking".
        deadline = time.monotonic() + cli_runtime.CLI_PROBE_TIMEOUT_SECONDS + 3.0
        row = None
        while time.monotonic() < deadline:
            row = next(
                row
                for row in client.get("/api/studio/providers").json()["providers"]
                if row["id"] == "claude_code"
            )
            if row["reason"] != cli_runtime.PROBE_CHECKING_REASON:
                break
            time.sleep(0.05)
        assert row is not None
        assert row["available"] is False
        assert "did not answer its version check" in (row["reason"] or "")


def test_a_signed_in_cli_makes_the_studio_online(tmp_path: Path) -> None:
    fake = _fake_cli(
        tmp_path,
        "claude",
        'if [ "${1:-}" = "--version" ]; then echo "2.1.303 (Claude Code)"; exit 0; fi\n'
        "exit 0",
    )
    environ = _hermetic_environment(
        tmp_path, KALA_STUDIO_CLAUDE_CLI_PATH=str(fake)
    )
    environ.pop("OPENAI_API_KEY", None)
    environ.pop("OPENROUTER_API_KEY", None)
    with _app(tmp_path / "data", environ) as client:
        health = client.get("/api/studio/health").json()
        settings = client.get("/api/studio/settings").json()
        providers = client.get("/api/studio/providers").json()["providers"]
        claude = next(row for row in providers if row["id"] == "claude_code")
        assert claude["available"] is True
        assert claude["configured"] is True
        # B16: a usable subscription CLI is a real remote route.
        assert health["offline"] is False
        assert settings["offline"] is False

    # Without the CLI the same app is honestly offline.
    offline_env = _hermetic_environment(tmp_path / "offline")
    with _app(tmp_path / "offline-data", offline_env) as client:
        assert client.get("/api/studio/health").json()["offline"] is True
        assert client.get("/api/studio/settings").json()["offline"] is True


def test_the_probe_rows_are_available_for_every_shipped_cli(tmp_path: Path) -> None:
    """The three rows keep their published ids and order."""
    environ = _hermetic_environment(tmp_path)
    with _app(tmp_path / "data", environ) as client:
        rows = client.get("/api/studio/providers").json()["providers"]
    ids = [row["id"] for row in rows]
    assert ids == [
        "local",
        "openai",
        "openrouter",
        "codex",
        "claude_code",
        "codex_cli",
        "opencode_cli",
    ]
    assert CODEX_SPEC.id == "codex_cli"
    assert OPENCODE_SPEC.id == "opencode_cli"
    assert os.path.basename(CODEX_SPEC.binary) == "codex"
