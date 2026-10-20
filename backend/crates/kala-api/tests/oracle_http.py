#!/usr/bin/env python3
"""Live Python differential oracle for the Kala API connection layer.

This is a test oracle, not product code. It builds the **live** Python
FastAPI application of `kala_agent.api` -- the real request boundary
middleware stack and the real error handlers -- and replays a deterministic
corpus of requests through it.

The corpus is written to disk so the Rust side can replay exactly the same
requests through the native connection layer and compare status, headers and
body bytes.

The application is driven at the ASGI boundary, which is the boundary this
crate ports. The HTTP/1.1 framing layer (uvicorn and its parser) is not part
of this cell, and driving the ASGI application directly is what makes the two
sides see byte-identical requests: a duplicated `Host` header, an empty
`Origin`, a chunked body without a declared length and a body that lies about
its length all reach both applications unchanged.

Usage:
    python3 oracle_http.py <corpus.json> <results.json>
"""

from __future__ import annotations

import asyncio
import base64
import json
import sys
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[4]
SRC = REPO / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

from fastapi import FastAPI, Request, Response  # noqa: E402
from pydantic import BaseModel, ConfigDict, Field, StringConstraints  # noqa: E402
from typing_extensions import Annotated  # noqa: E402

from kala_agent.api.app import build_middleware_stack  # noqa: E402
from kala_agent.api.errors import install_error_handlers  # noqa: E402

TOKEN = "unit-test-launch-token-0123456789abcdef"
MAX_BODY_BYTES = 4096
PROBE_BODY = b'{"ok":true}'


class Payload(BaseModel):
    """The validated payload of the error handler fixture.

    It is a test fixture, not a port of `kala_agent.api.schemas`: the routes
    and their schemas belong to a later cell. What matters here is that the
    failures it produces are the real Pydantic failures, so the sanitized
    detail lines of the error handlers can be compared.
    """

    model_config = ConfigDict(extra="forbid")

    name: Annotated[str, StringConstraints(min_length=1, max_length=8, pattern=r"^[a-z]+$")]
    count: Annotated[int, Field(ge=0, le=10)]
    nested: str | None = None


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
    declared_length: str | None = None,
) -> dict[str, Any]:
    """Build one corpus case."""
    request_headers = list(headers or [])
    if declared_length is None and not chunked and body:
        request_headers.append(("content-length", str(len(body))))
    if declared_length is not None:
        request_headers.append(("content-length", declared_length))
    return {
        "name": name,
        "method": method,
        "path": path,
        "headers": [[name, value] for name, value in request_headers],
        "body": base64.b64encode(body).decode("ascii"),
        "chunked": chunked,
    }


def host_cases() -> list[dict[str, Any]]:
    """Every Host variant the boundary judges."""
    cases: list[dict[str, Any]] = []
    permitted = [
        "127.0.0.1",
        "127.0.0.1:8000",
        "localhost",
        "localhost:8000",
        "LOCALHOST",
        "LocalHost:80",
        "[::1]",
        "[::1]:8000",
        "127.0.0.2",
        "127.0.0.255",
        "[::ffff:127.0.0.1]:80",
        "[::1%eth0]:80",
        "127.0.0.1:08000",
        "127.0.0.1:00001",
        "127.0.0.1:65535",
        "127.1.2.3",
    ]
    refused = [
        "evil.example",
        "attacker.example:8000",
        "127.0.0.1.evil.example",
        "localhost.evil.example",
        "127.0.0.1@evil.example",
        "0.0.0.0",
        "0.0.0.0:8000",
        "::",
        "[::]:8000",
        "[::0]",
        "0:0:0:0:0:0:0:0",
        "*",
        "10.0.0.5:8000",
        "169.254.169.254",
        "127.0.0.1.",
        "127.0.0.1:0",
        "127.0.0.1:99999",
        "127.0.0.1:port",
        "[::1",
        "",
        " 127.0.0.1",
        "127.0.0.1 ",
        "[::1]x",
        "[::1]x:8000",
        ":8000",
        "user@127.0.0.1",
        "127.0.0.1/path",
        "http://127.0.0.1",
        "xn--localhost",
        "localhost\u00a0",
        "::1%eth0",
        "127.0.0.1:65536",
        "127.0.0.1:0000000000000000000000001",
        "1" * 300,
    ]
    for value in permitted:
        cases.append(case(f"host.permitted.{value}", "GET", headers=[("host", value)]))
    for value in refused:
        cases.append(case(f"host.refused.{value}", "GET", headers=[("host", value)]))
    cases.append(case("host.absent", "GET"))
    cases.append(
        case(
            "host.duplicate",
            "GET",
            headers=[("host", "127.0.0.1"), ("host", "evil.example")],
        )
    )
    cases.append(
        case(
            "host.duplicate.same",
            "GET",
            headers=[("host", "127.0.0.1"), ("host", "127.0.0.1")],
        )
    )
    cases.append(
        case(
            "host.uppercase.name",
            "GET",
            headers=[("Host", "127.0.0.1")],
        )
    )
    return cases


