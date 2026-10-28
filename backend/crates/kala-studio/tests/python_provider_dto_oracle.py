#!/usr/bin/env python3
"""Monkeypatched Python provider oracle for the chat response DTO.

This is a test oracle, not product code. It drives the live FastAPI app with
a patched urllib opener (no network, no credential) and records the source's
actual status/body contract for provider replies that interact with
`ChatResponse` validation and `json.loads(bytes)` encoding detection. The
Rust `tests/chat_and_remote.rs` cases assert the same contract through the
native transport mock.

Run with the repository venv:
    .venv/bin/python tests/python_provider_dto_oracle.py
"""

from __future__ import annotations

import codecs
import json
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
SRC = REPO / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

TOKEN = "provider-dto-oracle-token-0123456789abcdef"
INTERNAL = (
    b'{"code":"internal_error","message":"the studio request could not be '
    b'completed","details":[]}'
)
PROTOCOL_JSON = (
    502,
    b'{"code":"provider_protocol_error","details":["the provider response '
    b'was not valid JSON"],"message":"the provider returned an unparseable '
    b'payload"}',
)


def utf16(text: str) -> bytes:
    return codecs.BOM_UTF16_LE + text.encode("utf-16-le")


PAYLOAD = '{"model":"served","choices":[{"message":{"content":"hi"}}]}'

CASES: list[tuple[str, bytes, int, bytes | None]] = [
    ("utf8", PAYLOAD.encode(), 200, None),
    ("utf8bom", codecs.BOM_UTF8 + PAYLOAD.encode(), 200, None),
    ("utf16le_bom", utf16(PAYLOAD), 200, None),
    (
        "utf32be_bom",
        codecs.BOM_UTF32_BE + PAYLOAD.encode("utf-32-be"),
        200,
        None,
    ),
    ("invalid_utf8", b"\xff", 500, INTERNAL),
    ("truncated_utf16", codecs.BOM_UTF16_LE + b"not json\x00", 500, INTERNAL),
    ("malformed_json", b"{nope", 502, PROTOCOL_JSON[1]),
    (
        "overlong_model",
        json.dumps(
            {"model": "m" * 257, "choices": [{"message": {"content": "hi"}}]}
        ).encode(),
        500,
        INTERNAL,
    ),
    (
        "model_256",
        json.dumps(
            {"model": "m" * 256, "choices": [{"message": {"content": "hi"}}]}
        ).encode(),
        200,
        None,
    ),
    (
        "nan_model_falls_back",
        b'{"model":NaN,"choices":[{"message":{"content":"hi"}}]}',
        200,
        b'{"text":"hi","provider":"openai","model":"gpt-4o-mini","offline":false,"design":null,"usage":null}',
    ),
    (
        "nan_unrelated",
        b'{"model":"served","choices":[{"message":{"content":"hi"}}],"usage":NaN}',
        200,
        None,
    ),
    (
        "nan_content",
        b'{"model":"served","choices":[{"message":{"content":NaN}}]}',
        502,
        None,
    ),
    (
        "huge_integer",
        b'{"model":"served","choices":[{"message":{"content":"hi"}}],"n":'
        + b"9" * 4301
        + b"}",
        500,
        INTERNAL,
    ),
    (
        "content_4097",
        json.dumps(
            {"model": "m", "choices": [{"message": {"content": "x" * 4097}}]}
        ).encode(),
        200,
        None,
    ),
    (
        "surrogate_content",
        b'{"model":"m","choices":[{"message":{"content":"\\ud800"}}]}',
        500,
        INTERNAL,
    ),
    (
        "surrogate_model",
        b'{"model":"m\\ud800","choices":[{"message":{"content":"hi"}}]}',
        500,
        INTERNAL,
    ),
    (
        "surrogate_unused",
        b'{"model":"m","choices":[{"message":{"content":"hi"}}],"extra":"\\ud800"}',
        200,
        None,
    ),
    (
        "valid_pair",
        b'{"model":"m","choices":[{"message":{"content":"\\ud83d\\ude00"}}]}',
        200,
        None,
    ),
    ("empty_choices", b'{"choices":[],"model":"m"}', 502, None),
    (
        "duplicate_model",
        b'{"model":"first","model":"second","choices":[{"message":{"content":"hi"}}]}',
        200,
        b'{"text":"hi","provider":"openai","model":"second","offline":false,"design":null,"usage":null}',
    ),
]


class FakeResponse:
    def __init__(self, data: bytes) -> None:
        self.data = data
        self.headers: dict[str, str] = {}

    def read(self, size: int = -1) -> bytes:
        data, self.data = self.data, b""
        return data

    def __enter__(self) -> FakeResponse:
        return self

    def __exit__(self, *_args: object) -> None:
        return None


def main() -> int:
    from fastapi.testclient import TestClient

    import kala_agent.studio.providers.openai_compat as compat
    from kala_agent.studio.server import create_studio_app

    work = Path(tempfile.mkdtemp(prefix="kala-studio-dto-oracle-"))
    app = create_studio_app(
        data_dir=work / "data",
        api_token=TOKEN,
        max_body_bytes=262_144,
        static_root=None,
        environ={"OPENAI_API_KEY": "sk-synthetic-not-a-live-credential"},
    )
    failures = 0
    with TestClient(
        app,
        base_url="http://127.0.0.1",
        raise_server_exceptions=False,
        follow_redirects=False,
    ) as client:
        for name, raw, expected_status, expected_body in CASES:
            compat.urllib.request.build_opener = lambda *a, **k: type(
                "Opener", (), {"open": lambda self, *a, **k: FakeResponse(raw)}
            )()
            response = client.post(
                "/api/studio/chat",
                headers={
                    "Authorization": f"Bearer {TOKEN}",
                    "Content-Type": "application/json",
                },
                json={
                    "provider": "openai",
                    "model": "gpt-4o-mini",
                    "prompt": "hello",
                    "allow_remote": True,
                },
            )
            ok = response.status_code == expected_status
            if expected_body is not None:
                ok = ok and response.content == expected_body
            if not ok:
                failures += 1
                print(
                    f"FAIL {name}: expected {expected_status} "
                    f"{expected_body!r}, got {response.status_code} "
                    f"{response.content[:160]!r}"
                )
    print(f"python provider DTO oracle cases: {len(CASES)}; failures: {failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
