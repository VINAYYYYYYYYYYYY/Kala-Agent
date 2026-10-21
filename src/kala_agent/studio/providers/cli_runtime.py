"""The shared machinery of the three subscription-CLI chat providers.

A subscription provider does not hold a credential in Kala. It runs the
operator's own, already-authenticated command-line tool as a subprocess and
lets that tool talk to its vendor. Everything in this module exists to make
that subprocess boring:

* **No shell.** The child is started from an argv list. Nothing is ever
  handed to a shell, so no prompt text, model id or path can be
  interpreted as shell syntax.
* **The prompt never rides on argv.** The prompt is written to the child's
  standard input and the standard input is closed afterwards. A prompt is
  therefore invisible in ``ps`` output and cannot overflow the argument
  limit.
* **A sanitized environment.** The child starts from an empty environment
  and receives only the names on :data:`ENV_ALLOWLIST` plus the one name
  the CLI itself needs to find its own login. Nothing of Kala's
  configuration - no stored key, no studio token, no data directory - is
  inherited, and no ``*_API_KEY`` is forwarded: the CLI reads its own
  credential store, Kala never sees it.
* **A private working directory.** Every run gets a fresh temporary
  directory that is deleted when the turn ends, so the CLI cannot read or
  write the project the Studio happens to run in.
* **A bounded wall time.** The turn is bounded by
  :data:`DEFAULT_CLI_TIMEOUT_SECONDS` (configurable through
  :data:`CLI_TIMEOUT_ENV`), and a child that overruns is killed and reaped.
* **Bounded output.** Standard output is capped at
  :data:`MAX_CLI_STDOUT_BYTES` and standard error at
  :data:`MAX_CLI_STDERR_BYTES`.

The availability probe is deliberately cheap and never reads a credential
file: it asks the CLI for its version and, when the CLI has one, for its
own authentication status. Only the exit status of that status command is
used; its output is never echoed, so an account name or an address can
never leak into a response.

The module is the Python reference of the native product's
``providers/cli_runtime.rs``; the two files are meant to be read side by
side.
"""

from __future__ import annotations

import errno
import os
import signal
import subprocess
import sys
import threading
import time
import unicodedata
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import IO, Final

from kala_agent.studio.providers.base import (
    ProviderSubprocessError,
    ProviderTimeoutError,
    ProviderUnavailableError,
    TooLargeResponseError,
)

#: The default wall-clock budget of one CLI chat turn.
DEFAULT_CLI_TIMEOUT_SECONDS: Final[int] = 180
#: The shortest wall-clock budget an operator may configure.
MIN_CLI_TIMEOUT_SECONDS: Final[int] = 1
#: The longest wall-clock budget an operator may configure.
MAX_CLI_TIMEOUT_SECONDS: Final[int] = 3_600
#: The environment variable that configures the wall-clock budget.
CLI_TIMEOUT_ENV: Final[str] = "KALA_STUDIO_CLI_TIMEOUT_SECONDS"
#: The largest standard output one CLI turn may produce.
MAX_CLI_STDOUT_BYTES: Final[int] = 1_000_000
#: The largest standard error one CLI turn may produce.
MAX_CLI_STDERR_BYTES: Final[int] = 65_536
#: The wall-clock budget of one availability probe.
#:
#: The probe is a background refresh, not a request-bound operation: a CLI
#: that hangs on ``--version`` must never hold a route open. The cap is short
#: on purpose, because the verdict only fills a provider row.
CLI_PROBE_TIMEOUT_SECONDS: Final[int] = 2
#: How long a probe verdict is reused before it is taken again.
CLI_PROBE_TTL_SECONDS: Final[int] = 120
#: How long ``GET /providers``, ``GET /settings`` and ``GET /health`` wait
#: for a first probe verdict before they answer with the honest "checking"
#: row.
PROBE_ROUTE_BUDGET_SECONDS: Final[float] = 0.6
#: The pause between two looks at the probe cache while a route waits.
PROBE_POLL_SECONDS: Final[float] = 0.01
#: The honest reason of a row whose first probe has not finished yet.
PROBE_CHECKING_REASON: Final[str] = "checking..."
#: The Windows executable extensions searched when ``PATHEXT`` is absent.
DEFAULT_PATHEXT: Final[str] = ".COM;.EXE;.BAT;.CMD"
#: Whether this interpreter runs on Windows.
IS_WINDOWS: Final[bool] = sys.platform.startswith("win")
#: The longest excerpt of a CLI failure that is reported to the operator.
MAX_CLI_MESSAGE_LENGTH: Final[int] = 512
#: How many times a start that the kernel refused as "text file busy" is
#: retried.
MAX_BUSY_START_ATTEMPTS: Final[int] = 4
#: The pause between two such attempts.
BUSY_START_BACKOFF_SECONDS: Final[float] = 0.05

