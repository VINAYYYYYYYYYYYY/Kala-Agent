from __future__ import annotations

import ctypes
import json
import os
import subprocess
import sys
import sysconfig
import tempfile
import time
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Final, cast

CLONE_NEWNS: Final[int] = 0x00020000
CLONE_NEWUTS: Final[int] = 0x04000000
CLONE_NEWIPC: Final[int] = 0x08000000
CLONE_NEWUSER: Final[int] = 0x10000000
CLONE_NEWPID: Final[int] = 0x20000000
CLONE_NEWNET: Final[int] = 0x40000000

MS_RDONLY: Final[int] = 1
MS_NOSUID: Final[int] = 2
MS_NODEV: Final[int] = 4
MS_NOEXEC: Final[int] = 8
MS_REMOUNT: Final[int] = 32
MS_BIND: Final[int] = 4_096
MS_REC: Final[int] = 16_384
MS_PRIVATE: Final[int] = 1 << 18

ISOLATION_FAILURE_EXIT: Final[int] = 121
PROBE_FAILURE_EXIT: Final[int] = 122
PROBE_TIMEOUT_SECONDS: Final[float] = 30.0
_PROBE_ENTRYPOINT: Final[str] = (
    "import json,sys;"
    "from kala_agent.tools.sandbox import run_probe;"
    "sys.exit(run_probe(sys.argv[1], sys.argv[2]))"
)
_MAX_PROBE_DETAIL: Final[int] = 480
_CANARY_PAYLOAD: Final[bytes] = b"kala-sandbox-canary"
_MIN_DEVICE_BYTES: Final[int] = 1 << 20

STRICT_REQUIRED_CONTROLS: Final[frozenset[str]] = frozenset(
    {
        "chroot_workspace_only",
        "mount_namespace",
        "network_isolation",
        "readonly_system_bindings",
        "user_namespace",
        "workspace_read_write",
    }
)
STRICT_OPTIONAL_CONTROLS: Final[frozenset[str]] = frozenset(
    {
        "device_nodes_minimal",
        "ipc_namespace",
        "pid_namespace",
        "procfs",
        "scratch_tmpfs",
        "uts_namespace",
    }
)
_NAMESPACE_CONTROLS: Final[Mapping[str, int]] = {
    "user_namespace": CLONE_NEWUSER,
    "network_isolation": CLONE_NEWNET,
    "mount_namespace": CLONE_NEWNS,
    "ipc_namespace": CLONE_NEWIPC,
    "uts_namespace": CLONE_NEWUTS,
    "pid_namespace": CLONE_NEWPID,
}
_UNSHARE_ORDER: Final[tuple[str, ...]] = (
    "user_namespace",
    "network_isolation",
    "mount_namespace",
    "ipc_namespace",
    "uts_namespace",
)
_DEVICE_NODES: Final[tuple[str, ...]] = (
    "/dev/full",
    "/dev/null",
    "/dev/random",
    "/dev/urandom",
    "/dev/zero",
)
_SCRATCH_PATHS: Final[tuple[str, ...]] = ("/dev/shm", "/tmp")
_OPTIONAL_SYSTEM_DIRECTORIES: Final[tuple[str, ...]] = (
    "/bin",
    "/lib",
    "/lib32",
    "/lib64",
    "/sbin",
    "/usr",
)
_OPTIONAL_SYSTEM_FILES: Final[tuple[str, ...]] = (
    "/etc/ld.so.cache",
    "/etc/localtime",
    "/etc/nsswitch.conf",
)


class SandboxError(RuntimeError):
    pass


class SandboxSetupError(SandboxError):
    pass


class SandboxProbeError(SandboxError):
    pass


class IsolationProfile(StrEnum):
    PORTABLE = "portable"
    STRICT_LINUX = "strict_linux"


def _load_libc() -> ctypes.CDLL:
    for candidate in ("libc.so.6", "libc.so", "libc.musl-x86_64.so.1", "libSystem.dylib"):
        try:
            return ctypes.CDLL(candidate, use_errno=True)
        except OSError:
            continue
    raise SandboxSetupError("the C library could not be loaded for namespace isolation")


