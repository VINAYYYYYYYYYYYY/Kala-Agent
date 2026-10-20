"""STA session, lazy imports, HRESULT normalization, and retry policy."""

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import cast

import pytest

from kala_solidedge_bridge.session import (
    RPC_E_CALL_REJECTED,
    RPC_E_SERVERCALL_RETRYLATER,
    BindingProtocol,
    ComCallError,
    ComUnavailable,
    Session,
    SessionTimeout,
    StaError,
    call_with_retry,
    hresult_of,
    is_retryable,
    normalize_hresult,
)


class ComLikeError(Exception):
    def __init__(self, hresult: int) -> None:
        super().__init__(f"com error {hresult:#x}")
        self.hresult = hresult


class FakeBinding:
    def __init__(self) -> None:
        self.initialized = 0
        self.uninitialized = 0
        self.pumped = 0
        self.initialize_error: Exception | None = None

    def coinitialize(self) -> None:
        if self.initialize_error is not None:
            raise self.initialize_error
        self.initialized += 1

    def couninitialize(self) -> None:
        self.uninitialized += 1

    def pump(self) -> None:
        self.pumped += 1


def _factory(binding: FakeBinding) -> BindingProtocol:
    return cast(BindingProtocol, binding)


def test_signed_and_unsigned_hresults_normalize() -> None:
    assert normalize_hresult(-2147418111) == RPC_E_CALL_REJECTED
    assert normalize_hresult(RPC_E_CALL_REJECTED) == RPC_E_CALL_REJECTED
    assert normalize_hresult(True) is None
    assert normalize_hresult("x") is None
    assert hresult_of(ComLikeError(-2147418111)) == RPC_E_CALL_REJECTED


def test_only_the_two_transient_hresults_are_retryable() -> None:
    assert is_retryable(ComLikeError(RPC_E_CALL_REJECTED))
    assert is_retryable(ComLikeError(RPC_E_SERVERCALL_RETRYLATER))
    assert not is_retryable(ComLikeError(0x80020005))
    assert not is_retryable(RuntimeError("not COM"))


def test_retry_retries_only_a_rejected_call() -> None:
    attempts = 0

    def operation() -> str:
        nonlocal attempts
        attempts += 1
        if attempts < 3:
            raise ComLikeError(RPC_E_CALL_REJECTED)
        return "ok"

    assert call_with_retry(operation, sleep=lambda _: None) == "ok"
    assert attempts == 3


def test_retry_does_not_resend_a_mutation() -> None:
    attempts = 0

    def operation() -> str:
        nonlocal attempts
        attempts += 1
        raise ComLikeError(RPC_E_CALL_REJECTED)

    with pytest.raises(ComLikeError):
        call_with_retry(operation, retry_safe=False, sleep=lambda _: None)
    assert attempts == 1


def test_retry_stops_at_the_five_second_deadline() -> None:
    state = {"now": 0.0}
    attempts = 0

    def clock() -> float:
        return state["now"]

    def sleep(seconds: float) -> None:
        state["now"] += seconds

    def operation() -> str:
        nonlocal attempts
        attempts += 1
        raise ComLikeError(RPC_E_SERVERCALL_RETRYLATER)

    with pytest.raises(ComLikeError):
        call_with_retry(operation, deadline_seconds=1.0, clock=clock, sleep=sleep)
    assert 1.0 <= state["now"] < 1.5
    assert 2 <= attempts < 20


def test_session_runs_calls_on_one_sta_thread_and_uninitializes() -> None:
    binding = FakeBinding()
    session = Session(binding_factory=lambda: _factory(binding))
    session.start()
    try:
        seen: list[int] = []

        def operation(current: BindingProtocol) -> int:
            seen.append(threading.get_ident())
            return 7

        assert session.run(operation) == 7
        assert session.run(operation) == 7
        assert len(set(seen)) == 1
        assert seen[0] != threading.get_ident()
    finally:
        assert session.stop() is True
    assert binding.initialized == 1
    assert binding.uninitialized == 1
    assert binding.pumped == 2


def test_session_reports_a_changed_apartment_as_a_hard_failure() -> None:
    binding = FakeBinding()
    binding.initialize_error = StaError("wrong apartment", 0x80010106)
    session = Session(binding_factory=lambda: _factory(binding))

    with pytest.raises(StaError):
        session.start()


def test_session_call_timeout_is_explicitly_indeterminate() -> None:
    binding = FakeBinding()
    session = Session(binding_factory=lambda: _factory(binding))
    session.start()
    try:
        with pytest.raises(SessionTimeout):
            session.run(lambda _: time.sleep(0.2), timeout=0.01)
    finally:
        assert session.stop() is True


def test_session_propagates_the_original_call_failure() -> None:
    binding = FakeBinding()
    session = Session(binding_factory=lambda: _factory(binding))
    session.start()
    try:
        with pytest.raises(ComCallError):
            session.run(lambda _: (_ for _ in ()).throw(ComCallError("bad call", 1)))
    finally:
        session.stop()


def test_importing_session_never_imports_pywin32() -> None:
    source = Path(__file__).resolve().parents[1] / "src"
    code = (
        "import sys; import kala_solidedge_bridge.session; "
        "assert 'pythoncom' not in sys.modules; "
        "assert 'win32com' not in sys.modules; "
        "assert 'pywintypes' not in sys.modules"
    )
    environment = dict(os.environ)
    environment["PYTHONPATH"] = str(source)
    completed = subprocess.run(
        [sys.executable, "-c", code],
        check=False,
        capture_output=True,
        text=True,
        env=environment,
        timeout=30,
    )
    assert completed.returncode == 0, completed.stderr


def test_session_without_pywin32_fails_usefully() -> None:
    if importlib.util.find_spec("pythoncom") is not None:
        pytest.skip("pywin32 is installed; the real missing-dependency path is unavailable here")
    session = Session()
    with pytest.raises(ComUnavailable):
        session.start()


def test_start_propagates_an_injected_binding_failure() -> None:
    """The typed failure path is asserted on every platform, including Windows."""

    def unavailable_binding() -> BindingProtocol:
        raise ComUnavailable("the injected binding reports pywin32 is absent")

    session = Session(binding_factory=unavailable_binding)
    with pytest.raises(ComUnavailable):
        session.start()