#: The environment names every subscription CLI receives.
#:
#: The list is the union of "where the CLI finds its own login" (``HOME``,
#: the ``XDG_*`` directories), "how the CLI finds its own programs"
#: (``PATH``) and the locale and temporary-directory names. It deliberately
#: carries no credential, no Kala variable and no Studio token.
ENV_ALLOWLIST: Final[tuple[str, ...]] = (
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
)


@dataclass(frozen=True, slots=True)
class CliSpec:
    """The identity and the fixed probe commands of one subscription CLI."""

    #: The provider id this CLI answers for.
    id: str
    #: The name the provider list shows.
    name: str
    #: The executable name looked up on ``PATH``.
    binary: str
    #: The environment variable that overrides the executable path.
    path_override_env: str
    #: The cheap version probe.
    version_args: tuple[str, ...]
    #: The cheap authentication-status probe.
    auth_args: tuple[str, ...]
    #: The extra environment names this CLI needs to find its own login.
    env_passthrough: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class CliProbe:
    """The verdict of the availability probe of one subscription CLI."""

    #: The provider id this verdict belongs to.
    id: str
    #: Whether the CLI is installed and reports a version.
    installed: bool
    #: Whether the CLI reports that it is signed in.
    authenticated: bool
    #: The honest reason, shown next to the provider row.
    reason: str
    #: Whether the first probe is still running and no verdict exists yet.
    #:
    #: A checking row is honest about not knowing: it reports
    #: ``available: null`` and :data:`PROBE_CHECKING_REASON`, never
    #: "unavailable".
    checking: bool = False

    def usable(self) -> bool:
        """Return whether the CLI can answer a turn right now."""
        return self.installed and self.authenticated


def checking_probe(spec: CliSpec) -> CliProbe:
    """Return the honest placeholder for a CLI whose probe is still running."""
    return CliProbe(
        id=spec.id,
        installed=False,
        authenticated=False,
        reason=PROBE_CHECKING_REASON,
        checking=True,
    )


@dataclass(frozen=True, slots=True)
class CliRun:
    """The result of one finished CLI run."""

    #: The exit code, when the child was terminated by a normal exit.
    status: int | None
    #: The bounded standard output.
    stdout: bytes
    #: The bounded standard error.
    stderr: bytes

    def succeeded(self) -> bool:
        """Return whether the child exited with status zero."""
        return self.status == 0


@dataclass(frozen=True, slots=True)
class CliInvocation:
    """One CLI invocation.

    The dataclass keeps the run's inputs together: the argv is fixed, the
    payload goes to standard input, the environment is sanitized and the
    working directory is private.
    """

    #: The CLI being run.
    spec: CliSpec
    #: The executable.
    program: str
    #: The fixed argument vector. The payload is never part of it.
    argv: tuple[str, ...]
    #: The payload written to the child's standard input.
    stdin: bytes = b""
    #: The wall-clock budget, in seconds.
    timeout: float = float(DEFAULT_CLI_TIMEOUT_SECONDS)
    #: The environment view the sanitized child environment is built from.
    environ: Mapping[str, str] = field(default_factory=dict)
    #: The private working directory.
    workdir: Path | None = None
    #: Extra environment names for the child; Kala's own sandboxing
    #: instructions, never a credential.
    extra_env: tuple[tuple[str, str], ...] = ()


def timeout_seconds(environ: Mapping[str, str]) -> int:
    """Return the wall-clock budget of one CLI turn, read from the environment.

    An unreadable or out-of-range value falls back to
    :data:`DEFAULT_CLI_TIMEOUT_SECONDS` rather than failing the turn: a bad
    configuration must not make the provider unavailable.
    """
    raw = environ.get(CLI_TIMEOUT_ENV)
    if raw is None:
        return DEFAULT_CLI_TIMEOUT_SECONDS
    trimmed = raw.strip()
    try:
        value = int(trimmed)
    except ValueError:
        return DEFAULT_CLI_TIMEOUT_SECONDS
    if value < 0:
        # A negative budget is not a number the native product accepts; it
        # falls back to the default there too.
        return DEFAULT_CLI_TIMEOUT_SECONDS
    return max(MIN_CLI_TIMEOUT_SECONDS, min(MAX_CLI_TIMEOUT_SECONDS, value))


