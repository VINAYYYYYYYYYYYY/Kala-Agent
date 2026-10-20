"""Subprocess tests for the worker readiness line, signals, and lifeline.

These tests start the real ``python -m kala_agent.studio --worker`` process,
parse its single readiness line, and call the real loopback socket. They are
the acceptance gate for the ADR-0010 process contract.
"""

from __future__ import annotations

import http.client
import json
import os
import signal
import subprocess
import sys
import threading
import time
from collections.abc import Iterator, Mapping
from contextlib import contextmanager
from pathlib import Path
from typing import IO, Any, cast

import pytest

READY_PREFIX = b"KALA_WORKER_READY "
ADR_VOLUME_MM3 = 35_571.416047
LAUNCH_TOKEN = "worker-subprocess-token-0123456789"
READY_TIMEOUT_SECONDS = 30.0
HTTP_TIMEOUT_SECONDS = 60.0
SHUTDOWN_TIMEOUT_SECONDS = 20.0


def _worker_environ(
    data_dir: Path,
    *,
    token: str | None,
    lifeline: bool = False,
    home: Path | None = None,
) -> dict[str, str]:
    env = {
        key: value
        for key, value in os.environ.items()
        if key
        not in {
            "KALA_WORKER_TOKEN",
            "KALA_WORKER_STDIN_LIFELINE",
            "KALA_STUDIO_TOKEN",
            "KALA_STUDIO_DATA_DIR",
        }
    }
    if token is not None:
        env["KALA_WORKER_TOKEN"] = token
    env["KALA_STUDIO_DATA_DIR"] = str(data_dir)
    env["PYTHONUNBUFFERED"] = "1"
    if lifeline:
        env["KALA_WORKER_STDIN_LIFELINE"] = "1"
    if home is not None:
        env["HOME"] = str(home)
    return env


def _spawn(
    data_dir: Path,
    *,
    token: str | None,
    lifeline: bool,
    home: Path | None = None,
) -> subprocess.Popen[bytes]:
    return subprocess.Popen(
        [sys.executable, "-m", "kala_agent.studio", "--worker"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=_worker_environ(data_dir, token=token, lifeline=lifeline, home=home),
    )


def _readline_with_timeout(stream: IO[bytes], timeout: float) -> bytes:
    collected: list[bytes] = []

    def read() -> None:
        collected.append(stream.readline())

    thread = threading.Thread(target=read, daemon=True)
    thread.start()
    thread.join(timeout)
    if not collected:
        raise AssertionError("the worker printed no readiness line before the timeout")
    return collected[0]


def _parse_ready(line: bytes) -> dict[str, Any]:
    assert line.startswith(READY_PREFIX), line
    assert line.count(b"\n") == 1, line
    return cast("dict[str, Any]", json.loads(line[len(READY_PREFIX) :]))


def _request(
    port: int,
    method: str,
    path: str,
    *,
    token: str | None = None,
    body: Mapping[str, object] | None = None,
    raw: bytes | None = None,
    headers: Mapping[str, str] | None = None,
) -> tuple[int, dict[str, str], bytes]:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=HTTP_TIMEOUT_SECONDS)
    request_headers = dict(headers or {})
    if token is not None:
        request_headers["Authorization"] = f"Bearer {token}"
    payload: bytes | None = raw
    if body is not None:
        payload = json.dumps(body).encode("utf-8")
        request_headers["Content-Type"] = "application/json"
    try:
        connection.request(method, path, body=payload, headers=request_headers)
        response = connection.getresponse()
        data = response.read()
        return response.status, {key.lower(): value for key, value in response.getheaders()}, data
    finally:
        connection.close()


def _terminate(
    process: subprocess.Popen[bytes],
    *,
    sig: int = signal.SIGTERM,
) -> tuple[int, bytes, bytes]:
    if process.poll() is None:
        process.send_signal(sig)
    out, err = process.communicate(timeout=SHUTDOWN_TIMEOUT_SECONDS)
    return int(process.returncode or 0), out, err


