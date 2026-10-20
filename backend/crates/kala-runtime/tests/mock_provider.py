#!/usr/bin/env python3
"""A deterministic mock OpenAI-compatible provider.

The script is test-only. It serves scripted answers for a set of paths and
records every request it received so the native Rust client and the Python
client can be compared on the wire.

Usage::

    python mock_provider.py

The chosen port is printed on stdout as the first line. The server then runs
until it is terminated.

Control endpoints:

* ``POST /__reset`` clears the recorded requests.
* ``GET /__log`` returns the recorded requests as canonical JSON.

The recorded headers are a fixed subset (``content-type``, ``content-length``
and the presence of ``authorization``). The credential itself is never
recorded: the ``authorization`` value is replaced with ``<redacted>`` before
the entry is stored.
"""

from __future__ import annotations

import json
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

RECORDS: list[dict[str, Any]] = []
LOCK = threading.Lock()

SUCCESS_BODY = json.dumps(
    {
        "choices": [
            {
                "finish_reason": "stop",
                "message": {"content": '{"ok": true}', "role": "assistant"},
            }
        ],
        "id": "provider-request-1",
        "model": "local-model",
        "usage": {"completion_tokens": 4, "prompt_tokens": 9},
    },
    sort_keys=True,
    separators=(",", ":"),
).encode("utf-8")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args: Any) -> None:  # noqa: D102 - the server stays quiet
        return

    def _respond(self, status: int, body: bytes, content_type: str = "application/json") -> None:
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802 - the http.server API
        if self.path == "/__log":
            with LOCK:
                payload = json.dumps(RECORDS, sort_keys=True, separators=(",", ":")).encode("utf-8")
            self._respond(200, payload)
            return
        self._respond(404, b"{}")

    def do_POST(self) -> None:  # noqa: N802 - the http.server API
        length = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(length)
        if self.path == "/__reset":
            with LOCK:
                RECORDS.clear()
            self._respond(200, b"{}")
            return
        record = {
            "body": body.decode("utf-8", "replace"),
            "headers": {
                "authorization": (
                    "<redacted>" if self.headers.get("authorization") is not None else None
                ),
                "content-length": self.headers.get("content-length"),
                "content-type": self.headers.get("content-type"),
            },
            "method": "POST",
            "path": self.path,
        }
        with LOCK:
            RECORDS.append(record)
        self._answer(self.path)

    def _answer(self, path: str) -> None:
        if path.endswith("/success"):
            self._respond(200, SUCCESS_BODY)
        elif path.endswith("/malformed"):
            self._respond(200, b"{not json")
        elif path.endswith("/oversize"):
            padding = b"x" * 4096
            self._respond(200, b'{"choices":[],"padding":"' + padding + b'"}')
        elif path.endswith("/slow"):
            time.sleep(2.0)
            self._respond(200, SUCCESS_BODY)
        elif path.endswith("/status503"):
            self._respond(503, b'{"error":"unavailable"}')
        elif path.endswith("/redirect"):
            self.send_response(302)
            self.send_header("location", "/case/success")
            self.send_header("content-length", "0")
            self.end_headers()
        elif path.endswith("/nochoices"):
            self._respond(200, b'{"choices":[]}')
        elif path.endswith("/violation"):
            body = json.dumps(
                {
                    "choices": [
                        {
                            "finish_reason": "stop",
                            "message": {"content": '{"unexpected": 1}'},
                        }
                    ]
                },
                sort_keys=True,
                separators=(",", ":"),
            ).encode("utf-8")
            self._respond(200, body)
        elif path.endswith("/large"):
            body = json.dumps(
                {
                    "choices": [
                        {
                            "finish_reason": "stop",
                            "message": {"content": "x" * 100_000},
                        }
                    ],
                    "id": "provider-request-large",
                    "model": "local-model",
                    "usage": {"completion_tokens": 4, "prompt_tokens": 9},
                },
                sort_keys=True,
                separators=(",", ":"),
            ).encode("utf-8")
            self._respond(200, body)
        elif path.endswith("/empty"):
            self._respond(200, b'{"choices":[{"finish_reason":"stop","message":{"content":"  "}}]}')
        else:
            self._respond(404, b"{}")


def main() -> int:
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    port = server.server_address[1]
    sys.stdout.write(f"{port}\n")
    sys.stdout.flush()
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        pass
    finally:
        server.shutdown()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