_LIBC: Final[ctypes.CDLL] = _load_libc()
_LIBC.mount.argtypes = (
    ctypes.c_char_p,
    ctypes.c_char_p,
    ctypes.c_char_p,
    ctypes.c_ulong,
    ctypes.c_char_p,
)
_LIBC.mount.restype = ctypes.c_int
_LIBC.unshare.argtypes = (ctypes.c_int,)
_LIBC.unshare.restype = ctypes.c_int
_LIBC.chroot.argtypes = (ctypes.c_char_p,)
_LIBC.chroot.restype = ctypes.c_int
_LIBC.chdir.argtypes = (ctypes.c_char_p,)
_LIBC.chdir.restype = ctypes.c_int
_LIBC.socket.argtypes = (ctypes.c_int, ctypes.c_int, ctypes.c_int)
_LIBC.socket.restype = ctypes.c_int
_LIBC.connect.argtypes = (ctypes.c_int, ctypes.c_void_p, ctypes.c_size_t)
_LIBC.connect.restype = ctypes.c_int
_LIBC.close.argtypes = (ctypes.c_int,)
_LIBC.close.restype = ctypes.c_int
_LIBC.umount2.argtypes = (ctypes.c_char_p, ctypes.c_int)
_LIBC.umount2.restype = ctypes.c_int


class _SockAddrIn(ctypes.Structure):
    _fields_ = (
        ("sin_family", ctypes.c_ushort),
        ("sin_port", ctypes.c_ushort),
        ("sin_addr", ctypes.c_ubyte * 4),
        ("sin_zero", ctypes.c_ubyte * 8),
    )


@dataclass(frozen=True, slots=True)
class SandboxProbe:
    """The outcome of a preflight probe of the strict profile.

    ``controls`` only ever contains controls that the probe established and
    verified inside a child process, so a profile can advertise exactly what
    this host actually enforces.
    """

    available: bool
    platform: str
    controls: tuple[str, ...] = ()
    detail: str = ""
    elapsed_ms: float = 0.0

    def to_payload(self) -> dict[str, object]:
        return {
            "available": self.available,
            "controls": list(self.controls),
            "detail": self.detail,
            "elapsed_ms": round(self.elapsed_ms, 3),
            "platform": self.platform,
        }


@dataclass(frozen=True, slots=True)
class SandboxSpec:
    """A resolved, minimal description of the jail a child must build."""

    controls: frozenset[str]
    root: Path
    workspace: Path
    readonly_bindings: tuple[str, ...]
    writable_bindings: tuple[str, ...]
    device_bindings: tuple[str, ...]
    scratch_paths: tuple[str, ...]
    scratch_bytes: int = _MIN_DEVICE_BYTES

    def to_payload(self) -> dict[str, object]:
        return {
            "controls": sorted(self.controls),
            "device_bindings": list(self.device_bindings),
            "readonly_bindings": list(self.readonly_bindings),
            "root": self.root.as_posix(),
            "scratch_bytes": self.scratch_bytes,
            "scratch_paths": list(self.scratch_paths),
            "writable_bindings": list(self.writable_bindings),
            "workspace": self.workspace.as_posix(),
        }

    @classmethod
    def from_payload(cls, payload: Mapping[str, object]) -> SandboxSpec:
        try:
            return cls(
                controls=frozenset(cast("Sequence[str]", payload["controls"])),
                root=Path(cast(str, payload["root"])),
                workspace=Path(cast(str, payload["workspace"])),
                readonly_bindings=tuple(cast("Sequence[str]", payload["readonly_bindings"])),
                writable_bindings=tuple(cast("Sequence[str]", payload["writable_bindings"])),
                device_bindings=tuple(cast("Sequence[str]", payload["device_bindings"])),
                scratch_paths=tuple(cast("Sequence[str]", payload["scratch_paths"])),
                scratch_bytes=int(cast(int, payload["scratch_bytes"])),
            )
        except (KeyError, TypeError, ValueError) as error:
            raise SandboxSetupError("the sandbox specification is malformed") from error


def platform_name() -> str:
    return (
        f"{sys.platform}-{os.uname().machine}" if sys.platform.startswith("linux") else sys.platform
    )


