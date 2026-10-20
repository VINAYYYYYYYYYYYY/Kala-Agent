"""The only lazy ``pywin32`` importer and the single COM apartment.

Nothing in this module imports ``pythoncom`` or ``win32com`` at module import
time. A :class:`Session` owns exactly one thread with an explicit
``CoInitializeEx(COINIT_APARTMENTTHREADED=2)``, a matched ``CoUninitialize``,
and an idle message pump. Every COM call is submitted to that thread; HTTP
threads never touch COM.

Retry policy: only ``RPC_E_CALL_REJECTED`` (``0x80010001``) and
``RPC_E_SERVERCALL_RETRYLATER`` (``0x8001010A``) are retried, with a five-second
deadline. Ambiguous mutations are never retried. HRESULTs are normalized to
unsigned 32-bit values so signed ``pywintypes`` values compare correctly.
"""

from __future__ import annotations

import importlib
import queue
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any, Final, Protocol, TypeVar, cast

T = TypeVar("T")

COINIT_APARTMENTTHREADED: Final[int] = 2
RPC_E_CALL_REJECTED: Final[int] = 0x80010001
RPC_E_SERVERCALL_RETRYLATER: Final[int] = 0x8001010A
RPC_E_CHANGED_MODE: Final[int] = 0x80010106
MK_E_UNAVAILABLE: Final[int] = 0x800401E3
DISP_E_TYPEMISMATCH: Final[int] = 0x80020005
E_INVALIDARG: Final[int] = 0x80070057
RETRYABLE_HRESULTS: Final[frozenset[int]] = frozenset(
    {RPC_E_CALL_REJECTED, RPC_E_SERVERCALL_RETRYLATER}
)
DEFAULT_RETRY_DEADLINE_SECONDS: Final[float] = 5.0
_MESSAGE_LIMIT: Final[int] = 400


class ComUnavailable(RuntimeError):
    """COM or the Windows automation dependency is not available here."""


class StaError(RuntimeError):
    """The explicit single-threaded apartment could not be established."""

    def __init__(self, message: str, hresult: int | None = None) -> None:
        super().__init__(message)
        self.hresult = normalize_hresult(hresult)


class SessionTimeout(TimeoutError):
    """A submitted call did not return before the caller's deadline.

    The call may still be running on the STA thread. This is not proof that it
    stopped and must be treated as indeterminate by whoever submitted it.
    """


class ComCallError(RuntimeError):
    """A COM call failed with a normalized HRESULT."""

    def __init__(self, message: str, hresult: int | None = None) -> None:
        super().__init__(message)
        self.hresult = normalize_hresult(hresult)


def normalize_hresult(value: object) -> int | None:
    """Normalize a signed or unsigned HRESULT to an unsigned 32-bit integer."""
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return value & 0xFFFFFFFF


def hresult_of(exc: BaseException) -> int | None:
    """Extract a normalized HRESULT from a pywin32 ``com_error`` or attribute."""
    hresult = normalize_hresult(getattr(exc, "hresult", None))
    if hresult is not None:
        return hresult
    args = getattr(exc, "args", ())
    if args and isinstance(args[0], int) and not isinstance(args[0], bool):
        return normalize_hresult(args[0])
    return None


def is_retryable(exc: BaseException) -> bool:
    hresult = hresult_of(exc)
    return hresult is not None and hresult in RETRYABLE_HRESULTS


def bounded_message(exc: BaseException) -> str:
    """Return a short diagnostic message without unbounded COM payloads."""
    text = str(exc).replace("\r", " ").replace("\n", " ").strip()
    return (text or exc.__class__.__name__)[:_MESSAGE_LIMIT]