@contextmanager
def _running_worker(
    data_dir: Path,
    *,
    token: str,
    lifeline: bool = False,
    home: Path | None = None,
) -> Iterator[tuple[subprocess.Popen[bytes], dict[str, Any], bytes]]:
    process = _spawn(data_dir, token=token, lifeline=lifeline, home=home)
    try:
        assert process.stdout is not None
        line = _readline_with_timeout(process.stdout, READY_TIMEOUT_SECONDS)
        yield process, _parse_ready(line), line
    finally:
        if process.poll() is None:
            process.kill()
        process.wait(timeout=SHUTDOWN_TIMEOUT_SECONDS)
        if process.stdout is not None and not process.stdout.closed:
            process.stdout.read()
        if process.stderr is not None and not process.stderr.closed:
            process.stderr.read()
        if process.stdin is not None and not process.stdin.closed:
            process.stdin.close()


# --- readiness ---


def test_readiness_line_and_design_call(tmp_path: Path) -> None:
    started = time.monotonic()
    with _running_worker(tmp_path, token=LAUNCH_TOKEN, lifeline=True) as (process, ready, line):
        assert time.monotonic() - started < READY_TIMEOUT_SECONDS
        assert set(ready) == {"port", "version", "design_available"}
        assert isinstance(ready["port"], int) and ready["port"] > 0
        assert ready["version"] == "0.1.0"
        assert ready["design_available"] is True
        assert LAUNCH_TOKEN.encode() not in line

        status, _, body = _request(
            ready["port"],
            "POST",
            "/internal/v1/design",
            token=LAUNCH_TOKEN,
            body={
                "template": {
                    "template": "l_bracket",
                    "width": 60,
                    "depth": 45,
                    "height": 40,
                    "wall": 8,
                    "bore_d": 10,
                    "hole_d": 5.5,
                    "hole_count": 4,
                },
                "material": "aluminium",
                "provider": "opencascade",
            },
        )
        assert status == 200, body
        design = cast("dict[str, Any]", json.loads(body))
        volume = design["measured"]["volume_mm3"]
        assert abs(volume - ADR_VOLUME_MM3) / ADR_VOLUME_MM3 < 1e-6
        assert design["verified"] is True

        status, _, step = _request(
            ready["port"],
            "POST",
            "/internal/v1/export",
            token=LAUNCH_TOKEN,
            body={"design_id": design["design_id"], "format": "step"},
        )
        assert status == 200, step[:256]
        assert step.startswith(b"ISO-10303-21")

        status, _, body = _request(
            ready["port"],
            "POST",
            "/internal/v1/import",
            token=LAUNCH_TOKEN,
            raw=step,
            headers={"Content-Type": "application/step"},
        )
        assert status == 200, body
        imported = cast("dict[str, Any]", json.loads(body))
        assert imported["solids"] == 1
        bounds = imported["view"]["bounds"]
        assert bounds["min"] == [pytest.approx(-30.0), pytest.approx(0.0), pytest.approx(0.0)]
        assert bounds["max"] == [pytest.approx(30.0), pytest.approx(45.0), pytest.approx(40.0)]

        returncode, out, err = _terminate(process)

    assert returncode == 0
    assert out == b""
    assert b"Traceback" not in err


def test_worker_refuses_a_missing_or_short_token(tmp_path: Path) -> None:
    for token in (None, "a" * 31):
        result = subprocess.run(
            [sys.executable, "-m", "kala_agent.studio", "--worker"],
            input=b"",
            capture_output=True,
            env=_worker_environ(tmp_path, token=token),
            timeout=SHUTDOWN_TIMEOUT_SECONDS,
        )
        assert result.returncode != 0, token
        assert result.stdout == b""
        assert b"KALA_WORKER_TOKEN" in result.stderr


def test_worker_refuses_a_token_with_unsupported_characters(tmp_path: Path) -> None:
    result = subprocess.run(
        [sys.executable, "-m", "kala_agent.studio", "--worker"],
        input=b"",
        capture_output=True,
        env=_worker_environ(tmp_path, token="a" * 31 + "!"),
        timeout=SHUTDOWN_TIMEOUT_SECONDS,
    )
    assert result.returncode != 0
    assert result.stdout == b""
    assert b"KALA_WORKER_TOKEN" in result.stderr