def child_resource_paths() -> tuple[str, ...]:
    """The host paths a jailed interpreter needs in order to start at all."""
    candidates: list[str] = []
    executable = os.path.realpath(sys.executable)
    for value in (
        sys.executable,
        executable,
        os.path.dirname(executable),
        os.path.dirname(sys.executable),
        sys.prefix,
        sys.base_prefix,
        sys.exec_prefix,
        sys.base_exec_prefix,
        os.path.dirname(os.__file__),
    ):
        if isinstance(value, str) and value and value not in candidates:
            candidates.append(value)
    for name in ("stdlib", "platstdlib", "purelib", "platlib"):
        try:
            value = sysconfig.get_paths().get(name, "")
        except (KeyError, OSError):  # pragma: no cover - defensive
            value = ""
        if isinstance(value, str) and value and value not in candidates:
            candidates.append(value)
    return tuple(candidates)


def resolve_system_bindings(import_roots: Sequence[Path]) -> tuple[str, ...]:
    """Return the read-only host paths a jailed worker must be able to reach."""
    resolved: list[str] = []
    candidates = (*child_resource_paths(), *(str(root) for root in import_roots))
    for candidate in candidates:
        target = Path(candidate)
        if not target.exists():
            continue
        for path in (target, target.resolve(strict=False)):
            text = path.as_posix()
            if text not in resolved:
                resolved.append(text)
    for candidate in _OPTIONAL_SYSTEM_DIRECTORIES:
        if Path(candidate).is_dir() and candidate not in resolved:
            resolved.append(candidate)
    for candidate in _OPTIONAL_SYSTEM_FILES:
        if Path(candidate).is_file() and candidate not in resolved:
            resolved.append(candidate)
    return tuple(resolved)


def build_sandbox_spec(
    *,
    controls: frozenset[str],
    root: Path,
    workspace: Path,
    import_roots: Sequence[Path] = (),
    scratch_bytes: int = _MIN_DEVICE_BYTES,
) -> SandboxSpec:
    missing = sorted(STRICT_REQUIRED_CONTROLS - controls)
    if missing:
        raise SandboxSetupError(
            "the strict profile requires controls that were not probed: " + ", ".join(missing)
        )
    if not str(root).startswith("/") or not str(workspace).startswith("/"):
        raise SandboxSetupError("sandbox paths must be absolute")
    return SandboxSpec(
        controls=controls,
        root=root,
        workspace=workspace,
        readonly_bindings=resolve_system_bindings(import_roots),
        writable_bindings=(workspace.resolve(strict=False).as_posix(),),
        device_bindings=tuple(candidate for candidate in _DEVICE_NODES if Path(candidate).exists()),
        scratch_paths=tuple(_SCRATCH_PATHS),
        scratch_bytes=scratch_bytes,
    )


def _libc_error(operation: str) -> SandboxSetupError:
    code = ctypes.get_errno()
    return SandboxSetupError(f"{operation} failed with errno {code} ({os.strerror(code)})")


def _unshare(flag: int, name: str) -> None:
    if _LIBC.unshare(ctypes.c_int(flag)) != 0:
        raise _libc_error(f"unshare({name})")


def _mount(
    source: str | None,
    target: str,
    filesystem: str | None,
    flags: int,
    data: str | None = None,
) -> None:
    source_bytes = b"" if source is None else source.encode()
    target_bytes = target.encode()
    type_bytes = b"" if filesystem is None else filesystem.encode()
    data_bytes = None if data is None else data.encode()
    if _LIBC.mount(source_bytes, target_bytes, type_bytes, ctypes.c_ulong(flags), data_bytes) != 0:
        raise _libc_error(f"mount({target})")


def _make_mountpoint(root: Path, target: str, *, directory: bool) -> Path:
    mountpoint = root / target.lstrip("/")
    try:
        if directory:
            mountpoint.mkdir(mode=0o700, parents=True, exist_ok=True)
        else:
            mountpoint.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            if not mountpoint.exists():
                mountpoint.touch(mode=0o600)
    except OSError as error:
        raise SandboxSetupError(
            f"the sandbox root cannot hold {target}: {error.strerror} ({error.filename})"
        ) from error
    return mountpoint


def _bind_readonly(root: Path, target: str) -> None:
    source = Path(target)
    if not source.exists():
        raise SandboxSetupError(f"the read-only binding {target} does not exist")
    mountpoint = _make_mountpoint(root, target, directory=source.is_dir())
    _mount(target, mountpoint.as_posix(), None, MS_BIND | MS_REC)
    _mount(
        None,
        mountpoint.as_posix(),
        None,
        MS_BIND | MS_REMOUNT | MS_RDONLY | MS_NOSUID | MS_NODEV,
    )