def origin_cases() -> list[dict[str, Any]]:
    """Every Origin variant the boundary refuses."""
    cases = [case("origin.absent", "GET", headers=[("host", "127.0.0.1")])]
    for value in [
        "http://evil.example",
        "http://localhost:5173",
        "http://127.0.0.1:5173",
        "http://0.0.0.0:5173",
        "https://127.0.0.1:5173",
        "null",
        "",
        "http://\u4f8b\u3048.\u30c6\u30b9\u30c8",
        "http://127.0.0.1:5173, http://evil.example",
    ]:
        cases.append(
            case(
                f"origin.refused.{value or 'empty'}",
                "GET",
                headers=[("host", "127.0.0.1"), ("origin", value)],
            )
        )
    cases.append(
        case(
            "origin.duplicate",
            "GET",
            headers=[
                ("host", "127.0.0.1"),
                ("origin", "http://127.0.0.1:5173"),
                ("origin", "http://evil.example"),
            ],
        )
    )
    return cases


def token_cases() -> list[dict[str, Any]]:
    """Every bearer credential variant the boundary judges."""
    cases: list[dict[str, Any]] = []
    read = [("host", "127.0.0.1")]
    for value in [
        None,
        "",
        "Bearer",
        "Bearer ",
        f"Basic {TOKEN}",
        f"Bearer {TOKEN}x",
        f"bearer {TOKEN[:-1]}",
        "Bearer sk-must-never-be-echoed-9876543210abcdef",
        TOKEN,
        f"Bearer {TOKEN}, Bearer {TOKEN}",
        f"Bearer\t{TOKEN}",
        f"Bearer {TOKEN} trailing",
        "bearer",
        "Bearer  ",
    ]:
        headers = list(read)
        if value is not None:
            headers.append(("authorization", value))
        cases.append(
            case(
                f"token.refused.{'absent' if value is None else value or 'empty'}",
                "POST",
                headers=headers,
                body=b'{"value":1}',
            )
        )
    for value in [
        f"Bearer {TOKEN}",
        f"bearer {TOKEN}",
        f"BEARER {TOKEN}",
        f"Bearer  {TOKEN}",
        f"Bearer {TOKEN}\t",
        f"Bearer \t{TOKEN}",
    ]:
        cases.append(
            case(
                f"token.permitted.{value.strip().replace(' ', '_')}",
                "POST",
                headers=[*read, ("authorization", value)],
                body=b'{"value":1}',
            )
        )
    cases.append(
        case(
            "token.duplicate",
            "POST",
            headers=[
                *read,
                ("authorization", f"Bearer {TOKEN}"),
                ("authorization", f"Bearer {TOKEN}"),
            ],
            body=b'{"value":1}',
        )
    )
    return cases


