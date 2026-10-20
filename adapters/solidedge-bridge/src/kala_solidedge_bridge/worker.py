"""The spawned worker process: one owner of the STA and any future COM executor.

The worker reads newline-delimited JSON commands from stdin and writes
newline-delimited JSON messages to stdout. It never handles HTTP and never
receives a client path. There is no production native build executor in this
milestone: a build is acknowledged and then fails honestly with the
``unavailable`` code, even when Solid Edge is attached. Only the separate
``probe`` command attempts measured native geometry.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from .session import Session, bounded_message, hresult_of
from .workspace import default_data_dir


def _is_windows() -> bool:
    return str(sys.platform) == "win32"


class Worker:
    def __init__(self, data_dir: Path, *, session_factory: type[Session] = Session) -> None:
        self.data_dir = data_dir
        self._session_factory = session_factory
        self._session: Session | None = None
        self._session_error: str | None = None
        self._cancel_requested = False

    def _ensure_session(self) -> Session | None:
        if self._session is not None:
            return self._session
        if self._session_error is not None:
            return None
        session = self._session_factory()
        try:
            session.start()
        except Exception as exc:
            self._session_error = bounded_message(exc)
            return None
        self._session = session
        return session

    def health(self) -> dict[str, Any]:
        if not _is_windows():
            return {
                "type": "health",
                "platform": sys.platform,
                "supported": False,
                "available": False,
                "attached": False,
                "se_version": None,
                "detail": (
                    "non-Windows host: Solid Edge COM is unavailable; this worker is "
                    "only a development and test process"
                ),
            }
        session = self._ensure_session()
        if session is None:
            return {
                "type": "health",
                "platform": sys.platform,
                "supported": False,
                "available": False,
                "attached": False,
                "se_version": None,
                "detail": self._session_error or "the COM apartment could not start",
            }

        def attach_and_read(binding: Any) -> tuple[Any, str]:  # noqa: ANN401
            application = binding.attach(False)
            return application, binding.version(application)

        try:
            application, version = session.run(attach_and_read, timeout=15.0)
        except Exception as exc:
            return {
                "type": "health",
                "platform": sys.platform,
                "supported": False,
                "available": False,
                "attached": False,
                "se_version": None,
                "detail": (
                    "Solid Edge is not attached in this interactive desktop session: "
                    f"{bounded_message(exc)} (hresult={hresult_of(exc)})"
                ),
            }
        del application
        return {
            "type": "health",
            "platform": sys.platform,
            "supported": False,
            "available": True,
            "attached": True,
            "se_version": version,
            "detail": (
                "Solid Edge is attached; no live-verified production native build executor "
                "exists, so builds remain unavailable until one is implemented and verified"
            ),
        }

    def build(self, message: dict[str, Any]) -> None:
        build_id = str(message.get("build_id", ""))
        self._send({"type": "build_started", "build_id": build_id, "accepted": True})
        self._send(
            {
                "type": "event",
                "event": {
                    "kind": "failed",
                    "error": {
                        "code": "unavailable",
                        "message": (
                            "this bridge has no live-verified production native build "
                            "executor; no Solid Edge features were created. Run the probe "
                            "for host diagnostics."
                        ),
                    },
                    "message": "production native executor unavailable",
                },
            }
        )

    @staticmethod
    def _send(payload: dict[str, Any]) -> None:
        sys.stdout.write(json.dumps(payload, ensure_ascii=True, separators=(",", ":")) + "\n")
        sys.stdout.flush()

    def run(self) -> int:
        try:
            for line in sys.stdin:
                line = line.strip()
                if not line:
                    continue
                try:
                    message = json.loads(line)
                except json.JSONDecodeError:
                    self._send({"type": "error", "message": "malformed JSON command"})
                    continue
                if not isinstance(message, dict):
                    self._send({"type": "error", "message": "command must be a JSON object"})
                    continue
                command = message.get("cmd")
                if command == "health":
                    self._send(self.health())
                elif command == "build":
                    self._cancel_requested = False
                    self.build(message)
                elif command == "cancel":
                    self._cancel_requested = True
                    self._send({"type": "cancel_ack", "accepted": True})
                elif command == "shutdown":
                    self._send({"type": "shutdown_ack"})
                    break
                else:
                    self._send({"type": "error", "message": "unknown command"})
        finally:
            if self._session is not None:
                self._session.stop()
        return 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="kala-se-bridge-worker",
        description="Internal worker process for the Kala Solid Edge bridge.",
    )
    parser.add_argument("--data-dir", type=Path, default=None)
    arguments = parser.parse_args(argv)
    data_dir = arguments.data_dir if arguments.data_dir is not None else default_data_dir()
    return Worker(data_dir).run()


if __name__ == "__main__":  # pragma: no cover - module entry point
    raise SystemExit(main())


__all__ = ("Worker", "main")