def _home_directory() -> str | None:
    """Return the per-user home directory on the current platform.

    The lookup mirrors the native product: ``HOME`` first, then
    ``USERPROFILE``, then the Windows ``HOMEDRIVE``/``HOMEPATH`` pair. The
    value is read from the process environment, never from the request's
    environment view, so a request cannot redirect the child's home.
    """
    home = os.environ.get("HOME")
    if home:
        return home
    profile = os.environ.get("USERPROFILE")
    if profile:
        return profile
    drive = os.environ.get("HOMEDRIVE")
    path = os.environ.get("HOMEPATH")
    if drive and path:
        return drive + path
    return None


def child_env(spec: CliSpec, environ: Mapping[str, str]) -> dict[str, str]:
    """Return the environment the child process receives.

    The list is the allowlist plus the CLI's own names; nothing else is
    inherited, and the environment starts empty.
    """
    result: dict[str, str] = {}
    for name in ENV_ALLOWLIST:
        value = environ.get(name)
        if isinstance(value, str):
            result[name] = value
    for name in spec.env_passthrough:
        value = environ.get(name)
        if isinstance(value, str):
            result[name] = value
    if "HOME" not in result:
        home = _home_directory()
        if home is not None:
            result["HOME"] = home
    return result


def is_executable(path: str | os.PathLike[str]) -> bool:
    """Return whether a path names an executable file."""
    candidate = os.fspath(path)
    if not os.path.isfile(candidate):
        return False
    return os.access(candidate, os.X_OK)


@dataclass(frozen=True, slots=True)
class PathSearch:
    """The extension search order of one ``PATH`` lookup."""

    #: The extensions to try, each beginning with a dot, in order.
    extensions: tuple[str, ...]
    #: Whether a file without an extension may be used directly.
    #:
    #: Unix scripts have no extension, so the bare name comes first there;
    #: Windows only runs extension-bearing images, so an extension-less file
    #: (for example the POSIX helper npm writes beside a ``.cmd`` shim) must
    #: never be chosen.
    allow_bare: bool


def parse_pathext(value: str | None) -> tuple[str, ...]:
    """Parse a ``PATHEXT`` value into the extension search order.

    A missing or blank value falls back to :data:`DEFAULT_PATHEXT`. Each
    entry is normalised to a leading dot, blanks are dropped and duplicates
    are removed case-insensitively while the order is kept.
    """
    raw = value if isinstance(value, str) and value.strip() else DEFAULT_PATHEXT
    result: list[str] = []
    seen: set[str] = set()
    for part in raw.split(";"):
        trimmed = part.strip()
        if not trimmed:
            continue
        extension = trimmed if trimmed.startswith(".") else f".{trimmed}"
        lowered = extension.lower()
        if lowered in seen:
            continue
        seen.add(lowered)
        result.append(extension)
    return tuple(result)


def _for_current_platform(environ: Mapping[str, str]) -> PathSearch:
    """Return the search order of the platform this interpreter runs on."""
    if IS_WINDOWS:
        return PathSearch(extensions=parse_pathext(environ.get("PATHEXT")), allow_bare=False)
    return PathSearch(extensions=(".exe",), allow_bare=True)


def _candidate_names(binary: str, search: PathSearch) -> tuple[str, ...]:
    """Return the file names one program may carry in a directory."""
    lowered = binary.lower()
    has_known_extension = any(lowered.endswith(extension.lower()) for extension in search.extensions)
    names: list[str] = []
    if search.allow_bare or has_known_extension:
        names.append(binary)
    names.extend(f"{binary}{extension}" for extension in search.extensions)
    return tuple(names)


