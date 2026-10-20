"""Authenticated loopback HTTP surface.

The server binds ``127.0.0.1`` only. Every route, including ``GET
/v1/health``, requires a bearer token. Requests carrying ``Origin`` are
rejected, the exact loopback ``Host`` authority is required, duplicate or
malformed headers are refused, and bodies are bounded before they are read.
No handler on an HTTP thread calls COM: the manager exposes only ledger and
readiness operations.
"""

from __future__ import annotations

import json
import re
import sys
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Final
from urllib.parse import urlsplit

from pydantic import ValidationError

from .auth import ensure_token, token_matches, token_path
from .errors import (
    BridgeError,
    BusyError,
    CalibrationRequiredError,
    ConflictError,
    InputError,
    UnavailableError,
)
from .jobs import ChildProcessBackend, JobManager
from .protocol import BuildRequest, ErrorCode, ErrorResponse
from .version import BRIDGE_VERSION
from .workspace import default_data_dir

MAX_REQUEST_BYTES: Final[int] = 256 * 1024
_SOCKET_TIMEOUT_SECONDS: Final[float] = 15.0
_BEARER_PATTERN: Final[re.Pattern[str]] = re.compile(r"Bearer ([A-Za-z0-9_-]{16,256})")
_CONTENT_LENGTH_PATTERN: Final[re.Pattern[str]] = re.compile(r"[0-9]{1,12}")
_BUILD_ID_PATTERN: Final[re.Pattern[str]] = re.compile(r"[A-Za-z0-9][A-Za-z0-9._:-]{0,63}")
_ARTIFACT_PATTERN: Final[re.Pattern[str]] = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}")


class _HTTPProblem(Exception):
    def __init__(
        self, status: HTTPStatus, code: ErrorCode, message: str, *, close: bool = True
    ) -> None:
        super().__init__(message)
        self.status = status
        self.code = code
        self.message = message
        self.close = close


def _status_for(error: BridgeError) -> HTTPStatus:
    if isinstance(error, (BusyError, ConflictError)):
        return HTTPStatus.CONFLICT
    if isinstance(error, (UnavailableError, CalibrationRequiredError)):
        return HTTPStatus.SERVICE_UNAVAILABLE
    if isinstance(error, InputError):
        return HTTPStatus.BAD_REQUEST
    return HTTPStatus.SERVICE_UNAVAILABLE


class BridgeHTTPServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, address: tuple[str, int], manager: JobManager, token: str) -> None:
        self.manager = manager
        self.bearer_token = token
        super().__init__(address, BridgeRequestHandler)

    @property
    def authority(self) -> str:
        host, port = self.server_address[0], self.server_address[1]
        return f"{host!s}:{port}"


class BridgeRequestHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = f"kala-se-bridge/{BRIDGE_VERSION}"
    sys_version = ""

    server: BridgeHTTPServer

    def log_message(self, format: str, *args: object) -> None:
        sys.stderr.write("kala-se-bridge: " + (format % args) + "\n")

    # ------------------------------------------------------------------ methods

    def do_GET(self) -> None:
        self._guarded("GET")

    def do_POST(self) -> None:
        self._guarded("POST")

    def do_HEAD(self) -> None:
        self._method_not_allowed()

    def do_PUT(self) -> None:
        self._method_not_allowed()

    def do_PATCH(self) -> None:
        self._method_not_allowed()

    def do_DELETE(self) -> None:
        self._method_not_allowed()

    def do_OPTIONS(self) -> None:
        self._send_error(
            HTTPStatus.METHOD_NOT_ALLOWED,
            ErrorCode.INVALID_REQUEST,
            "method not allowed",
            extra_headers={"Allow": "GET, POST"},
        )

    def _guarded(self, method: str) -> None:
        try:
            self._check_host()
            self._check_origin()
            self._check_auth()
            body = self._read_body(method)
            self._route(method, body)
        except _HTTPProblem as problem:
            self._send_error(
                problem.status,
                problem.code,
                problem.message,
                close=problem.close,
            )
        except KeyError:
            self._send_error(HTTPStatus.NOT_FOUND, ErrorCode.INVALID_REQUEST, "not found")
        except BridgeError as error:
            self._send_error(_status_for(error), error.code, error.message)
        except Exception:
            self._send_error(
                HTTPStatus.SERVICE_UNAVAILABLE,
                ErrorCode.INTERNAL_ERROR,
                "the bridge could not complete this request",
            )

    # ------------------------------------------------------------------ guards

    def _check_host(self) -> None:
        values = self.headers.get_all("Host") or []
        if len(values) != 1:
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "exactly one Host header is required",
            )
        if values[0] != self.server.authority:
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "the Host authority must be the bridge loopback address and port",
            )

    def _check_origin(self) -> None:
        if self.headers.get_all("Origin") is not None:
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "browser origin headers are not accepted by this loopback bridge",
            )

    def _check_auth(self) -> None:
        values = self.headers.get_all("Authorization") or []
        if len(values) != 1:
            raise _HTTPProblem(
                HTTPStatus.UNAUTHORIZED,
                ErrorCode.INVALID_REQUEST,
                "exactly one bearer Authorization header is required",
            )
        match = _BEARER_PATTERN.fullmatch(values[0])
        if match is None or not token_matches(self.server.bearer_token, match.group(1)):
            raise _HTTPProblem(
                HTTPStatus.UNAUTHORIZED,
                ErrorCode.INVALID_REQUEST,
                "authentication failed",
            )

    def _read_body(self, method: str) -> bytes:
        transfer_encoding = self.headers.get_all("Transfer-Encoding")
        if transfer_encoding is not None:
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "Transfer-Encoding is not accepted",
            )
        lengths = self.headers.get_all("Content-Length") or []
        if len(lengths) > 1:
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "duplicate Content-Length headers are not accepted",
            )
        if not lengths:
            return b""
        if _CONTENT_LENGTH_PATTERN.fullmatch(lengths[0]) is None:
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "Content-Length is malformed",
            )
        length = int(lengths[0])
        if length > MAX_REQUEST_BYTES:
            raise _HTTPProblem(
                HTTPStatus.REQUEST_ENTITY_TOO_LARGE,
                ErrorCode.INVALID_REQUEST,
                "request body exceeds the 256 KiB limit",
            )
        if method != "POST" and length != 0:
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "this route does not accept a request body",
            )
        if length == 0:
            return b""
        self.connection.settimeout(_SOCKET_TIMEOUT_SECONDS)
        data = self.rfile.read(length)
        if len(data) != length:
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "request body ended before Content-Length bytes were read",
            )
        return data

    # ------------------------------------------------------------------ routing

    def _route(self, method: str, body: bytes) -> None:
        parsed = urlsplit(self.path)
        if parsed.scheme or parsed.netloc or parsed.query or parsed.fragment:
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "absolute, query, and fragment request targets are not accepted",
            )
        path = parsed.path
        if "%" in path or "\\" in path or "//" in path:
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "encoded or ambiguous request paths are not accepted",
            )
        parts = path.split("/")
        if len(parts) < 3 or parts[0] != "" or parts[1] != "v1":
            raise _HTTPProblem(HTTPStatus.NOT_FOUND, ErrorCode.INVALID_REQUEST, "not found")

        if method == "GET" and parts == ["", "v1", "health"]:
            self._send_model(HTTPStatus.OK, self.server.manager.health())
            return
        if method == "POST" and parts == ["", "v1", "builds"]:
            self._submit(body)
            return
        if len(parts) == 4 and parts[2] == "builds":
            build_id = parts[3]
            if _BUILD_ID_PATTERN.fullmatch(build_id) is None:
                raise _HTTPProblem(
                    HTTPStatus.BAD_REQUEST, ErrorCode.INVALID_REQUEST, "invalid build id"
                )
            if method == "GET":
                self._send_model(HTTPStatus.OK, self.server.manager.status(build_id))
                return
            raise _HTTPProblem(
                HTTPStatus.METHOD_NOT_ALLOWED,
                ErrorCode.INVALID_REQUEST,
                "method not allowed",
                close=False,
            )
        if len(parts) == 5 and parts[2] == "builds" and parts[4] == "cancel":
            build_id = parts[3]
            if _BUILD_ID_PATTERN.fullmatch(build_id) is None:
                raise _HTTPProblem(
                    HTTPStatus.BAD_REQUEST, ErrorCode.INVALID_REQUEST, "invalid build id"
                )
            if method == "POST":
                self._send_model(HTTPStatus.OK, self.server.manager.cancel(build_id))
                return
            raise _HTTPProblem(
                HTTPStatus.METHOD_NOT_ALLOWED,
                ErrorCode.INVALID_REQUEST,
                "method not allowed",
                close=False,
            )
        if len(parts) == 5 and parts[2] == "builds" and parts[4] == "artifacts":
            raise _HTTPProblem(HTTPStatus.NOT_FOUND, ErrorCode.INVALID_REQUEST, "not found")
        if len(parts) == 6 and parts[2] == "builds" and parts[4] == "artifacts" and method == "GET":
            build_id, name = parts[3], parts[5]
            if _BUILD_ID_PATTERN.fullmatch(build_id) is None:
                raise _HTTPProblem(
                    HTTPStatus.BAD_REQUEST, ErrorCode.INVALID_REQUEST, "invalid build id"
                )
            if _ARTIFACT_PATTERN.fullmatch(name) is None:
                raise _HTTPProblem(
                    HTTPStatus.BAD_REQUEST, ErrorCode.INVALID_REQUEST, "invalid artifact name"
                )
            self._serve_artifact(build_id, name)
            return
        raise _HTTPProblem(HTTPStatus.NOT_FOUND, ErrorCode.INVALID_REQUEST, "not found")

    def _submit(self, body: bytes) -> None:
        content_type = (self.headers.get("Content-Type") or "").split(";")[0].strip().lower()
        if content_type != "application/json":
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "Content-Type must be application/json",
            )
        try:
            payload = json.loads(body.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "request body is not valid UTF-8 JSON",
            ) from exc
        if not isinstance(payload, dict):
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                "request body must be a JSON object",
            )
        try:
            request = BuildRequest.model_validate(payload)
        except ValidationError as exc:
            first = exc.errors()[0]
            location = ".".join(str(part) for part in first.get("loc", ())) or "request"
            kind = str(first.get("type", "invalid"))
            raise _HTTPProblem(
                HTTPStatus.BAD_REQUEST,
                ErrorCode.INVALID_REQUEST,
                f"invalid BuildRequest at {location}: {kind}",
            ) from exc
        accepted = self.server.manager.submit(request)
        self._send_model(HTTPStatus.ACCEPTED, accepted)

    def _emit(self, status: HTTPStatus, headers: list[tuple[str, str]], body: bytes) -> None:
        try:
            self.send_response(status)
            for header, value in headers:
                self.send_header(header, value)
            self.end_headers()
            self.wfile.write(body)
        except OSError:
            self.close_connection = True

    def _serve_artifact(self, build_id: str, name: str) -> None:
        artifact, data = self.server.manager.artifact(build_id, name)
        self._emit(
            HTTPStatus.OK,
            [
                ("Content-Type", artifact.media_type),
                ("Content-Length", str(len(data))),
                ("Content-Disposition", f'attachment; filename="{artifact.name}"'),
                ("X-Artifact-Sha256", artifact.sha256),
                ("Cache-Control", "no-store"),
            ],
            data,
        )

    # ------------------------------------------------------------------ responses

    def _send_model(self, status: HTTPStatus, model: Any) -> None:  # noqa: ANN401
        if hasattr(model, "model_dump_json"):
            body = model.model_dump_json().encode("utf-8")
        else:  # pragma: no cover - every route passes a Pydantic model
            body = json.dumps(model).encode("utf-8")
        self._emit(
            status,
            [
                ("Content-Type", "application/json"),
                ("Content-Length", str(len(body))),
                ("Cache-Control", "no-store"),
            ],
            body,
        )

    def _send_error(
        self,
        status: HTTPStatus,
        code: ErrorCode,
        message: str,
        *,
        close: bool = True,
        extra_headers: dict[str, str] | None = None,
    ) -> None:
        body = ErrorResponse(code=code, message=" ".join(message.split())[:500] or "error")
        encoded = body.model_dump_json().encode("utf-8")
        headers: list[tuple[str, str]] = [
            ("Content-Type", "application/json"),
            ("Content-Length", str(len(encoded))),
            ("Cache-Control", "no-store"),
        ]
        if status is HTTPStatus.UNAUTHORIZED:
            headers.append(("WWW-Authenticate", "Bearer"))
        if extra_headers:
            headers.extend(extra_headers.items())
        if close:
            headers.append(("Connection", "close"))
            self.close_connection = True
        self._emit(status, headers, encoded)

    def _method_not_allowed(self) -> None:
        self._send_error(
            HTTPStatus.METHOD_NOT_ALLOWED,
            ErrorCode.INVALID_REQUEST,
            "method not allowed",
            close=False,
            extra_headers={"Allow": "GET, POST"},
        )


