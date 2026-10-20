"""Test-only oracle for the kala-sandbox differential.

The script imports the legacy Python implementation from ``src`` and returns
observable values for comparison. It is never used by the Rust product path.

The protocol is a single JSON request on standard input and a single JSON
document on standard output. Every case is answered independently so one
failure cannot hide another.
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import sys
import sysconfig
from pathlib import Path
from typing import Any


def _install_source_path() -> None:
    """Put the repository ``src`` directory on the import path.

    The oracle runs with ``-I``, so ``PYTHONPATH`` is ignored by design. The
    path is therefore installed explicitly.
    """

    here = Path(__file__).resolve()
    for parent in here.parents:
        candidate = parent / "src" / "kala_agent"
        if candidate.is_dir():
            sys.path.insert(0, str(parent / "src"))
            return
    raise SystemExit("the repository src directory could not be located")


_install_source_path()

from kala_agent.tools import sandbox  # noqa: E402
from kala_agent.tools.sandbox import (  # noqa: E402
    IsolationProfile,
    SandboxProbe,
    SandboxSetupError,
    SandboxSpec,
    SandboxVerdict,
)


def _flag_value(flag: object) -> int:
    """The integer value of a ``ctypes`` flag or of a plain integer."""

    if isinstance(flag, int):
        return flag
    return int(getattr(flag, "value", flag))


class _FakeLibc:
    """A recording stand-in for the C library.

    The real libc is never called, so the jail plan can be observed without
    changing the namespace of the oracle.
    """

    def __init__(self, *, fail: set[str] | None = None) -> None:
        self.fail = set() if fail is None else set(fail)
        self.calls: list[tuple[str, tuple[Any, ...]]] = []
        self.socket_handle = 7
        self.connect_result = -1

    def unshare(self, flag: object) -> int:
        self.calls.append(("unshare", (_flag_value(flag),)))
        return -1 if "unshare" in self.fail else 0

    def mount(
        self,
        source: bytes | None,
        target: bytes,
        filesystem: bytes | None,
        flags: object,
        data: bytes | None,
    ) -> int:
        self.calls.append(("mount", (source, target, filesystem, _flag_value(flags), data)))
        return -1 if "mount" in self.fail else 0

    def chroot(self, target: bytes) -> int:
        self.calls.append(("chroot", (target,)))
        return -1 if "chroot" in self.fail else 0

    def chdir(self, target: bytes) -> int:
        self.calls.append(("chdir", (target,)))
        return -1 if "chdir" in self.fail else 0

    def socket(self, domain: object, kind: object, protocol: object) -> int:
        self.calls.append(
            ("socket", (_flag_value(domain), _flag_value(kind), _flag_value(protocol)))
        )
        return -1 if "socket" in self.fail else self.socket_handle

    def connect(self, handle: int, address: object, length: int) -> int:
        self.calls.append(("connect", (int(handle),)))
        return self.connect_result

    def close(self, handle: int) -> int:
        self.calls.append(("close", (int(handle),)))
        return 0

    def umount2(self, target: bytes, flags: int) -> int:
        self.calls.append(("umount2", (target, int(flags))))
        return 0


def _ok(value: Any) -> dict[str, Any]:
    return {"ok": True, "value": value}


def _failed(error: BaseException) -> dict[str, Any]:
    return {"ok": False, "error": type(error).__name__, "message": str(error)}


def _probe_from(payload: dict[str, Any] | None) -> SandboxProbe | None:
    if payload is None:
        return None
    return SandboxProbe(
        available=bool(payload.get("available", False)),
        platform=str(payload.get("platform", "")),
        controls=tuple(payload.get("controls", ())),
        detail=str(payload.get("detail", "")),
        elapsed_ms=float(payload.get("elapsed_ms", 0.0)),
    )


def _case_platform_name(_case: dict[str, Any]) -> dict[str, Any]:
    return _ok(sandbox.platform_name())


def _interpreter_layout() -> dict[str, Any]:
    """The interpreter facts the Python module reads from ``sys``."""

    paths = sysconfig.get_paths()
    return {
        "executable": sys.executable,
        "prefix": sys.prefix,
        "base_prefix": sys.base_prefix,
        "exec_prefix": sys.exec_prefix,
        "base_exec_prefix": sys.base_exec_prefix,
        "module_directory": os.path.dirname(os.__file__),
        "stdlib": paths.get("stdlib", ""),
        "platstdlib": paths.get("platstdlib", ""),
        "purelib": paths.get("purelib", ""),
        "platlib": paths.get("platlib", ""),
    }


def _case_interpreter_layout(_case: dict[str, Any]) -> dict[str, Any]:
    return _ok(_interpreter_layout())


def _case_child_resource_paths(_case: dict[str, Any]) -> dict[str, Any]:
    return _ok(list(sandbox.child_resource_paths()))


def _case_resolve_system_bindings(case: dict[str, Any]) -> dict[str, Any]:
    bindings = list(
        sandbox.resolve_system_bindings([Path(item) for item in case["import_roots"]])
    )
    return _ok(bindings)


def _case_build_spec(case: dict[str, Any]) -> dict[str, Any]:
    try:
        spec = sandbox.build_sandbox_spec(
            controls=frozenset(case["controls"]),
            root=Path(case["root"]),
            workspace=Path(case["workspace"]),
            import_roots=[Path(item) for item in case.get("import_roots", ())],
            scratch_bytes=int(case.get("scratch_bytes", 1 << 20)),
        )
    except SandboxSetupError as error:
        return _failed(error)
    return _ok(json.dumps(spec.to_payload(), ensure_ascii=True))


def _case_spec_from_payload(case: dict[str, Any]) -> dict[str, Any]:
    try:
        spec = SandboxSpec.from_payload(case["payload"])
    except SandboxSetupError as error:
        return _failed(error)
    return _ok(json.dumps(spec.to_payload(), ensure_ascii=True))


def _case_spec_payload(case: dict[str, Any]) -> dict[str, Any]:
    spec = SandboxSpec.from_payload(case["payload"])
    return _ok(spec.to_payload())


def _case_probe_payload(case: dict[str, Any]) -> dict[str, Any]:
    probe = SandboxProbe(
        available=bool(case["probe"]["available"]),
        platform=str(case["probe"]["platform"]),
        controls=tuple(case["probe"].get("controls", ())),
        detail=str(case["probe"].get("detail", "")),
        elapsed_ms=float(case["probe"].get("elapsed_ms", 0.0)),
    )
    return _ok(json.dumps(probe.to_payload(), ensure_ascii=True))


def _case_verdict_payload(case: dict[str, Any]) -> dict[str, Any]:
    verdict = SandboxVerdict(
        allowed=bool(case["verdict"]["allowed"]),
        reason=str(case["verdict"].get("reason", "")),
        controls=tuple(case["verdict"].get("controls", ())),
    )
    return _ok(json.dumps(verdict.to_payload(), ensure_ascii=True))


def _case_judge(case: dict[str, Any]) -> dict[str, Any]:
    profile = IsolationProfile(case["profile"])
    probe = _probe_from(case.get("probe"))
    verdict = sandbox.judge_sandbox(profile, probe)
    return _ok(json.dumps(verdict.to_payload(), ensure_ascii=True))


def _case_unshadowed(case: dict[str, Any]) -> dict[str, Any]:
    spec = SandboxSpec.from_payload(case["payload"])
    return _ok(list(sandbox._unshadowed_scratch_paths(spec)))


def _case_os_strerror(case: dict[str, Any]) -> dict[str, Any]:
    """The operating-system reason of a set of error codes."""

    return _ok({str(code): os.strerror(int(code)) for code in case["codes"]})


def _case_exit_code(case: dict[str, Any]) -> dict[str, Any]:
    return _ok(sandbox._exit_code_from_status(int(case["status"])))


def _case_jail_plan(case: dict[str, Any]) -> dict[str, Any]:
    """Record the operating-system calls of ``isolate_process``.

    The C library is replaced by a recorder, so the plan is observed without
    touching the namespace of the oracle.
    """

    spec = SandboxSpec.from_payload(case["payload"])
    fake = _FakeLibc(fail=set(case.get("fail", ())))
    original = sandbox._LIBC
    original_scratch = sandbox._scratch_is_writable
    sandbox._LIBC = fake  # type: ignore[assignment]
    sandbox._scratch_is_writable = lambda _c: bool(case.get("scratch_writable", True))  # type: ignore[assignment]
    try:
        established = sandbox.isolate_process(spec)
    except SandboxSetupError as error:
        return _failed(error)
    finally:
        sandbox._LIBC = original  # type: ignore[assignment]
        sandbox._scratch_is_writable = original_scratch  # type: ignore[assignment]
    calls = [
        [name, [_jsonable(value) for value in values]]
        for name, values in fake.calls
    ]
    return _ok({"established": list(established), "calls": calls})


def _jsonable(value: Any) -> Any:
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return value


def _case_probe_entry(case: dict[str, Any]) -> dict[str, Any]:
    try:
        spec, canary = sandbox._probe_entry(case["spec_payload"], case["canary_payload"])
    except sandbox.SandboxProbeError as error:
        return _failed(error)
    return _ok({"spec": json.dumps(spec.to_payload(), ensure_ascii=True), "canary": canary})


def _case_run_probe(case: dict[str, Any]) -> dict[str, Any]:
    """Run the probe entry point in a forked child.

    The probe builds the jail in the calling process, so the call is made in a
    child and the answer travels through a pipe.
    """

    read_fd, write_fd = os.pipe()
    pid = os.fork()
    if pid == 0:  # pragma: no cover - the child never returns
        try:
            os.close(read_fd)
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                code = sandbox.run_probe(case["spec_payload"], case["canary_payload"])
            answer = {"exit": code, "stdout": buffer.getvalue()}
        except BaseException as error:  # noqa: BLE001 - the child reports the failure
            answer = {"error": type(error).__name__, "message": str(error)}
        with os.fdopen(write_fd, "w", encoding="utf-8") as handle:
            json.dump(answer, handle)
        os._exit(0)
    os.close(write_fd)
    with os.fdopen(read_fd, "r", encoding="utf-8") as handle:
        answer = json.loads(handle.read())
    os.waitpid(pid, 0)
    return _ok(answer)


def _case_probe_sandbox(case: dict[str, Any]) -> dict[str, Any]:
    probe = sandbox.probe_sandbox(
        import_roots=[Path(item) for item in case.get("import_roots", ())],
        scratch_bytes=int(case.get("scratch_bytes", 1 << 20)),
        python_executable=case.get("python_executable"),
    )
    return _ok(
        {
            "available": probe.available,
            "platform": probe.platform,
            "controls": sorted(probe.controls),
            "detail": probe.detail,
        }
    )


def _case_jail_fingerprint(case: dict[str, Any]) -> dict[str, Any]:
    """Build the jail in a forked child and report what the child sees."""

    spec = SandboxSpec.from_payload(case["payload"])
    canary = case["canary"]
    read_fd, write_fd = os.pipe()
    pid = os.fork()
    if pid == 0:  # pragma: no cover - the child never returns
        try:
            os.close(read_fd)
            established = sandbox.isolate_process(spec)
            verified, failed = sandbox._verify_controls(spec, canary, spec.workspace.as_posix())
            fingerprint = {
                "established": list(established),
                "verified": list(verified),
                "failed": list(failed),
                "root_entries": sorted(os.listdir("/")),
                "pid_is_one": os.getpid() == 1,
            }
            with os.fdopen(write_fd, "w", encoding="utf-8") as handle:
                json.dump(fingerprint, handle)
        except BaseException as error:  # noqa: BLE001 - the child reports the failure
            with os.fdopen(write_fd, "w", encoding="utf-8") as handle:
                json.dump({"error": type(error).__name__, "message": str(error)}, handle)
        finally:
            os._exit(0)
    os.close(write_fd)
    with os.fdopen(read_fd, "r", encoding="utf-8") as handle:
        payload = handle.read()
    _, status = os.waitpid(pid, 0)
    return _ok({"fingerprint": json.loads(payload), "status": status})


_CASES = {
    "platform_name": _case_platform_name,
    "interpreter_layout": _case_interpreter_layout,
    "child_resource_paths": _case_child_resource_paths,
    "resolve_system_bindings": _case_resolve_system_bindings,
    "build_spec": _case_build_spec,
    "spec_from_payload": _case_spec_from_payload,
    "spec_payload": _case_spec_payload,
    "probe_payload": _case_probe_payload,
    "verdict_payload": _case_verdict_payload,
    "judge": _case_judge,
    "unshadowed": _case_unshadowed,
    "exit_code": _case_exit_code,
    "os_strerror": _case_os_strerror,
    "jail_plan": _case_jail_plan,
    "probe_entry": _case_probe_entry,
    "run_probe": _case_run_probe,
    "probe_sandbox": _case_probe_sandbox,
    "jail_fingerprint": _case_jail_fingerprint,
}


def main() -> int:
    request = json.load(sys.stdin)
    results = []
    for case in request["cases"]:
        kind = case["kind"]
        handler = _CASES.get(kind)
        if handler is None:
            results.append({"ok": False, "error": "UnknownCase", "message": kind})
            continue
        try:
            results.append(handler(case))
        except BaseException as error:  # noqa: BLE001 - the oracle reports the class
            results.append(_failed(error))
    json.dump({"results": results}, sys.stdout, sort_keys=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