def resolve_program_with(
    spec: CliSpec,
    environ: Mapping[str, str],
    search: PathSearch,
    is_exec: Callable[[str], bool] | None = None,
) -> str | None:
    """Find a program on ``PATH`` with an injected extension order.

    The configured path wins because an operator who names an executable
    means it, even when another copy sits earlier on ``PATH``. ``is_exec``
    lets a test inject a fake file system; production passes
    :func:`is_executable`.
    """
    executable = is_executable if is_exec is None else is_exec
    raw = environ.get(spec.path_override_env)
    if isinstance(raw, str):
        trimmed = raw.strip()
        if trimmed:
            if executable(trimmed):
                return trimmed
            for extension in search.extensions:
                with_extension = f"{trimmed}{extension}"
                if executable(with_extension):
                    return with_extension
    path = environ.get("PATH")
    if not isinstance(path, str):
        return None
    for directory in path.split(os.pathsep):
        if not directory:
            continue
        for name in _candidate_names(spec.binary, search):
            candidate = os.path.join(directory, name)
            if executable(candidate):
                return candidate
    return None


def resolve_program(spec: CliSpec, environ: Mapping[str, str]) -> str | None:
    """Find the CLI: the configured path first, then ``PATH``.

    The configured path wins because an operator who names an executable
    means it, even when another copy sits earlier on ``PATH``. On Windows
    the ``PATHEXT`` order is searched, so an npm-installed ``.cmd`` shim is
    found without the operator configuring anything.
    """
    return resolve_program_with(spec, environ, _for_current_platform(environ))


def is_batch_shim(path: str | os.PathLike[str]) -> bool:
    """Return whether a program is a Windows batch shim ``cmd.exe`` runs."""
    return Path(os.fspath(path)).suffix.lower() in {".cmd", ".bat"}


def _escape_cmd_argument(value: str) -> str:
    """Quote one argument for the ``cmd.exe`` command line.

    The escaping follows the well-tested ``cross-spawn`` algorithm: backslash
    runs before a quote are doubled and the quote is escaped, a trailing
    backslash run is doubled, the token is wrapped in quotes, and every cmd
    metacharacter is prefixed with ``^``.
    """
    escaped: list[str] = []
    backslashes = 0
    for character in value:
        if character == "\\":
            backslashes += 1
            continue
        if character == '"':
            escaped.append("\\" * (backslashes * 2 + 1))
            escaped.append('"')
            backslashes = 0
            continue
        escaped.append("\\" * backslashes)
        escaped.append(character)
        backslashes = 0
    escaped.append("\\" * (backslashes * 2))
    quoted = '"' + "".join(escaped) + '"'
    meta = set('()%!^"`<>&|;, *?')
    return "".join(("^" + character) if character in meta else character for character in quoted)


def cmd_shim_argv(program: str, argv: Sequence[str]) -> list[str]:
    """Return the ``cmd.exe`` argv that runs a batch shim with an argv list.

    The result is ``["/d", "/s", "/c", "<quoted command line>"]``: a normal
    argument list, never a string concatenated for a shell.

    Raises :class:`ProviderSubprocessError` when the program or one argument
    contains a control character, which would end the ``cmd.exe`` line early.
    """
    tokens = [_escape_cmd_argument(program)]
    tokens.extend(_escape_cmd_argument(argument) for argument in argv)
    for token in tokens:
        if any(character in token for character in "\n\r\0"):
            raise ProviderSubprocessError(
                "a Windows batch shim argument contains a control character"
            )
    return ["/d", "/s", "/c", '"' + " ".join(tokens) + '"']


def program_or_reason(spec: CliSpec, environ: Mapping[str, str]) -> str:
    """Return the executable the probe and the turn share.

    Raises :class:`ProviderUnavailableError` when the CLI is not installed:
    the route answers a sanitized 503 and the operator sees the honest
    reason.
    """
    program = resolve_program(spec, environ)
    if program is None:
        raise ProviderUnavailableError(
            f"the {spec.name} was not found on PATH or at {spec.path_override_env}"
        )
    return program


def _error_kind(error: OSError) -> str:
    """Return the native product's name for one I/O error kind."""
    import errno

    code = error.errno
    if code is None:
        return "Uncategorized"
    return {
        errno.ENOENT: "NotFound",
        errno.EACCES: "PermissionDenied",
        errno.EPERM: "PermissionDenied",
        errno.ETXTBSY: "ExecutableFileBusy",
        errno.ENOTDIR: "NotADirectory",
        errno.EISDIR: "IsADirectory",
        errno.ENOMEM: "OutOfMemory",
        errno.E2BIG: "ArgumentListTooLong",
        errno.ENOEXEC: "InvalidData",
    }.get(code, "Uncategorized")


