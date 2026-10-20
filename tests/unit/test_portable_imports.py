"""Portable import regression tests for the Windows-only deployment.

The target runtime runs Kala and its CAD integration on one Windows desktop.
Importing the portable surfaces must therefore never require the Linux
execution layer: the strict sandbox loads ``libc`` with ``ctypes`` at import
time and refuses to load on Windows.

These tests prove the property the only way it can be proven on a Linux
development host: each import runs in a fresh interpreter whose import hook
refuses ``kala_agent.application``, ``kala_agent.tools`` and
``kala_agent.runtime``, and whose ``ctypes.CDLL`` refuses to load a C library.
A regression that reintroduces an eager Linux import fails the child process
rather than passing silently on this host.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import cast

import pytest

PROJECT_ROOT = Path(__file__).parents[2]
SOURCE_ROOT = PROJECT_ROOT / "src"
BLOCKED_PREFIXES = ("kala_agent.application", "kala_agent.tools", "kala_agent.runtime")

_PRELUDE = """
import ctypes
import json
import sys
import tempfile
from pathlib import Path

BLOCKED_PREFIXES = {blocked!r}
libc_attempts = []

_real_cdll = ctypes.CDLL


def _refusing_cdll(name, *args, **kwargs):
    text = str(name)
    if any(token in text for token in ("libc", "libSystem")):
        libc_attempts.append(text)
        raise OSError(f"the portable probe refuses to load {{text}}")
    return _real_cdll(name, *args, **kwargs)


ctypes.CDLL = _refusing_cdll


class _RefusingFinder:
    def find_spec(self, fullname, path=None, target=None):
        for prefix in BLOCKED_PREFIXES:
            if fullname == prefix or fullname.startswith(prefix + "."):
                raise ImportError(f"the portable probe refuses {{fullname}}")
        return None


sys.meta_path.insert(0, _RefusingFinder())
"""


def _run_isolated(script: str) -> dict[str, object]:
    """Run ``script`` in a fresh interpreter with the refusing import hook."""
    environment = dict(os.environ)
    existing = environment.get("PYTHONPATH", "")
    environment["PYTHONPATH"] = (
        f"{SOURCE_ROOT}{os.pathsep}{existing}" if existing else str(SOURCE_ROOT)
    )
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    completed = subprocess.run(
        [sys.executable, "-c", _PRELUDE.format(blocked=BLOCKED_PREFIXES) + script],
        capture_output=True,
        text=True,
        env=environment,
        cwd=str(PROJECT_ROOT),
        timeout=120,
        check=False,
    )
    assert completed.returncode == 0, completed.stderr
    payload = json.loads(completed.stdout.strip().splitlines()[-1])
    return cast("dict[str, object]", payload)


def test_root_import_stays_portable() -> None:
    """Importing ``kala_agent`` must not load the composition root or tool plane."""
    result = _run_isolated(
        """
import kala_agent

print(json.dumps({
    "loaded": sorted(name for name in sys.modules if name.startswith("kala_agent")),
    "settings_module": kala_agent.Settings.__module__,
    "libc_attempts": libc_attempts,
}))
"""
    )
    loaded = cast("list[str]", result["loaded"])
    assert result["settings_module"] == "kala_agent.config"
    assert result["libc_attempts"] == []
    assert "kala_agent.application" not in loaded
    assert not any(name.startswith(BLOCKED_PREFIXES) for name in loaded)


def test_contracts_and_studio_import_without_the_linux_execution_layer() -> None:
    """Contracts, the API boundary and Studio work with the core layers refused."""
    result = _run_isolated(
        """
import kala_agent.api.boundary
import kala_agent.contracts
import kala_agent.studio
from fastapi.testclient import TestClient

TOKEN = "portable-probe-token-0123456789abcdef"

with tempfile.TemporaryDirectory() as scratch:
    data_dir = Path(scratch) / "studio"
    app = kala_agent.studio.create_studio_app(data_dir=data_dir, api_token=TOKEN)
    client = TestClient(app, base_url="http://127.0.0.1")
    health = client.get("/api/studio/health")
    refused_host = client.get("/api/studio/health", headers={"host": "attacker.example"})
    refused_token = client.post(
        "/api/studio/preview",
        json={"kind": "box", "parameters": {"length_mm": 10.0}},
    )
    data_dir_exists = data_dir.is_dir()

