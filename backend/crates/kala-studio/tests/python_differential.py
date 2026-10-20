#!/usr/bin/env python3
"""Live Python-vs-native Studio differential probe.

This is a test oracle, not product code. It launches the real native binary
over a loopback socket, builds the live Python FastAPI application in this
process with an empty environment view, and replays an extended matrix of
schema, middleware, static, settings, geometry and local-chat requests. It
compares status, exact body bytes and stable/security headers.

The two STEP download routes are compared structurally: OpenCascade's writer
embeds a timestamp and its own entity numbering, so the fixture contract for
those cases is `body_contract: "structural"` (a real ISO 10303-21 file with
one manifold solid and the stable headers), not byte equality. The native
metrics are independently re-imported by `tests/step_export_ocp.rs`.
"""

from __future__ import annotations

import argparse
import codecs
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[4]
SRC = REPO / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

TOKEN = "corpus-studio-token-0123456789abcdef"
SECURITY = [
    "cache-control",
    "content-security-policy",
    "cross-origin-opener-policy",
    "cross-origin-resource-policy",
    "referrer-policy",
    "x-content-type-options",
    "x-frame-options",
]
STABLE = [
    "content-type",
    "content-length",
    "location",
    "content-disposition",
    "x-studio-handoff",
    "www-authenticate",
]
ABSENT = ["server", "x-powered-by", "set-cookie"]


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def native_request(
    port: int,
    method: str,
    path: str,
    headers: list[tuple[str, str]],
    body: bytes | None,
    chunked: list[bytes] | None,
) -> tuple[int, dict[str, str], bytes]:
    header_map = {name.lower(): value for name, value in headers}
    with socket.create_connection(("127.0.0.1", port), timeout=60) as sock:
        lines = [f"{method} {path} HTTP/1.1"]
        if "host" not in header_map:
            lines.append("Host: 127.0.0.1")
        lines.append("Connection: close")
        for name, value in headers:
            lines.append(f"{name}: {value}")
        if chunked is not None:
            lines.append("Transfer-Encoding: chunked")
        elif body is not None and "content-length" not in header_map:
            lines.append(f"Content-Length: {len(body)}")
        lines.append("")
        lines.append("")
        sock.sendall("\r\n".join(lines).encode("latin-1"))
        if chunked is not None:
            for chunk in chunked:
                sock.sendall(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
            sock.sendall(b"0\r\n\r\n")
        elif body is not None:
            sock.sendall(body)
        raw = bytearray()
        while True:
            piece = sock.recv(65536)
            if not piece:
                break
            raw.extend(piece)
    head, _, payload = bytes(raw).partition(b"\r\n\r\n")
    head_lines = head.decode("latin-1").split("\r\n")
    status = int(head_lines[0].split()[1])
    response_headers: dict[str, str] = {}
    for line in head_lines[1:]:
        if ":" in line:
            name, value = line.split(":", 1)
            response_headers.setdefault(name.strip().lower(), value.strip())
    declared = response_headers.get("content-length")
    if declared is not None and declared.isdigit():
        payload = payload[: int(declared)]
    return status, response_headers, payload


def case(
    name: str,
    method: str,
    path: str,
    *,
    body: bytes | None = None,
    json_body: Any = None,
    headers: list[tuple[str, str]] | None = None,
    chunked: list[bytes] | None = None,
    auth: bool = True,
) -> dict[str, Any]:
    request_headers = list(headers or [])
    if json_body is not None:
        body = json.dumps(json_body, separators=(",", ":")).encode()
        if not any(key.lower() == "content-type" for key, _ in request_headers):
            request_headers.append(("content-type", "application/json"))
    if auth:
        request_headers.append(("Authorization", f"Bearer {TOKEN}"))
    return {
        "name": name,
        "method": method,
        "path": path,
        "body": body,
        "headers": request_headers,
        "chunked": chunked,
    }


def _is_json_object(body: bytes) -> bool:
    try:
        document = json.loads(body)
    except ValueError:
        return False
    return isinstance(document, dict)


def build_cases() -> list[dict[str, Any]]:
    cases: list[dict[str, Any]] = []

    # Preview DTO and geometry edges.
    for name, payload in [
        ("preview.box_default", {"kind": "box"}),
        ("preview.cylinder_default", {"kind": "cylinder"}),
        ("preview.sphere_default", {"kind": "sphere"}),
        (
            "preview.box_custom",
            {"kind": "box", "parameters": {"size_x": 20, "size_y": 10, "size_z": 5, "subdivisions": 2}},
        ),
        (
            "preview.cylinder_custom",
            {"kind": "cylinder", "parameters": {"radius": 2.5, "height": 7.5, "radial_segments": 12, "height_segments": 3}},
        ),
        (
            "preview.sphere_custom",
            {"kind": "sphere", "parameters": {"radius": 3.5, "rings": 12, "sectors": 16}},
        ),
        ("preview.min_dimension", {"kind": "box", "parameters": {"size_x": 0.1, "size_y": 0.1, "size_z": 0.1}}),
        ("preview.max_dimension", {"kind": "sphere", "parameters": {"radius": 2000}}),
        ("preview.integral_float", {"kind": "box", "parameters": {"size_x": 10.0}}),
        ("preview.negative_zero", {"kind": "box", "parameters": {"size_x": -0.0}}),
        ("preview.tiny", {"kind": "box", "parameters": {"size_x": 1e-7}}),
        ("preview.string_dimension", {"kind": "box", "parameters": {"size_x": "10"}}),
        ("preview.bool_dimension", {"kind": "box", "parameters": {"size_x": True}}),
        ("preview.float_count", {"kind": "box", "parameters": {"subdivisions": 2.0}}),
        ("preview.unknown_parameter", {"kind": "box", "parameters": {"nope": 1}}),
        ("preview.filtered_key", {"kind": "box", "parameters": {" size_x ": 1}}),
        ("preview.too_many_parameters", {"kind": "box", "parameters": {f"k{i}": i for i in range(33)}}),
        ("preview.empty_key", {"kind": "box", "parameters": {"": 1}}),
        ("preview.long_key", {"kind": "box", "parameters": {"k" * 65: 1}}),
        ("preview.string_value", {"kind": "box", "parameters": {"size_x": "x" * 129}}),
        ("preview.list_value", {"kind": "box", "parameters": {"size_x": [1, 2]}}),
        ("preview.object_value", {"kind": "box", "parameters": {"size_x": {"a": 1}}}),
        ("preview.null_value", {"kind": "box", "parameters": {"size_x": None}}),
        ("preview.missing_kind", {"parameters": {}}),
        ("preview.null_kind", {"kind": None}),
        ("preview.bad_kind", {"kind": "torus"}),
        ("preview.extra", {"kind": "box", "extra": 1}),
        ("preview.parameters_array", {"kind": "box", "parameters": []}),
        ("preview.parameters_null", {"kind": "box", "parameters": None}),
    ]:
        cases.append(case(name, "POST", "/api/studio/preview", json_body=payload))

    cases.extend(
        [
            case(
                "preview.inf_raw",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box","parameters":{"size_z":1e400}}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.nan_raw",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box","parameters":{"size_z":NaN}}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.wide_integer",
                "POST",
                "/api/studio/preview",
                body=(
                    b'{"kind":"box","parameters":{"size_x":'
                    + b"9" * 400
                    + b',"size_y":1,"size_z":1}}'
                ),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.too_many_digits",
                "POST",
                "/api/studio/preview",
                body=(
                    b'{"kind":"box","parameters":{"size_x":'
                    + b"9" * 4400
                    + b"}}"
                ),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.invalid_utf8",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box","parameters":{"size_x":"\xff"}}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.malformed_json",
                "POST",
                "/api/studio/preview",
                body=b"{",
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.trailing_json",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box"} trailing',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.array_top",
                "POST",
                "/api/studio/preview",
                body=b"[]",
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.null_top",
                "POST",
                "/api/studio/preview",
                body=b"null",
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.empty_body",
                "POST",
                "/api/studio/preview",
                body=b"",
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.text_content_type",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box"}',
                headers=[("content-type", "text/plain")],
            ),
            case(
                "preview.no_content_type",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box"}',
            ),

            # CPython `json.loads(bytes)` encoding detection: Windows-generated
            # UTF-8 BOM / UTF-16 / UTF-32 bodies are accepted by both.
            case(
                "preview.utf8_bom",
                "POST",
                "/api/studio/preview",
                body=codecs.BOM_UTF8 + b'{"kind":"box"}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.utf16le",
                "POST",
                "/api/studio/preview",
                body='{"kind":"box"}'.encode("utf-16-le"),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.utf16be",
                "POST",
                "/api/studio/preview",
                body='{"kind":"box"}'.encode("utf-16-be"),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.utf16le_bom",
                "POST",
                "/api/studio/preview",
                body=codecs.BOM_UTF16_LE + '{"kind":"box"}'.encode("utf-16-le"),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.utf16be_bom",
                "POST",
                "/api/studio/preview",
                body=codecs.BOM_UTF16_BE + '{"kind":"box"}'.encode("utf-16-be"),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.utf32le",
                "POST",
                "/api/studio/preview",
                body='{"kind":"box"}'.encode("utf-32-le"),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.utf32be",
                "POST",
                "/api/studio/preview",
                body='{"kind":"box"}'.encode("utf-32-be"),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.utf32le_bom",
                "POST",
                "/api/studio/preview",
                body=codecs.BOM_UTF32_LE + '{"kind":"box"}'.encode("utf-32-le"),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.utf32be_bom",
                "POST",
                "/api/studio/preview",
                body=codecs.BOM_UTF32_BE + '{"kind":"box"}'.encode("utf-32-be"),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.utf16_malformed",
                "POST",
                "/api/studio/preview",
                body=codecs.BOM_UTF16_LE + '{"kind":'.encode("utf-16-le") + b"\x00",
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.bom_only",
                "POST",
                "/api/studio/preview",
                body=codecs.BOM_UTF8,
                headers=[("content-type", "application/json")],
            ),
            # Non-standard constants keep CPython's position for the token
            # after them and must not shift later malformed offsets.
            case(
                "preview.nan123",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":NaN123}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.infinity1",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":Infinity1}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.neg_infinity_boundary",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":-Infinityx}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.nan_before_later_error",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":NaN,"parameters":}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.nan_utf16_offset",
                "POST",
                "/api/studio/preview",
                body=codecs.BOM_UTF16_LE + '{"kind":NaN123}'.encode("utf-16-le"),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.nan_enum",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":NaN}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.nan_number_after_value",
                "POST",
                "/api/studio/preview",
                body=b"{\"kind\":\"box\"} NaN",
                headers=[("content-type", "application/json")],
            ),
            # CPython and the native parser both keep the last duplicate key.
            case(
                "preview.duplicate_kind",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"sphere","kind":"box"}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.duplicate_parameters",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box","parameters":{"size_x":100000},"parameters":{"size_y":1}}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.nan_duplicate_parameters",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box","parameters":NaN,"parameters":{"size_x":1}}',
                headers=[("content-type", "application/json")],
            ),

            case(
                "preview.malformed_unicode_offset",
                "POST",
                "/api/studio/preview",
                body='{"é":"x",}'.encode(),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.trailing_comma",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box",}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.trailing_comma_spaced",
                "POST",
                "/api/studio/preview",
                body=b'{"a":1 , }',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.trailing_unicode",
                "POST",
                "/api/studio/preview",
                body='{"é":1} x'.encode(),
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.duplicate_keys",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"torus","kind":"box"}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.negative_infinity",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box","parameters":{"size_x":-Infinity}}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.number_top",
                "POST",
                "/api/studio/preview",
                body=b"1",
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.bool_top",
                "POST",
                "/api/studio/preview",
                body=b"true",
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.plus_json",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box"}',
                headers=[("content-type", "application/vnd.kala+json")],
            ),
            case(
                "chat.prompt_1c",
                "POST",
                "/api/studio/chat",
                body=b'{"provider":"local","prompt":"\\u001cx\\u001c"}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "chat.model_1c",
                "POST",
                "/api/studio/chat",
                body=b'{"provider":"local","prompt":"x","model":"\\u001cm\\u001c"}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "handoff.integration_1c",
                "POST",
                "/api/studio/handoff",
                body=b'{"integration_id":"\\u001cfreecad\\u001c","kind":"box","format":"stl"}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "settings.key_1c_spaces",
                "POST",
                "/api/studio/settings",
                body=b'{"provider":"openai","api_key":"\\u001c\\u001c"}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "chat.surrogate_pair",
                "POST",
                "/api/studio/chat",
                body=b'{"provider":"local","prompt":"\\ud83d\\ude00"}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "chat.nul_prompt",
                "POST",
                "/api/studio/chat",
                body=b'{"provider":"local","prompt":"a\\u0000b"}',
                headers=[("content-type", "application/json")],
            ),
            case(
                "preview.charset",
                "POST",
                "/api/studio/preview",
                body=b'{"kind":"box"}',
                headers=[("content-type", "application/json; charset=utf-8")],
            ),
        ]
    )

    # Export/handoff parity. STL is byte-exact; the STEP downloads are real
    # ISO 10303-21 solids compared structurally (see the module docstring).
    cases.extend(
        [
            case("export.stl_box", "POST", "/api/studio/export", json_body={"kind": "box", "format": "stl"}),
            case(
                "export.stl_sphere",
                "POST",
                "/api/studio/export",
                json_body={"kind": "sphere", "format": "stl", "parameters": {"rings": 12, "sectors": 16}},
            ),
            case(
                "export.step_box",
                "POST",
                "/api/studio/export",
                json_body={"kind": "box", "format": "step"},
            ),
            case(
                "export.step_cylinder",
                "POST",
                "/api/studio/export",
                json_body={"kind": "cylinder", "format": "step", "parameters": {"radius": 2.5, "height": 7.5}},
            ),
            case(
                "export.step_sphere",
                "POST",
                "/api/studio/export",
                json_body={"kind": "sphere", "format": "step", "parameters": {"radius": 3.5}},
            ),
            case(
                "export.step_invalid",
                "POST",
                "/api/studio/export",
                json_body={"kind": "box", "format": "step", "parameters": {"size_x": 0.01}},
            ),
            case(
                "handoff.stl_freecad",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "freecad", "kind": "cylinder", "format": "stl"},
            ),
            case(
                "handoff.stl_openscad",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "openscad", "kind": "box", "format": "stl"},
            ),
            case(
                "handoff.step_solidworks",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "solidworks", "kind": "box", "format": "step"},
            ),
            case(
                "handoff.step_solid_edge",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "solid_edge", "kind": "sphere", "format": "step", "parameters": {"radius": 1.5}},
            ),
            case(
                "handoff.unknown",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "nope", "kind": "box", "format": "step"},
            ),
            case(
                "handoff.unsupported",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "openscad", "kind": "box", "format": "step"},
            ),
            case(
                "handoff.bad_id",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "Bad", "kind": "box", "format": "stl"},
            ),
            case(
                "handoff.long_id",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "a" * 65, "kind": "box", "format": "stl"},
            ),
        ]
    )

    # Malformed JSON diagnostics: FastAPI reports CPython's `json.JSONDecodeError.pos`
    # as `body.<pos>`; the native scanner must reproduce every position.
    malformed: list[tuple[str, bytes]] = [
        ("tru", b"tru"),
        ("nul", b"nul"),
        ("t", b"t"),
        ("f", b"f"),
        ("nulll", b"nulll"),
        ("truex", b"truex"),
        ("array-partial-literal", b"[tru]"),
        ("object-partial-literal", b"{\"a\":tru}"),
        ("invalid-escape-q", b"{\"a\":\"\\q\"}"),
        ("invalid-escape-x", b"{\"a\":\"\\x\"}"),
        ("invalid-unicode-short", b"{\"a\":\"\\u12\"}"),
        ("invalid-unicode-hex", b"{\"a\":\"\\u12g4\"}"),
        ("invalid-unicode-empty", b"{\"a\":\"\\u\"}"),
        ("unterminated-string", b"\"abc"),
        ("unterminated-in-object", b"{\"a\":\"abc}"),
        ("unterminated-trailing-backslash", b"\"\\"),
        ("number-fraction", b"1."),
        ("number-exponent", b"1e"),
        ("number-exponent-sign", b"1e+"),
        ("number-double-minus", b"--1"),
        ("number-minus", b"-"),
        ("number-dot-five", b".5"),
        ("number-dot-exponent", b"1.e5"),
        ("number-leading-zero", b"01"),
        ("number-negative-leading-zero", b"-01"),
        ("number-hex", b"0x1"),
        ("array-trailing-comma", b"[1,]"),
        ("object-trailing-comma", b"{\"a\":1,}"),
        ("array-double-comma", b"[1,,2]"),
        ("object-double-comma", b"{\"a\":1,,}"),
        ("array-empty-comma", b"[,1]"),
        ("object-empty-comma", b"{,}"),
        ("array-brace", b"[}"),
        ("root-bracket", b"]"),
        ("root-brace", b"}"),
        ("missing-colon", b"{\"a\" 1}"),
        ("missing-comma-array", b"[1 2]"),
        ("extra-data-number", b"1.5.2"),
        ("extra-data-object", b"{\"a\":1}{"),
        ("extra-data-space", b"{\"a\":1} x"),
        ("control-newline", b"\"a\nb\""),
        ("minus-value", b"{\"a\":-}"),
        ("array-literal-x", b"[truex]"),
        ("constant-boundary", b"{\"a\":NaN123}"),
        ("infinity-boundary", b"{\"a\":Infinity1}"),
        ("negative-infinity-boundary", b"{\"a\":-Infinityx}"),
        ("constant-before-later-error", b"{\"kind\":NaN,\"parameters\":}"),
        ("utf8-truncated", b"{\"kind\":"),
        ("bom-only", b"\xef\xbb\xbf"),
    ]
    for name, body in malformed:
        cases.append(
            case(
                f"malformed.{name}",
                "POST",
                "/api/studio/preview",
                body=body,
                headers=[("content-type", "application/json")],
            )
        )

    # Chat: local, configuration failures and codex fail-closed.
    for index, prompt in enumerate(
        [
            "x",
            "A box 10x20x30 mm for FreeCAD. Show the envelope.",
            "cylinder radius 5 mm height 10 mm",
            "sphere radius 2.5 cm",
            "shaft 10 mm by 20 mm",
            "10×20×30",
            "10 cm x 20 cm x 30 cm",
            "First! Second? Third. Fourth.",
            "bøks 10 mm",
            "box 0.0000001 x 0.0000002 x 0.0000003 mm",
            "box 10000000000000000 x 2 x 3 mm",
            "é" * 4096,
        ]
    ):
        cases.append(
            case(
                f"chat.local_{index}",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": prompt},
            )
        )
    cases.extend(
        [
            case(
                "chat.local_model",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": "x", "model": "custom-model-1"},
            ),
            case(
                "chat.empty_prompt",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": ""},
            ),
            case(
                "chat.spaces_prompt",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": "   "},
            ),
            case(
                "chat.prompt_too_long",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": "x" * 4097},
            ),
            case(
                "chat.model_pattern",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": "x", "model": "bad model"},
            ),
            case(
                "chat.model_empty",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": "x", "model": ""},
            ),
            case(
                "chat.model_type",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": "x", "model": 1},
            ),
            case(
                "chat.allow_remote_yes",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": "x", "allow_remote": "yes"},
            ),
            case(
                "chat.allow_remote_bad",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": "x", "allow_remote": "nope"},
            ),
            case(
                "chat.allow_remote_float",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": "x", "allow_remote": 1.0},
            ),
            case("chat.missing_provider", "POST", "/api/studio/chat", json_body={"prompt": "x"}),
            case(
                "chat.bad_provider",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "nope", "prompt": "x"},
            ),
            case(
                "chat.openai_no_model",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "openai", "prompt": "hello", "allow_remote": True},
            ),
            case(
                "chat.openai_no_consent",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "openai", "model": "gpt-4o-mini", "prompt": "hello"},
            ),
            case(
                "chat.openai_missing_key",
                "POST",
                "/api/studio/chat",
                json_body={
                    "provider": "openai",
                    "model": "gpt-4o-mini",
                    "prompt": "hello",
                    "allow_remote": True,
                },
            ),
            case(
                "chat.openrouter_missing_key",
                "POST",
                "/api/studio/chat",
                json_body={
                    "provider": "openrouter",
                    "model": "or/model",
                    "prompt": "hello",
                    "allow_remote": True,
                },
            ),
            case(
                "chat.codex_unavailable",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "codex", "prompt": "hello", "allow_remote": True},
            ),
            case(
                "chat.codex_no_consent",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "codex", "prompt": "hello"},
            ),
        ]
    )

    # Settings validation and persistence sequence.
    cases.extend(
        [
            case("settings.read", "GET", "/api/studio/settings"),
            case("settings.bad_key", "POST", "/api/studio/settings", json_body={"provider": "openai", "api_key": "not a key"}),
            case("settings.bad_provider", "POST", "/api/studio/settings", json_body={"provider": "codex", "api_key": "x"}),
            case("settings.key_type", "POST", "/api/studio/settings", json_body={"provider": "openai", "api_key": 1}),
            case("settings.key_long", "POST", "/api/studio/settings", json_body={"provider": "openai", "api_key": "x" * 257}),
            case("settings.key_control", "POST", "/api/studio/settings", json_body={"provider": "openai", "api_key": "a\u0001b"}),
            case(
                "settings.store.openai",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openai", "api_key": "sk-corpus-synthetic-not-a-live-credential-0001"},
            ),
            case("settings.read_stored", "GET", "/api/studio/settings"),
            case("providers.configured", "GET", "/api/studio/providers"),
            case(
                "settings.store.openrouter",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openrouter", "api_key": "sk-or-corpus-synthetic-not-a-live-credential-0002"},
            ),
            case("settings.read_both", "GET", "/api/studio/settings"),
            case("settings.clear.openai", "POST", "/api/studio/settings", json_body={"provider": "openai", "api_key": None}),
            case("settings.read_cleared", "GET", "/api/studio/settings"),
            case("providers.cleared", "GET", "/api/studio/providers"),
            case("health.offline", "GET", "/api/studio/health"),
        ]
    )

    # Middleware and routing edges.
    cases.extend(
        [
            case("read.health", "GET", "/api/studio/health"),
            case("read.integrations", "GET", "/api/studio/integrations"),
            case("read.providers", "GET", "/api/studio/providers"),
            case("read.health_query", "GET", "/api/studio/health?x=1"),
            case("read.unknown", "GET", "/api/studio/not-a-route"),
            case("read.trailing_slash", "GET", "/api/studio/health/"),
            case("read.trailing_slash_query", "GET", "/api/studio/health/?x=1"),
            case(
                "read.trailing_slash_port",
                "GET",
                "/api/studio/health/",
                headers=[("Host", "127.0.0.1:8765")],
            ),
            case("method.delete_preview", "DELETE", "/api/studio/preview"),
            case("method.get_preview", "GET", "/api/studio/preview"),
            case("method.post_health", "POST", "/api/studio/health"),
            case("method.head_health", "HEAD", "/api/studio/health"),
            case("method.options_health", "OPTIONS", "/api/studio/health", headers=[("Origin", "http://127.0.0.1")]),
            case("host.evil", "GET", "/api/studio/health", headers=[("Host", "evil.example")]),
            case("host.wildcard", "GET", "/api/studio/health", headers=[("Host", "0.0.0.0")]),
            case("host.bad_port", "GET", "/api/studio/health", headers=[("Host", "127.0.0.1:70000")]),
            case("host.uppercase", "GET", "/api/studio/health", headers=[("Host", "LOCALHOST:8765")]),
            case("host.ipv6", "GET", "/api/studio/health", headers=[("Host", "[::1]:8765")]),
            case(
                "origin.same",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "http://127.0.0.1")],
            ),
            case(
                "origin.same_port",
                "GET",
                "/api/studio/health",
                headers=[("Host", "127.0.0.1:8765"), ("Origin", "http://127.0.0.1:8765")],
            ),
            case(
                "origin.ipv6",
                "GET",
                "/api/studio/health",
                headers=[("Host", "[::1]:8765"), ("Origin", "http://[::1]:8765")],
            ),
            case(
                "origin.evil",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "http://evil.example")],
            ),
            case(
                "origin.empty_port",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "http://127.0.0.1:")],
            ),
            case(
                "origin.tab_port",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "http://127.0.0.1:\t80")],
            ),
            case(
                "origin.space_port",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "http://127.0.0.1 :80")],
            ),
            case(
                "origin.bad_port",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "http://127.0.0.1:abc")],
            ),
            case(
                "origin.port_range",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "http://127.0.0.1:99999")],
            ),
            case(
                "origin.unclosed_ipv6",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "http://[::1")],
            ),
            case(
                "origin.null",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "null")],
            ),
            case(
                "origin.https",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "https://127.0.0.1")],
            ),
            case(
                "origin.userinfo",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "http://user@127.0.0.1")],
            ),
            case(
                "origin.path",
                "GET",
                "/api/studio/health",
                headers=[("Origin", "http://127.0.0.1/path")],
            ),
            case(
                "auth.missing",
                "POST",
                "/api/studio/preview",
                json_body={"kind": "box"},
                auth=False,
            ),
            case(
                "auth.wrong",
                "POST",
                "/api/studio/preview",
                json_body={"kind": "box"},
                headers=[("Authorization", "Bearer wrong-token-0123456789abcdef")],
                auth=False,
            ),
            case(
                "auth.lower_scheme",
                "POST",
                "/api/studio/preview",
                json_body={"kind": "box"},
                headers=[("Authorization", f"bearer   {TOKEN}   " )],
                auth=False,
            ),
            case(
                "auth.basic",
                "POST",
                "/api/studio/preview",
                json_body={"kind": "box"},
                headers=[("Authorization", f"Basic {TOKEN}")],
                auth=False,
            ),
            case(
                "body.declared_oversized",
                "POST",
                "/api/studio/preview",
                body=b"x" * 300_000,
                headers=[("content-type", "application/json")],
            ),
            case(
                "body.chunked_oversized",
                "POST",
                "/api/studio/preview",
                chunked=[b"x" * 150_000, b"x" * 150_000],
                headers=[("content-type", "application/json")],
            ),
            case("static.index", "GET", "/"),
            case("static.asset", "GET", "/asset.txt"),
            case("static.fallback", "GET", "/missing"),
            case("static.query", "GET", "/asset.txt?cache=1"),
            case("static.head", "HEAD", "/asset.txt"),
            case("static.api_reserved", "GET", "/api/not-a-route"),
            case("static.traversal_encoded", "GET", "/%2e%2e/%2e%2e/etc/passwd"),
            case("static.symlink_escape", "GET", "/escape"),
        ]
    )

    # A valid stored key in a settings file with legal trailing JSON
    # whitespace beyond 1 MiB: Python has no size cap, and the native reader
    # must not invent one. The main loop writes the large files just before
    # these cases so earlier store/clear sequences are unchanged.
    cases.extend(
        [
            case("settings.large_read", "GET", "/api/studio/settings"),
            case(
                "settings.large_clear",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openai", "api_key": None},
            ),
            case("settings.large_read_cleared", "GET", "/api/studio/settings"),
            # Python `read_text(encoding="utf-8")` raises UnicodeDecodeError:
            # reads are sanitized 500s and a write reports the codec message
            # as a 422 without touching the corrupt file.
            case("settings.invalid_utf8_read", "GET", "/api/studio/settings"),
            case(
                "settings.invalid_utf8_write",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openai", "api_key": None},
            ),
            case("settings.invalid_utf8_read_again", "GET", "/api/studio/settings"),
            case("settings.invalid_utf8_health", "GET", "/api/studio/health"),
            case("settings.invalid_utf8_providers", "GET", "/api/studio/providers"),
            # A stored key containing an escaped lone surrogate: Python
            # `json.loads` keeps the code point, reports configured=true and
            # writes it back as `\ud800`; a read that reaches a provider
            # header raises UnicodeEncodeError and becomes a sanitized 500.
            case("settings.surrogate_read", "GET", "/api/studio/settings"),
            case("settings.surrogate_health", "GET", "/api/studio/health"),
            case("settings.surrogate_providers", "GET", "/api/studio/providers"),
            case(
                "settings.surrogate_chat",
                "POST",
                "/api/studio/chat",
                json_body={
                    "provider": "openai",
                    "model": "gpt-4o-mini",
                    "prompt": "hello",
                    "allow_remote": True,
                },
            ),
            case(
                "settings.surrogate_write",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openrouter", "api_key": "sk-or-surrogate-0006"},
            ),
            case("settings.surrogate_read_after_write", "GET", "/api/studio/settings"),
            case(
                "settings.surrogate_clear",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openai", "api_key": None},
            ),
            case("settings.surrogate_read_cleared", "GET", "/api/studio/settings"),
        ]
    )
    # Duplicate keys plus escaped surrogates: Python keeps the last duplicate
    # `api_keys`/provider value, accepts the lone surrogate escapes and writes
    # them back; an unrelated save/clear must preserve the surviving key. The
    # final settings bytes are compared between both stores at the end.
    cases.extend(
        [
            case("settings.dup_surrogate_read", "GET", "/api/studio/settings"),
            case("settings.dup_surrogate_health", "GET", "/api/studio/health"),
            case("settings.dup_surrogate_providers", "GET", "/api/studio/providers"),
            case(
                "settings.dup_surrogate_write",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openrouter", "api_key": "sk-or-diff-duplicate-0012"},
            ),
            case(
                "settings.dup_surrogate_read_after_write",
                "GET",
                "/api/studio/settings",
            ),
            case(
                "settings.dup_surrogate_clear",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openai", "api_key": None},
            ),
            case(
                "settings.dup_surrogate_read_cleared",
                "GET",
                "/api/studio/settings",
            ),
            # The same Python-accepted domain extended with a deep unrelated
            # value and the non-finite constants; the surviving signature key
            # is a lone surrogate that the source keeps and re-emits.
            case("settings.mixed_valid_read", "GET", "/api/studio/settings"),
            case(
                "settings.mixed_valid_write",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openai", "api_key": "sk-diff-mixed-new-0018"},
            ),
            case("settings.mixed_valid_read_after_write", "GET", "/api/studio/settings"),
            case(
                "settings.mixed_valid_clear",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openrouter", "api_key": None},
            ),
            case("settings.mixed_valid_read_cleared", "GET", "/api/studio/settings"),
            # A valid key plus an unrelated integer past CPython's 4,300-digit
            # conversion limit: `json.loads` raises ValueError, which the
            # store does not catch. Reads are sanitized 500s; the write is a
            # 422 with the exact message and leaves the file byte-identical.
            case("settings.integer_limit_read", "GET", "/api/studio/settings"),
            case("settings.integer_limit_health", "GET", "/api/studio/health"),
            case("settings.integer_limit_providers", "GET", "/api/studio/providers"),
            case(
                "settings.integer_limit_write",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openrouter", "api_key": "sk-or-diff-refused-0014"},
            ),
            case(
                "settings.integer_limit_read_again",
                "GET",
                "/api/studio/settings",
            ),
            # Exactly 4,300 digits is accepted by CPython and the key is read.
            case("settings.at_limit_read", "GET", "/api/studio/settings"),
            case(
                "settings.at_limit_clear",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openai", "api_key": None},
            ),
            case("settings.at_limit_read_cleared", "GET", "/api/studio/settings"),
        ]
    )
    return cases