def call_with_retry(
    operation: Callable[[], T],
    *,
    retry_safe: bool = True,
    deadline_seconds: float = DEFAULT_RETRY_DEADLINE_SECONDS,
    clock: Callable[[], float] = time.monotonic,
    sleep: Callable[[float], None] = time.sleep,
) -> T:
    """Call ``operation`` and retry only the two documented transient HRESULTs.

    ``retry_safe=False`` is used for every ambiguous mutation (document create,
    feature create, save, close): a rejected call is reported instead of being
    sent a second time.
    """
    started = clock()
    delay = 0.05
    while True:
        try:
            return operation()
        except Exception as exc:
            remaining = deadline_seconds - (clock() - started)
            if not retry_safe or not is_retryable(exc) or remaining <= 0:
                raise
            sleep(min(delay, remaining))
            delay = min(delay * 2.0, 0.5)


class BindingProtocol(Protocol):
    """The narrow COM surface used by the probe; implemented by :class:`ComBinding`."""

    def coinitialize(self) -> None: ...

    def couninitialize(self) -> None: ...

    def pump(self) -> None: ...

    def attach(self, launch: bool) -> Any: ...  # noqa: ANN401

    def version(self, application: Any) -> str: ...  # noqa: ANN401

    def resolve_constants(
        self,
        application: Any,  # noqa: ANN401
        names: tuple[str, ...],
    ) -> tuple[dict[str, int], list[str]]: ...

    def describe_typelib(self, obj: Any) -> dict[str, Any]: ...  # noqa: ANN401

    def variant_byref_float(self, value: float) -> Any: ...  # noqa: ANN401

    def variant_byref_int(self, value: int) -> Any: ...  # noqa: ANN401

    def variant_byref_float_array(self, values: list[float]) -> Any: ...  # noqa: ANN401

    def variant_byref_variant_array(self, values: list[Any]) -> Any: ...  # noqa: ANN401

    def call(
        self,
        operation: Callable[[], T],
        *,
        retry_safe: bool = True,
        deadline_seconds: float = DEFAULT_RETRY_DEADLINE_SECONDS,
    ) -> T: ...