def _drain(
    stream: IO[bytes] | None,
    cap: int,
    sink: list[bytes],
    overflow: threading.Event,
    process: subprocess.Popen[bytes],
) -> None:
    """Read one pipe into ``sink``, refusing to grow past ``cap``.

    A child that produces more than the bound is killed at once: it would
    otherwise block on a full pipe and the turn would wait for the whole
    budget before it could report the refusal.
    """
    if stream is None:
        return
    total = 0
    while True:
        try:
            chunk = stream.read(8192)
        except (OSError, ValueError):
            return
        if not chunk:
            return
        total += len(chunk)
        if total > cap:
            overflow.set()
            _kill_tree(process)
            return
        sink.append(chunk)


def _write_stdin(process: subprocess.Popen[bytes], payload: bytes) -> None:
    """Write the payload to the child's standard input and close it."""
    stream = process.stdin
    if stream is None:
        return
    try:
        if payload:
            stream.write(payload)
        stream.flush()
    except (BrokenPipeError, OSError, ValueError):
        pass
    finally:
        try:
            stream.close()
        except (BrokenPipeError, OSError, ValueError):
            pass


def _spawn(
    argv: list[str],
    environment: Mapping[str, str],
    invocation: CliInvocation,
) -> subprocess.Popen[bytes]:
    """Start the child, retrying the "text file busy" refusal.

    The kernel refuses to execute a file that is still open for writing, and
    the release of the last reference to a file is deferred: a program that
    writes an executable and starts it in the same breath can be refused with
    ``ETXTBSY`` even though the writer has closed its handle. The refusal is
    transient, so the start is retried a few times before the turn is
    refused. A CLI that an installer is replacing at that exact moment is
    the realistic case.

    The child is the leader of its own process group (POSIX
    ``start_new_session=True``; Windows ``CREATE_NEW_PROCESS_GROUP``), so the
    whole tree can be killed as a unit. A Windows ``.cmd``/``.bat`` shim is
    run through ``cmd.exe /d /s /c`` with an escaped argv; the prompt still
    goes to standard input only.
    """
    if is_batch_shim(argv[0]):
        if not IS_WINDOWS:
            raise ProviderSubprocessError(
                f"the {invocation.spec.name} resolved to a Windows batch shim "
                "(.cmd/.bat), which this platform cannot run"
            )
        argv = ["cmd.exe", *cmd_shim_argv(argv[0], argv[1:])]
    attempt = 0
    while True:
        try:
            return subprocess.Popen(
                argv,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                cwd=(str(invocation.workdir) if invocation.workdir is not None else None),
                env=dict(environment),
                shell=False,
                start_new_session=not IS_WINDOWS,
                creationflags=(
                    getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0) if IS_WINDOWS else 0
                ),
            )
        except OSError as error:
            if error.errno == errno.ETXTBSY and attempt < MAX_BUSY_START_ATTEMPTS:
                attempt += 1
                time.sleep(BUSY_START_BACKOFF_SECONDS)
                continue
            raise ProviderSubprocessError(
                f"the {invocation.spec.name} could not be started ({_error_kind(error)})"
            ) from error


def _kill_tree(process: subprocess.Popen[bytes]) -> None:
    """Kill the child's whole process tree, without waiting.

    POSIX: ``killpg`` on the group the child leads. Windows: ``taskkill /T
    /F`` walks the parent/child tree. The direct ``kill`` is the backstop for
    a descendant that left the group.
    """
    if IS_WINDOWS:
        try:
            subprocess.run(
                ["taskkill", "/T", "/F", "/PID", str(process.pid)],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=1,
                check=False,
            )
        except (OSError, subprocess.SubprocessError):
            pass
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except OSError:
            pass
    try:
        process.kill()
    except OSError:
        pass