def body_cases() -> list[dict[str, Any]]:
    """Every body variant the size limit judges."""
    cases: list[dict[str, Any]] = []
    over = b"x" * (MAX_BODY_BYTES + 1)
    within = b"y" * 64
    for method in ("GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"):
        cases.append(
            case(
                f"body.over.{method}",
                method,
                headers=[("host", "127.0.0.1")],
                body=over,
            )
        )
        cases.append(
            case(
                f"body.chunked.over.{method}",
                method,
                headers=[("host", "127.0.0.1")],
                body=over,
                chunked=True,
            )
        )
        cases.append(
            case(
                f"body.within.{method}",
                method,
                headers=[("host", "127.0.0.1")],
                body=within,
            )
        )
    cases.append(
        case(
            "body.declared.over",
            "POST",
            headers=[("host", "127.0.0.1")],
            body=b"small",
            declared_length=str(MAX_BODY_BYTES + 1),
        )
    )
    cases.append(
        case(
            "body.declared.not.a.number",
            "POST",
            headers=[("host", "127.0.0.1")],
            body=within,
            declared_length="not-a-number",
        )
    )
    cases.append(
        case(
            "body.declared.negative",
            "POST",
            headers=[("host", "127.0.0.1")],
            body=within,
            declared_length="-1",
        )
    )
    cases.append(
        case(
            "body.declared.exact.limit",
            "POST",
            headers=[("host", "127.0.0.1")],
            body=b"z" * MAX_BODY_BYTES,
        )
    )
    cases.append(
        case(
            "body.declared.lies",
            "POST",
            headers=[("host", "127.0.0.1")],
            body=over,
            declared_length="10",
        )
    )
    # A declared length longer than a machine integer still answers with the
    # refusal: the header value is bounded before it is read as a number.
    cases.append(
        case(
            "body.declared.huge",
            "POST",
            headers=[("host", "127.0.0.1")],
            body=within,
            declared_length="9" * 512,
        )
    )
    cases.append(
        case(
            "body.declared.huge.short",
            "POST",
            headers=[("host", "127.0.0.1")],
            body=within,
            declared_length="9" * 39,
        )
    )
    cases.append(
        case(
            "body.declared.padded",
            "POST",
            headers=[("host", "127.0.0.1")],
            body=within,
            declared_length=" 12 ",
        )
    )
    return cases


def payload_cases() -> list[dict[str, Any]]:
    """Malformed and unusual payloads the transport must not choke on."""
    cases: list[dict[str, Any]] = []
    payloads = [
        ("empty", b""),
        ("json.object", b'{"value":1}'),
        ("json.malformed.open", b"{"),
        ("json.malformed.text", b"not json"),
        ("json.malformed.trailing", b'{"value":1} trailing'),
        ("json.malformed.trailing.comma", b'{"value":1,}'),
        ("json.array", b"[]"),
        ("json.number", b"1"),
        ("json.nan", b"NaN"),
        ("json.infinity", b"1e400"),
        ("json.utf8", "{\"caf\u00e9\":1}".encode()),
        ("json.invalid.utf8", b'{"value":"\xff"}'),
        ("json.deep", b"[" * 64 + b"]" * 64),
        ("json.whitespace", b"   "),
    ]
    for name, payload in payloads:
        cases.append(
            case(
                f"payload.{name}",
                "POST",
                headers=[("host", "127.0.0.1"), ("content-type", "application/json")],
                body=payload,
            )
        )
    return cases


def routing_cases() -> list[dict[str, Any]]:
    """The routing errors the error handlers answer."""
    cases: list[dict[str, Any]] = []
    cases.append(case("route.missing", "GET", path="/missing", headers=[("host", "127.0.0.1")]))
    cases.append(
        case("route.missing.post", "POST", path="/missing", headers=[("host", "127.0.0.1")])
    )
    for method in ("PUT", "DELETE", "OPTIONS", "PATCH"):
        cases.append(
            case(f"route.method.{method}", method, headers=[("host", "127.0.0.1")])
        )
    cases.append(
        case(
            "route.probe.read",
            "GET",
            headers=[("host", "127.0.0.1")],
        )
    )
    cases.append(
        case(
            "route.probe.write",
            "POST",
            headers=[("host", "127.0.0.1"), ("authorization", f"Bearer {TOKEN}")],
            body=b'{"value":1}',
        )
    )
    return cases


def ordering_cases() -> list[dict[str, Any]]:
    """The order of the judgements, which is the policy."""
    cases: list[dict[str, Any]] = []
    cases.append(
        case(
            "order.host.before.token",
            "POST",
            headers=[("host", "evil.example")],
            body=b"{}",
        )
    )
    cases.append(
        case(
            "order.host.before.origin",
            "GET",
            headers=[("host", "evil.example"), ("origin", "http://evil.example")],
        )
    )
    cases.append(
        case(
            "order.body.before.token",
            "POST",
            headers=[("host", "127.0.0.1")],
            body=b"x" * (MAX_BODY_BYTES + 1),
        )
    )
    cases.append(
        case(
            "order.body.before.origin",
            "GET",
            headers=[("host", "127.0.0.1"), ("origin", "http://evil.example")],
            body=b"x" * (MAX_BODY_BYTES + 1),
        )
    )
    cases.append(
        case(
            "order.token.before.origin",
            "POST",
            headers=[("host", "127.0.0.1"), ("origin", "http://evil.example")],
            body=b"{}",
        )
    )
    return cases


