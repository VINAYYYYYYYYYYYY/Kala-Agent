"""Shared helpers for corpus generation.

Only the standard library and the live ``kala_agent`` package are used. The
helpers here are deliberately mechanical: file digests, deterministic JSON
writing, environment capture, and the fixed clock. No product rule lives here.
"""

from __future__ import annotations

import contextlib
import hashlib
import json
import os
import platform
import re
import sqlite3
import subprocess
import sys
from collections.abc import Callable, Iterable, Iterator
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Final, TypeAlias

from corpuslib import FIXED_CLOCK_START_ISO

REPO_ROOT: Final[Path] = Path(__file__).resolve().parents[3]
SOURCE_ROOT: Final[Path] = REPO_ROOT / "src" / "kala_agent"

Json: TypeAlias = "dict[str, object] | list[object] | str | int | float | bool | None"

# The OCP STEP writer stamps the wall clock into the FILE_NAME header line and
# numbers the translated product with a per-process counter. Both are host
# facts, not product facts, so the corpus normalises them to fixed values
# before it records a digest. The replacement keeps the byte length, so the
# recorded length is still the length of the real response.
FIXED_STEP_TIMESTAMP: Final[str] = "2026-01-01T00:00:00"
STEP_TIMESTAMP_PATTERN: Final[re.Pattern[bytes]] = re.compile(
    rb"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}"
)
STEP_TRANSLATOR_PATTERN: Final[re.Pattern[bytes]] = re.compile(
    rb"(Open CASCADE STEP translator [0-9.]+ )([0-9]+)"
)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_text(text: str) -> str:
    return sha256_bytes(text.encode("utf-8"))


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def normalise_step_bytes(raw: bytes) -> bytes:
    """Return the STEP bytes with the writer's run-specific values fixed.

    Two values in an OCP STEP export change on every run and on every export
    inside one process:

    * the ``FILE_NAME`` header line carries the wall clock of the write, and
    * the translated product is numbered with a per-process counter, so the
      second export of one process says ``... 8.0 2`` where the first said
      ``... 8.0 1``.

    Neither is a product fact, so the corpus records the digest of the body
    with both values normalised. The replacements keep the byte length, so
    the recorded length is still the length of the real response.
    """
    normalised = STEP_TIMESTAMP_PATTERN.sub(
        FIXED_STEP_TIMESTAMP.encode("ascii"), raw, count=1
    )
    return STEP_TRANSLATOR_PATTERN.sub(
        lambda match: match.group(1) + b"0" * len(match.group(2)), normalised
    )


def step_normalisation_facts() -> list[dict[str, str]]:
    """Describe, for the manifest, what the STEP normalisation replaces."""
    return [
        {
            "field": "FILE_NAME header write timestamp",
            "replacement": FIXED_STEP_TIMESTAMP,
            "reason": "the OCP writer stamps the wall clock of the write",
        },
        {
            "field": "Open CASCADE STEP translator product revision counter",
            "replacement": "zero padded to the original digit width",
            "reason": "OCCT numbers the translated product per process, so the value depends on how many STEP exports ran before it",
        },
    ]


def write_json(path: Path, payload: object) -> None:
    """Write deterministic, human-readable JSON with a trailing newline."""
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(payload, ensure_ascii=True, indent=2, sort_keys=True)
    path.write_text(text + "\n", encoding="utf-8")