def run(invocation: CliInvocation) -> CliRun:
    """Run one CLI with a fixed argv, a sanitized environment, a private
    working directory and the payload on standard input.

    Raises :class:`ProviderSubprocessError` when the child cannot be
    started, :class:`TooLargeResponseError` when it produced more output
    than the bounded size, and :class:`ProviderTimeoutError` when it
    overran its budget. The child's own text is bounded and never carries a
    credential: the CLI's output is the only thing that can reach the
    operator.
    """
    spec = invocation.spec
    environment = child_env(spec, invocation.environ)
    for name, value in invocation.extra_env:
        environment[name] = value
    argv = [invocation.program, *invocation.argv]
    process = _spawn(argv, environment, invocation)

    stdout_chunks: list[bytes] = []
    stderr_chunks: list[bytes] = []
    overflow = threading.Event()
    readers = [
        threading.Thread(
            target=_drain,
            args=(
                process.stdout,
                MAX_CLI_STDOUT_BYTES,
                stdout_chunks,
                overflow,
                process,
            ),
            daemon=True,
        ),
        threading.Thread(
            target=_drain,
            args=(
                process.stderr,
                MAX_CLI_STDERR_BYTES,
                stderr_chunks,
                overflow,
                process,
            ),
            daemon=True,
        ),
    ]
    for reader in readers:
        reader.start()
    writer = threading.Thread(
        target=_write_stdin,
        args=(process, invocation.stdin),
        daemon=True,
    )
    writer.start()

    try:
        process.wait(timeout=invocation.timeout)
    except subprocess.TimeoutExpired as error:
        _terminate(process)
        raise ProviderTimeoutError(f"the {spec.name} exceeded its bounded timeout") from error
    finally:
        writer.join(timeout=5)
        for reader in readers:
            reader.join(timeout=5)
    if overflow.is_set():
        _terminate(process)
        raise TooLargeResponseError(f"the {spec.name} produced more output than the bounded size")
    status = process.returncode
    return CliRun(
        status=status if status is not None and status >= 0 else None,
        stdout=b"".join(stdout_chunks),
        stderr=b"".join(stderr_chunks),
    )


def _terminate(process: subprocess.Popen[bytes]) -> None:
    """Kill the whole process tree and reap the direct child."""
    _kill_tree(process)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:  # pragma: no cover - the child is gone
        pass


def failure_excerpt(data: bytes) -> str:
    """Return the bounded, control-character-free excerpt of a CLI failure."""
    text = data.decode("utf-8", errors="replace")
    out: list[str] = []
    for character in text:
        if len(out) >= MAX_CLI_MESSAGE_LENGTH:
            break
        if unicodedata.category(character) == "Cc" and character not in "\n\t":
            out.append(" ")
            continue
        out.append(character)
    return "".join(out).strip()


def failure_text(text: str) -> str:
    """Return the bounded, control-character-free excerpt of a CLI failure text."""
    return failure_excerpt(text.encode("utf-8", errors="replace"))


def sanitize_version(data: bytes) -> str:
    """Return the bounded version token of a CLI's own version output.

    Only a token that looks like a version survives, so a CLI that prints
    something unexpected cannot smuggle text into a response. The first
    version-shaped word of the first line wins; a CLI that prints no
    version at all yields an empty token.
    """
    text = data.decode("utf-8", errors="replace")
    first_line = text.splitlines()[0] if text.splitlines() else ""
    fallback = ""
    for token in first_line.split():
        clean = "".join(
            character
            for character in token
            if character.isascii() and (character.isalnum() or character in "._+-")
        )
        if not clean:
            continue
        if clean[0].isdigit():
            return clean[:64]
        if not fallback:
            fallback = clean[:64]
    return fallback


@dataclass(frozen=True, slots=True)
class _CachedProbe:
    """One cached probe verdict."""

    fetched_at: float
    program: str | None
    probe: CliProbe


_PROBE_CACHE: dict[str, _CachedProbe] = {}
_PROBE_REFRESHING: set[tuple[str, str | None]] = set()
_PROBE_LOCK = threading.Lock()


def clear_probe_cache() -> None:
    """Drop every cached probe verdict.

    The tests call this so one case never sees another's verdict.
    """
    with _PROBE_LOCK:
        _PROBE_CACHE.clear()
        _PROBE_REFRESHING.clear()


def _store_probe(spec: CliSpec, program: str | None, verdict: CliProbe) -> None:
    """Store one verdict and mark its refresh finished."""
    with _PROBE_LOCK:
        _PROBE_CACHE[spec.id] = _CachedProbe(
            fetched_at=time.monotonic(),
            program=program,
            probe=verdict,
        )
        _PROBE_REFRESHING.discard((spec.id, program))


def _refresh(spec: CliSpec, environ: Mapping[str, str], program: str | None) -> None:
    """Take one verdict in the background and store it."""
    try:
        verdict = _probe_with_deadline(spec, environ, program)
    except Exception:  # a background probe must never escape its thread
        with _PROBE_LOCK:
            _PROBE_REFRESHING.discard((spec.id, program))
        return
    _store_probe(spec, program, verdict)


