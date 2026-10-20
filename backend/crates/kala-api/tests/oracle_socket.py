#!/usr/bin/env python3
"""Live Python differential oracle over a real loopback socket.

This is a test oracle, not product code. It starts the **live** Python
FastAPI application of `kala_agent.api` on a loopback port with uvicorn, and
replays a corpus of raw HTTP/1.1 requests against it and against the native
server, which the Rust test starts on another loopback port.

The two servers are different HTTP stacks (uvicorn over h11 or httptools, and
hyper), so the comparison is over the status, the body bytes, and the headers
the application itself produces. The framing headers the two servers add on
their own (`date`, `server`, `connection`) are excluded, because they belong
to the servers and not to the application.

Usage:
    python3 oracle_socket.py <corpus.json> <results.json> <native-port>
"""

from __future__ import annotations

import base64
import json
import socket
import sys
import threading
import time
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[4]
SRC = REPO / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

TOKEN = "unit-test-launch-token-0123456789abcdef"
MAX_BODY_BYTES = 4096

# Headers the two HTTP servers add on their own. They are not the
# application's headers and they are allowed to differ.
SERVER_HEADERS = {"date", "server", "connection"}

from fastapi import FastAPI, Request, Response  # noqa: E402

from kala_agent.api.app import build_middleware_stack  # noqa: E402
from kala_agent.api.errors import install_error_handlers  # noqa: E402


def build_app() -> FastAPI:
    """Build the live transport of the Python API over a probe router."""
    app = FastAPI(
        middleware=build_middleware_stack(
            api_token=TOKEN,
            max_body_bytes=MAX_BODY_BYTES,
        ),
    )

    @app.get("/probe")
    def probe_read() -> dict[str, bool]:
        return {"ok": True}

    @app.post("/probe")
    async def probe_write(request: Request) -> Response:
        body = await request.body()
        return Response(content=body, media_type="application/octet-stream")

    install_error_handlers(app)
    return app


def case(
    name: str,
    method: str,
    path: str = "/probe",
    headers: list[tuple[str, str]] | None = None,
    body: bytes = b"",
    chunked: bool = False,
) -> dict[str, Any]:
    """Build one socket case."""
    request_headers = list(headers or [])
    if not chunked and body:
        request_headers.append(("content-length", str(len(body))))
    return {
        "name": name,
        "method": method,
        "path": path,
        "headers": [[name, value] for name, value in request_headers],
        "body": base64.b64encode(body).decode("ascii"),
        "chunked": chunked,
    }


def build_cases() -> list[dict[str, Any]]:
    """Build the socket corpus: well-formed requests only."""
    cases: list[dict[str, Any]] = []
    host = [("host", "127.0.0.1")]
    cases.append(case("socket.probe.read", "GET", headers=host))
    cases.append(
        case(
            "socket.probe.write",
            "POST",
            headers=[*host, ("authorization", f"Bearer {TOKEN}")],
            body=b'{"value":1}',
        )
    )
    cases.append(case("socket.probe.echo", "POST", headers=host, body=b"raw bytes"))
    cases.append(case("socket.probe.missing", "GET", path="/missing", headers=host))
    cases.append(case("socket.probe.method", "PUT", headers=host))
    for value in [
        "127.0.0.1",
        "127.0.0.1:8000",
        "localhost",
        "LOCALHOST",
        "[::1]:8000",
        "127.0.0.2",
    ]:
        cases.append(case(f"socket.host.{value}", "GET", headers=[("host", value)]))
    for value in [
        "evil.example",
        "0.0.0.0",
        "[::]:8000",
        "127.0.0.1.",
        "127.0.0.1:99999",
        "127.0.0.1:0",
        "*",
    ]:
        cases.append(case(f"socket.host.refused.{value}", "GET", headers=[("host", value)]))
    for value in [
        "http://evil.example",
        "null",
        "http://127.0.0.1:5173",
    ]:
        cases.append(
            case(
                f"socket.origin.{value}",
                "GET",
                headers=[*host, ("origin", value)],
            )
        )
    for value in [
        "",
        "Bearer",
        f"Basic {TOKEN}",
        f"Bearer {TOKEN}x",
        f"Bearer {TOKEN}, Bearer {TOKEN}",
    ]:
        cases.append(
            case(
                f"socket.token.{value or 'empty'}",
                "POST",
                headers=[*host, ("authorization", value)],
                body=b"{}",
            )
        )
    cases.append(
        case(
            "socket.token.permitted",
            "POST",
            headers=[*host, ("authorization", f"Bearer {TOKEN}")],
            body=b"{}",
        )
    )
    cases.append(
        case(
            "socket.body.over",
            "POST",
            headers=host,
            body=b"x" * (MAX_BODY_BYTES + 1),
        )
    )
    cases.append(
        case(
            "socket.body.chunked.over",
            "POST",
            headers=host,
            body=b"x" * (MAX_BODY_BYTES + 1),
            chunked=True,
        )
    )
    cases.append(
        case(
            "socket.body.chunked.within",
            "POST",
            headers=host,
            body=b"y" * 64,
            chunked=True,
        )
    )
    cases.append(
        case(
            "socket.body.malformed.json",
            "POST",
            headers=[*host, ("content-type", "application/json")],
            body=b'{"value":',
        )
    )
    return cases