def _bind_read_write(root: Path, target: str) -> None:
    source = Path(target)
    if not source.is_dir():
        raise SandboxSetupError(f"the writable binding {target} is not a directory")
    mountpoint = _make_mountpoint(root, target, directory=True)
    _mount(target, mountpoint.as_posix(), None, MS_BIND | MS_REC)
    _mount(None, mountpoint.as_posix(), None, MS_BIND | MS_REMOUNT | MS_NOSUID | MS_NODEV)


def _bind_device(root: Path, target: str) -> None:
    mountpoint = _make_mountpoint(root, target, directory=False)
    _mount(target, mountpoint.as_posix(), None, MS_BIND)
    _mount(None, mountpoint.as_posix(), None, MS_BIND | MS_REMOUNT | MS_NOSUID | MS_NODEV)


def _unshadowed_scratch_paths(spec: SandboxSpec) -> tuple[str, ...]:
    """Drop scratch paths that would hide a required binding from the child."""
    reserved = (*spec.readonly_bindings, *spec.writable_bindings)
    return tuple(
        candidate
        for candidate in spec.scratch_paths
        if not any(
            binding == candidate or binding.startswith(f"{candidate}/") for binding in reserved
        )
    )


def _mount_scratch_set(root: Path, targets: tuple[str, ...], scratch_bytes: int) -> bool:
    mounted: list[str] = []
    for target in targets:
        try:
            _mount_scratch(root, target, scratch_bytes)
        except SandboxSetupError:
            continue
        mounted.append(target)
    if not mounted:
        return False
    if _scratch_is_writable(mounted):
        return True
    for target in mounted:
        try:
            _LIBC.umount2((root / target.lstrip("/")).as_posix().encode(), 0)
        except OSError:
            continue
    return False


def _mount_scratch(root: Path, target: str, scratch_bytes: int) -> None:
    mountpoint = _make_mountpoint(root, target, directory=True)
    size = f"size={max(int(scratch_bytes), 0)}"
    _mount("tmpfs", mountpoint.as_posix(), "tmpfs", MS_NOSUID | MS_NODEV | MS_NOEXEC, size)
    _mount(
        None,
        mountpoint.as_posix(),
        "tmpfs",
        MS_REMOUNT | MS_NOSUID | MS_NODEV | MS_NOEXEC,
        size,
    )


def _exit_code_from_status(status: int) -> int:
    if os.WIFSIGNALED(status):
        return 128 + os.WTERMSIG(status)
    if os.WIFEXITED(status):
        return os.WEXITSTATUS(status)
    return 1


def _close_inherited_fds() -> None:
    """Drop every descriptor above the standard streams in the forked child."""
    try:
        entries = os.listdir("/proc/self/fd")
    except OSError:
        os.closerange(3, 4_096)
        return
    for entry in entries:
        try:
            descriptor = int(entry)
        except ValueError:  # pragma: no cover - defensive
            continue
        if descriptor > 2:
            try:
                os.close(descriptor)
            except OSError:  # pragma: no cover - racing close
                continue


def _enter_pid_namespace() -> None:
    """Re-parent the worker so the kernel reaps its whole process namespace.

    ``CLONE_NEWPID`` only takes effect for children, so the pre-exec child
    forks once more and becomes PID 1 inside the new namespace. The
    intermediate process waits for that PID 1, releases the descriptors the
    supervisor uses to track the direct child, and mirrors the exit status so
    the supervisor still reaps a single process.
    """
    _unshare(CLONE_NEWPID, "pid_namespace")
    worker = os.fork()
    if worker == 0:
        return
    _close_inherited_fds()
    _, status = os.waitpid(worker, 0)
    os._exit(min(255, _exit_code_from_status(status)))


