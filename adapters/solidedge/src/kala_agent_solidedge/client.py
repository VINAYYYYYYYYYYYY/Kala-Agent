"""Authenticated stdlib HTTP client for the local Solid Edge bridge.

Only ``http://127.0.0.1[:port]`` is accepted. A URL with a userinfo, query,
fragment, base path, another host, or another scheme is refused before a socket
exists. Redirects are refused rather than followed, so a compromised or
misconfigured service cannot move the client off loopback.

Response bodies are bounded by reading at most one byte past the cap. Artifacts
are streamed to a temporary file in the caller's output directory, hashed while
they are written, and only renamed into place after the declared size and
SHA-256 both match. ``Content-Length`` is never trusted as proof of size.

The bearer token is read from a private file and never appears in a message,
an exception, a URL, or a log line.
"""

from __future__ import annotations

import hashlib
import http.client
import json
import os
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Final, TypeVar

from pydantic import BaseModel, ValidationError

from kala_solidedge_bridge.protocol import (
    Artifact,
    BuildAccepted,
    BuildRequest,
    BuildStatus,
    ErrorCode,
    ErrorResponse,
    Health,
    JobState,
)

from .errors import (
    ArtifactError,
    AuthenticationError,
    ClientError,
    NetworkError,
    ProtocolError,
)
from .paths import DEFAULT_BRIDGE_URL

MAX_JSON_BYTES: Final[int] = 1024 * 1024
MAX_ARTIFACT_BYTES: Final[int] = 256 * 1024 * 1024
DEFAULT_TIMEOUT_SECONDS: Final[float] = 30.0
DEFAULT_ARTIFACT_TIMEOUT_SECONDS: Final[float] = 300.0
_LOOPBACK_HOSTS: Final[frozenset[str]] = frozenset({"127.0.0.1", "::1"})
_CHUNK_BYTES: Final[int] = 64 * 1024
_ModelT = TypeVar("_ModelT", bound=BaseModel)
_TERMINAL_STATES: Final[frozenset[JobState]] = frozenset(
    {
        JobState.SUCCEEDED,
        JobState.FAILED,
        JobState.CANCELLED,
        JobState.INDETERMINATE,
        JobState.INTERRUPTED,
    }
)