def _start_refresh(spec: CliSpec, environ: Mapping[str, str], program: str | None) -> None:
    """Start one background refresh when none is running for this CLI/program.

    The program is part of the key so an operator who replaces a CLI (a
    different resolved path) does not wait for the previous environment's
    refresh to drain.
    """
    with _PROBE_LOCK:
        if (spec.id, program) in _PROBE_REFRESHING:
            return
        _PROBE_REFRESHING.add((spec.id, program))
    thread = threading.Thread(
        target=_refresh,
        args=(spec, dict(environ), program),
        name=f"kala-cli-probe-{spec.id}",
        daemon=True,
    )
    thread.start()


def probe_cached(spec: CliSpec, environ: Mapping[str, str]) -> CliProbe:
    """Return the cached verdict of one CLI, or the honest "checking" row.

    A cached verdict is returned at once, even when it is stale; a stale or
    missing verdict starts a background refresh, so no route ever waits for
    a CLI process. The refresh is capped by
    :data:`CLI_PROBE_TIMEOUT_SECONDS` and kills its own process group.
    """
    program = resolve_program(spec, environ)
    now = time.monotonic()
    stale: CliProbe | None = None
    with _PROBE_LOCK:
        entry = _PROBE_CACHE.get(spec.id)
        if entry is not None and entry.program == program:
            if now - entry.fetched_at < CLI_PROBE_TTL_SECONDS:
                return entry.probe
            stale = entry.probe
    _start_refresh(spec, environ, program)
    return stale if stale is not None else checking_probe(spec)


def probe_subscription_clis_cached(
    specs: Sequence[CliSpec],
    environ: Mapping[str, str],
    budget: float = PROBE_ROUTE_BUDGET_SECONDS,
) -> tuple[CliProbe, ...]:
    """Return the verdicts of ``specs``, waiting at most ``budget``.

    The first call starts the refreshes and waits up to the route budget for
    them: a normal CLI answers inside that window, and a hung one leaves the
    row in the honest "checking" state instead of blocking the response.
    Later calls answer from the cache.
    """
    probes = tuple(probe_cached(spec, environ) for spec in specs)
    deadline = time.monotonic() + max(0.0, budget)
    while any(probe.checking for probe in probes) and time.monotonic() < deadline:
        time.sleep(PROBE_POLL_SECONDS)
        probes = tuple(probe_cached(spec, environ) for spec in specs)
    return probes


def probe(spec: CliSpec, environ: Mapping[str, str]) -> CliProbe:
    """Return the availability probe of one subscription CLI.

    The probe runs the CLI's version command and, when the CLI has one, its
    authentication-status command. Only the exit status of the status command
    is read: the CLI's own text never leaves this function, so no account
    name, address or token can reach a response.

    The verdict is cached for :data:`CLI_PROBE_TTL_SECONDS`: the provider
    list is polled by the Studio UI, and the CLI must not be launched on
    every poll. This is the blocking form; the routes use
    :func:`probe_cached`, which never waits for a CLI.
    """
    program = resolve_program(spec, environ)
    now = time.monotonic()
    with _PROBE_LOCK:
        entry = _PROBE_CACHE.get(spec.id)
        if (
            entry is not None
            and now - entry.fetched_at < CLI_PROBE_TTL_SECONDS
            and entry.program == program
        ):
            return entry.probe
    verdict = _probe_with_deadline(spec, environ, program)
    _store_probe(spec, program, verdict)
    return verdict


def _probe_with_deadline(
    spec: CliSpec,
    environ: Mapping[str, str],
    program: str | None,
) -> CliProbe:
    """Take one probe under the per-probe cap.

    A CLI that never answers leaves a resolved binary with no verdict; the
    honest answer is "installed, but its availability check did not finish",
    never "unavailable" and never a guess that it is signed in.
    """
    if program is None:
        return _probe_uncached(spec, environ, None)
    started = time.monotonic()
    verdict = _probe_uncached(spec, environ, program, deadline=started + CLI_PROBE_TIMEOUT_SECONDS)
    if verdict is None:
        return CliProbe(
            id=spec.id,
            installed=True,
            authenticated=False,
            checking=False,
            reason=(
                f"the {spec.name} is installed but its availability check did not "
                f"finish within {CLI_PROBE_TIMEOUT_SECONDS} seconds"
            ),
        )
    return verdict