def isolate_process(spec: SandboxSpec) -> tuple[str, ...]:
    """Establish the requested controls in the calling process.

    Raises :class:`SandboxSetupError` with a short sanitized reason as soon as
    a control cannot be established; the caller must then fail closed.
    """
    established: list[str] = []
    for name in _UNSHARE_ORDER:
        if name not in spec.controls:
            continue
        _unshare(_NAMESPACE_CONTROLS[name], name)
        established.append(name)
    if "pid_namespace" in spec.controls:
        try:
            _enter_pid_namespace()
        except (OSError, SandboxSetupError):
            pass
        else:
            established.append("pid_namespace")
    if "mount_namespace" in spec.controls:
        _mount(None, "/", None, MS_REC | MS_PRIVATE)
    if "device_nodes_minimal" in spec.controls:
        try:
            for target in spec.device_bindings:
                _bind_device(spec.root, target)
        except SandboxSetupError:
            pass
        else:
            established.append("device_nodes_minimal")
    for target in spec.readonly_bindings:
        _bind_readonly(spec.root, target)
    for target in spec.writable_bindings:
        _bind_read_write(spec.root, target)
    if "scratch_tmpfs" in spec.controls:
        targets = _unshadowed_scratch_paths(spec)
        if _mount_scratch_set(spec.root, targets, spec.scratch_bytes):
            established.append("scratch_tmpfs")
    if "procfs" in spec.controls:
        mountpoint = _make_mountpoint(spec.root, "/proc", directory=True)
        try:
            _mount("proc", mountpoint.as_posix(), "proc", MS_NOSUID | MS_NODEV | MS_NOEXEC)
        except SandboxSetupError:
            pass
        else:
            established.append("procfs")
    if _LIBC.chroot(spec.root.as_posix().encode()) != 0:
        raise _libc_error("chroot")
    if _LIBC.chdir(b"/") != 0:
        raise _libc_error("chdir to the sandbox root")
    if _LIBC.chdir(spec.workspace.as_posix().encode()) != 0:
        raise _libc_error("chdir into the workspace")
    return tuple(established)


def _network_is_isolated() -> bool:
    """Confirm the child has no route to any address outside its namespace."""
    handle = _LIBC.socket(ctypes.c_int(2), ctypes.c_int(1), ctypes.c_int(0))
    if handle < 0:
        return False
    try:
        address = _SockAddrIn()
        address.sin_family = 2
        address.sin_port = int.from_bytes((53).to_bytes(2, "big"), "big")
        address.sin_addr[:] = bytes((8, 8, 8, 8))
        return int(_LIBC.connect(handle, ctypes.byref(address), ctypes.c_size_t(16))) != 0
    finally:
        _LIBC.close(handle)


def _device_list_is_minimal() -> bool:
    try:
        listing = Path("/proc/net/dev").read_text(encoding="utf-8", errors="replace")
    except OSError:
        return True
    names = {line.split(":", 1)[0].strip() for line in listing.splitlines()[2:] if ":" in line}
    return names <= {"lo"}


def _verify_controls(
    spec: SandboxSpec,
    canary: str,
    workspace: str,
) -> tuple[tuple[str, ...], tuple[str, ...]]:
    """Return the controls that hold inside the jail and the ones that do not."""
    verified: list[str] = []
    failed: list[str] = []
    for name in sorted(STRICT_REQUIRED_CONTROLS | STRICT_OPTIONAL_CONTROLS):
        if name not in spec.controls:
            continue
        if name == "chroot_workspace_only":
            holds = _canary_is_invisible(canary)
        elif name == "network_isolation":
            holds = _network_is_isolated()
        elif name == "workspace_read_write":
            holds = _workspace_is_writable(workspace)
        elif name == "readonly_system_bindings":
            holds = _system_bindings_are_readonly()
        elif name == "pid_namespace":
            holds = os.getpid() == 1
        elif name in {"ipc_namespace", "uts_namespace", "user_namespace", "mount_namespace"}:
            holds = True
        elif name == "procfs":
            holds = _procfs_is_namespaced()
        elif name == "device_nodes_minimal":
            holds = _device_list_is_minimal()
        elif name == "scratch_tmpfs":
            holds = _scratch_is_writable()
        else:  # pragma: no cover - the control table is closed
            holds = False
        (verified if holds else failed).append(name)
    return tuple(verified), tuple(failed)


def _canary_is_invisible(canary: str) -> bool:
    try:
        Path(canary).read_bytes()
    except OSError:
        return True
    return False


def _workspace_is_writable(workspace: str) -> bool:
    probe = Path(workspace) / ".sandbox-write-probe"
    try:
        probe.write_bytes(_CANARY_PAYLOAD)
        probe.unlink()
    except OSError:
        return False
    return True


def _system_bindings_are_readonly() -> bool:
    system = Path("/usr")
    probe = system / ".sandbox-write-probe"
    if not system.is_dir():
        return False
    try:
        probe.write_bytes(b"")
    except OSError:
        return True
    probe.unlink(missing_ok=True)
    return False