class ComBinding:
    """Lazy pywin32 binding, created on the STA thread only."""

    def __init__(self) -> None:
        try:
            self.pythoncom = importlib.import_module("pythoncom")
            self.win32com = importlib.import_module("win32com.client")
            self.pywintypes = importlib.import_module("pywintypes")
        except ImportError as exc:  # pragma: no cover - exercised on Linux
            raise ComUnavailable(
                "pywin32 is not installed; Solid Edge COM is unavailable on this host"
            ) from exc
        self._generated_constants_module: str | None = None

    def coinitialize(self) -> None:
        try:
            self.pythoncom.CoInitializeEx(COINIT_APARTMENTTHREADED)
        except Exception as exc:
            hresult = hresult_of(exc)
            if hresult == RPC_E_CHANGED_MODE:
                raise StaError(
                    "the thread is already in a multi-threaded apartment; "
                    "refusing to run COM outside the explicit STA",
                    hresult,
                ) from exc
            raise StaError(f"CoInitializeEx failed: {bounded_message(exc)}", hresult) from exc

    def couninitialize(self) -> None:
        self.pythoncom.CoUninitialize()

    def pump(self) -> None:
        self.pythoncom.PumpWaitingMessages()

    def call(
        self,
        operation: Callable[[], T],
        *,
        retry_safe: bool = True,
        deadline_seconds: float = DEFAULT_RETRY_DEADLINE_SECONDS,
    ) -> T:
        return call_with_retry(operation, retry_safe=retry_safe, deadline_seconds=deadline_seconds)

    def attach(self, launch: bool) -> Any:  # noqa: ANN401
        """Attach to the running application, or launch it when asked."""
        if launch:
            application = self.win32com.Dispatch("SolidEdge.Application")
            application.Visible = True
            return application
        return self.win32com.GetActiveObject("SolidEdge.Application")

    def version(self, application: Any) -> str:  # noqa: ANN401
        value = getattr(application, "Version", None)
        if value is None:
            return "unknown"
        text = str(value).strip()
        return text[:64] or "unknown"

    def _ensure_generated_module(self, application: Any) -> str | None:  # noqa: ANN401
        if self._generated_constants_module is not None:
            return self._generated_constants_module
        try:
            ole_object = application._oleobj_
            type_info = ole_object.GetTypeInfo()
            typelib, _index = type_info.GetContainingTypeLib()
            attributes = typelib.GetLibAttr()
            module = self.win32com.gencache.EnsureModule(
                str(attributes.guid),
                int(attributes.lcid),
                int(attributes.wMajorVerNum),
                int(attributes.wMinorVerNum),
            )
            if module is None:
                return None
            importlib.import_module(module.__name__)
        except Exception:
            return None
        self._generated_constants_module = str(module.__name__)
        return self._generated_constants_module

    def resolve_constants(
        self,
        application: Any,  # noqa: ANN401
        names: tuple[str, ...],
    ) -> tuple[dict[str, int], list[str]]:
        """Resolve enum names from the installed type library; never substitute literals."""
        self._ensure_generated_module(application)
        found: dict[str, int] = {}
        missing: list[str] = []
        constants: Any = None
        try:
            constants = self.win32com.constants
        except Exception:
            constants = None
        for name in names:
            value = getattr(constants, name, None) if constants is not None else None
            if isinstance(value, bool) or not isinstance(value, int):
                missing.append(name)
            else:
                found[name] = int(value)
        return found, missing

    def describe_typelib(self, obj: Any) -> dict[str, Any]:  # noqa: ANN401
        """Read installed type-library metadata from a live COM object."""
        try:
            ole_object = obj._oleobj_
            type_info = ole_object.GetTypeInfo()
            type_attributes = type_info.GetTypeAttr()
            typelib, _index = type_info.GetContainingTypeLib()
            library_attributes = typelib.GetLibAttr()
            library_name = ""
            try:
                library_name = str(typelib.GetDocumentation(-1)[0])
            except Exception:
                library_name = ""
            methods: dict[str, Any] = {}
            for index in range(int(type_attributes.cFuncs)):
                try:
                    function = type_info.GetFuncDesc(index)
                    names = tuple(type_info.GetNames(function.memid, 64))
                except Exception:
                    continue
                if not names:
                    continue
                name = str(names[0])
                parameters: list[dict[str, Any]] = []
                for parameter in function.args:
                    flags = int(parameter.flags)
                    transport = int(parameter.tdesc.vt)
                    parameters.append({"flags": flags, "vt": transport})
                methods[name] = {
                    "c_params": int(function.cParams),
                    "invkind": int(function.invkind),
                    "parameters": parameters,
                }
            return {
                "status": "ok",
                "library": library_name,
                "library_version": (
                    f"{int(library_attributes.wMajorVerNum)}.{int(library_attributes.wMinorVerNum)}"
                ),
                "type_kind": int(type_attributes.typekind),
                "c_funcs": int(type_attributes.cFuncs),
                "methods": methods,
            }
        except Exception as exc:
            return {"status": "unavailable", "error": bounded_message(exc), "methods": {}}

    def variant_byref_float(self, value: float) -> Any:  # noqa: ANN401
        return self.pythoncom.VARIANT(self.pythoncom.VT_BYREF | self.pythoncom.VT_R8, float(value))

    def variant_byref_int(self, value: int) -> Any:  # noqa: ANN401
        return self.pythoncom.VARIANT(self.pythoncom.VT_BYREF | self.pythoncom.VT_I2, int(value))

    def variant_byref_float_array(self, values: list[float]) -> Any:  # noqa: ANN401
        return self.pythoncom.VARIANT(
            self.pythoncom.VT_BYREF | self.pythoncom.VT_ARRAY | self.pythoncom.VT_R8,
            [float(value) for value in values],
        )

    def variant_byref_variant_array(self, values: list[Any]) -> Any:  # noqa: ANN401
        return self.pythoncom.VARIANT(
            self.pythoncom.VT_BYREF | self.pythoncom.VT_ARRAY | self.pythoncom.VT_VARIANT,
            list(values),
        )