def validate_bridge_url(url: str) -> str:
    """Return the normalised loopback base URL or raise :class:`ClientError`."""
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != "http":
        raise ClientError("the bridge URL must use http on loopback; no TLS or other scheme")
    if parsed.username is not None or parsed.password is not None or "@" in parsed.netloc:
        raise ClientError("the bridge URL must not contain user information")
    if parsed.query or parsed.fragment:
        raise ClientError("the bridge URL must not contain a query or fragment")
    if parsed.path not in ("", "/"):
        raise ClientError("the bridge URL must not contain a base path")
    host = parsed.hostname
    if host is None or host not in _LOOPBACK_HOSTS:
        raise ClientError("the bridge URL must address loopback (127.0.0.1) only")
    if parsed.port is not None and not 1 <= parsed.port <= 65535:
        raise ClientError("the bridge URL port is out of range")
    authority = host if parsed.port is None else f"{host}:{parsed.port}"
    if ":" in host:
        authority = f"[{host}]" if parsed.port is None else f"[{host}]:{parsed.port}"
    return f"http://{authority}"


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Refuse redirects: a bridge never needs one."""

    def redirect_request(
        self,
        req: urllib.request.Request,
        fp: object,
        code: int,
        msg: str,
        headers: http.client.HTTPMessage,
        newurl: str,
    ) -> None:
        del req, fp, code, msg, headers, newurl
        raise ProtocolError("the bridge answered with a redirect, which is not accepted")


@dataclass(frozen=True)
class Download:
    """A verified local artifact."""

    artifact: Artifact
    path: Path


class BridgeClient:
    """A small typed client. The token file is read once per client instance."""

    def __init__(
        self,
        *,
        base_url: str = DEFAULT_BRIDGE_URL,
        token: str,
        timeout: float = DEFAULT_TIMEOUT_SECONDS,
        artifact_timeout: float = DEFAULT_ARTIFACT_TIMEOUT_SECONDS,
        clock: Callable[[], float] | None = None,
    ) -> None:
        if not token.strip():
            raise AuthenticationError("the bridge token is empty")
        if not timeout > 0 or not artifact_timeout > 0:
            raise ClientError("the bridge timeouts must be positive")
        self.base_url = validate_bridge_url(base_url)
        self._token = token.strip()
        self.timeout = timeout
        self.artifact_timeout = artifact_timeout
        self._clock = time.monotonic if clock is None else clock
        self._opener = urllib.request.build_opener(_NoRedirect())

    @classmethod
    def from_token_file(
        cls,
        token_file: Path,
        *,
        base_url: str = DEFAULT_BRIDGE_URL,
        timeout: float = DEFAULT_TIMEOUT_SECONDS,
        artifact_timeout: float = DEFAULT_ARTIFACT_TIMEOUT_SECONDS,
    ) -> BridgeClient:
        """Build a client from the token file the bridge created."""
        try:
            token = token_file.read_text(encoding="ascii").strip()
        except FileNotFoundError as exc:
            raise AuthenticationError(
                "the bridge token file does not exist; run the bridge token command"
            ) from exc
        except (OSError, UnicodeError) as exc:
            raise AuthenticationError("the bridge token file could not be read") from exc
        return cls(
            base_url=base_url,
            token=token,
            timeout=timeout,
            artifact_timeout=artifact_timeout,
        )

    # ------------------------------------------------------------------ routes

    def health(self) -> Health:
        """Return the bridge's honest health document."""
        body = self._request("GET", "/v1/health")
        return self._parse(Health, body, "health")

    def submit(self, request: BuildRequest) -> BuildAccepted:
        """Submit one build.

        A network failure here is ambiguous: the bridge may have admitted the
        job. The caller must keep ``request.request_id`` and retry the same
        semantic request rather than mint a new one.
        """
        payload = request.model_dump_json().encode("utf-8")
        if len(payload) > MAX_JSON_BYTES:
            raise ClientError("the build request exceeds the 1 MiB JSON limit")
        body = self._request(
            "POST",
            "/v1/builds",
            payload=payload,
            content_type="application/json",
            ambiguous=True,
        )
        return self._parse(BuildAccepted, body, "build acceptance")

    def status(self, build_id: str) -> BuildStatus:
        """Return the status of one build."""
        body = self._request("GET", f"/v1/builds/{_path_id(build_id)}")
        return self._parse(BuildStatus, body, "build status")

    def cancel(self, build_id: str) -> BuildStatus:
        """Ask the bridge to cancel cooperatively between operations.

        This is a request, not a promise: a COM call already in progress is not
        proven to have stopped.
        """
        body = self._request("POST", f"/v1/builds/{_path_id(build_id)}/cancel", payload=b"")
        return self._parse(BuildStatus, body, "cancel status")

    def wait(
        self,
        build_id: str,
        *,
        timeout: float,
        interval: float = 1.0,
        clock: Callable[[], float] | None = None,
        sleep: Callable[[float], None] | None = None,
    ) -> BuildStatus:
        """Poll a build with one overall deadline. No unbounded wait."""
        now = time.monotonic if clock is None else clock
        waiter = time.sleep if sleep is None else sleep
        deadline = now() + timeout
        while True:
            status = self.status(build_id)
            if status.state in _TERMINAL_STATES:
                return status
            if now() >= deadline:
                raise NetworkError(
                    "the build did not finish before the polling deadline; the job may still "
                    "be running, so keep the build id instead of resubmitting"
                )
            waiter(interval)

    def download(
        self,
        build_id: str,
        artifact: Artifact,
        output_dir: Path,
    ) -> Download:
        """Stream one artifact into ``output_dir`` and verify size and SHA-256."""
        name = _safe_artifact_name(artifact.name)
        output_dir.mkdir(parents=True, exist_ok=True)
        target = output_dir / name
        descriptor, temporary = tempfile.mkstemp(prefix=f".{name}.", dir=output_dir)
        digest = hashlib.sha256()
        written = 0
        try:
            with os.fdopen(descriptor, "wb") as handle:
                for chunk in self._stream(
                    f"/v1/builds/{_path_id(build_id)}/artifacts/{urllib.parse.quote(name)}"
                ):
                    written += len(chunk)
                    if written > MAX_ARTIFACT_BYTES:
                        raise ArtifactError(
                            f"artifact {name} exceeds the 256 MiB client limit; it was discarded"
                        )
                    digest.update(chunk)
                    handle.write(chunk)
                handle.flush()
                os.fsync(handle.fileno())
            if written != artifact.size_bytes:
                raise ArtifactError(
                    f"artifact {name} is {written} bytes but the bridge declared "
                    f"{artifact.size_bytes}; it was discarded"
                )
            actual = digest.hexdigest()
            if actual != artifact.sha256:
                raise ArtifactError(
                    f"artifact {name} has digest {actual} but the bridge declared "
                    f"{artifact.sha256}; it was discarded"
                )
            os.replace(temporary, target)
        except BaseException:
            try:
                os.unlink(temporary)
            except OSError:
                pass
            raise
        return Download(artifact=artifact, path=target)

    # ------------------------------------------------------------------ transport

    def _request(
        self,
        method: str,
        path: str,
        *,
        payload: bytes | None = None,
        content_type: str | None = None,
        ambiguous: bool = False,
    ) -> bytes:
        request = urllib.request.Request(
            f"{self.base_url}{path}",
            data=payload,
            method=method,
        )
        request.add_header("Authorization", f"Bearer {self._token}")
        request.add_header("Accept", "application/json")
        if content_type is not None:
            request.add_header("Content-Type", content_type)
        deadline = self._clock() + self.timeout
        try:
            with self._opener.open(request, timeout=self.timeout) as response:
                return _read_capped(response, deadline=deadline, clock=self._clock)
        except urllib.error.HTTPError as error:
            raise self._typed_http_error(error, deadline=deadline) from None
        except (urllib.error.URLError, TimeoutError, OSError, http.client.HTTPException) as error:
            reason = _safe_reason(error)
            if ambiguous:
                raise NetworkError(
                    "the submission may or may not have reached the bridge; keep the request "
                    f"id and retry the same request ({reason})"
                ) from None
            raise NetworkError(f"the bridge could not be reached ({reason})") from None

    def _stream(self, path: str) -> Iterator[bytes]:
        request = urllib.request.Request(f"{self.base_url}{path}", method="GET")
        request.add_header("Authorization", f"Bearer {self._token}")
        deadline = self._clock() + self.artifact_timeout
        try:
            with self._opener.open(request, timeout=self.artifact_timeout) as response:
                declared = response.headers.get("Content-Length")
                if (
                    declared is not None
                    and declared.isdigit()
                    and int(declared) > MAX_ARTIFACT_BYTES
                ):
                    raise ArtifactError(
                        "the bridge declared an artifact larger than the 256 MiB client limit"
                    )
                while True:
                    if self._clock() >= deadline:
                        raise NetworkError(
                            "the artifact download exceeded the overall timeout; the partial "
                            "file was discarded"
                        )
                    chunk = response.read(_CHUNK_BYTES)
                    if not chunk:
                        return
                    yield chunk
        except urllib.error.HTTPError as error:
            raise self._typed_http_error(error) from None
        except ArtifactError:
            raise
        except (urllib.error.URLError, TimeoutError, OSError, http.client.HTTPException) as error:
            raise NetworkError(
                f"the artifact download failed ({_safe_reason(error)}); the partial file was "
                "discarded"
            ) from None

    def _typed_http_error(
        self,
        error: urllib.error.HTTPError,
        *,
        deadline: float | None = None,
    ) -> ClientError:
        body = b""
        try:
            body = _read_capped(error, deadline=deadline, clock=self._clock)
        except ClientError:
            body = b""
        finally:
            error.close()
        code, message = _wire_error(body)
        if error.code == 401:
            return AuthenticationError(
                "the bridge rejected the token; the token file is stale or wrong"
            )
        if error.code == 403:
            return AuthenticationError("the bridge refused this client")
        if error.code == 409:
            return ClientError(f"the bridge refused the request as busy or conflicting: {message}")
        if error.code == 503:
            return ClientError(f"the bridge is not ready: {message}")
        if error.code == 413:
            return ClientError("the bridge refused the request as too large")
        if error.code == 404:
            return ClientError(f"the bridge has no such build or artifact: {message}")
        if code is ErrorCode.CALIBRATION_REQUIRED:
            return ClientError(f"the host is not calibrated: {message}")
        return ClientError(f"the bridge refused the request ({error.code}): {message}")

    @staticmethod
    def _parse(model: type[_ModelT], body: bytes, label: str) -> _ModelT:
        try:
            return model.model_validate_json(body)
        except ValidationError as error:
            first = error.errors()[0]
            where = ".".join(str(part) for part in first.get("loc", ())) or "document"
            raise ProtocolError(
                f"the bridge {label} is not protocol v1: {where}: {first.get('msg', 'invalid')}"
            ) from None


