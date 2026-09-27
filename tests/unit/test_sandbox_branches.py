from __future__ import annotations

import ctypes
import errno
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any, cast

import pytest

from kala_agent.tools import sandbox as sandbox_module
from kala_agent.tools.sandbox import (
    CLONE_NEWNS,
    MS_BIND,
    MS_NODEV,
    MS_NOEXEC,
    MS_NOSUID,
    MS_PRIVATE,
    MS_RDONLY,
    MS_REC,
    MS_REMOUNT,
    PROBE_FAILURE_EXIT,
    PROBE_TIMEOUT_SECONDS,
    STRICT_OPTIONAL_CONTROLS,
    STRICT_REQUIRED_CONTROLS,
    IsolationProfile,
    SandboxProbe,
    SandboxProbeError,
    SandboxSetupError,
    SandboxSpec,
    SandboxVerdict,
    _bind_device,
    _bind_read_write,
    _bind_readonly,
    _canary_is_invisible,
    _close_inherited_fds,
    _device_list_is_minimal,
    _emit_probe_result,
    _enter_pid_namespace,
    _exit_code_from_status,
    _libc_error,
    _make_mountpoint,
    _mount_scratch,
    _mount_scratch_set,
    _network_is_isolated,
    _probe_entry,
    _procfs_is_namespaced,
    _scratch_is_writable,
    _system_bindings_are_readonly,
    _unavailable,
    _unshadowed_scratch_paths,
    _unshare,
    _verify_controls,
    _workspace_is_writable,
    build_sandbox_spec,
    child_resource_paths,
    isolate_process,
    judge_sandbox,
    platform_name,
    probe_sandbox,
    resolve_system_bindings,
    run_probe,
)


def _flag_value(flag: object) -> int:
    if isinstance(flag, int):
        return flag
    return int(cast("ctypes.c_int | ctypes.c_uint", flag).value)


class _FakeLibc:
    def __init__(
        self,
        *,
        fail: set[str] | None = None,
        errno_value: int = errno.EPERM,
    ) -> None:
        self.fail = set() if fail is None else fail
        self.errno_value = errno_value
        self.calls: list[tuple[str, tuple[Any, ...]]] = []
        self.socket_handle = 7
        self.connect_result = -1

    def unshare(self, flag: object) -> int:
        self.calls.append(("unshare", (_flag_value(flag),)))
        if "unshare" in self.fail:
            ctypes.set_errno(self.errno_value)
            return -1
        return 0

    def mount(
        self,
        source: bytes | None,
        target: bytes,
        filesystem: bytes | None,
        flags: object,
        data: bytes | None,
    ) -> int:
        self.calls.append(
            ("mount", (source, target, filesystem, _flag_value(flags), data)),
        )
        if "mount" in self.fail:
            ctypes.set_errno(self.errno_value)
            return -1
        return 0

    def chroot(self, target: bytes) -> int:
        self.calls.append(("chroot", (target,)))
        if "chroot" in self.fail:
            ctypes.set_errno(self.errno_value)
            return -1
        return 0

    def chdir(self, target: bytes) -> int:
        self.calls.append(("chdir", (target,)))
        if "chdir" in self.fail:
            ctypes.set_errno(self.errno_value)
            return -1
        return 0

    def socket(self, domain: object, kind: object, protocol: object) -> int:
        self.calls.append(
            ("socket", (_flag_value(domain), _flag_value(kind), _flag_value(protocol)))
        )
        if "socket" in self.fail:
            return -1
        return self.socket_handle

    def connect(self, handle: int, address: object, length: int) -> int:
        self.calls.append(("connect", (int(handle),)))
        return self.connect_result

    def close(self, handle: int) -> int:
        self.calls.append(("close", (int(handle),)))
        return 0

    def umount2(self, target: bytes, flags: int) -> int:
        self.calls.append(("umount2", (target, int(flags))))
        if "umount2" in self.fail:
            raise OSError(errno.EBUSY, "busy")
        return 0


def _install(monkeypatch: pytest.MonkeyPatch, fake: _FakeLibc) -> _FakeLibc:
    monkeypatch.setattr(sandbox_module, "_LIBC", cast(Any, fake))
    return fake


def _spec(
    root: Path,
    workspace: Path,
    *,
    controls: frozenset[str] | None = None,
    readonly: tuple[str, ...] = (),
    writable: tuple[str, ...] = (),
    device: tuple[str, ...] = (),
    scratch: tuple[str, ...] = ("/tmp",),
    scratch_bytes: int = 1 << 20,
) -> SandboxSpec:
    return SandboxSpec(
        controls=STRICT_REQUIRED_CONTROLS if controls is None else controls,
        root=root,
        workspace=workspace,
        readonly_bindings=readonly,
        writable_bindings=writable,
        device_bindings=device,
        scratch_paths=scratch,
        scratch_bytes=scratch_bytes,
    )


def test_platform_name_reports_linux_machine() -> None:
    assert platform_name().startswith(sys.platform)
    if sys.platform.startswith("linux"):
        assert platform_name() == f"{sys.platform}-{os.uname().machine}"


def test_sandbox_probe_payload_reports_rounded_elapsed() -> None:
    probe = SandboxProbe(
        available=True,
        platform="linux-x86_64",
        controls=("a", "b"),
        detail="fine",
        elapsed_ms=1.23456,
    )
    payload = probe.to_payload()
    assert payload == {
        "available": True,
        "controls": ["a", "b"],
        "detail": "fine",
        "elapsed_ms": 1.235,
        "platform": "linux-x86_64",
    }


def test_sandbox_probe_payload_defaults_are_empty() -> None:
    payload = SandboxProbe(available=False, platform="darwin").to_payload()
    assert payload["controls"] == []
    assert payload["elapsed_ms"] == 0.0
    assert payload["detail"] == ""


