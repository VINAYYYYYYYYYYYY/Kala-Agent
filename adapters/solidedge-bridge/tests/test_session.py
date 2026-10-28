"""STA session, lazy imports, HRESULT normalization, and retry policy."""

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import threading
import time
from pathlib import Path
from types import SimpleNamespace
from typing import Any, cast

import pytest

from kala_solidedge_bridge.session import (
    RPC_E_CALL_REJECTED,
    RPC_E_SERVERCALL_RETRYLATER,
    BindingProtocol,
    ComBinding,
    ComCallError,
    ComUnavailable,
    ConstantResolution,
    Session,
    SessionTimeout,
    StaError,
    _server_executable,
    call_with_retry,
    enum_members,
    function_arguments,
    hresult_of,
    is_retryable,
    lib_attributes,
    member_names,
    normalize_hresult,
    resolve_enum_names,
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


# --- pywin32 312 type-library and VARIANT shapes (fakes, runs on any host) ---


class FakeVariable:
    def __init__(self, memid: int, value: int) -> None:
        self.memid = memid
        self.value = value


class FakeTypeInfo:
    """A ``TKIND_ENUM``-shaped type description with pywin32 312 method shapes."""

    def __init__(
        self,
        name: str,
        members: tuple[tuple[str, int], ...] = (),
        *,
        typekind: int = 0,
        broken_index: int | None = None,
        nested_names: bool = False,
    ) -> None:
        self.name = name
        self.members = members
        self.typekind = typekind
        self.broken_index = broken_index
        self.nested_names = nested_names
        self.released_type_attrs: list[Any] = []
        self.released_vars: list[Any] = []

    def GetTypeAttr(self) -> Any:  # noqa: ANN401
        return SimpleNamespace(typekind=self.typekind, cVars=len(self.members), cFuncs=0)

    def ReleaseTypeAttr(self, attributes: Any) -> None:  # noqa: ANN401
        self.released_type_attrs.append(attributes)

    def ReleaseVarDesc(self, variable: Any) -> None:  # noqa: ANN401
        self.released_vars.append(variable)

    def GetVarDesc(self, index: int) -> FakeVariable:
        if self.broken_index == index:
            raise RuntimeError("GetVarDesc failed")
        name, value = self.members[index]
        del name
        return FakeVariable(index, value)

    def GetNames(self, memid: int) -> Any:  # noqa: ANN401
        # pywin32 312 accepts only memid; a second argument raises TypeError,
        # which is what member_names' fallback must cope with.
        name = self.members[memid][0]
        if self.nested_names:
            return (["nested"], name)
        return (name,)

    def GetDocumentation(self, index: int) -> tuple[str, str, str, str]:
        del index
        return (self.name, "", "", "")


class FakeTypeLib:
    def __init__(self, library: str, infos: tuple[FakeTypeInfo, ...]) -> None:
        self.library = library
        self.infos = infos

    def GetDocumentation(self, index: int) -> tuple[str, str, str, str]:
        del index
        return (self.library, "", "", "")

    def GetLibAttr(self) -> tuple[str, int, int, int, int, int]:
        return ("{00000000-0000-0000-0000-000000000003}", 0, 1, 4, 0, 0)

    def GetTypeInfoCount(self) -> int:
        return len(self.infos)

    def GetTypeInfo(self, index: int) -> FakeTypeInfo:
        return self.infos[index]


class FakeFunction:
    """A pywin32 312 ``FUNCDESC``-shaped method description."""

    def __init__(self, memid: int, c_params: int, arguments: tuple[tuple[int, int], ...]) -> None:
        self.memid = memid
        self.cParams = c_params
        self.invkind = 1
        self.args = tuple((transport, flags, None) for transport, flags in arguments)


class FakeMethodTypeInfo:
    """A ``TKIND_DISPATCH``-shaped type description with methods."""

    def __init__(
        self,
        name: str,
        methods: tuple[tuple[str, int, tuple[tuple[int, int], ...]], ...],
        *,
        broken_index: int | None = None,
        library: str = "SolidEdgePart",
    ) -> None:
        self.name = name
        self.methods = methods
        self.broken_index = broken_index
        self.library = library
        self.released_type_attrs: list[Any] = []
        self.released_funcs: list[Any] = []

    def GetTypeAttr(self) -> Any:  # noqa: ANN401
        return SimpleNamespace(typekind=3, cVars=0, cFuncs=len(self.methods))

    def ReleaseTypeAttr(self, attributes: Any) -> None:  # noqa: ANN401
        self.released_type_attrs.append(attributes)

    def GetFuncDesc(self, index: int) -> FakeFunction:
        if self.broken_index == index:
            raise RuntimeError("GetFuncDesc failed")
        name, c_params, arguments = self.methods[index]
        del name
        return FakeFunction(index, c_params, arguments)

    def ReleaseFuncDesc(self, function: Any) -> None:  # noqa: ANN401
        self.released_funcs.append(function)

    def GetNames(self, memid: int) -> tuple[str, ...]:
        return (self.methods[memid][0],)

    def GetContainingTypeLib(self) -> tuple[FakeTypeLib, int]:
        return (FakeTypeLib(self.library, ()), 0)


class FakeObjectWithTypeInfo:
    def __init__(self, type_info: Any) -> None:  # noqa: ANN401
        self._oleobj_ = SimpleNamespace(GetTypeInfo=lambda: type_info)


class BrokenTypeLib:
    def GetDocumentation(self, index: int) -> tuple[str, str, str, str]:
        raise RuntimeError("this library cannot be read")


def test_lib_attributes_accepts_the_pywin32_tuple_shape() -> None:
    assert lib_attributes(("{00000000-0000-0000-0000-000000000001}", 0, 1, 2, 3, 0)) == {
        "guid": "{00000000-0000-0000-0000-000000000001}",
        "lcid": 0,
        "major": 2,
        "minor": 3,
    }


def test_lib_attributes_accepts_an_object_shape_and_wraps_single_values() -> None:
    raw = SimpleNamespace(
        guid="{00000000-0000-0000-0000-000000000002}",
        lcid=1033,
        wMajorVerNum=1,
        wMinorVerNum=0,
    )
    assert lib_attributes(raw) == {
        "guid": "{00000000-0000-0000-0000-000000000002}",
        "lcid": 1033,
        "major": 1,
        "minor": 0,
    }
    assert lib_attributes((raw,))["guid"] == raw.guid


def test_lib_attributes_rejects_an_empty_result() -> None:
    with pytest.raises(ValueError):
        lib_attributes(())


def test_member_names_reads_one_argument_and_drops_nested_tuples() -> None:
    info = FakeTypeInfo(
        "FeaturePropertyConstants",
        (("igSymmetric", 3),),
        nested_names=True,
    )
    assert member_names(info, 0) == ("igSymmetric",)


def test_function_arguments_reads_the_pywin32_tuple_shape() -> None:
    function = SimpleNamespace(args=(((26, (27, 5)), 3, None),))
    assert function_arguments(function) == ((26, 3),)


def test_function_arguments_accepts_object_shaped_descriptions() -> None:
    parameter = SimpleNamespace(tdesc=SimpleNamespace(vt=5), flags=1)
    function = SimpleNamespace(args=(parameter,))
    assert function_arguments(function) == ((5, 1),)


def test_enum_members_reads_only_tkind_enum_members() -> None:
    enum = FakeTypeInfo(
        "ProfileValidationType",
        (("igProfileClosed", 1), ("igProfileSingle", 4)),
    )
    module = FakeTypeInfo("SomeModule", (("igSymmetric", 3),), typekind=1)

    assert enum_members(enum) == {"igProfileClosed": 1, "igProfileSingle": 4}
    assert enum_members(module) == {}


def test_enum_members_skips_a_member_that_cannot_be_read() -> None:
    enum = FakeTypeInfo(
        "FeaturePropertyConstants",
        (("igSymmetric", 3), ("igBroken", 9)),
        broken_index=1,
    )
    assert enum_members(enum) == {"igSymmetric": 3}


def test_resolve_enum_names_scans_libraries_and_records_the_source() -> None:
    part = FakeTypeLib(
        "SolidEdgePart",
        (
            FakeTypeInfo(
                "ProfileValidationType",
                (("igProfileClosed", 1),),
            ),
            FakeTypeInfo(
                "FeaturePropertyConstants",
                (("igSymmetric", 3),),
            ),
        ),
    )
    constants = FakeTypeLib(
        "SolidEdgeConstants",
        (
            FakeTypeInfo(
                "ModelingModeConstants",
                (("seModelingModeOrdered", 2),),
            ),
        ),
    )

    resolution = resolve_enum_names(
        (part, constants),
        ("igSymmetric", "igProfileClosed", "seModelingModeOrdered"),
    )

    assert isinstance(resolution, ConstantResolution)
    assert resolution.values == {
        "igSymmetric": 3,
        "igProfileClosed": 1,
        "seModelingModeOrdered": 2,
    }
    assert resolution.missing == []
    assert resolution.sources["igSymmetric"] == "SolidEdgePart.FeaturePropertyConstants"
    assert resolution.sources["seModelingModeOrdered"] == (
        "SolidEdgeConstants.ModelingModeConstants"
    )


def test_resolve_enum_names_reports_missing_names_without_substituting_a_literal() -> None:
    part = FakeTypeLib(
        "SolidEdgePart",
        (FakeTypeInfo("FeaturePropertyConstants", (("igSymmetric", 3),)),),
    )

    resolution = resolve_enum_names((part,), ("igSymmetric", "igNotInThisVersion"))

    assert resolution.values == {"igSymmetric": 3}
    assert resolution.missing == ["igNotInThisVersion"]
    assert "igNotInThisVersion" not in resolution.sources


def test_resolve_enum_names_survives_an_unreadable_library() -> None:
    readable = FakeTypeLib(
        "SolidEdgePart",
        (FakeTypeInfo("FeaturePropertyConstants", (("igSymmetric", 3),)),),
    )

    resolution = resolve_enum_names((BrokenTypeLib(), readable), ("igSymmetric",))

    assert resolution.values == {"igSymmetric": 3}
    assert resolution.missing == []


def test_server_executable_extracts_a_quoted_or_bare_path() -> None:
    quoted = '"C:\\Program Files\\Siemens\\Solid Edge 2024\\Program\\Edge.exe" /Automation'
    bare = "C:\\SolidEdge\\Program\\Edge.exe -automation"
    assert _server_executable(quoted) == (
        "C:\\Program Files\\Siemens\\Solid Edge 2024\\Program\\Edge.exe"
    )
    assert _server_executable(bare) == "C:\\SolidEdge\\Program\\Edge.exe"
    assert _server_executable("   ") is None


class FakeVariantConstructor:
    def __init__(self) -> None:
        self.calls: list[tuple[int, Any]] = []

    def __call__(self, variant_type: int, value: Any) -> SimpleNamespace:  # noqa: ANN401
        self.calls.append((variant_type, value))
        return SimpleNamespace(vt=variant_type, value=value)


def _binding_without_pywin32() -> ComBinding:
    """Build a ComBinding instance without importing pywin32 (Linux CI safe)."""
    return object.__new__(ComBinding)


def _attach_modules(binding: ComBinding, win32com: Any, pythoncom: Any) -> None:  # noqa: ANN401
    """Attach fake pywin32 modules; ``ComBinding`` types them as real modules."""
    binding.win32com = win32com
    binding.pythoncom = pythoncom


def test_variant_helpers_prefer_the_win32com_constructor() -> None:
    constructor = FakeVariantConstructor()
    binding = _binding_without_pywin32()
    _attach_modules(
        binding,
        SimpleNamespace(VARIANT=constructor),
        SimpleNamespace(
            VT_BYREF=0x4000,
            VT_R8=5,
            VT_I2=2,
            VT_ARRAY=0x2000,
            VT_VARIANT=12,
        ),
    )

    variant = binding.variant_byref_float(1.5)

    assert variant.value == 1.5
    assert constructor.calls == [(0x4000 | 5, 1.5)]


def test_enum_members_releases_every_type_info_descriptor() -> None:
    enum = FakeTypeInfo(
        "ProfileValidationType",
        (("igProfileClosed", 1), ("igProfileSingle", 4)),
    )

    assert enum_members(enum) == {"igProfileClosed": 1, "igProfileSingle": 4}
    assert len(enum.released_type_attrs) == 1
    assert [variable.value for variable in enum.released_vars] == [1, 4]


def test_enum_members_releases_descriptors_even_when_a_member_is_unreadable() -> None:
    enum = FakeTypeInfo(
        "FeaturePropertyConstants",
        (("igSymmetric", 3), ("igBroken", 9)),
        broken_index=1,
    )

    assert enum_members(enum) == {"igSymmetric": 3}
    assert len(enum.released_type_attrs) == 1
    assert len(enum.released_vars) == 1


def test_enum_members_tolerates_a_fake_without_release_methods() -> None:
    class NoReleaseTypeInfo:
        def GetTypeAttr(self) -> Any:  # noqa: ANN401
            return SimpleNamespace(typekind=0, cVars=1, cFuncs=0)

        def GetVarDesc(self, index: int) -> Any:  # noqa: ANN401
            return SimpleNamespace(memid=index, value=3)

        def GetNames(self, memid: int) -> tuple[str, ...]:
            del memid
            return ("igSymmetric",)

    assert enum_members(NoReleaseTypeInfo()) == {"igSymmetric": 3}


def test_a_failing_release_does_not_fail_the_type_info_read() -> None:
    class ExplodingReleaseTypeInfo(FakeTypeInfo):
        def ReleaseTypeAttr(self, attributes: Any) -> None:  # noqa: ANN401
            raise RuntimeError("already released")

    enum = ExplodingReleaseTypeInfo("ProfileValidationType", (("igProfileClosed", 1),))

    assert enum_members(enum) == {"igProfileClosed": 1}


def test_describe_typelib_releases_type_and_function_descriptors() -> None:
    info = FakeMethodTypeInfo(
        "PartDocument",
        (
            ("Measure", 1, ((26, 3),)),
            ("SaveAs", 2, ((8, 1), (8, 1))),
        ),
    )

    described = ComBinding.describe_typelib(
        _binding_without_pywin32(), FakeObjectWithTypeInfo(info)
    )

    assert described["status"] == "ok"
    assert described["library_version"] == "4.0"
    assert described["c_funcs"] == 2
    assert described["methods"]["Measure"]["c_params"] == 1
    assert described["methods"]["SaveAs"]["c_params"] == 2
    assert len(info.released_type_attrs) == 1
    assert len(info.released_funcs) == 2


def test_describe_typelib_releases_descriptors_when_a_method_cannot_be_read() -> None:
    info = FakeMethodTypeInfo(
        "PartDocument",
        (
            ("Measure", 1, ((26, 3),)),
            ("Broken", 1, ()),
        ),
        broken_index=1,
    )

    described = ComBinding.describe_typelib(
        _binding_without_pywin32(), FakeObjectWithTypeInfo(info)
    )

    assert described["status"] == "ok"
    assert list(described["methods"]) == ["Measure"]
    assert len(info.released_type_attrs) == 1
    assert len(info.released_funcs) == 1


def test_variant_helpers_fall_back_to_pythoncom_and_fail_usefully() -> None:
    constructor = FakeVariantConstructor()
    binding = _binding_without_pywin32()
    _attach_modules(
        binding,
        SimpleNamespace(),
        SimpleNamespace(
            VARIANT=constructor,
            VT_BYREF=0x4000,
            VT_R8=5,
            VT_I2=2,
            VT_ARRAY=0x2000,
            VT_VARIANT=12,
        ),
    )

    assert binding.variant_byref_int(7).value == 7

    broken = _binding_without_pywin32()
    _attach_modules(
        broken,
        SimpleNamespace(),
        SimpleNamespace(VT_BYREF=0x4000, VT_R8=5),
    )
    with pytest.raises(ComUnavailable):
        broken.variant_byref_float(1.0)
