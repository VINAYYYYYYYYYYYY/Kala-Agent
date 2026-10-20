"""The Solid Edge provider: honest probe, no fake build (ADR-0008/0011).

Solid Edge runs on Windows behind the ADR-0008 loopback bridge. This module
does not import the bridge or client distributions; it resolves the bridge URL
and its token file the way the documented client does, then asks the
authenticated ``/v1/health`` route. On a non-Windows host it refuses without
reading a token or opening a socket.

The bridge milestone that landed has no production native executor and reports
``supported: false``. A build is therefore refused with a 503-style
:class:`CadProviderUnavailableError`; nothing is ever returned as a
success. The exact off-Windows refusal text is part of the ADR-0011 contract:
"Solid Edge build requires the Windows bridge (not reachable)".
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import TYPE_CHECKING, ClassVar, Final

from kala_agent.studio.design.providers.base import (
    Availability,
    CadProviderBuildError,
    CadProviderUnavailableError,
    ProviderBuild,
)

if TYPE_CHECKING:
    from kala_agent.contracts.construction import ConstructionScript
    from kala_agent.studio.design.intent import DesignIntent

DEFAULT_BRIDGE_URL: Final[str] = "http://127.0.0.1:8797"
BRIDGE_URL_ENV_VAR: Final[str] = "KALA_SOLIDEDGE_BRIDGE_URL"
TOKEN_FILE_ENV_VAR: Final[str] = "KALA_SOLIDEDGE_TOKEN_FILE"
TOKEN_FILENAME: Final[str] = "bridge.token"
DATA_DIR_PARTS: Final[tuple[str, str]] = ("Kala", "SolidEdgeBridge")
HEALTH_PATH: Final[str] = "/v1/health"
DEFAULT_TIMEOUT_SECONDS: Final[float] = 5.0
MAX_HEALTH_BYTES: Final[int] = 64 * 1024
MIN_TOKEN_LENGTH: Final[int] = 16
MAX_TOKEN_LENGTH: Final[int] = 256
DEFAULT_PROBE_CACHE_SECONDS: Final[float] = 5.0
NOT_REACHABLE_MESSAGE: Final[str] = "Solid Edge build requires the Windows bridge (not reachable)"
UNAVAILABLE_REASON: Final[str] = (
    "Solid Edge requires Windows with the ADR-0008 loopback bridge; this host cannot run it"
)


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Refuse a redirect: the bridge is a fixed loopback endpoint."""

    def redirect_request(
        self,
        req: urllib.request.Request,
        fp: object,
        code: int,
        msg: str,
        headers: object,
        newurl: str,
    ) -> None:
        return None