def _read_capped(
    response: object,
    *,
    deadline: float | None = None,
    clock: Callable[[], float] | None = None,
) -> bytes:
    """Read a bounded body, never trusting ``Content-Length``.

    ``read(n)`` on a urllib response returns whatever is buffered, so the loop
    reads until EOF or one byte past the cap. When a deadline is supplied, the
    overall read is abandoned once it passes instead of waiting on a drip-fed
    body.
    """
    reader = getattr(response, "read", None)
    if reader is None:  # pragma: no cover - every response object is file-like
        raise ProtocolError("the bridge response could not be read")
    now = time.monotonic if clock is None else clock
    chunks: list[bytes] = []
    total = 0
    while True:
        if deadline is not None and now() >= deadline:
            raise NetworkError("the bridge response exceeded the overall timeout")
        chunk = reader(min(_CHUNK_BYTES, MAX_JSON_BYTES + 1 - total))
        if not isinstance(chunk, bytes):  # pragma: no cover - urllib returns bytes
            raise ProtocolError("the bridge response was not bytes")
        if not chunk:
            break
        chunks.append(chunk)
        total += len(chunk)
        if total > MAX_JSON_BYTES:
            raise ProtocolError("the bridge response exceeds the 1 MiB JSON limit")
    return b"".join(chunks)