def _procfs_is_namespaced() -> bool:
    try:
        listing = Path("/proc/1/stat").read_text(encoding="utf-8", errors="replace")
    except OSError:
        return False
    return listing.strip() != ""


def _scratch_is_writable(candidates: Sequence[str] = _SCRATCH_PATHS) -> bool:
    for candidate in candidates:
        try:
            probe = Path(candidate) / ".sandbox-scratch-probe"
            probe.write_bytes(b"")
            probe.unlink()
        except OSError:
            return False
    return True


def _probe_entry(spec_payload: str, canary_payload: str) -> tuple[SandboxSpec, str]:
    try:
        decoded = json.loads(spec_payload)
        spec = SandboxSpec.from_payload(decoded)
        canary = str(json.loads(canary_payload))
    except (json.JSONDecodeError, TypeError, ValueError) as error:
        raise SandboxProbeError("the probe payload is malformed") from error
    return spec, canary


def run_probe(spec_payload: str, canary_payload: str) -> int:
    """Build the strict jail in this process and report what really worked."""
    try:
        spec, canary = _probe_entry(spec_payload, canary_payload)
    except SandboxProbeError as error:
        _emit_probe_result((), (), str(error))
        return PROBE_FAILURE_EXIT
    try:
        isolate_process(spec)
    except SandboxSetupError as error:
        _emit_probe_result((), (), str(error))
        return PROBE_FAILURE_EXIT
    except OSError as error:
        _emit_probe_result((), (), f"the probe could not build the jail: {type(error).__name__}")
        return PROBE_FAILURE_EXIT
    verified, _unverified = _verify_controls(spec, canary, spec.workspace.as_posix())
    unavailable = sorted(set(spec.controls) - set(verified))
    detail = "" if not unavailable else "not enforced: " + ", ".join(unavailable)
    if not STRICT_REQUIRED_CONTROLS <= set(verified):
        return PROBE_FAILURE_EXIT
    _emit_probe_result(verified, (), detail)
    return 0


def _emit_probe_result(
    verified: tuple[str, ...],
    _unused: tuple[str, ...],
    detail: str,
) -> None:
    payload = {
        "controls": sorted(verified),
        "detail": detail[:_MAX_PROBE_DETAIL],
    }
    sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
    sys.stdout.flush()


def _unavailable(reason: str, *, elapsed_ms: float = 0.0) -> SandboxProbe:
    return SandboxProbe(
        available=False,
        platform=platform_name(),
        controls=(),
        detail=reason[:_MAX_PROBE_DETAIL],
        elapsed_ms=elapsed_ms,
    )