class SolidEdgeProvider:
    """Probe the Windows bridge; never invent a build off Windows."""

    name: ClassVar[str] = "solidedge"
    display_name: ClassVar[str] = "Solid Edge"
    measured_by: ClassVar[str] = "Solid Edge"
    mode: ClassVar[str] = "bridge"

    def __init__(
        self,
        *,
        platform: str | None = None,
        environ: dict[str, str] | None = None,
        url: str | None = None,
        token_file: str | Path | None = None,
        timeout_seconds: float = DEFAULT_TIMEOUT_SECONDS,
        probe_cache_seconds: float = DEFAULT_PROBE_CACHE_SECONDS,
    ) -> None:
        if timeout_seconds <= 0:
            raise ValueError("the bridge probe timeout must be positive")
        if probe_cache_seconds < 0:
            raise ValueError("the probe cache duration must be non-negative")
        self._platform = sys.platform if platform is None else platform
        self._environ = dict(os.environ if environ is None else environ)
        self._url = url
        self._token_file = str(token_file) if token_file is not None else None
        self._timeout_seconds = float(timeout_seconds)
        self._probe_cache_seconds = float(probe_cache_seconds)
        self._probe_cached: Availability | None = None
        self._probe_at = 0.0

    # ------------------------------------------------------------------ probe

    def probe(self) -> Availability:
        """Return whether the bridge is reachable and reports a build capability."""
        now = time.monotonic()
        if self._probe_cached is not None and now - self._probe_at < self._probe_cache_seconds:
            return self._probe_cached
        availability = self._probe_uncached()
        self._probe_cached = availability
        self._probe_at = now
        return availability

    def _probe_uncached(self) -> Availability:
        if self._platform != "win32":
            return Availability(
                available=False,
                version=None,
                reason=(
                    f"{NOT_REACHABLE_MESSAGE}; this host is {self._platform}, so the "
                    "ADR-0008 bridge cannot run here"
                ),
            )
        bridge_url = self._resolve_url()
        if bridge_url is None:
            return Availability(
                available=False,
                version=None,
                reason="the Solid Edge bridge URL is not a loopback HTTP URL",
            )
        token = self._read_token()
        if token is None:
            return Availability(
                available=False,
                version=None,
                reason=(
                    "the Solid Edge bridge token file was not found; run the bridge "
                    "token command on this Windows host"
                ),
            )
        health = self._fetch_health(bridge_url, token)
        if health is None:
            return Availability(
                available=False,
                version=None,
                reason=f"the Solid Edge bridge at {bridge_url} did not answer a valid health probe",
            )
        detail = health.get("detail")
        reason = detail if isinstance(detail, str) and detail.strip() else None
        available = health.get("available") is True and health.get("supported") is True
        version = health.get("se_version")
        return Availability(
            available=available,
            version=version if isinstance(version, str) and version else None,
            reason=None
            if available
            else (reason or "the bridge reports no native build capability"),
        )

    def _resolve_url(self) -> str | None:
        raw = self._url or self._environ.get(BRIDGE_URL_ENV_VAR) or DEFAULT_BRIDGE_URL
        try:
            parsed = urllib.parse.urlparse(raw)
        except ValueError:
            return None
        if parsed.scheme != "http" or parsed.username or parsed.password:
            return None
        if parsed.path not in ("", "/") or parsed.query or parsed.fragment:
            return None
        host = (parsed.hostname or "").lower()
        if host not in {"127.0.0.1", "localhost", "::1"}:
            return None
        port = parsed.port
        if port is None or not 1 <= port <= 65_535:
            return None
        return f"http://{host}:{port}"

    def _read_token(self) -> str | None:
        path = self._token_file_path()
        if path is None:
            return None
        try:
            text = path.read_text(encoding="ascii")
        except (OSError, UnicodeError):
            return None
        token = text.strip()
        if not MIN_TOKEN_LENGTH <= len(token) <= MAX_TOKEN_LENGTH:
            return None
        return token

    def _token_file_path(self) -> Path | None:
        if self._token_file is not None:
            return Path(self._token_file)
        from_environment = self._environ.get(TOKEN_FILE_ENV_VAR)
        if from_environment:
            return Path(from_environment)
        local_app_data = self._environ.get("LOCALAPPDATA")
        base = Path(local_app_data) if local_app_data else Path.home() / "AppData" / "Local"
        return base.joinpath(*DATA_DIR_PARTS) / TOKEN_FILENAME

    def _fetch_health(self, bridge_url: str, token: str) -> dict[str, object] | None:
        request = urllib.request.Request(
            bridge_url + HEALTH_PATH,
            headers={"Authorization": f"Bearer {token}", "Accept": "application/json"},
            method="GET",
        )
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect)
        try:
            with opener.open(request, timeout=self._timeout_seconds) as response:
                if response.status != 200:
                    return None
                raw = response.read(MAX_HEALTH_BYTES + 1)
        except (urllib.error.URLError, OSError, ValueError):
            return None
        if len(raw) > MAX_HEALTH_BYTES:
            return None
        try:
            parsed = json.loads(raw)
        except (UnicodeError, json.JSONDecodeError):
            return None
        return parsed if isinstance(parsed, dict) else None

    # ------------------------------------------------------------------- build

    def build(self, script: ConstructionScript, intent: DesignIntent) -> ProviderBuild:
        """Refuse honestly: this milestone has no verified native build path."""
        del script, intent
        availability = self.probe()
        if not availability.available:
            raise CadProviderUnavailableError(
                self.name,
                NOT_REACHABLE_MESSAGE,
                reason=availability.reason or UNAVAILABLE_REASON,
            )
        raise CadProviderBuildError(
            "the Solid Edge bridge is reachable but this Studio worker has no "
            "verified native build executor; no Solid Edge build was attempted",
            op="bridge.build",
        )


__all__ = (
    "BRIDGE_URL_ENV_VAR",
    "DEFAULT_BRIDGE_URL",
    "NOT_REACHABLE_MESSAGE",
    "TOKEN_FILE_ENV_VAR",
    "UNAVAILABLE_REASON",
    "SolidEdgeProvider",
)