LARGE_SETTINGS_KEY = "sk-diff-large-synthetic-not-a-live-credential-0005"


def write_large_settings(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    payload = json.dumps({"api_keys": {"openai": LARGE_SETTINGS_KEY}})
    (directory / "studio-settings.json").write_text(payload + " " * (1024 * 1024 + 4096))


def write_invalid_utf8_settings(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "studio-settings.json").write_bytes(
        b'{"api_keys":{"openai":"sk-bad-utf8"}}\xff'
    )


SURROGATE_SETTINGS_FILE = (
    b'{"api_keys":{"openai":"sk-\\ud800","openrouter":"sk-or-existing"}}'
)


def write_surrogate_settings(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "studio-settings.json").write_bytes(SURROGATE_SETTINGS_FILE)


# REVIEW-3-shaped file with duplicate `api_keys` members at the root, a
# duplicate provider key, an ignored surrogate field and an actual surrogate
# in the surviving provider value. Python keeps the last member/value.
DUPLICATE_SURROGATE_SETTINGS_FILE = (
    b'{"api_keys":{},"api_keys":{"openai":"first",'
    b'"openai":"synthetic-review-key","openrouter":"sk-\\ud800"},'
    b'"unused":"\\ud800"}'
)


def write_duplicate_surrogate_settings(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "studio-settings.json").write_bytes(
        DUPLICATE_SURROGATE_SETTINGS_FILE
    )


INTEGER_LIMIT_DIGITS = "7" * 4301
AT_LIMIT_DIGITS = "7" * 4300


def write_integer_limit_settings(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    payload = (
        '{"api_keys":{"openai":"sk-diff-integer-synthetic-0013"},"unused":'
        + INTEGER_LIMIT_DIGITS
        + "}"
    )
    (directory / "studio-settings.json").write_text(payload)


def write_at_limit_settings(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    payload = (
        '{"api_keys":{"openai":"sk-diff-at-limit-synthetic-0015"},"unused":'
        + AT_LIMIT_DIGITS
        + "}"
    )
    (directory / "studio-settings.json").write_text(payload)


# Duplicates, a surviving escaped surrogate key, a deep value and CPython's
# non-finite constants in one Python-accepted document.
MIXED_VALID_SETTINGS_FILE = (
    '{"api_keys":{},"api_keys":{"openai":"sk-diff-mixed-synthetic-0016",'
    '"openai":"sk-\\ud800","openrouter":"sk-or-mixed-synthetic-0017"},'
    '"other":' + "[" * 400 + "]" * 400 + ',"nonfinite":NaN,"huge":1e999}'
)


def write_mixed_valid_settings(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "studio-settings.json").write_text(MIXED_VALID_SETTINGS_FILE)


# Cases whose response bodies are STEP downloads: OpenCascade embeds a
# timestamp and its own entity numbering, so the contract is structural.
STEP_STRUCTURAL = {
    "export.step_box",
    "export.step_cylinder",
    "export.step_sphere",
    "handoff.step_solidworks",
    "handoff.step_solid_edge",
}


def step_structure(body: bytes) -> dict[str, object]:
    return {
        "iso": body.startswith(b"ISO-10303-21;"),
        "trailer": body.endswith(b"END-ISO-10303-21;\n"),
        "breps": body.count(b"MANIFOLD_SOLID_BREP"),
        "bytes": len(body),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--binary",
        default=str(REPO / "backend/target/release/kala-studio"),
    )
    arguments = parser.parse_args()
    binary = Path(arguments.binary)
    if not binary.is_file():
        print(f"native binary not found: {binary}", file=sys.stderr)
        return 2

    from fastapi.testclient import TestClient
    from kala_agent.studio.geometry.step import probe_step_capability
    from kala_agent.studio.server import create_studio_app

    if not probe_step_capability().available:
        print(
            "the live differential requires the OCP bindings in the oracle virtualenv "
            "(both Python and the native service must report step_available=true)",
            file=sys.stderr,
        )
        return 2

    work = Path(tempfile.mkdtemp(prefix="kala-studio-diff-"))
    native_static = work / "native-static"
    python_static = work / "python-static"
    for root in (native_static, python_static):
        root.mkdir()
        (root / "index.html").write_text("<!doctype html><html><body>diff</body></html>")
        (root / "asset.txt").write_text("asset\n")
    outside = work / "outside.txt"
    outside.write_text("outside")
    os.symlink(outside, native_static / "escape")
    os.symlink(outside, python_static / "escape")

    port = free_port()
    environment = {
        "KALA_STUDIO_TOKEN": TOKEN,
        "PATH": "",
    }
    process = subprocess.Popen(
        [
            str(binary),
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--data-dir",
            str(work / "native-data"),
            "--static-dist",
            str(native_static),
        ],
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    deadline = time.time() + 15
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.25):
                break
        except OSError:
            if process.poll() is not None:
                print(process.stderr.read(), file=sys.stderr)
                return 2
            time.sleep(0.02)
    else:
        print("native server did not become ready", file=sys.stderr)
        return 2

    # The geometry worker has to be ready before the differential runs: the
    # front door answers 503 on the proxied routes while it is starting.
    ready_deadline = time.time() + 90
    while time.time() < ready_deadline:
        try:
            status, _headers, body = native_request(
                port, "GET", "/api/studio/health", [], None, None
            )
        except OSError:
            status, body = 0, b""
        if status == 200:
            try:
                document = json.loads(body)
            except ValueError:
                document = {}
            if document.get("geometry_worker", {}).get("state") == "ready":
                break
        time.sleep(0.2)

    python_app = create_studio_app(
        data_dir=work / "python-data",
        api_token=TOKEN,
        max_body_bytes=262_144,
        static_root=python_static,
        environ={},
    )
    cases = build_cases()
    mismatches: list[str] = []
    checked = 0
    with TestClient(
        python_app,
        base_url="http://127.0.0.1",
        raise_server_exceptions=False,
        follow_redirects=False,
    ) as python_client:
        for item in cases:
            if item["name"] in {"settings.large_read", "settings.large_clear"}:
                write_large_settings(work / "native-data")
                write_large_settings(work / "python-data")
            if item["name"].startswith("settings.invalid_utf8_"):
                write_invalid_utf8_settings(work / "native-data")
                write_invalid_utf8_settings(work / "python-data")
            if item["name"] in {
                "settings.surrogate_read",
                "settings.surrogate_health",
                "settings.surrogate_providers",
                "settings.surrogate_chat",
            }:
                write_surrogate_settings(work / "native-data")
                write_surrogate_settings(work / "python-data")
            if item["name"] in {
                "settings.dup_surrogate_read",
                "settings.dup_surrogate_health",
                "settings.dup_surrogate_providers",
            }:
                write_duplicate_surrogate_settings(work / "native-data")
                write_duplicate_surrogate_settings(work / "python-data")
            if item["name"] in {
                "settings.integer_limit_read",
                "settings.integer_limit_health",
                "settings.integer_limit_providers",
            }:
                write_integer_limit_settings(work / "native-data")
                write_integer_limit_settings(work / "python-data")
            if item["name"] == "settings.at_limit_read":
                write_at_limit_settings(work / "native-data")
                write_at_limit_settings(work / "python-data")
            if item["name"] == "settings.mixed_valid_read":
                write_mixed_valid_settings(work / "native-data")
                write_mixed_valid_settings(work / "python-data")
            request_headers = item["headers"]
            kwargs: dict[str, Any] = {"headers": request_headers}
            if item["chunked"] is not None:
                kwargs["content"] = item["chunked"]
            elif item["body"] is not None:
                kwargs["content"] = item["body"]
            python_response = python_client.request(
                item["method"], item["path"], **kwargs
            )
            native_status, native_headers, native_body = native_request(
                port,
                item["method"],
                item["path"],
                request_headers,
                item["body"],
                item["chunked"],
            )
            python_headers = {
                key.lower(): value for key, value in python_response.headers.items()
            }
            python_status = python_response.status_code
            python_body = python_response.content
            checked += 1
            if item["path"].split("?")[0] == "/api/studio/health" and _is_json_object(
                python_body
            ) and _is_json_object(native_body):
                # The Rust front door publishes the ADR-0010 worker state,
                # which the Python public server has no field for.
                python_document = json.loads(python_body)
                native_document = json.loads(native_body)
                native_document.pop("geometry_worker", None)
                if python_document != native_document:
                    mismatches.append(
                        f"{item['name']}: health documents differ\n"
                        f"python={json.dumps(python_document, sort_keys=True)}\n"
                        f"native={json.dumps(native_document, sort_keys=True)}"
                    )
                continue
            if item["name"] in STEP_STRUCTURAL:
                # Real ISO 10303-21 solids with stable headers on both sides;
                # the OCP timestamps/entity numbering are not a contract.
                native_structure = step_structure(native_body)
                python_structure = step_structure(python_body)
                if (
                    python_status != native_status
                    or python_status != 200
                    or not native_structure["iso"]
                    or not native_structure["trailer"]
                    or native_structure["breps"] != 1
                    or python_structure["breps"] != 1
                    or native_headers.get("content-type") != "application/step"
                    or python_headers.get("content-type") != "application/step"
                    or native_headers.get("content-disposition")
                    != python_headers.get("content-disposition")
                    or native_headers.get("content-length") != str(native_structure["bytes"])
                    or any(
                        python_headers.get(name) != native_headers.get(name)
                        for name in SECURITY
                    )
                    or any(native_headers.get(name) is not None for name in ABSENT)
                ):
                    mismatches.append(
                        f"{item['name']}: python={python_status} {python_structure} "
                        f"native={native_status} {native_structure} "
                        f"{ {name: native_headers.get(name) for name in STABLE} }"
                    )
                continue
            if (
                python_status != native_status
                or python_body != native_body
                or any(python_headers.get(name) != native_headers.get(name) for name in STABLE)
                or any(python_headers.get(name) != native_headers.get(name) for name in SECURITY)
                or any(native_headers.get(name) is not None for name in ABSENT)
            ):
                mismatches.append(
                    f"{item['name']}: python={python_status} {python_body[:220]!r} "
                    f"{ {name: python_headers.get(name) for name in STABLE} } "
                    f"native={native_status} {native_body[:220]!r} "
                    f"{ {name: native_headers.get(name) for name in STABLE} }"
                )

    python_settings = work / "python-data" / "studio-settings.json"
    native_settings = work / "native-data" / "studio-settings.json"
    if python_settings.read_bytes() != native_settings.read_bytes():
        mismatches.append("settings file bytes differ from the Python store")
    process.terminate()
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
    if process.returncode != 0:
        mismatches.append(f"native process exit code {process.returncode}")
    print(f"differential cases: {checked}")
    print(f"mismatches: {len(mismatches)}")
    for mismatch in mismatches:
        print(mismatch)
    return 0 if not mismatches else 1


if __name__ == "__main__":
    raise SystemExit(main())