def probe_sandbox(
    *,
    import_roots: Sequence[Path] = (),
    scratch_bytes: int = _MIN_DEVICE_BYTES,
    python_executable: str | None = None,
) -> SandboxProbe:
    """Probe whether the strict Linux profile can be established on this host.

    The probe runs in a throwaway interpreter that builds exactly the jail the
    executor would build and then verifies, from the inside, that the network is
    unreachable, the host filesystem is invisible, and the workspace is
    writable. Only controls that the probe observed are returned.
    """
    if not sys.platform.startswith("linux"):
        return _unavailable("the strict profile requires Linux namespaces")
    interpreter = python_executable if python_executable is not None else sys.executable
    if not interpreter or not Path(interpreter).is_file():
        return _unavailable("no usable interpreter is available for the sandbox probe")
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="kala-sandbox-probe-") as scratch:
            root = Path(scratch) / "root"
            workspace = Path(scratch) / "workspace"
            workspace.mkdir(mode=0o700)
            canary = Path(scratch) / "canary"
            canary.write_bytes(_CANARY_PAYLOAD)
            spec = build_sandbox_spec(
                controls=STRICT_REQUIRED_CONTROLS | STRICT_OPTIONAL_CONTROLS,
                root=root,
                workspace=workspace,
                import_roots=import_roots,
                scratch_bytes=scratch_bytes,
            )
            spec.root.mkdir(mode=0o700, parents=True, exist_ok=True)
            environment = {
                "HOME": workspace.as_posix(),
                "LANG": "C.UTF-8",
                "PATH": "/usr/local/bin:/usr/bin:/bin",
                "PYTHONDONTWRITEBYTECODE": "1",
                "PYTHONHASHSEED": "0",
                "PYTHONIOENCODING": "utf-8",
                "PYTHONPATH": os.pathsep.join(str(item) for item in import_roots),
                "TMPDIR": workspace.as_posix(),
            }
            completed = subprocess.run(
                [
                    interpreter,
                    "-I",
                    "-B",
                    "-c",
                    _PROBE_ENTRYPOINT,
                    json.dumps(spec.to_payload(), ensure_ascii=True),
                    json.dumps(canary.resolve(strict=False).as_posix(), ensure_ascii=True),
                ],
                capture_output=True,
                env=environment,
                cwd=workspace,
                timeout=PROBE_TIMEOUT_SECONDS,
                check=False,
            )
    except subprocess.TimeoutExpired:
        return _unavailable(
            "the sandbox probe did not finish", elapsed_ms=(time.monotonic() - started) * 1_000.0
        )
    except (OSError, SandboxError, ValueError) as error:
        return _unavailable(
            f"the sandbox probe could not start: {type(error).__name__}",
            elapsed_ms=(time.monotonic() - started) * 1_000.0,
        )
    elapsed_ms = (time.monotonic() - started) * 1_000.0
    if completed.returncode != 0:
        return _unavailable("the strict profile is unavailable on this host", elapsed_ms=elapsed_ms)
    try:
        decoded = json.loads(completed.stdout.decode("utf-8", errors="replace").strip() or "{}")
    except (json.JSONDecodeError, AttributeError):
        return _unavailable("the sandbox probe returned no result", elapsed_ms=elapsed_ms)
    controls = frozenset(name for name in decoded.get("controls", []) if isinstance(name, str))
    if not STRICT_REQUIRED_CONTROLS <= controls:
        return _unavailable("a required sandbox control was not verified", elapsed_ms=elapsed_ms)
    return SandboxProbe(
        available=True,
        platform=platform_name(),
        controls=tuple(sorted(controls & (STRICT_REQUIRED_CONTROLS | STRICT_OPTIONAL_CONTROLS))),
        elapsed_ms=elapsed_ms,
    )


def child_pre_exec(spec: SandboxSpec | None) -> None:  # pragma: no cover - runs in the child
    """Build the jail in a freshly forked child before the worker is executed."""
    if spec is None:
        return
    try:
        isolate_process(spec)
    except BaseException as error:
        try:
            os.write(2, f"sandbox_setup_failed: {type(error).__name__}\n".encode())
        except OSError:
            pass
        os._exit(ISOLATION_FAILURE_EXIT)


@dataclass(frozen=True, slots=True)
class SandboxVerdict:
    """The result of asking whether a side effect may proceed."""

    allowed: bool
    reason: str = ""
    controls: tuple[str, ...] = ()

    def to_payload(self) -> dict[str, object]:
        return {"allowed": self.allowed, "controls": list(self.controls), "reason": self.reason}


def judge_sandbox(profile: IsolationProfile, probe: SandboxProbe | None) -> SandboxVerdict:
    """Decide whether the configured profile can back a side-effecting tool."""
    if profile is not IsolationProfile.STRICT_LINUX:
        return SandboxVerdict(
            allowed=True,
            reason="the portable profile confines a process, not the filesystem",
        )
    if probe is None or not probe.available:
        detail = "" if probe is None else probe.detail
        return SandboxVerdict(
            allowed=False,
            reason=(
                "the strict profile is not available on this host, so side effects are "
                f"refused{f': {detail}' if detail else ''}"
            ),
        )
    return SandboxVerdict(allowed=True, controls=probe.controls)


__all__ = (
    "PROBE_FAILURE_EXIT",
    "PROBE_TIMEOUT_SECONDS",
    "STRICT_OPTIONAL_CONTROLS",
    "STRICT_REQUIRED_CONTROLS",
    "IsolationProfile",
    "SandboxError",
    "SandboxProbe",
    "SandboxProbeError",
    "SandboxSetupError",
    "SandboxSpec",
    "SandboxVerdict",
    "build_sandbox_spec",
    "child_pre_exec",
    "child_resource_paths",
    "isolate_process",
    "judge_sandbox",
    "platform_name",
    "probe_sandbox",
    "resolve_system_bindings",
    "run_probe",
)