def _wire_error(body: bytes) -> tuple[ErrorCode | None, str]:
    if not body:
        return None, "no error document"
    try:
        payload = json.loads(body.decode("utf-8"))
        error = ErrorResponse.model_validate(payload)
    except (UnicodeDecodeError, json.JSONDecodeError, ValidationError):
        return None, "the bridge returned an untyped error"
    return error.code, error.message


def _safe_reason(error: BaseException) -> str:
    """Describe a transport failure without ever echoing a token or a URL."""
    if isinstance(error, urllib.error.URLError):
        inner = error.reason
        if isinstance(inner, BaseException):
            return f"{type(inner).__name__}"
        return str(inner)[:100]
    return type(error).__name__


def _path_id(value: str) -> str:
    if not value or "/" in value or "\\" in value or "%" in value or not value.isprintable():
        raise ClientError("the build id is not a safe path segment")
    return urllib.parse.quote(value, safe="._:-")


def _safe_artifact_name(name: str) -> str:
    """Reject traversal and separators before a local file is created."""
    if (
        not name
        or name in {".", ".."}
        or "/" in name
        or "\\" in name
        or "%" in name
        or ":" in name
        or not name.isprintable()
    ):
        raise ArtifactError("the bridge supplied an artifact name that is not safe locally")
    return name


__all__ = (
    "DEFAULT_ARTIFACT_TIMEOUT_SECONDS",
    "DEFAULT_TIMEOUT_SECONDS",
    "MAX_ARTIFACT_BYTES",
    "MAX_JSON_BYTES",
    "BridgeClient",
    "Download",
    "validate_bridge_url",
)