def _probe_uncached(
    spec: CliSpec,
    environ: Mapping[str, str],
    program: str | None,
    deadline: float | None = None,
) -> CliProbe | None:
    """Take one probe verdict without consulting the cache."""
    if program is None:
        return CliProbe(
            id=spec.id,
            installed=False,
            authenticated=False,
            reason=f"the {spec.name} was not found on PATH or at {spec.path_override_env}",
        )
    import tempfile

    try:
        workdir = tempfile.TemporaryDirectory(prefix="kala-cli-")
    except OSError:
        return CliProbe(
            id=spec.id,
            installed=False,
            authenticated=False,
            reason=f"a private working directory for the {spec.name} could not be created",
        )
    with workdir:
        try:
            version = _probe_run(spec, program, spec.version_args, environ, workdir.name)
        except ProviderTimeoutError:
            return CliProbe(
                id=spec.id,
                installed=True,
                authenticated=False,
                checking=False,
                reason=(
                    f"the {spec.name} is installed but did not answer its version check "
                    f"within {CLI_PROBE_TIMEOUT_SECONDS} seconds"
                ),
            )
        except (ProviderSubprocessError, TooLargeResponseError):
            return CliProbe(
                id=spec.id,
                installed=False,
                authenticated=False,
                reason=f"the {spec.name} did not report a usable version: it could not be started",
            )
        if not version.succeeded():
            return CliProbe(
                id=spec.id,
                installed=False,
                authenticated=False,
                reason=(
                    f"the {spec.name} did not report a usable version: it exited "
                    f"with status {version.status if version.status is not None else -1}"
                ),
            )
        token = sanitize_version(version.stdout)
        label = spec.name if not token else f"{spec.name} {token}"
        if deadline is not None and time.monotonic() >= deadline:
            # The version answered but the cap is spent; do not start a
            # second command that could run another full budget.
            return None
        try:
            auth = _probe_run(spec, program, spec.auth_args, environ, workdir.name)
        except (ProviderSubprocessError, ProviderTimeoutError, TooLargeResponseError):
            auth = None
        authenticated = auth is not None and auth.succeeded()
        reason = (
            f"{label} is installed and signed in"
            if authenticated
            else f"{label} is installed but reports that it is not signed in"
        )
        return CliProbe(
            id=spec.id,
            installed=True,
            authenticated=authenticated,
            reason=reason,
        )


def _probe_run(
    spec: CliSpec,
    program: str,
    argv: Sequence[str],
    environ: Mapping[str, str],
    workdir: str,
) -> CliRun:
    """Run one probe command and return its run.

    The provider errors propagate: a timeout is a different fact from a
    binary that cannot be started, and the row must say which one happened.
    """
    return run(
        CliInvocation(
            spec=spec,
            program=program,
            argv=tuple(argv),
            stdin=b"",
            timeout=float(CLI_PROBE_TIMEOUT_SECONDS),
            environ=environ,
            workdir=Path(workdir),
        )
    )


def consent_message(provider_id: str) -> str:
    """Return the refusal the route shows when the operator did not consent."""
    return f"provider={provider_id} requires allow_remote=true at request time"


__all__ = (
    "BUSY_START_BACKOFF_SECONDS",
    "CLI_PROBE_TIMEOUT_SECONDS",
    "CLI_PROBE_TTL_SECONDS",
    "CLI_TIMEOUT_ENV",
    "DEFAULT_CLI_TIMEOUT_SECONDS",
    "DEFAULT_PATHEXT",
    "ENV_ALLOWLIST",
    "MAX_BUSY_START_ATTEMPTS",
    "MAX_CLI_MESSAGE_LENGTH",
    "MAX_CLI_STDERR_BYTES",
    "MAX_CLI_STDOUT_BYTES",
    "MAX_CLI_TIMEOUT_SECONDS",
    "MIN_CLI_TIMEOUT_SECONDS",
    "PROBE_CHECKING_REASON",
    "PROBE_POLL_SECONDS",
    "PROBE_ROUTE_BUDGET_SECONDS",
    "CliInvocation",
    "CliProbe",
    "CliRun",
    "CliSpec",
    "PathSearch",
    "checking_probe",
    "child_env",
    "clear_probe_cache",
    "cmd_shim_argv",
    "consent_message",
    "failure_excerpt",
    "failure_text",
    "is_batch_shim",
    "is_executable",
    "parse_pathext",
    "probe",
    "probe_cached",
    "probe_subscription_clis_cached",
    "program_or_reason",
    "resolve_program",
    "resolve_program_with",
    "run",
    "sanitize_version",
    "timeout_seconds",
)
