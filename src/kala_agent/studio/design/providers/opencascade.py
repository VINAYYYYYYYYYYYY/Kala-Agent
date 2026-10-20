"""The built-in OpenCascade provider (ADR-0011, opt-in only).

This provider wraps the existing ADR-0009 OCP compiler, measurement, and STEP
writer. It is **never selected automatically**: the caller must name
``provider: "opencascade"``. Its STEP export is stored like any other
provider's, and the service imports those bytes back for the display view, so
the viewer always shows what the provider exported.

OCP is imported lazily. Importing this module on a host without the bindings
must be safe; :meth:`OpenCascadeProvider.probe` reports the absence instead of
raising.
"""

from __future__ import annotations

import os
import tempfile
from pathlib import Path
from typing import TYPE_CHECKING, ClassVar, Final

from kala_agent.studio.design.importer import resolve_step_import_max_bytes
from kala_agent.studio.design.providers.base import (
    Availability,
    CadProviderBuildError,
    CadProviderUnavailableError,
    MeasuredBounds,
    MeasuredProperties,
    ProviderBuild,
)

if TYPE_CHECKING:
    from OCP.TopoDS import TopoDS_Shape  # type: ignore[import-untyped]

    from kala_agent.contracts.construction import ConstructionScript
    from kala_agent.studio.design.intent import DesignIntent

DEFAULT_PROVIDER_VERSION: Final[str] = "unknown"
STEP_HEADER: Final[bytes] = b"ISO-10303-21"
STEP_FILE_PREFIX: Final[str] = "kala-ocp-provider-"
UNAVAILABLE_REASON: Final[str] = (
    "the OCP bindings are not importable; install 'cadquery-ocp' to enable the "
    "opt-in OpenCascade provider and the view-only STEP importer"
)


class OpenCascadeProvider:
    """The opt-in builder: the existing OCP compiler and measure path."""

    name: ClassVar[str] = "opencascade"
    display_name: ClassVar[str] = "OpenCascade (opt-in)"
    measured_by: ClassVar[str] = "OpenCascade (opt-in)"
    mode: ClassVar[str] = "opt-in"

    def __init__(self, *, max_step_bytes: int | None = None) -> None:
        self._max_step_bytes = (
            resolve_step_import_max_bytes() if max_step_bytes is None else max_step_bytes
        )

    def probe(self) -> Availability:
        """Return whether the OCP bindings are importable."""
        try:
            import OCP  # type: ignore[import-untyped]
        except Exception:
            return Availability(available=False, version=None, reason=UNAVAILABLE_REASON)
        version = getattr(OCP, "__version__", None)
        return Availability(
            available=True,
            version=str(version) if version else DEFAULT_PROVIDER_VERSION,
            reason=None,
        )

    def build(self, script: ConstructionScript, intent: DesignIntent) -> ProviderBuild:
        """Compile, measure, and export ``script`` with OpenCascade."""
        del intent
        availability = self.probe()
        if not availability.available:
            raise CadProviderUnavailableError(
                self.name,
                f"the {self.name} CAD provider is not available",
                reason=availability.reason or UNAVAILABLE_REASON,
            )

        from kala_agent.contracts.construction import ConstructionError
        from kala_agent.studio.design.compile_ocp import compile_script
        from kala_agent.studio.design.measure import measure_shape

        try:
            compilation = compile_script(script)
        except ConstructionError as error:
            raise CadProviderBuildError(f"opencascade refused the script: {error}") from None
        try:
            measurement = measure_shape(compilation.assembly_shape)
        except Exception as error:
            raise CadProviderBuildError(
                f"OpenCascade could not measure the compiled shape: {type(error).__name__}"
            ) from None
        step_bytes = _write_step(compilation.assembly_shape, max_step_bytes=self._max_step_bytes)
        if len(step_bytes) > self._max_step_bytes:
            raise CadProviderBuildError(
                "the OpenCascade STEP export is larger than the configured STEP import "
                f"limit of {self._max_step_bytes} bytes"
            )
        bounds = MeasuredBounds(
            min_mm=measurement.bounds.min_mm,
            max_mm=measurement.bounds.max_mm,
        )
        return ProviderBuild(
            step_bytes=step_bytes,
            measured=MeasuredProperties(
                volume_mm3=measurement.volume_mm3,
                area_mm2=measurement.surface_area_mm2,
                bounds=bounds,
                mass_g=None,
                centroid_mm=measurement.centroid_mm,
            ),
            provider_version=availability.version,
            warnings=compilation.warnings,
            native_bytes=None,
            native_kind=None,
            valid=measurement.valid,
            solid_count=measurement.solid_count,
        )


def _write_step(shape: TopoDS_Shape, *, max_step_bytes: int) -> bytes:
    """Write one shape to STEP bytes and remove the temporary file.

    The file size is checked before it is read back, so a writer that produces
    an export above the configured ceiling is refused without loading it.
    """
    from OCP.IFSelect import IFSelect_RetDone  # type: ignore[import-untyped]
    from OCP.STEPControl import (  # type: ignore[import-untyped]
        STEPControl_AsIs,
        STEPControl_Writer,
    )

    descriptor, path_str = tempfile.mkstemp(prefix=STEP_FILE_PREFIX, suffix=".step")
    os.close(descriptor)
    path = Path(path_str)
    try:
        writer = STEPControl_Writer()
        transfer_status = writer.Transfer(shape, STEPControl_AsIs)
        if transfer_status != IFSelect_RetDone:
            raise CadProviderBuildError("the OpenCascade STEP writer refused the compiled shape")
        write_status = writer.Write(str(path))
        if write_status != IFSelect_RetDone:
            raise CadProviderBuildError("the OpenCascade STEP writer did not finish serialising")
        size = path.stat().st_size
        if size > max_step_bytes:
            raise CadProviderBuildError(
                f"the OpenCascade STEP export is {size} bytes, above the configured "
                f"{max_step_bytes} byte STEP import limit"
            )
        data = path.read_bytes()
    except CadProviderBuildError:
        raise
    except Exception as error:
        raise CadProviderBuildError(
            f"the OpenCascade STEP writer failed: {type(error).__name__}"
        ) from None
    finally:
        try:
            path.unlink()
        except OSError:
            pass
    if not data.startswith(STEP_HEADER):
        raise CadProviderBuildError(
            "the OpenCascade STEP writer produced a non-ISO file; refusing to ship it"
        )
    return data


__all__ = ("UNAVAILABLE_REASON", "OpenCascadeProvider")
