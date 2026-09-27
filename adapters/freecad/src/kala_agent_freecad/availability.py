from __future__ import annotations

import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final

from kala_agent.tools.protocol import JSONValue

MAX_DETAIL_LENGTH: Final[int] = 512
MAX_VERSION_LENGTH: Final[int] = 64
MAX_PATH_LENGTH: Final[int] = 1_024
MAX_VERSION_PARTS: Final[int] = 6
MAX_SOLVER_BYTES: Final[int] = 64 * 1_048_576

FREECAD_EXECUTABLE_NAMES: Final[tuple[str, ...]] = ("freecadcmd", "freecad")
CALCULIX_EXECUTABLE_NAMES: Final[tuple[str, ...]] = (
    "ccx",
    "CalculiX",
    "ccx_2.22",
    "ccx_2.21",
    "ccx_2.20",
    "ccx_2.19",
)
LIB_ROOT_RELATIVE_CANDIDATES: Final[tuple[tuple[str, ...], ...]] = (
    ("lib", "freecad", "lib"),
    ("lib", "freecad-python3", "lib"),
    ("lib64", "freecad", "lib"),
    ("lib", "freecad", "lib64"),
    ("freecad", "lib"),
    ("lib", "freecad"),
)
SEARCH_PREFIXES: Final[tuple[str, ...]] = (
    "/usr",
    "/usr/local",
    "/opt/freecad",
    "/opt/FreeCAD",
    "/app",
    "/snap/freecad/current",
)
MODULE_MARKERS: Final[tuple[str, ...]] = ("FreeCAD.so", "FreeCAD.dylib", "FreeCAD.py")
VERSION_PROBE_TIMEOUT_SECONDS: Final[float] = 30.0
PROBE_ROOTS_ENVIRONMENT_KEY: Final[str] = "KALA_FREECAD_PROBE_ROOTS"
_VERSION_PATTERN: Final[re.Pattern[str]] = re.compile(r"(\d+)\.(\d+)\.(\d+)")
_CALCULIX_NAME_PATTERN: Final[re.Pattern[str]] = re.compile(
    r"^(?:ccx|ccx[_-][A-Za-z0-9.+-]{0,31}|[Cc]alculiX[A-Za-z0-9._+-]{0,31})$"
)
_VERSION_PROBE_SOURCE: Final[str] = (
    "import json, os, sys;"
    f"roots = os.environ.get({PROBE_ROOTS_ENVIRONMENT_KEY!r}, '');"
    "[sys.path.insert(0, root) for root in roots.split(os.pathsep) if root];"
    "import FreeCAD;"
    "sys.stdout.write(json.dumps([str(part) for part in FreeCAD.Version()]))"
)


@dataclass(frozen=True, slots=True)
class FreeCADAvailability:
    """What this host actually provides, resolved without importing FreeCAD.

    The record is immutable and every field is a bounded string or a tuple, so
    a caller can hold and share it. Nothing here reaches the network and
    nothing here loads FreeCAD into this process: discovery only looks at the
    filesystem and, at most, runs the discovered interpreter once to read its
    own version.
    """

    available: bool
    interpreter_path: str
    executable_path: str
    import_roots: tuple[str, ...]
    version: str
    version_parts: tuple[int, ...]
    calculix_path: str
    detail: str

    def __post_init__(self) -> None:
        if not isinstance(self.available, bool):
            raise TypeError("available must be a boolean")
        for name in ("interpreter_path", "executable_path", "version", "calculix_path"):
            value = getattr(self, name)
            if not isinstance(value, str):
                raise TypeError(f"{name} must be a string")
            if len(value) > MAX_PATH_LENGTH:
                raise TypeError(f"{name} exceeds the bounded length")
        if not isinstance(self.detail, str) or len(self.detail) > MAX_DETAIL_LENGTH:
            raise TypeError("detail must be a bounded string")
        if not isinstance(self.import_roots, tuple) or any(
            not isinstance(root, str) for root in self.import_roots
        ):
            raise TypeError("import_roots must be a tuple of strings")
        if not isinstance(self.version_parts, tuple) or any(
            isinstance(part, bool) or not isinstance(part, int) or not 0 <= part <= 1_000_000
            for part in self.version_parts
        ):
            raise TypeError("version_parts must be a tuple of bounded integers")
        if len(self.version_parts) > MAX_VERSION_PARTS:
            raise TypeError("version_parts exceeds the bounded length")
        if len(self.import_roots) > len(SEARCH_PREFIXES) * len(LIB_ROOT_RELATIVE_CANDIDATES):
            raise TypeError("import_roots exceeds the bounded count")

    @property
    def solver_available(self) -> bool:
        return bool(self.calculix_path)

    @property
    def import_root_paths(self) -> tuple[Path, ...]:
        return tuple(Path(root) for root in self.import_roots)

    def to_payload(self) -> dict[str, JSONValue]:
        return {
            "available": self.available,
            "calculix_path": self.calculix_path or None,
            "detail": self.detail,
            "executable_path": self.executable_path or None,
            "import_roots": list(self.import_roots),
            "interpreter_path": self.interpreter_path or None,
            "version": self.version or None,
            "version_parts": list(self.version_parts),
        }