print(json.dumps({
    "health_status": health.status_code,
    "health_body_status": health.json().get("status"),
    "health_version": health.json().get("version"),
    "refused_host_status": refused_host.status_code,
    "refused_token_status": refused_token.status_code,
    "data_dir_exists": data_dir_exists,
    "loaded": sorted(name for name in sys.modules if name.startswith("kala_agent")),
    "libc_attempts": libc_attempts,
}))
"""
    )
    loaded = cast("list[str]", result["loaded"])
    assert result["health_status"] == 200
    assert result["health_body_status"] == "ok"
    assert result["health_version"] == "0.1.0"
    assert result["refused_host_status"] == 421
    assert result["refused_token_status"] == 401
    assert result["data_dir_exists"] is True
    assert result["libc_attempts"] == []
    assert "kala_agent.studio" in loaded
    assert "kala_agent.api.boundary" in loaded
    assert "kala_agent.contracts" in loaded
    for blocked in BLOCKED_PREFIXES:
        assert blocked not in loaded
    assert "kala_agent.tools.sandbox" not in loaded
    assert "kala_agent.tools.executor" not in loaded


def test_api_boundary_import_does_not_reach_the_application() -> None:
    """The studio's only core dependency, ``api.boundary``, stays portable."""
    result = _run_isolated(
        """
import kala_agent.api.boundary as boundary

print(json.dumps({
    "loopback": boundary.is_loopback_host("127.0.0.1"),
    "wildcard_refused": boundary.is_permitted_authority("0.0.0.0:8000"),
    "loaded": sorted(name for name in sys.modules if name.startswith("kala_agent")),
    "libc_attempts": libc_attempts,
}))
"""
    )
    loaded = cast("list[str]", result["loaded"])
    assert result["loopback"] is True
    assert result["wildcard_refused"] is False
    assert result["libc_attempts"] == []
    assert not any(name.startswith(BLOCKED_PREFIXES) for name in loaded)


def test_api_package_import_is_lazy_but_complete() -> None:
    """A bare ``import kala_agent.api`` loads no transport module eagerly."""
    result = _run_isolated(
        """
import kala_agent.api

loaded = sorted(name for name in sys.modules if name.startswith("kala_agent"))
print(json.dumps({
    "loaded": loaded,
    "public": sorted(kala_agent.api.__all__),
    "libc_attempts": libc_attempts,
}))
"""
    )
    loaded = cast("list[str]", result["loaded"])
    assert result["libc_attempts"] == []
    assert loaded == ["kala_agent", "kala_agent.api", "kala_agent.config"]
    assert "create_app" in cast("list[str]", result["public"])


@pytest.mark.skipif(
    sys.platform != "linux",
    reason=(
        "resolves kala_agent.application, the Linux core executor that is not ported to "
        "Windows; the portable import boundary is covered by the isolated tests above"
    ),
)
def test_root_lazy_exports_resolve_to_the_real_objects() -> None:
    """An explicit request for the composition root still returns the same object."""
    from importlib import import_module

    import kala_agent

    application = import_module("kala_agent.application")
    assert kala_agent.KalaApplication is application.KalaApplication
    assert kala_agent.Settings is import_module("kala_agent.config").Settings
    assert set(kala_agent.__all__) == {"KalaApplication", "Settings"}


def test_root_dir_and_unknown_attribute_behaviour() -> None:
    """``dir`` advertises the public surface; an unknown name is an AttributeError."""
    import kala_agent

    advertised = dir(kala_agent)
    assert "KalaApplication" in advertised
    assert "Settings" in advertised
    with pytest.raises(AttributeError) as error:
        _ = kala_agent.NotAnExport
    assert "NotAnExport" in str(error.value)
    assert "kala_agent" in str(error.value)


@pytest.mark.skipif(
    sys.platform != "linux",
    reason=(
        "imports the Linux core API stack whose executor dependency is not ported to "
        "Windows; the Windows-safe lazy-import check is the isolated test above"
    ),
)
def test_api_lazy_exports_match_their_defining_modules() -> None:
    """Every public API name resolves to the object its module defines."""
    from importlib import import_module

    import kala_agent.api as api

    app_module = import_module("kala_agent.api.app")
    boundary_module = import_module("kala_agent.api.boundary")
    dependencies_module = import_module("kala_agent.api.dependencies")
    envelope_module = import_module("kala_agent.api.envelope")
    middleware_module = import_module("kala_agent.api.middleware")
    schemas_module = import_module("kala_agent.api.schemas")

    assert api.create_app is app_module.create_app
    assert api.router is app_module.router
    assert api.build_middleware_stack is app_module.build_middleware_stack
    assert api.HostGuardMiddleware is boundary_module.HostGuardMiddleware
    assert api.is_permitted_authority is boundary_module.is_permitted_authority
    assert api.get_application is dependencies_module.get_application
    assert api.error_response is envelope_module.error_response
    assert api.error_response is schemas_module.error_response
    assert api.MaxBodySizeMiddleware is middleware_module.MaxBodySizeMiddleware
    assert api.HealthResponse is schemas_module.HealthResponse
    for name in api.__all__:
        assert getattr(api, name) is not None


def test_api_dir_and_unknown_attribute_behaviour() -> None:
    """The API package advertises ``__all__`` and refuses unknown names."""
    import kala_agent.api as api

    advertised = dir(api)
    for name in api.__all__:
        assert name in advertised
    with pytest.raises(AttributeError) as error:
        _ = api.NotAnExport
    assert "NotAnExport" in str(error.value)


def test_settings_public_import_compatibility() -> None:
    """``from kala_agent import Settings`` keeps working and stays validated."""
    from kala_agent import Settings
    from kala_agent.config import Settings as ConfigSettings

    assert Settings is ConfigSettings
    settings = Settings.from_env({"KALA_AGENT_WORKER_COUNT": "2"})
    assert settings.worker_count == 2