def read_json(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"))


def _distribution_version(name: str) -> str | None:
    from importlib import metadata

    try:
        return metadata.version(name)
    except metadata.PackageNotFoundError:
        return None


def _cad_provider_facts() -> list[dict[str, object]]:
    """Return the host's CAD provider probe results.

    ADR-0011 makes availability an evidence from the host, never a constant.
    The corpus records the probe result beside the package versions so a
    reader can tell an unavailable provider from a missing fixture.
    """
    try:
        from kala_agent.studio.design.providers import probe_providers
    except Exception as error:  # pragma: no cover - the probe is defensive
        return [{"probe_error": type(error).__name__}]
    facts: list[dict[str, object]] = []
    for status in probe_providers():
        facts.append(
            {
                "id": str(status.id),
                "available": bool(status.available),
                "version": status.version,
                "reason": status.reason,
                "mode": status.mode,
            }
        )
    return facts


def environment_facts() -> dict[str, object]:
    """Return the interpreter/package facts that can change oracle bytes."""
    import fastapi
    import httpx
    import pydantic
    import starlette
    import uvicorn

    import kala_agent

    return {
        "python_implementation": platform.python_implementation(),
        "python_version": platform.python_version(),
        "python_executable_basename": Path(sys.executable).name,
        "platform_system": platform.system(),
        "platform_machine": platform.machine(),
        "sqlite3_library_version": sqlite3.sqlite_version,
        "packages": {
            "fastapi": fastapi.__version__,
            "httpx": httpx.__version__,
            "kala-agent": _distribution_version("kala-agent"),
            "pydantic": pydantic.VERSION,
            "starlette": starlette.__version__,
            "uvicorn": uvicorn.__version__,
            "cadquery-ocp": _distribution_version("cadquery-ocp"),
        },
        "kala_agent_module_version": getattr(kala_agent, "__version__", None),
        "json_module": json.__name__,
        "decimal_context": _decimal_context(),
        "cad_providers": _cad_provider_facts(),
    }


def _decimal_context() -> dict[str, object]:
    from decimal import getcontext

    context = getcontext()
    return {
        "prec": context.prec,
        "rounding": str(context.rounding),
        "emin": context.Emin,
        "emax": context.Emax,
        "capitals": context.capitals,
        "clamp": context.clamp,
        "traps": sorted(
            getattr(name, "__name__", str(name))
            for name, enabled in context.traps.items()
            if enabled
        ),
    }


def git_state() -> dict[str, object]:
    def _run(*args: str) -> str | None:
        try:
            completed = subprocess.run(
                ["git", *args],
                cwd=REPO_ROOT,
                check=False,
                capture_output=True,
                text=True,
                timeout=10,
            )
        except (OSError, subprocess.SubprocessError):
            return None
        if completed.returncode != 0:
            return None
        return completed.stdout.rstrip("\n")

    commit = _run("rev-parse", "HEAD")
    status = _run("status", "--porcelain")
    return {
        "commit": commit,
        "dirty": None if status is None else bool(status),
        "tracked_changes": None
        if status is None
        else sorted(line[3:] for line in status.splitlines() if line[:2].strip()),
    }


def python_source_files(root: Path = SOURCE_ROOT) -> list[Path]:
    return sorted(path for path in root.rglob("*.py") if "__pycache__" not in path.parts)


def source_tree_state(root: Path = SOURCE_ROOT) -> dict[str, object]:
    """Digest every product Python source file, order-independent.

    The value identifies the oracle tree that produced the corpus. A source
    edit is expected to change it, which is why the corpus is regenerated
    after a source change rather than compared across revisions.
    """
    entries = [
        [path.relative_to(REPO_ROOT).as_posix(), sha256_file(path)]
        for path in python_source_files(root)
    ]
    return {
        "root": root.relative_to(REPO_ROOT).as_posix(),
        "file_count": len(entries),
        "tree_digest": sha256_text(
            json.dumps(entries, ensure_ascii=True, separators=(",", ":"), sort_keys=True)
        ),
        "files": [{"path": path, "sha256": digest} for path, digest in entries],
    }


class FixedClock:
    """A monotonic second-granularity clock for deterministic timestamps."""

    def __init__(self, start: str = FIXED_CLOCK_START_ISO, step_seconds: int = 1) -> None:
        self._current = datetime.fromisoformat(start).astimezone(UTC)
        self._step = timedelta(seconds=step_seconds)

    def __call__(self) -> datetime:
        value = self._current
        self._current += self._step
        return value


@contextlib.contextmanager
def silenced_fds() -> Iterator[None]:
    """Redirect C-level stdout/stderr to /dev/null, then restore them.

    ``contextlib.redirect_stdout`` only rebinds the Python object; OCP writes
    transfer statistics through file descriptor 1. The fixture must not depend
    on that output, and generator logs must stay readable.
    """
    with open(os.devnull, "wb") as devnull:
        saved = (os.dup(1), os.dup(2))
        try:
            os.dup2(devnull.fileno(), 1)
            os.dup2(devnull.fileno(), 2)
            yield None
        finally:
            os.dup2(saved[0], 1)
            os.dup2(saved[1], 2)
            os.close(saved[0])
            os.close(saved[1])


def silence_native_shutdown() -> None:
    """Leave C-level stdout/stderr on /dev/null for interpreter shutdown.

    OCP's STEP writer flushes its transfer statistics from a native destructor
    after the Python program has printed its result. Those lines are not part
    of any fixture. Call this only after the final report has been flushed.
    """
    try:
        sys.stdout.flush()
        sys.stderr.flush()
        descriptor = os.open(os.devnull, os.O_WRONLY)
        os.dup2(descriptor, 1)
        os.dup2(descriptor, 2)
        os.close(descriptor)
    except OSError:
        pass


@contextlib.contextmanager
def fixed_journal_clock(clock: Callable[[], datetime]) -> Iterator[None]:
    """Make ``EventJournal``'s wall-clock timestamps deterministic.

    The application accepts an injected clock, but ``EventJournal.append``
    calls ``datetime.now(UTC)`` directly. Production behavior is wall-clock;
    the fixture harness substitutes a fixed subclass for the module's
    ``datetime`` so event timestamps -- and therefore event digests -- are
    reproducible. The hashing code and every other byte remain the product's.
    """
    from kala_agent.persistence import journal as journal_module

    original = journal_module.datetime
    journal_module.datetime = _fixed_datetime_subclass(original, clock)
    try:
        yield None
    finally:
        journal_module.datetime = original


@contextlib.contextmanager
def fixed_studio_journal_clock(clock: Callable[[], datetime]) -> Iterator[None]:
    """Make the Studio session journal's wall-clock timestamps deterministic.

    ``StudioSessionJournal.append`` calls ``datetime.now(UTC)`` directly, the
    same way the core journal does. The fixture harness substitutes a fixed
    subclass for ``kala_agent.studio.session_journal.datetime`` so the Studio
    journal timestamps -- and therefore the chain digests that bind them -- are
    reproducible. The chain code and every other byte remain the product's.
    """
    from kala_agent.studio import session_journal as session_journal_module

    original = session_journal_module.datetime
    session_journal_module.datetime = _fixed_datetime_subclass(original, clock)
    try:
        yield None
    finally:
        session_journal_module.datetime = original


def _fixed_datetime_subclass(
    original: type[datetime], clock: Callable[[], datetime]
) -> type[datetime]:
    class _FixedDatetime(original):  # type: ignore[misc, valid-type]
        @classmethod
        def now(cls, tz: object = None) -> datetime:
            value = clock()
            if tz is None:
                return value
            return value.astimezone(tz)  # type: ignore[arg-type]

    return _FixedDatetime


def summarized_array(values: Iterable[float | int]) -> dict[str, object]:
    """Return a compact, order-sensitive summary of a flat numeric array."""
    materialized = list(values)
    return {
        "length": len(materialized),
        "first": materialized[:6],
        "last": materialized[-6:],
    }


def call_or_error(function: Callable[[], object]) -> dict[str, object]:
    try:
        return {"ok": True, "value": function()}
    except Exception as error:
        return {"ok": False, "error_type": type(error).__name__, "message": str(error)}


def relative_posix(path: Path, root: Path = REPO_ROOT) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return path.resolve().as_posix()


def file_record(path: Path, root: Path = REPO_ROOT) -> dict[str, object]:
    data = path.read_bytes()
    return {
        "path": relative_posix(path, root),
        "size_bytes": len(data),
        "sha256": sha256_bytes(data),
    }


__all__ = (
    "FIXED_STEP_TIMESTAMP",
    "REPO_ROOT",
    "SOURCE_ROOT",
    "FixedClock",
    "call_or_error",
    "environment_facts",
    "file_record",
    "fixed_journal_clock",
    "fixed_studio_journal_clock",
    "git_state",
    "normalise_step_bytes",
    "python_source_files",
    "read_json",
    "relative_posix",
    "sha256_bytes",
    "sha256_file",
    "sha256_text",
    "silence_native_shutdown",
    "silenced_fds",
    "source_tree_state",
    "step_normalisation_facts",
    "summarized_array",
    "write_json",
)