def unavailable(detail: str) -> FreeCADAvailability:
    return FreeCADAvailability(
        available=False,
        interpreter_path="",
        executable_path="",
        import_roots=(),
        version="",
        version_parts=(),
        calculix_path="",
        detail=_bounded_detail(detail),
    )


def parse_version(text: object) -> tuple[str, tuple[int, ...]]:
    """Read a ``major.minor.patch`` version out of any FreeCAD banner."""
    if not isinstance(text, str):
        return "", ()
    match = _VERSION_PATTERN.search(text)
    if match is None:
        return "", ()
    parts = tuple(int(group) for group in match.groups())
    version = ".".join(str(part) for part in parts)
    return version, parts


def parse_version_payload(payload: str) -> tuple[str, tuple[int, ...]]:
    """Read the version out of the JSON list a probed interpreter prints."""
    text = payload.strip() if isinstance(payload, str) else ""
    if not text:
        return "", ()
    try:
        decoded = json.loads(text)
    except (json.JSONDecodeError, RecursionError, TypeError, ValueError):
        return parse_version(text)
    if not isinstance(decoded, list):
        return parse_version(text)
    numbers = _leading_integers(decoded)
    if len(numbers) >= 3:
        selected = numbers[:3]
        return ".".join(str(part) for part in selected), tuple(selected)
    return parse_version(" ".join(str(part) for part in decoded))


def _leading_integers(values: Sequence[object]) -> list[int]:
    numbers: list[int] = []
    for entry in values:
        if isinstance(entry, bool):
            break
        if isinstance(entry, int) and 0 <= entry <= 1_000_000:
            numbers.append(entry)
            continue
        if isinstance(entry, str) and entry.isdigit() and len(entry) <= 7:
            numbers.append(int(entry))
            continue
        break
    return numbers


def is_safe_calculix(path: str) -> bool:
    """Report whether a discovered path is a plausible solver binary.

    A solver is only ever executed when it was found by name on the fixed
    search list, resolves to a regular file, is small enough to be a program
    rather than a data file, and carries an execute bit. The name itself is
    bounded and free of path separators, so a discovery result can never carry
    an argument or a traversal segment into an invocation.
    """
    if not isinstance(path, str) or not path or len(path) > MAX_PATH_LENGTH:
        return False
    if "\x00" in path or "\\" in path or "/" in os.path.basename(path):
        return False
    if _CALCULIX_NAME_PATTERN.fullmatch(os.path.basename(path)) is None:
        return False
    try:
        candidate = Path(path)
        status = candidate.stat()
    except (OSError, ValueError):
        return False
    if not stat.S_ISREG(status.st_mode) or status.st_size <= 0:
        return False
    if status.st_size > MAX_SOLVER_BYTES:
        return False
    if not os.access(candidate, os.X_OK):
        return False
    return True


def resolve_calculix(
    which: Callable[[str], str | None] | None = None,
) -> str:
    """Return the first safe CalculiX executable, or the empty string."""
    locate = which if which is not None else shutil.which
    for name in CALCULIX_EXECUTABLE_NAMES:
        try:
            found = locate(name)
        except (OSError, ValueError):
            continue
        if not isinstance(found, str) or not found:
            continue
        if is_safe_calculix(found):
            return found
    return ""


def candidate_lib_roots(
    interpreter_path: str,
    prefixes: Sequence[Path] = (),
) -> tuple[Path, ...]:
    """Return every plausible FreeCAD import root, most specific first."""
    found: list[Path] = []
    seen: set[str] = set()

    def remember(candidate: Path) -> None:
        try:
            resolved = candidate.resolve(strict=False)
        except OSError:  # pragma: no cover - defensive
            return
        text = resolved.as_posix()
        if text in seen or not _holds_module_marker(resolved):
            return
        seen.add(text)
        found.append(resolved)

    if interpreter_path:
        for parent in Path(interpreter_path).resolve(strict=False).parents:
            for relative in LIB_ROOT_RELATIVE_CANDIDATES:
                remember(parent.joinpath(*relative))
    for prefix in prefixes:
        for relative in LIB_ROOT_RELATIVE_CANDIDATES:
            remember(prefix.joinpath(*relative))
    return tuple(found)


def _holds_module_marker(root: Path) -> bool:
    try:
        if not root.is_dir():
            return False
    except OSError:  # pragma: no cover - defensive
        return False
    for marker in MODULE_MARKERS:
        if (root / marker).exists():
            return True
    return False