def credential_length_cases() -> list[dict[str, Any]]:
    """The bounds of the launch token."""
    cases: list[dict[str, Any]] = []
    for length in (0, 1, 15, 16, 17, 127, 128, 129, 200):
        token = "a" * length
        cases.append(
            case(
                f"credential.length.{length}",
                "POST",
                headers=[("host", "127.0.0.1"), ("authorization", f"Bearer {token}")],
                body=b"{}",
            )
        )
    cases.append(
        case(
            "credential.non.ascii",
            "POST",
            headers=[("host", "127.0.0.1"), ("authorization", "Bearer caf\u00e9-0123456789abc")],
            body=b"{}",
        )
    )
    cases.append(
        case(
            "credential.spaces.inside",
            "POST",
            headers=[("host", "127.0.0.1"), ("authorization", f"Bearer {TOKEN[:10]} {TOKEN[10:]}")],
            body=b"{}",
        )
    )
    return cases


def randomized_cases(count: int = 240) -> list[dict[str, Any]]:
    """A deterministic randomized corpus over the same decisions."""
    import random

    generator = random.Random(20261019)
    hosts = [
        "127.0.0.1",
        "127.0.0.1:1",
        "127.0.0.1:65535",
        "127.0.0.2",
        "localhost",
        "LOCALHOST:8000",
        "[::1]",
        "[::1]:9",
        "[::ffff:127.0.0.1]",
        "0.0.0.0",
        "[::]",
        "example.com",
        "127.0.0.1.",
        "127.0.0.1:0",
        "127.0.0.1:65536",
        "",
        "*",
        "0:0:0:0:0:0:0:0",
        "169.254.169.254",
        "10.0.0.1",
        "127.0.0.1:port",
        "[::1",
        ":8000",
        "127.0.0.1@evil.example",
    ]
    origins = [
        "http://evil.example",
        "http://localhost:5173",
        "https://127.0.0.1",
        "null",
        "",
        "http://0.0.0.0",
    ]
    credentials = [
        None,
        "",
        "Bearer",
        f"Bearer {TOKEN}",
        f"bearer {TOKEN}",
        f"Bearer {TOKEN}x",
        f"Basic {TOKEN}",
        f"Bearer {TOKEN}, Bearer {TOKEN}",
        "Bearer  " + TOKEN,
        "Bearer " + "a" * 200,
    ]
    methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD"]
    # The paths stay free of Starlette's `redirect_slashes` behaviour: that is
    # a routing decision owned by the later routes cell, not a boundary one.
    paths = ["/probe", "/missing", "/"]
    cases: list[dict[str, Any]] = []
    for index in range(count):
        host = generator.choice(hosts)
        origin = generator.choice(origins) if generator.random() < 0.25 else None
        credential = generator.choice(credentials)
        method = generator.choice(methods)
        path = generator.choice(paths)
        size = generator.choice([0, 1, 64, 1024, MAX_BODY_BYTES, MAX_BODY_BYTES + 1, 8192])
        body = bytes(generator.getrandbits(8) for _ in range(min(size, 64)))
        body = (body * (size // max(1, len(body)) + 1))[:size]
        headers: list[tuple[str, str]] = [("host", host)]
        if origin is not None:
            headers.append(("origin", origin))
        if credential is not None:
            headers.append(("authorization", credential))
        cases.append(
            case(
                f"random.{index:04d}",
                method,
                path=path,
                headers=headers,
                body=body,
            )
        )
    return cases


def build_corpus() -> dict[str, Any]:
    """Build the whole deterministic corpus."""
    cases: list[dict[str, Any]] = []
    cases.extend(host_cases())
    cases.extend(origin_cases())
    cases.extend(token_cases())
    cases.extend(body_cases())
    cases.extend(payload_cases())
    cases.extend(routing_cases())
    cases.extend(ordering_cases())
    cases.extend(credential_length_cases())
    cases.extend(randomized_cases())
    return {
        "token": TOKEN,
        "max_body_bytes": MAX_BODY_BYTES,
        "cases": cases,
    }


async def call_app(app: FastAPI, item: dict[str, Any]) -> dict[str, Any]:
    """Drive one request through the live application at the ASGI boundary."""
    scope = {
        "type": "http",
        "asgi": {"version": "3.0", "spec_version": "2.3"},
        "http_version": "1.1",
        "method": item["method"],
        "scheme": "http",
        "path": item["path"],
        "raw_path": item["path"].encode("utf-8"),
        "query_string": b"",
        "root_path": "",
        "headers": [
            (name.lower().encode("utf-8"), value.encode("utf-8"))
            for name, value in item["headers"]
        ],
        "client": ("127.0.0.1", 54321),
        "server": ("127.0.0.1", 8000),
        "state": {},
    }
    payload = base64.b64decode(item["body"])
    chunks: list[bytes] = []
    if item.get("chunked"):
        half = max(1, len(payload) // 2)
        chunks = [payload[:half], payload[half:]]
        chunks = [chunk for chunk in chunks if chunk]
    elif payload:
        chunks = [payload]
    pending = list(chunks)

    async def receive() -> dict[str, Any]:
        if pending:
            chunk = pending.pop(0)
            return {"type": "http.request", "body": chunk, "more_body": bool(pending)}
        return {"type": "http.request", "body": b"", "more_body": False}

    messages: list[dict[str, Any]] = []

    async def send(message: dict[str, Any]) -> None:
        messages.append(message)

    await app(scope, receive, send)
    status: int | None = None
    headers: list[list[str]] = []
    body = b""
    for message in messages:
        if message["type"] == "http.response.start":
            status = int(message["status"])
            headers = [
                [name.decode("latin-1"), value.decode("latin-1")]
                for name, value in message.get("headers", [])
            ]
        elif message["type"] == "http.response.body":
            body += message.get("body", b"")
    return {
        "name": item["name"],
        "status": status,
        "headers": headers,
        "body": base64.b64encode(body).decode("ascii"),
    }


async def run(corpus: dict[str, Any]) -> list[dict[str, Any]]:
    """Replay the corpus through the live application."""
    app = build_app()
    results: list[dict[str, Any]] = []
    for item in corpus["cases"]:
        results.append(await call_app(app, item))
    return results


def validation_cases() -> list[dict[str, Any]]:
    """The payloads that make the live error handlers answer."""
    payloads = [
        ("missing", b"{}"),
        ("empty.name", b'{"name": "", "count": 1}'),
        ("long.name", b'{"name": "abcdefghijkl", "count": 1}'),
        ("pattern.name", b'{"name": "ABC", "count": 1}'),
        ("count.high", b'{"name": "abc", "count": 100}'),
        ("count.low", b'{"name": "abc", "count": -1}'),
        ("count.text", b'{"name": "abc", "count": "x"}'),
        ("extra", b'{"name": "abc", "count": 1, "extra": 1}'),
        ("malformed", b"{"),
        ("malformed.text", b"not json"),
        ("malformed.trailing", b'{"name": "abc", "count": 1} trailing'),
        ("malformed.comma", b'{"name": "abc", "count": 1,}'),
        ("array", b"[]"),
        ("text", b'"abc"'),
        ("number", b"1"),
        ("null", b"null"),
        ("deep", b'{"name": ' + b'[' * 40 + b']' * 40 + b', "count": 1}'),
    ]
    cases: list[dict[str, Any]] = []
    for name, payload in payloads:
        cases.append(
            {
                "name": name,
                "body": base64.b64encode(payload).decode("ascii"),
            }
        )
    return cases


def build_validation_app() -> FastAPI:
    """Build a live application whose route validates its payload.

    The model is a test fixture, not a port of `kala_agent.api.schemas`:
    the routes and their schemas belong to a later cell. What matters here is
    that the failures it produces are the real Pydantic failures, so the
    sanitized detail lines of the error handlers can be compared.
    """
    app = FastAPI(
        middleware=build_middleware_stack(
            api_token=TOKEN,
            max_body_bytes=MAX_BODY_BYTES,
        ),
    )

    @app.post("/validated")
    async def validated(payload: Payload) -> dict[str, object]:
        return {"name": payload.name, "count": payload.count}

    install_error_handlers(app)
    return app


async def run_validation(cases: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Replay the validation payloads through the live error handlers."""
    from fastapi.exceptions import RequestValidationError

    from kala_agent.api.errors import handle_request_validation

    app = build_validation_app()
    recorded: list[list[dict[str, Any]]] = []

    async def capture(request: Any, error: Any) -> Any:
        if isinstance(error, RequestValidationError):
            recorded.append(
                [
                    {
                        "kind": failure["type"],
                        "location": [str(part) for part in failure["loc"]],
                        "message": failure["msg"],
                    }
                    for failure in error.errors()
                ]
            )
        return await handle_request_validation(request, error)

    app.add_exception_handler(RequestValidationError, capture)
    results: list[dict[str, Any]] = []
    for item in cases:
        scope = {
            "type": "http",
            "asgi": {"version": "3.0", "spec_version": "2.3"},
            "http_version": "1.1",
            "method": "POST",
            "scheme": "http",
            "path": "/validated",
            "raw_path": b"/validated",
            "query_string": b"",
            "root_path": "",
            "headers": [
                (b"host", b"127.0.0.1"),
                (b"authorization", f"Bearer {TOKEN}".encode("utf-8")),
                (b"content-type", b"application/json"),
                (b"content-length", str(len(base64.b64decode(item["body"]))).encode()),
            ],
            "client": ("127.0.0.1", 54321),
            "server": ("127.0.0.1", 8000),
            "state": {},
        }
        payload = base64.b64decode(item["body"])
        pending = [payload] if payload else []

        async def receive() -> dict[str, Any]:
            if pending:
                chunk = pending.pop(0)
                return {"type": "http.request", "body": chunk, "more_body": False}
            return {"type": "http.request", "body": b"", "more_body": False}

        messages: list[dict[str, Any]] = []

        async def send(message: dict[str, Any]) -> None:
            messages.append(message)

        await app(scope, receive, send)
        status: int | None = None
        body = b""
        for message in messages:
            if message["type"] == "http.response.start":
                status = int(message["status"])
            elif message["type"] == "http.response.body":
                body += message.get("body", b"")
        document = json.loads(body.decode("utf-8")) if body else {}
        results.append(
            {
                "name": item["name"],
                "status": status,
                "body": base64.b64encode(body).decode("ascii"),
                "details": document.get("details", []),
                "code": document.get("error", {}).get("code"),
                "message": document.get("error", {}).get("message"),
                "failures": recorded[-1] if recorded else [],
            }
        )
    return results


def http_exception_cases() -> list[dict[str, Any]]:
    """The framework errors the error handlers answer."""
    return [
        {"name": "http.client", "status": 400, "detail": "the request was rejected"},
        {"name": "http.client.empty", "status": 400, "detail": ""},
        {"name": "http.client.long", "status": 400, "detail": "d" * 200},
        {"name": "http.client.exact", "status": 400, "detail": "e" * 128},
        {"name": "http.client.oneOver", "status": 400, "detail": "e" * 129},
        {"name": "http.client.unicode", "status": 400, "detail": "café refusé"},
        {"name": "http.client.unicode.long", "status": 400, "detail": "é" * 129},
        {"name": "http.client.quoted", "status": 400, "detail": 'a "quoted" \\ detail'},
        {"name": "http.client.teapot", "status": 418, "detail": "a short teapot"},
        {"name": "http.unauthorized", "status": 401, "detail": "a valid token is required"},
        {"name": "http.forbidden", "status": 403, "detail": "the request is refused"},
        {"name": "http.conflict", "status": 409, "detail": "the resource is in conflict"},
        {"name": "http.tooMany", "status": 429, "detail": "too many requests"},
        {"name": "http.clientClosed", "status": 499, "detail": "the client closed the request"},
        {"name": "http.server", "status": 500, "detail": "an internal detail"},
        {"name": "http.server.unavailable", "status": 503, "detail": "an internal detail"},
        {"name": "http.notfound", "status": 404, "detail": "Not Found"},
        {"name": "http.notallowed", "status": 405, "detail": "Method Not Allowed"},
        {"name": "http.tooLarge", "status": 413, "detail": "too large"},
        {"name": "http.unexpected", "status": 0, "detail": "an unexpected failure"},
    ]


async def run_http_exceptions(cases: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Replay the framework errors through the live error handlers."""
    from fastapi import HTTPException

    results: list[dict[str, Any]] = []
    for item in cases:
        app = FastAPI(
            middleware=build_middleware_stack(
                api_token=TOKEN,
                max_body_bytes=MAX_BODY_BYTES,
            ),
        )
        status = int(item["status"])
        detail = str(item["detail"])

        @app.get("/rejected")
        async def rejected() -> dict[str, object]:
            if status == 0:
                raise ValueError(detail)
            raise HTTPException(status_code=status, detail=detail)

        install_error_handlers(app)
        scope = {
            "type": "http",
            "asgi": {"version": "3.0", "spec_version": "2.3"},
            "http_version": "1.1",
            "method": "GET",
            "scheme": "http",
            "path": "/rejected",
            "raw_path": b"/rejected",
            "query_string": b"",
            "root_path": "",
            "headers": [(b"host", b"127.0.0.1")],
            "client": ("127.0.0.1", 54321),
            "server": ("127.0.0.1", 8000),
            "state": {},
        }

        async def receive() -> dict[str, Any]:
            return {"type": "http.request", "body": b"", "more_body": False}

        messages: list[dict[str, Any]] = []

        async def send(message: dict[str, Any]) -> None:
            messages.append(message)

        try:
            await app(scope, receive, send)
        except Exception:
            # The server error middleware answers and re-raises; the answer is
            # already in the collected messages.
            pass
        status: int | None = None
        body = b""
        for message in messages:
            if message["type"] == "http.response.start":
                status = int(message["status"])
            elif message["type"] == "http.response.body":
                body += message.get("body", b"")
        results.append(
            {
                "name": item["name"],
                "status": status,
                "body": base64.b64encode(body).decode("ascii"),
            }
        )
    return results


def application_error_statuses() -> list[dict[str, Any]]:
    """The status every composition root failure answers with."""
    from kala_agent.api.errors import _status_for
    from kala_agent import application

    names = [
        "UnknownDomainError",
        "UnknownRevisionError",
        "UnknownPreparedRevisionError",
        "UnknownWorkflowError",
        "ProvenanceUnavailableError",
        "CheckpointNotPendingError",
        "RevisionCommitRejectedError",
        "RevisionForkRefusedError",
        "RevisionHeadMovedError",
        "CheckpointDecisionDeniedError",
        "CheckpointDigestMismatchError",
        "WorkflowFinishedError",
        "CorruptKernelStateError",
        "ClosedApplicationError",
        "InvalidIdentifierError",
        "ReservedIdentifierError",
        "InvalidParametersError",
    ]
    results: list[dict[str, Any]] = []
    for name in names:
        error_type = getattr(application, name)
        error = error_type("message")
        results.append(
            {
                "name": name,
                "status": _status_for(error),
                "code": error.code,
            }
        )
    return results


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print("usage: oracle_http.py <corpus.json> <results.json>", file=sys.stderr)
        return 2
    corpus = build_corpus()
    validation = validation_cases()
    http_exceptions = http_exception_cases()
    corpus["validation_cases"] = validation
    corpus["http_exception_cases"] = http_exceptions
    corpus_path = Path(argv[1])
    results_path = Path(argv[2])
    corpus_path.parent.mkdir(parents=True, exist_ok=True)
    corpus_path.write_text(json.dumps(corpus, indent=1, sort_keys=True), encoding="utf-8")
    results = asyncio.run(run(corpus))
    results.extend(asyncio.run(run_validation(validation)))
    http_exceptions = http_exception_cases()
    results.extend(asyncio.run(run_http_exceptions(http_exceptions)))
    payload = {
        "http": results[: len(corpus["cases"])],
        "validation": results[len(corpus["cases"]) : len(corpus["cases"]) + len(validation)],
        "http_exceptions": results[len(corpus["cases"]) + len(validation) :],
        "application_errors": application_error_statuses(),
    }
    results_path.write_text(json.dumps(payload, indent=1, sort_keys=True), encoding="utf-8")
    print(f"oracle cases: {len(results)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