def create_server(
    port: int,
    manager: JobManager,
    token: str,
    *,
    host: str = "127.0.0.1",
) -> BridgeHTTPServer:
    """Create the loopback server without starting its loop (also a test seam)."""
    return BridgeHTTPServer((host, port), manager, token)


def production_manager(data_dir: Path) -> JobManager:
    return JobManager(
        data_dir,
        lambda: ChildProcessBackend(data_dir),
        start_supervisor=True,
    )


def is_windows_host() -> bool:
    """Return whether this is the Windows runtime, without static narrowing.

    ``mypy`` special-cases ``sys.platform`` comparisons, which would mark the
    Windows-only branch unreachable when type-checking on a Linux host. A plain
    function boundary keeps the real runtime check honest for both the CLI and
    the serving entry point.
    """
    return str(sys.platform) == "win32"


def serve(port: int, data_dir: Path | None = None) -> int:
    """Run the authenticated bridge until interrupted.

    The serving loop owns the calling thread. On exit it closes the socket and
    the manager; it never calls :meth:`socketserver.BaseServer.shutdown` from
    the thread that is serving, because that call waits for the loop to stop and
    would deadlock.

    This is the public Windows-only entry point: off Windows it refuses with
    exit code 2 *before* any default Linux data directory or token is created.
    Tests and embedding code use :func:`create_server` / :func:`production_manager`
    with an injected backend instead.
    """
    if not is_windows_host():
        sys.stderr.write(
            "kala-se-bridge: serve requires Windows; this is a development host with no "
            "Solid Edge runtime. No data directory or token was created. The authenticated "
            "HTTP surface can be exercised in tests through create_server with an injected "
            "backend.\n"
        )
        return 2
    directory = data_dir if data_dir is not None else default_data_dir()
    directory.mkdir(parents=True, exist_ok=True)
    token = ensure_token(directory)
    manager = production_manager(directory)
    try:
        server = create_server(port, manager, token)
    except OSError:
        manager.close()
        raise
    sys.stderr.write(
        f"kala-se-bridge {BRIDGE_VERSION} listening on http://127.0.0.1:{server.server_address[1]}\n"
    )
    sys.stderr.write(f"token file: {token_path(directory)}\n")
    try:
        server.serve_forever(poll_interval=0.2)
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        manager.close()
    return 0


__all__ = (
    "MAX_REQUEST_BYTES",
    "BridgeHTTPServer",
    "create_server",
    "is_windows_host",
    "production_manager",
    "serve",
)