@dataclass
class _Command:
    operation: Callable[[BindingProtocol], Any]
    reply: queue.Queue[tuple[bool, Any]]


class Session:
    """One STA thread with a matched COM initialization and idle message pump."""

    def __init__(
        self,
        *,
        binding_factory: Callable[[], BindingProtocol] = ComBinding,
        join_timeout: float = 5.0,
    ) -> None:
        self._binding_factory = binding_factory
        self._join_timeout = join_timeout
        self._commands: queue.Queue[_Command | None] = queue.Queue()
        self._ready = threading.Event()
        self._thread: threading.Thread | None = None
        self._binding: BindingProtocol | None = None
        self._start_error: BaseException | None = None
        self._stopped = False

    def start(self, timeout: float = 15.0) -> None:
        """Start the STA thread and wait until COM is initialized."""
        if self._thread is not None:
            raise RuntimeError("session already started")
        self._thread = threading.Thread(target=self._thread_main, name="kala-se-sta", daemon=True)
        self._thread.start()
        if not self._ready.wait(timeout):
            raise SessionTimeout("the COM apartment did not start before the deadline")
        if self._start_error is not None:
            raise self._start_error

    def _thread_main(self) -> None:
        binding: BindingProtocol | None = None
        try:
            binding = self._binding_factory()
            binding.coinitialize()
            self._binding = binding
            self._ready.set()
            while True:
                command = self._commands.get()
                if command is None:
                    break
                try:
                    value = command.operation(binding)
                except Exception as exc:
                    command.reply.put((False, exc))
                else:
                    command.reply.put((True, value))
                try:
                    binding.pump()
                except Exception:
                    pass
        except BaseException as exc:  # startup failures are delivered to the caller
            self._start_error = exc
            self._ready.set()
        finally:
            if binding is not None:
                try:
                    binding.couninitialize()
                except Exception:
                    pass
            self._stopped = True

    def run(self, operation: Callable[[BindingProtocol], T], *, timeout: float | None = None) -> T:
        """Submit one callable to the STA thread and return its result."""
        if self._thread is None:
            raise RuntimeError("session is not started")
        if self._stopped:
            raise ComUnavailable("the COM apartment has stopped")
        reply: queue.Queue[tuple[bool, Any]] = queue.Queue(maxsize=1)
        self._commands.put(_Command(operation=operation, reply=reply))
        try:
            ok, value = reply.get(timeout=timeout)
        except queue.Empty as exc:
            raise SessionTimeout(
                "the COM call did not return before the deadline; its state is indeterminate"
            ) from exc
        if ok:
            return cast(T, value)
        if isinstance(value, BaseException):
            raise value
        raise ComCallError("the COM call failed with an unknown error")

    def stop(self) -> bool:
        """Ask the thread to uninitialize and return whether it stopped cleanly."""
        if self._thread is None:
            return True
        self._commands.put(None)
        self._thread.join(self._join_timeout)
        return not self._thread.is_alive()

    @property
    def stopped(self) -> bool:
        return self._stopped


__all__ = (
    "COINIT_APARTMENTTHREADED",
    "DEFAULT_RETRY_DEADLINE_SECONDS",
    "DISP_E_TYPEMISMATCH",
    "E_INVALIDARG",
    "MK_E_UNAVAILABLE",
    "RETRYABLE_HRESULTS",
    "RPC_E_CALL_REJECTED",
    "RPC_E_CHANGED_MODE",
    "RPC_E_SERVERCALL_RETRYLATER",
    "BindingProtocol",
    "ComBinding",
    "ComCallError",
    "ComUnavailable",
    "Session",
    "SessionTimeout",
    "StaError",
    "bounded_message",
    "call_with_retry",
    "hresult_of",
    "is_retryable",
    "normalize_hresult",
)