def test_sandbox_spec_payload_roundtrips(tmp_path: Path) -> None:
    spec = _spec(tmp_path / "root", tmp_path / "ws", readonly=("/usr",), writable=("/ws",))
    payload = spec.to_payload()
    assert payload["controls"] == sorted(spec.controls)
    assert payload["root"] == (tmp_path / "root").as_posix()
    assert payload["readonly_bindings"] == ["/usr"]
    assert payload["writable_bindings"] == ["/ws"]
    assert payload["scratch_bytes"] == 1 << 20
    restored = SandboxSpec.from_payload(payload)
    assert restored == spec


def test_sandbox_spec_from_payload_rejects_missing_key() -> None:
    with pytest.raises(SandboxSetupError) as error:
        SandboxSpec.from_payload({"controls": frozenset({"a"})})
    assert "malformed" in str(error.value)


def test_sandbox_spec_from_payload_rejects_wrong_types() -> None:
    payload = _spec(Path("/root"), Path("/ws")).to_payload()
    payload["root"] = 7
    with pytest.raises(SandboxSetupError):
        SandboxSpec.from_payload(payload)


def test_sandbox_spec_from_payload_rejects_non_integer_scratch() -> None:
    payload = _spec(Path("/root"), Path("/ws")).to_payload()
    payload["scratch_bytes"] = "not-a-number"
    with pytest.raises(SandboxSetupError):
        SandboxSpec.from_payload(payload)


def test_child_resource_paths_are_unique_and_resolvable() -> None:
    paths = child_resource_paths()
    assert paths
    assert len(set(paths)) == len(paths)
    assert sys.executable in paths or os.path.realpath(sys.executable) in paths


def test_resolve_system_bindings_drops_absent_paths(tmp_path: Path) -> None:
    present = tmp_path / "pkg"
    present.mkdir()
    bindings = resolve_system_bindings((present, tmp_path / "missing"))
    assert present.resolve().as_posix() in bindings
    assert all(Path(item).exists() for item in bindings)


def test_build_sandbox_spec_requires_all_strict_controls(tmp_path: Path) -> None:
    with pytest.raises(SandboxSetupError) as error:
        build_sandbox_spec(
            controls=STRICT_REQUIRED_CONTROLS - {"user_namespace"},
            root=tmp_path / "root",
            workspace=tmp_path / "ws",
        )
    assert "user_namespace" in str(error.value)


def test_build_sandbox_spec_rejects_relative_paths(tmp_path: Path) -> None:
    with pytest.raises(SandboxSetupError) as error:
        build_sandbox_spec(
            controls=STRICT_REQUIRED_CONTROLS,
            root=Path("relative/root"),
            workspace=tmp_path / "ws",
        )
    assert "absolute" in str(error.value)

    with pytest.raises(SandboxSetupError):
        build_sandbox_spec(
            controls=STRICT_REQUIRED_CONTROLS,
            root=tmp_path / "root",
            workspace=Path("relative/ws"),
        )


def test_build_sandbox_spec_binds_workspace_and_devices(tmp_path: Path) -> None:
    workspace = tmp_path / "ws"
    workspace.mkdir()
    spec = build_sandbox_spec(
        controls=STRICT_REQUIRED_CONTROLS | {"device_nodes_minimal", "scratch_tmpfs"},
        root=tmp_path / "root",
        workspace=workspace,
        scratch_bytes=4096,
    )
    assert spec.writable_bindings == (workspace.resolve(strict=False).as_posix(),)
    assert spec.scratch_paths == ("/dev/shm", "/tmp")
    assert spec.scratch_bytes == 4096
    assert spec.device_bindings == tuple(
        item
        for item in ("/dev/full", "/dev/null", "/dev/random", "/dev/urandom", "/dev/zero")
        if Path(item).exists()
    )


def test_libc_error_reports_errno_text() -> None:
    ctypes.set_errno(errno.ENOENT)
    error = _libc_error("unshare(pid_namespace)")
    assert isinstance(error, SandboxSetupError)
    assert "unshare(pid_namespace)" in str(error)
    assert "2" in str(error)


def test_unshare_raises_on_failure(monkeypatch: pytest.MonkeyPatch) -> None:
    _install(monkeypatch, _FakeLibc(fail={"unshare"}))
    with pytest.raises(SandboxSetupError) as error:
        _unshare(CLONE_NEWNS, "mount_namespace")
    assert "unshare(mount_namespace)" in str(error.value)


def test_unshare_succeeds_silently(monkeypatch: pytest.MonkeyPatch) -> None:
    fake = _install(monkeypatch, _FakeLibc())
    _unshare(CLONE_NEWNS, "mount_namespace")
    assert fake.calls == [("unshare", (CLONE_NEWNS,))]


def test_mount_raises_on_failure(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _install(monkeypatch, _FakeLibc(fail={"mount"}))
    monkeypatch.setattr(sandbox_module, "_unshare", lambda _flag, _name: None)
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=frozenset({"mount_namespace"}))
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    with pytest.raises(SandboxSetupError) as error:
        isolate_process(spec)
    assert "mount(/)" in str(error.value)


def test_make_mountpoint_creates_directory(tmp_path: Path) -> None:
    root = tmp_path / "root"
    mountpoint = _make_mountpoint(root, "/usr/lib", directory=True)
    assert mountpoint == root / "usr/lib"
    assert mountpoint.is_dir()


def test_make_mountpoint_creates_file_and_parents(tmp_path: Path) -> None:
    root = tmp_path / "root"
    mountpoint = _make_mountpoint(root, "/dev/null", directory=False)
    assert mountpoint == root / "dev/null"
    assert mountpoint.is_file()
    second = _make_mountpoint(root, "/dev/null", directory=False)
    assert second == mountpoint