def free_port() -> int:
    """Return a free loopback port."""
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def send(port: int, item: dict[str, Any]) -> dict[str, Any]:
    """Send one raw request and read the raw response."""
    payload = base64.b64decode(item["body"])
    lines = [f"{item['method']} {item['path']} HTTP/1.1"]
    seen = {name.lower() for name, _ in item["headers"]}
    if "host" not in seen:
        lines.append("Host: 127.0.0.1")
    for name, value in item["headers"]:
        lines.append(f"{name}: {value}")
    if item.get("chunked"):
        lines.append("Transfer-Encoding: chunked")
    elif payload and "content-length" not in seen:
        lines.append(f"Content-Length: {len(payload)}")
    lines.append("Connection: close")
    request = ("\r\n".join(lines) + "\r\n\r\n").encode("latin-1")
    with socket.create_connection(("127.0.0.1", port), timeout=30) as connection:
        connection.sendall(request)
        if item.get("chunked"):
            half = max(1, len(payload) // 2)
            for chunk in (payload[:half], payload[half:]):
                if chunk:
                    connection.sendall(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
            connection.sendall(b"0\r\n\r\n")
        elif payload:
            connection.sendall(payload)
        raw = bytearray()
        while True:
            try:
                piece = connection.recv(65536)
            except ConnectionResetError:
                # The server answered and closed a socket that still held
                # unread request bytes. The answer is already in the buffer.
                break
            if not piece:
                break
            raw.extend(piece)
    head, _, body = bytes(raw).partition(b"\r\n\r\n")
    head_lines = head.decode("latin-1").split("\r\n")
    status = int(head_lines[0].split()[1])
    headers: dict[str, str] = {}
    for line in head_lines[1:]:
        if ":" in line:
            name, value = line.split(":", 1)
            headers[name.strip().lower()] = value.strip()
    declared = headers.get("content-length")
    if declared is not None and declared.isdigit():
        body = body[: int(declared)]
    return {
        "status": status,
        "headers": {
            name: value for name, value in headers.items() if name not in SERVER_HEADERS
        },
        "body": base64.b64encode(body).decode("ascii"),
    }


def start_python_server() -> tuple[int, Any]:
    """Start the live Python application on a loopback port."""
    import uvicorn

    port = free_port()
    config = uvicorn.Config(
        build_app(),
        host="127.0.0.1",
        port=port,
        log_level="error",
        access_log=False,
    )
    server = uvicorn.Server(config)
    thread = threading.Thread(target=server.run, daemon=True)
    thread.start()
    deadline = time.time() + 30
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return port, server
        except OSError:
            time.sleep(0.1)
    raise RuntimeError("the Python server did not start")


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(
            "usage: oracle_socket.py <corpus.json> <results.json> <native-port>",
            file=sys.stderr,
        )
        return 2
    corpus = {"cases": build_cases()}
    Path(argv[1]).write_text(json.dumps(corpus, indent=1, sort_keys=True), encoding="utf-8")
    native_port = int(argv[3])
    python_port, server = start_python_server()
    mismatches: list[str] = []
    try:
        for item in corpus["cases"]:
            python_answer = send(python_port, item)
            native_answer = send(native_port, item)
            if (
                python_answer["status"] != native_answer["status"]
                or python_answer["headers"] != native_answer["headers"]
                or python_answer["body"] != native_answer["body"]
            ):
                mismatches.append(
                    f"{item['name']}: python={python_answer['status']} "
                    f"{python_answer['headers']} {python_answer['body'][:200]!r} "
                    f"native={native_answer['status']} {native_answer['headers']} "
                    f"{native_answer['body'][:200]!r}"
                )
    finally:
        server.should_exit = True
        time.sleep(0.5)
    Path(argv[2]).write_text(
        json.dumps(
            {"cases": len(corpus["cases"]), "mismatches": mismatches},
            indent=1,
            sort_keys=True,
        ),
        encoding="utf-8",
    )
    print(f"socket cases: {len(corpus['cases'])}")
    print(f"socket mismatches: {len(mismatches)}")
    for mismatch in mismatches:
        print(mismatch)
    return 0 if not mismatches else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