def read_version(
    interpreter_path: str,
    import_roots: Sequence[Path],
    *,
    python_executable: str | None = None,
) -> str:
    """Return the text a probed interpreter printed about its version.

    The worker will import FreeCAD from an ordinary Python interpreter, so
    that interpreter is probed first: an isolated probe process is started
    with every inherited import setting ignored, and the import roots travel in
    one dedicated variable because an isolated interpreter ignores
    ``PYTHONPATH`` on purpose. The FreeCAD banner is only a fallback. Both
    probes are local subprocesses with a bounded lifetime and no network use.
    """
    candidate = python_executable if python_executable is not None else sys.executable
    if candidate and _is_usable_executable(candidate):
        payload = _run_probe([candidate, "-I", "-B", "-c", _VERSION_PROBE_SOURCE], import_roots)
        if payload:
            return payload
    if not interpreter_path:
        return ""
    return _run_probe([interpreter_path, "-v"], ())


def _run_probe(command: Sequence[str], import_roots: Sequence[Path]) -> str:
    with tempfile.TemporaryDirectory(prefix="kala-freecad-probe-") as home:
        environment = {
            "HOME": home,
            "LANG": "C.UTF-8",
            PROBE_ROOTS_ENVIRONMENT_KEY: os.pathsep.join(root.as_posix() for root in import_roots),
            "PATH": "/usr/local/bin:/usr/bin:/bin",
            "PYTHONDONTWRITEBYTECODE": "1",
            "PYTHONHASHSEED": "0",
            "PYTHONIOENCODING": "utf-8",
        }
        try:
            completed = subprocess.run(
                list(command),
                capture_output=True,
                check=False,
                cwd=home,
                env=environment,
                stdin=subprocess.DEVNULL,
                timeout=VERSION_PROBE_TIMEOUT_SECONDS,
            )
        except (OSError, ValueError, subprocess.SubprocessError):
            return ""
    if completed.returncode != 0:
        return ""
    return completed.stdout.decode("utf-8", errors="replace").strip()


def discover(
    *,
    which: Callable[[str], str | None] | None = None,
    version_reader: Callable[[str, Sequence[Path]], str] | None = None,
    prefixes: Sequence[Path] = (),
    read_version_text: bool = True,
) -> FreeCADAvailability:
    """Probe the host for FreeCAD, its import roots, its version, and ccx."""
    locate = which if which is not None else shutil.which
    search_prefixes = tuple(prefixes) if prefixes else tuple(Path(item) for item in SEARCH_PREFIXES)
    resolved: dict[str, str] = {}
    for name in FREECAD_EXECUTABLE_NAMES:
        try:
            found = locate(name)
        except (OSError, ValueError):
            continue
        if isinstance(found, str) and found and _is_usable_executable(found):
            resolved[name] = found
    interpreter = resolved.get("freecadcmd") or resolved.get("freecad") or ""
    if not interpreter:
        return unavailable("no freecadcmd or freecad executable is on the search path")
    executable = resolved.get("freecad", "")
    roots = candidate_lib_roots(interpreter, search_prefixes)
    if not roots:
        return FreeCADAvailability(
            available=False,
            interpreter_path=interpreter,
            executable_path=executable,
            import_roots=(),
            version="",
            version_parts=(),
            calculix_path=resolve_calculix(locate),
            detail=_bounded_detail(
                f"no FreeCAD import root was found near {Path(interpreter).name}"
            ),
        )
    version = ""
    if read_version_text:
        reader = version_reader if version_reader is not None else read_version
        version, parts = parse_version_payload(reader(interpreter, roots))
        if not version:
            parts = ()
    else:
        parts = ()
    if not version:
        return FreeCADAvailability(
            available=False,
            interpreter_path=interpreter,
            executable_path=executable,
            import_roots=tuple(root.as_posix() for root in roots),
            version="",
            version_parts=(),
            calculix_path=resolve_calculix(locate),
            detail="the discovered FreeCAD interpreter did not report a usable version",
        )
    return FreeCADAvailability(
        available=True,
        interpreter_path=interpreter,
        executable_path=executable,
        import_roots=tuple(root.as_posix() for root in roots),
        version=version,
        version_parts=parts,
        calculix_path=resolve_calculix(locate),
        detail=f"FreeCAD {version} is available for import",
    )


def _is_usable_executable(path: str) -> bool:
    try:
        candidate = Path(path)
        return candidate.is_file() and os.access(candidate, os.X_OK)
    except (OSError, ValueError):  # pragma: no cover - defensive
        return False


def _bounded_detail(detail: str) -> str:
    collapsed = " ".join(str(detail).split())
    return collapsed[:MAX_DETAIL_LENGTH]


__all__ = (
    "CALCULIX_EXECUTABLE_NAMES",
    "FREECAD_EXECUTABLE_NAMES",
    "LIB_ROOT_RELATIVE_CANDIDATES",
    "MAX_DETAIL_LENGTH",
    "PROBE_ROOTS_ENVIRONMENT_KEY",
    "SEARCH_PREFIXES",
    "VERSION_PROBE_TIMEOUT_SECONDS",
    "FreeCADAvailability",
    "candidate_lib_roots",
    "discover",
    "is_safe_calculix",
    "parse_version",
    "parse_version_payload",
    "read_version",
    "resolve_calculix",
    "unavailable",
)