def test_make_mountpoint_reports_oserror(tmp_path: Path) -> None:
    blocker = tmp_path / "root"
    blocker.write_text("not a directory")
    with pytest.raises(SandboxSetupError) as error:
        _make_mountpoint(blocker, "/usr", directory=True)
    assert "cannot hold /usr" in str(error.value)


def test_bind_readonly_requires_existing_source(tmp_path: Path) -> None:
    with pytest.raises(SandboxSetupError) as error:
        _bind_readonly(tmp_path, "/definitely/not/here")
    assert "does not exist" in str(error.value)


def test_bind_readonly_binds_then_remounts_readonly(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = tmp_path / "src"
    source.mkdir()
    fake = _install(monkeypatch, _FakeLibc())
    _bind_readonly(tmp_path / "root", str(source))
    kinds = [name for name, _ in fake.calls]
    assert kinds == ["mount", "mount"]
    first_flags = fake.calls[0][1][3]
    second_flags = fake.calls[1][1][3]
    assert first_flags & MS_BIND and first_flags & MS_REC
    assert second_flags & MS_RDONLY and second_flags & MS_REMOUNT


def test_bind_read_write_requires_directory(tmp_path: Path) -> None:
    plain = tmp_path / "file"
    plain.write_text("x")
    with pytest.raises(SandboxSetupError) as error:
        _bind_read_write(tmp_path / "root", str(plain))
    assert "is not a directory" in str(error.value)


def test_bind_read_write_binds_without_rdonly(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = tmp_path / "ws"
    source.mkdir()
    fake = _install(monkeypatch, _FakeLibc())
    _bind_read_write(tmp_path / "root", str(source))
    assert [name for name, _ in fake.calls] == ["mount", "mount"]
    assert not fake.calls[1][1][3] & MS_RDONLY
    assert fake.calls[0][1][3] & MS_BIND


def test_bind_device_uses_plain_bind(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    fake = _install(monkeypatch, _FakeLibc())
    _bind_device(tmp_path / "root", "/dev/null")
    assert [name for name, _ in fake.calls] == ["mount", "mount"]
    assert fake.calls[0][1][3] == MS_BIND


def test_unshadowed_scratch_paths_drops_shadowed_candidates(tmp_path: Path) -> None:
    spec = _spec(
        tmp_path / "root",
        tmp_path / "ws",
        readonly=("/tmp",),
        writable=("/tmp/work", "/var"),
        scratch=("/dev/shm", "/tmp", "/var"),
    )
    assert _unshadowed_scratch_paths(spec) == ("/dev/shm",)


def test_mount_scratch_set_returns_false_when_all_fail(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _install(monkeypatch, _FakeLibc(fail={"mount"}))
    assert _mount_scratch_set(tmp_path / "root", ("/tmp",), 1 << 20) is False


def test_mount_scratch_set_umounts_when_not_writable(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    fake = _install(monkeypatch, _FakeLibc())
    monkeypatch.setattr(sandbox_module, "_scratch_is_writable", lambda _c: False)
    root = tmp_path / "root"
    assert _mount_scratch_set(root, ("/tmp",), 1 << 20) is False
    assert [name for name, _ in fake.calls].count("umount2") == 1
    assert fake.calls[-1][1][0] == (root / "tmp").as_posix().encode()


def test_mount_scratch_set_survives_umount_errors(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _install(monkeypatch, _FakeLibc(fail={"umount2"}))
    monkeypatch.setattr(sandbox_module, "_scratch_is_writable", lambda _c: False)
    assert _mount_scratch_set(tmp_path / "root", ("/tmp",), 1 << 20) is False


def test_mount_scratch_set_succeeds_when_writable(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _install(monkeypatch, _FakeLibc())
    monkeypatch.setattr(sandbox_module, "_scratch_is_writable", lambda _c: True)
    assert _mount_scratch_set(tmp_path / "root", ("/tmp",), 1 << 20) is True


def test_mount_scratch_clamps_negative_size(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    fake = _install(monkeypatch, _FakeLibc())
    _mount_scratch(tmp_path / "root", "/tmp", -5)
    assert fake.calls[0][1][4] == b"size=0"
    assert fake.calls[0][1][3] == MS_NOSUID | MS_NODEV | MS_NOEXEC


def test_exit_code_from_status_handles_signalled_exited_and_other() -> None:
    assert _exit_code_from_status(0) == 0
    assert _exit_code_from_status(9 << 8) == 9
    assert _exit_code_from_status(3 << 8) == 3
    assert _exit_code_from_status(0xFFFF) == 1


def test_close_inherited_fds_falls_back_when_procfs_missing(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    recorded: list[tuple[int, int]] = []
    monkeypatch.setattr(
        os,
        "listdir",
        lambda _path: (_ for _ in ()).throw(OSError(errno.ENOENT, "gone")),
    )
    monkeypatch.setattr(os, "closerange", lambda low, high: recorded.append((low, high)))
    _close_inherited_fds()
    assert recorded == [(3, 4_096)]


def test_close_inherited_fds_ignores_non_numeric_entries(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    closed: list[int] = []
    monkeypatch.setattr(os, "listdir", lambda _path: ["0", "1", "not-a-number", "9"])
    monkeypatch.setattr(os, "close", lambda descriptor: closed.append(descriptor))
    _close_inherited_fds()
    assert closed == [9]


def test_close_inherited_fds_tolerates_racing_close(monkeypatch: pytest.MonkeyPatch) -> None:
    def explode(descriptor: int) -> None:
        raise OSError(errno.EBADF, "already closed")

    monkeypatch.setattr(os, "listdir", lambda _path: ["7", "8"])
    monkeypatch.setattr(os, "close", explode)
    _close_inherited_fds()


def test_enter_pid_namespace_mirrors_child_status(monkeypatch: pytest.MonkeyPatch) -> None:
    _install(monkeypatch, _FakeLibc())
    monkeypatch.setattr(sandbox_module, "_close_inherited_fds", lambda: None)
    monkeypatch.setattr(os, "fork", lambda: 4242)
    monkeypatch.setattr(os, "waitpid", lambda _pid, _flags: (0, 7 << 8))
    exits: list[int] = []
    monkeypatch.setattr(os, "_exit", lambda code: exits.append(code))
    _enter_pid_namespace()
    assert exits == [7]


def test_enter_pid_namespace_returns_in_child(monkeypatch: pytest.MonkeyPatch) -> None:
    _install(monkeypatch, _FakeLibc())
    monkeypatch.setattr(sandbox_module, "_close_inherited_fds", lambda: None)
    monkeypatch.setattr(os, "fork", lambda: 0)
    monkeypatch.setattr(
        os,
        "_exit",
        lambda _code: pytest.fail("the child must not exit"),
    )
    _enter_pid_namespace()


def test_enter_pid_namespace_clamps_high_exit_codes(monkeypatch: pytest.MonkeyPatch) -> None:
    _install(monkeypatch, _FakeLibc())
    monkeypatch.setattr(sandbox_module, "_close_inherited_fds", lambda: None)
    monkeypatch.setattr(os, "fork", lambda: 11)
    monkeypatch.setattr(os, "waitpid", lambda _pid, _flags: (0, 0))
    monkeypatch.setattr(os, "WIFSIGNALED", lambda _status: True)
    monkeypatch.setattr(os, "WTERMSIG", lambda _status: 999)
    exits: list[int] = []
    monkeypatch.setattr(os, "_exit", lambda code: exits.append(code))
    _enter_pid_namespace()
    assert exits == [255]


def test_isolate_process_establishes_declared_namespaces(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    fake = _install(monkeypatch, _FakeLibc())
    monkeypatch.setattr(sandbox_module, "_enter_pid_namespace", lambda: None)
    spec = _spec(
        tmp_path / "root",
        tmp_path / "ws",
        controls=frozenset({"user_namespace", "mount_namespace"}),
    )
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    established = isolate_process(spec)
    assert established == ("user_namespace", "mount_namespace")
    unshared = [value for name, values in fake.calls if name == "unshare" for value in values]
    assert unshared == [0x10000000, CLONE_NEWNS]
    assert ("mount", (b"", b"/", b"", MS_REC | MS_PRIVATE, None)) in fake.calls
    assert ("chroot", ((tmp_path / "root").as_posix().encode(),)) in fake.calls
    assert ("chdir", (b"/",)) in fake.calls
    assert ("chdir", ((tmp_path / "ws").as_posix().encode(),)) in fake.calls


def test_isolate_process_skips_undeclared_namespaces(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    fake = _install(monkeypatch, _FakeLibc())
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=frozenset())
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    assert isolate_process(spec) == ()
    assert not [name for name, _ in fake.calls if name == "unshare"]


def test_isolate_process_tolerates_pid_namespace_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _install(monkeypatch, _FakeLibc())
    monkeypatch.setattr(
        sandbox_module,
        "_enter_pid_namespace",
        lambda: (_ for _ in ()).throw(OSError(errno.EPERM, "denied")),
    )
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=frozenset({"pid_namespace"}))
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    assert isolate_process(spec) == ()


def test_isolate_process_establishes_pid_namespace(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _install(monkeypatch, _FakeLibc())
    monkeypatch.setattr(sandbox_module, "_enter_pid_namespace", lambda: None)
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=frozenset({"pid_namespace"}))
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    assert isolate_process(spec) == ("pid_namespace",)


def test_isolate_process_ignores_device_bind_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _install(monkeypatch, _FakeLibc(fail={"mount"}))
    monkeypatch.setattr(sandbox_module, "_unshare", lambda _flag, _name: None)
    spec = _spec(
        tmp_path / "root",
        tmp_path / "ws",
        controls=frozenset({"device_nodes_minimal"}),
        device=("/dev/null",),
    )
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    assert isolate_process(spec) == ()


def test_isolate_process_establishes_device_nodes(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _install(monkeypatch, _FakeLibc())
    monkeypatch.setattr(sandbox_module, "_unshare", lambda _flag, _name: None)
    spec = _spec(
        tmp_path / "root",
        tmp_path / "ws",
        controls=frozenset({"device_nodes_minimal"}),
        device=("/dev/null",),
    )
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    assert isolate_process(spec) == ("device_nodes_minimal",)


def test_isolate_process_skips_scratch_when_mounts_fail(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _install(monkeypatch, _FakeLibc(fail={"mount"}))
    monkeypatch.setattr(sandbox_module, "_unshare", lambda _flag, _name: None)
    spec = _spec(
        tmp_path / "root",
        tmp_path / "ws",
        controls=frozenset({"scratch_tmpfs"}),
        scratch=("/tmp",),
    )
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    assert isolate_process(spec) == ()


def test_isolate_process_establishes_scratch(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _install(monkeypatch, _FakeLibc())
    monkeypatch.setattr(sandbox_module, "_unshare", lambda _flag, _name: None)
    monkeypatch.setattr(sandbox_module, "_scratch_is_writable", lambda _c: True)
    spec = _spec(
        tmp_path / "root",
        tmp_path / "ws",
        controls=frozenset({"scratch_tmpfs"}),
        scratch=("/tmp",),
    )
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    assert isolate_process(spec) == ("scratch_tmpfs",)


def test_isolate_process_ignores_procfs_mount_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _install(monkeypatch, _FakeLibc(fail={"mount"}))
    monkeypatch.setattr(sandbox_module, "_unshare", lambda _flag, _name: None)
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=frozenset({"procfs"}))
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    assert isolate_process(spec) == ()


def test_isolate_process_establishes_procfs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    fake = _install(monkeypatch, _FakeLibc())
    monkeypatch.setattr(sandbox_module, "_unshare", lambda _flag, _name: None)
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=frozenset({"procfs"}))
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    assert isolate_process(spec) == ("procfs",)
    proc_calls = [values for name, values in fake.calls if name == "mount" and values[0] == b"proc"]
    assert len(proc_calls) == 1
    assert proc_calls[0][2] == b"proc"


def test_isolate_process_raises_when_chroot_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _install(monkeypatch, _FakeLibc(fail={"chroot"}))
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=frozenset())
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    with pytest.raises(SandboxSetupError) as error:
        isolate_process(spec)
    assert "chroot" in str(error.value)


def test_isolate_process_raises_when_chdir_root_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    fake = _install(monkeypatch, _FakeLibc(fail={"chdir"}))
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=frozenset())
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    with pytest.raises(SandboxSetupError) as error:
        isolate_process(spec)
    assert "chdir to the sandbox root" in str(error.value)
    assert len([name for name, _ in fake.calls if name == "chdir"]) == 1


def test_network_isolation_detects_unreachable_network(monkeypatch: pytest.MonkeyPatch) -> None:
    fake = _install(monkeypatch, _FakeLibc())
    fake.connect_result = -1
    assert _network_is_isolated() is True
    assert ("close", (fake.socket_handle,)) in fake.calls


def test_network_isolation_detects_reachable_network(monkeypatch: pytest.MonkeyPatch) -> None:
    fake = _install(monkeypatch, _FakeLibc())
    fake.connect_result = 0
    assert _network_is_isolated() is False


def test_network_isolation_reports_socket_failure(monkeypatch: pytest.MonkeyPatch) -> None:
    fake = _install(monkeypatch, _FakeLibc(fail={"socket"}))
    assert _network_is_isolated() is False
    assert not [name for name, _ in fake.calls if name == "close"]


def test_device_list_is_minimal_accepts_only_loopback(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    listing = "Inter-|   Receive\n face |bytes\n  lo: 1 2\n"
    monkeypatch.setattr(Path, "read_text", lambda self, **kwargs: listing)
    assert _device_list_is_minimal() is True


def test_device_list_is_minimal_rejects_extra_interfaces(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    listing = "header\nheader\n  lo: 1 2\n eth0: 3 4\n"
    monkeypatch.setattr(Path, "read_text", lambda self, **kwargs: listing)
    assert _device_list_is_minimal() is False


def test_device_list_is_minimal_tolerates_unreadable_procfs(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def explode(self: Path, **kwargs: object) -> str:
        raise OSError(errno.ENOENT, "gone")

    monkeypatch.setattr(Path, "read_text", explode)
    assert _device_list_is_minimal() is True


def test_canary_is_invisible_when_absent(tmp_path: Path) -> None:
    assert _canary_is_invisible(str(tmp_path / "absent")) is True


def test_canary_is_invisible_when_readable(tmp_path: Path) -> None:
    canary = tmp_path / "canary"
    canary.write_bytes(b"x")
    assert _canary_is_invisible(str(canary)) is False


def test_workspace_is_writable_probes_and_cleans_up(tmp_path: Path) -> None:
    assert _workspace_is_writable(str(tmp_path)) is True
    assert not (tmp_path / ".sandbox-write-probe").exists()


def test_workspace_is_writable_reports_readonly_workspace(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def explode(self: Path, _payload: bytes) -> int:
        raise OSError(errno.EROFS, "read-only")

    monkeypatch.setattr(Path, "write_bytes", explode)
    assert _workspace_is_writable(str(tmp_path)) is False


def test_system_bindings_are_readonly_when_usr_is_readonly(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(Path, "is_dir", lambda self: True)

    def explode(self: Path, _payload: bytes) -> int:
        raise OSError(errno.EROFS, "read-only")

    monkeypatch.setattr(Path, "write_bytes", explode)
    assert _system_bindings_are_readonly() is True


def test_system_bindings_are_readonly_when_usr_absent(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(Path, "is_dir", lambda self: False)
    assert _system_bindings_are_readonly() is False


def test_system_bindings_are_readonly_reports_writable_usr(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(Path, "is_dir", lambda self: True)
    monkeypatch.setattr(Path, "write_bytes", lambda self, _payload: 0)
    monkeypatch.setattr(Path, "unlink", lambda self, **_kwargs: None)
    assert _system_bindings_are_readonly() is False


def test_procfs_is_namespaced_reads_pid_one(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        Path,
        "read_text",
        lambda self, **kwargs: "1 (systemd) S 0 1 1 0\n",
    )
    assert _procfs_is_namespaced() is True


def test_procfs_is_namespaced_reports_empty_stat(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(Path, "read_text", lambda self, **kwargs: "   \n")
    assert _procfs_is_namespaced() is False


def test_procfs_is_namespaced_tolerates_missing_file(monkeypatch: pytest.MonkeyPatch) -> None:
    def explode(self: Path, **kwargs: object) -> str:
        raise OSError(errno.ENOENT, "gone")

    monkeypatch.setattr(Path, "read_text", explode)
    assert _procfs_is_namespaced() is False


def test_scratch_is_writable_checks_every_candidate(tmp_path: Path) -> None:
    assert _scratch_is_writable((str(tmp_path),)) is True
    assert not (tmp_path / ".sandbox-scratch-probe").exists()


def test_scratch_is_writable_returns_false_on_first_failure(tmp_path: Path) -> None:
    assert _scratch_is_writable((str(tmp_path / "missing"),)) is False


def test_verify_controls_splits_verified_and_failed(tmp_path: Path) -> None:
    spec = _spec(
        tmp_path / "root",
        tmp_path,
        controls=frozenset(
            {
                "chroot_workspace_only",
                "mount_namespace",
                "user_namespace",
                "ipc_namespace",
                "uts_namespace",
                "pid_namespace",
            }
        ),
    )
    verified, failed = _verify_controls(spec, str(tmp_path / "absent"), str(tmp_path))
    assert "chroot_workspace_only" in verified
    assert "mount_namespace" in verified
    assert "user_namespace" in verified
    assert "ipc_namespace" in verified
    assert "uts_namespace" in verified
    assert "pid_namespace" in failed
    assert failed == ("pid_namespace",)
    assert set(verified).isdisjoint(failed)


def test_verify_controls_reports_readonly_and_network_failures(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    canary = tmp_path / "canary"
    canary.write_bytes(b"x")
    monkeypatch.setattr(sandbox_module, "_network_is_isolated", lambda: False)
    monkeypatch.setattr(sandbox_module, "_system_bindings_are_readonly", lambda: False)
    monkeypatch.setattr(sandbox_module, "_workspace_is_writable", lambda _w: False)
    monkeypatch.setattr(sandbox_module, "_procfs_is_namespaced", lambda: True)
    monkeypatch.setattr(sandbox_module, "_device_list_is_minimal", lambda: False)
    monkeypatch.setattr(sandbox_module, "_scratch_is_writable", lambda: False)
    spec = _spec(
        tmp_path / "root",
        tmp_path,
        controls=STRICT_REQUIRED_CONTROLS
        | frozenset(
            {
                "procfs",
                "device_nodes_minimal",
                "scratch_tmpfs",
            }
        ),
    )
    verified, failed = _verify_controls(spec, str(canary), str(tmp_path))
    assert "procfs" in verified
    assert "mount_namespace" in verified
    assert "user_namespace" in verified
    assert set(failed) == {
        "chroot_workspace_only",
        "network_isolation",
        "readonly_system_bindings",
        "workspace_read_write",
        "device_nodes_minimal",
        "scratch_tmpfs",
    }


def test_verify_controls_ignores_controls_absent_from_spec(tmp_path: Path) -> None:
    spec = _spec(tmp_path / "root", tmp_path, controls=frozenset({"mount_namespace"}))
    verified, failed = _verify_controls(spec, str(tmp_path), str(tmp_path))
    assert verified == ("mount_namespace",)
    assert failed == ()


def test_probe_entry_decodes_valid_payload(tmp_path: Path) -> None:
    spec = _spec(tmp_path / "root", tmp_path / "ws")
    decoded, canary = _probe_entry(json.dumps(spec.to_payload()), json.dumps("/host/canary"))
    assert decoded == spec
    assert canary == "/host/canary"


def test_probe_entry_rejects_malformed_spec_payload() -> None:
    with pytest.raises(SandboxProbeError) as error:
        _probe_entry("{not json", json.dumps("/canary"))
    assert "malformed" in str(error.value)


def test_probe_entry_rejects_malformed_canary_payload(tmp_path: Path) -> None:
    spec = _spec(tmp_path / "root", tmp_path / "ws")
    with pytest.raises(SandboxProbeError):
        _probe_entry(json.dumps(spec.to_payload()), "{not json")


def test_run_probe_reports_malformed_payload(
    capsys: pytest.CaptureFixture[str],
) -> None:
    assert run_probe("{bad", "nope") == PROBE_FAILURE_EXIT
    payload = json.loads(capsys.readouterr().out.strip())
    assert payload["controls"] == []
    assert "malformed" in payload["detail"]


def test_run_probe_reports_setup_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    _install(monkeypatch, _FakeLibc(fail={"unshare"}))
    spec = _spec(
        tmp_path / "root",
        tmp_path / "ws",
        controls=frozenset({"user_namespace"}),
    )
    monkeypatch.setattr(sandbox_module, "_unshare", _failing_unshare)
    assert run_probe(json.dumps(spec.to_payload()), json.dumps(str(tmp_path))) == (
        PROBE_FAILURE_EXIT
    )
    payload = json.loads(capsys.readouterr().out.strip())
    assert payload["controls"] == []
    assert "unshare(user_namespace)" in payload["detail"]


def test_run_probe_reports_os_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=frozenset())

    def explode(_spec_value: SandboxSpec) -> tuple[str, ...]:
        raise OSError(errno.ENOSPC, "full")

    monkeypatch.setattr(sandbox_module, "isolate_process", explode)
    assert run_probe(json.dumps(spec.to_payload()), json.dumps(str(tmp_path))) == (
        PROBE_FAILURE_EXIT
    )
    payload = json.loads(capsys.readouterr().out.strip())
    assert "OSError" in payload["detail"]


def test_run_probe_fails_when_required_control_missing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=frozenset({"mount_namespace"}))
    monkeypatch.setattr(sandbox_module, "isolate_process", lambda _s: ())
    monkeypatch.setattr(sandbox_module, "_verify_controls", lambda *_a: ((), ()))
    assert run_probe(json.dumps(spec.to_payload()), json.dumps(str(tmp_path))) == (
        PROBE_FAILURE_EXIT
    )
    assert capsys.readouterr().out == ""


def test_run_probe_succeeds_and_notes_unverified_optional_controls(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    spec = _spec(
        tmp_path / "root",
        tmp_path / "ws",
        controls=STRICT_REQUIRED_CONTROLS | {"procfs"},
    )
    monkeypatch.setattr(sandbox_module, "isolate_process", lambda _s: ())
    monkeypatch.setattr(
        sandbox_module,
        "_verify_controls",
        lambda *_a: (tuple(sorted(STRICT_REQUIRED_CONTROLS)), ()),
    )
    assert run_probe(json.dumps(spec.to_payload()), json.dumps(str(tmp_path))) == 0
    payload = json.loads(capsys.readouterr().out.strip())
    assert payload["controls"] == sorted(STRICT_REQUIRED_CONTROLS)
    assert payload["detail"] == "not enforced: procfs"


def test_run_probe_succeeds_with_empty_detail(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=STRICT_REQUIRED_CONTROLS)
    monkeypatch.setattr(sandbox_module, "isolate_process", lambda _s: ())
    monkeypatch.setattr(
        sandbox_module,
        "_verify_controls",
        lambda *_a: (tuple(sorted(STRICT_REQUIRED_CONTROLS)), ()),
    )
    assert run_probe(json.dumps(spec.to_payload()), json.dumps(str(tmp_path))) == 0
    payload = json.loads(capsys.readouterr().out.strip())
    assert payload["detail"] == ""


def test_emit_probe_result_truncates_detail(capsys: pytest.CaptureFixture[str]) -> None:
    _emit_probe_result(("a", "b"), (), "d" * 1_000)
    payload = json.loads(capsys.readouterr().out.strip())
    assert payload["controls"] == ["a", "b"]
    assert len(payload["detail"]) == 480


def test_unavailable_truncates_reason() -> None:
    probe = _unavailable("r" * 1_000, elapsed_ms=12.5)
    assert probe.available is False
    assert probe.controls == ()
    assert len(probe.detail) == 480
    assert probe.elapsed_ms == 12.5
    assert probe.platform == platform_name()


def test_probe_sandbox_refuses_non_linux(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(sys, "platform", "darwin")
    probe = probe_sandbox()
    assert probe.available is False
    assert "requires Linux namespaces" in probe.detail


def test_probe_sandbox_refuses_missing_interpreter(tmp_path: Path) -> None:
    probe = probe_sandbox(python_executable=str(tmp_path / "absent"))
    assert probe.available is False
    assert "no usable interpreter" in probe.detail


def test_probe_sandbox_refuses_empty_interpreter() -> None:
    probe = probe_sandbox(python_executable="")
    assert probe.available is False
    assert "no usable interpreter" in probe.detail


def test_probe_sandbox_reports_timeout(monkeypatch: pytest.MonkeyPatch) -> None:
    def explode(*args: object, **kwargs: object) -> object:
        raise subprocess.TimeoutExpired(cmd="python", timeout=PROBE_TIMEOUT_SECONDS)

    monkeypatch.setattr(subprocess, "run", explode)
    probe = probe_sandbox()
    assert probe.available is False
    assert "did not finish" in probe.detail
    assert probe.elapsed_ms >= 0.0


def test_probe_sandbox_reports_startup_failure(monkeypatch: pytest.MonkeyPatch) -> None:
    def explode(*args: object, **kwargs: object) -> object:
        raise OSError(errno.EIO, "unrecoverable")

    monkeypatch.setattr(subprocess, "run", explode)
    probe = probe_sandbox()
    assert probe.available is False
    assert "could not start: OSError" in probe.detail


def test_probe_sandbox_reports_sandbox_error_during_build(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def explode(*args: object, **kwargs: object) -> object:
        raise SandboxSetupError("nope")

    monkeypatch.setattr(subprocess, "run", explode)
    probe = probe_sandbox()
    assert "could not start: SandboxSetupError" in probe.detail


def test_probe_sandbox_reports_value_error_during_build(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def explode(*args: object, **kwargs: object) -> object:
        raise ValueError("bad")

    monkeypatch.setattr(subprocess, "run", explode)
    probe = probe_sandbox()
    assert "could not start: ValueError" in probe.detail


def test_probe_sandbox_reports_nonzero_exit(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        subprocess,
        "run",
        lambda *_a, **_k: subprocess.CompletedProcess(
            args=[], returncode=121, stdout=b"", stderr=b""
        ),
    )
    probe = probe_sandbox()
    assert probe.available is False
    assert "unavailable on this host" in probe.detail


def test_probe_sandbox_reports_unparsable_output(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        subprocess,
        "run",
        lambda *_a, **_k: subprocess.CompletedProcess(
            args=[], returncode=0, stdout=b"not json", stderr=b""
        ),
    )
    probe = probe_sandbox()
    assert probe.available is False
    assert "returned no result" in probe.detail


def test_probe_sandbox_reports_empty_output(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        subprocess,
        "run",
        lambda *_a, **_k: subprocess.CompletedProcess(
            args=[], returncode=0, stdout=b"   ", stderr=b""
        ),
    )
    probe = probe_sandbox()
    assert probe.available is False
    assert "required sandbox control was not verified" in probe.detail


def test_probe_sandbox_reports_missing_required_control(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        subprocess,
        "run",
        lambda *_a, **_k: subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=json.dumps({"controls": ["mount_namespace"]}).encode(),
            stderr=b"",
        ),
    )
    probe = probe_sandbox()
    assert probe.available is False
    assert "required sandbox control was not verified" in probe.detail


def test_probe_sandbox_ignores_non_string_controls(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        subprocess,
        "run",
        lambda *_a, **_k: subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=json.dumps({"controls": [*sorted(STRICT_REQUIRED_CONTROLS), 7]}).encode(),
            stderr=b"",
        ),
    )
    probe = probe_sandbox()
    assert probe.available is True
    assert set(probe.controls) == STRICT_REQUIRED_CONTROLS
    assert all(isinstance(name, str) for name in probe.controls)


def test_probe_sandbox_filters_unknown_controls(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        subprocess,
        "run",
        lambda *_a, **_k: subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=json.dumps(
                {"controls": [*sorted(STRICT_REQUIRED_CONTROLS), "made_up"]}
            ).encode(),
            stderr=b"",
        ),
    )
    probe = probe_sandbox()
    assert probe.available is True
    assert "made_up" not in probe.controls


def test_probe_sandbox_reports_success_with_optional_controls(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    controls = sorted(STRICT_REQUIRED_CONTROLS | STRICT_OPTIONAL_CONTROLS)
    captured: dict[str, Any] = {}

    def fake_run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[bytes]:
        captured["command"] = command
        captured["env"] = kwargs["env"]
        captured["timeout"] = kwargs["timeout"]
        captured["cwd"] = kwargs["cwd"]
        return subprocess.CompletedProcess(
            args=command,
            returncode=0,
            stdout=json.dumps({"controls": controls}).encode(),
            stderr=b"",
        )

    monkeypatch.setattr(subprocess, "run", fake_run)
    probe = probe_sandbox(import_roots=(Path("/tmp"),), scratch_bytes=1 << 21)
    assert probe.available is True
    assert set(probe.controls) == set(STRICT_REQUIRED_CONTROLS | STRICT_OPTIONAL_CONTROLS)
    assert captured["timeout"] == PROBE_TIMEOUT_SECONDS
    assert captured["env"]["PYTHONDONTWRITEBYTECODE"] == "1"
    assert captured["env"]["PYTHONPATH"] == "/tmp"
    assert captured["command"][1:3] == ["-I", "-B"]
    assert captured["cwd"].name == "workspace"


def test_probe_sandbox_uses_supplied_interpreter(monkeypatch: pytest.MonkeyPatch) -> None:
    fake_executable = Path(__file__)
    captured: dict[str, Any] = {}

    def fake_run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[bytes]:
        captured["command"] = command
        return subprocess.CompletedProcess(
            args=command,
            returncode=0,
            stdout=json.dumps({"controls": sorted(STRICT_REQUIRED_CONTROLS)}).encode(),
            stderr=b"",
        )

    monkeypatch.setattr(subprocess, "run", fake_run)
    probe = probe_sandbox(python_executable=str(fake_executable))
    assert probe.available is True
    assert captured["command"][0] == str(fake_executable)


def test_judge_sandbox_allows_portable_profile() -> None:
    verdict = judge_sandbox(IsolationProfile.PORTABLE, None)
    assert verdict == SandboxVerdict(allowed=True, reason=verdict.reason)
    assert verdict.controls == ()
    assert "confines a process" in verdict.reason


def test_judge_sandbox_refuses_strict_without_probe() -> None:
    verdict = judge_sandbox(IsolationProfile.STRICT_LINUX, None)
    assert verdict.allowed is False
    assert verdict.reason.endswith("refused")


def test_judge_sandbox_refuses_strict_and_includes_detail() -> None:
    probe = SandboxProbe(available=False, platform="linux", detail="namespaces denied")
    verdict = judge_sandbox(IsolationProfile.STRICT_LINUX, probe)
    assert verdict.allowed is False
    assert verdict.reason.endswith("refused: namespaces denied")


def test_judge_sandbox_allows_strict_with_verified_controls() -> None:
    probe = SandboxProbe(
        available=True,
        platform="linux",
        controls=("mount_namespace", "user_namespace"),
    )
    verdict = judge_sandbox(IsolationProfile.STRICT_LINUX, probe)
    assert verdict.allowed is True
    assert verdict.controls == ("mount_namespace", "user_namespace")
    assert verdict.reason == ""


def test_judge_sandbox_refuses_unavailable_probe_without_detail() -> None:
    probe = SandboxProbe(available=False, platform="linux")
    verdict = judge_sandbox(IsolationProfile.STRICT_LINUX, probe)
    assert verdict.allowed is False
    assert verdict.reason.endswith("refused")


def test_isolate_process_raises_when_workspace_chdir_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    class _ChdirFailsOnce(_FakeLibc):
        def chdir(self, target: bytes) -> int:
            self.calls.append(("chdir", (target,)))
            if target != b"/":
                ctypes.set_errno(errno.EACCES)
                return -1
            return 0

    _install(monkeypatch, _ChdirFailsOnce())
    spec = _spec(tmp_path / "root", tmp_path / "ws", controls=frozenset())
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    with pytest.raises(SandboxSetupError) as error:
        isolate_process(spec)
    assert "chdir into the workspace" in str(error.value)


def test_isolate_process_binds_readonly_and_writable_targets(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    readonly = tmp_path / "ro"
    readonly.mkdir()
    writable = tmp_path / "rw"
    writable.mkdir()
    fake = _install(monkeypatch, _FakeLibc())
    spec = _spec(
        tmp_path / "root",
        tmp_path / "ws",
        controls=frozenset(),
        readonly=(str(readonly),),
        writable=(str(writable),),
    )
    (tmp_path / "root").mkdir()
    (tmp_path / "ws").mkdir()
    assert isolate_process(spec) == ()
    targets = [values[1].decode() for name, values in fake.calls if name == "mount"]
    assert any(target.endswith(f"root{readonly.as_posix()}") for target in targets)
    assert any(target.endswith(f"root{writable.as_posix()}") for target in targets)


def test_sandbox_verdict_payload_reports_every_field() -> None:
    verdict = SandboxVerdict(allowed=True, reason="because", controls=("a",))
    assert verdict.to_payload() == {
        "allowed": True,
        "controls": ["a"],
        "reason": "because",
    }
    assert SandboxVerdict(allowed=False).to_payload() == {
        "allowed": False,
        "controls": [],
        "reason": "",
    }


def _failing_unshare(flag: int, name: str) -> None:
    raise SandboxSetupError(f"unshare({name})")