def test_worker_refuses_startup_without_an_explicit_data_dir(tmp_path: Path) -> None:
    """The worker never falls back to the home directory or a shared temp dir."""
    home = tmp_path / "home"
    home.mkdir()
    base = {
        key: value
        for key, value in os.environ.items()
        if key
        not in {
            "KALA_WORKER_TOKEN",
            "KALA_WORKER_STDIN_LIFELINE",
            "KALA_STUDIO_TOKEN",
            "KALA_STUDIO_DATA_DIR",
        }
    }
    base["KALA_WORKER_TOKEN"] = LAUNCH_TOKEN
    base["HOME"] = str(home)
    base["PYTHONUNBUFFERED"] = "1"
    for blank in (None, "", "   "):
        environment = dict(base)
        if blank is not None:
            environment["KALA_STUDIO_DATA_DIR"] = blank
        result = subprocess.run(
            [sys.executable, "-m", "kala_agent.studio", "--worker"],
            input=b"",
            capture_output=True,
            env=environment,
            timeout=SHUTDOWN_TIMEOUT_SECONDS,
        )
        assert result.returncode == 2, (blank, result.stderr)
        assert result.stdout == b"", blank
        assert b"--data-dir" in result.stderr, blank
        assert b"KALA_STUDIO_DATA_DIR" in result.stderr, blank
        assert list(home.iterdir()) == [], "the worker wrote into HOME"
    assert list(home.iterdir()) == [], "the refusal must not create a home directory"


def test_the_journal_lives_in_the_explicit_data_dir(tmp_path: Path) -> None:
    """The explicit data dir owns the journal; an empty HOME stays empty."""
    data_dir = tmp_path / "data"
    home = tmp_path / "home"
    home.mkdir()
    with _running_worker(
        data_dir,
        token=LAUNCH_TOKEN,
        lifeline=True,
        home=home,
    ) as (process, ready, _line):
        status, _, body = _request(
            ready["port"],
            "POST",
            "/internal/v1/design",
            token=LAUNCH_TOKEN,
            body={
                "template": {
                    "template": "l_bracket",
                    "width": 60,
                    "depth": 45,
                    "height": 40,
                    "wall": 8,
                    "bore_d": 10,
                    "hole_d": 5.5,
                    "hole_count": 4,
                },
                "material": "aluminium",
                "provider": "opencascade",
            },
        )
        assert status == 200, body
        status, _, body = _request(
            ready["port"],
            "GET",
            "/internal/v1/journal?after=0&limit=10",
            token=LAUNCH_TOKEN,
        )
        assert status == 200, body
        assert json.loads(body)["events"], "the design turn must be journalled"
        returncode, out, err = _terminate(process)

    assert returncode == 0
    assert out == b""
    assert b"Traceback" not in err
    journal = data_dir / "studio-journal.sqlite3"
    assert journal.is_file(), "the journal must live under the explicit data dir"
    assert list(home.iterdir()) == [], "the worker wrote into HOME"


# --- shutdown ---


def test_sigterm_shuts_down_cleanly(tmp_path: Path) -> None:
    with _running_worker(tmp_path, token=LAUNCH_TOKEN) as (process, ready, _line):
        assert process.poll() is None
        status, _, body = _request(
            ready["port"],
            "GET",
            "/internal/v1/intent/schema",
            token=LAUNCH_TOKEN,
        )
        assert status == 200, body
        returncode, out, err = _terminate(process)

    assert returncode == 0
    assert out == b""
    assert b"Traceback" not in err


def test_stdin_eof_ends_the_worker_when_the_lifeline_is_enabled(tmp_path: Path) -> None:
    with _running_worker(tmp_path, token=LAUNCH_TOKEN, lifeline=True) as (process, _ready, _line):
        assert process.stdin is not None
        process.stdin.close()
        started = time.monotonic()
        returncode = process.wait(timeout=SHUTDOWN_TIMEOUT_SECONDS)
        elapsed = time.monotonic() - started
        assert process.stdout is not None and process.stderr is not None
        out = process.stdout.read()
        err = process.stderr.read()

    assert returncode == 0
    assert elapsed < SHUTDOWN_TIMEOUT_SECONDS
    assert out == b""
    assert b"Traceback" not in err


def test_stdin_eof_without_the_lifeline_keeps_the_worker_running(tmp_path: Path) -> None:
    with _running_worker(tmp_path, token=LAUNCH_TOKEN, lifeline=False) as (process, _ready, _line):
        assert process.stdin is not None
        process.stdin.close()
        time.sleep(0.5)
        assert process.poll() is None
        returncode, out, err = _terminate(process)

    assert returncode == 0
    assert out == b""
    assert b"Traceback" not in err
