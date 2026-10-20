"""CAD provider routing, honesty checks, and API contracts (ADR-0011).

These tests use the opt-in OpenCascade provider and small injected fake
providers. The real FreeCADCmd path is covered by
``test_cad_providers_freecad.py``; the large STEP import by
``test_cad_providers_import.py``.
"""

from __future__ import annotations

import hashlib
from collections.abc import Iterator, Mapping
from pathlib import Path
from typing import Any, cast

import pytest
from fastapi.testclient import TestClient

from kala_agent.contracts.construction import ConstructionScript
from kala_agent.studio.design.intent import DesignIntent
from kala_agent.studio.design.measure import Bounds
from kala_agent.studio.design.providers import (
    ALLOWED_PROVIDER_IDS,
    OpenCascadeProvider,
    SolidEdgeProvider,
)
from kala_agent.studio.design.providers.base import (
    Availability,
    CadProvider,
    CadProviderUnavailableError,
    MeasuredBounds,
    MeasuredProperties,
    ProviderBuild,
)
from kala_agent.studio.design.service import (
    DesignRejectedError,
    DesignService,
    build_provider_checks,
    design_available,
    design_id_for,
)
from kala_agent.studio.design.templates import l_bracket
from kala_agent.studio.server import create_studio_app

pytestmark = pytest.mark.skipif(
    not design_available(), reason="the OCP geometry kernel is required for provider routing"
)

TOKEN = "studio-provider-token-0123456789abcdef"
AUTH = {"Authorization": f"Bearer {TOKEN}"}
STITCH_PROMPT = (
    "Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm wall "
    "thickness, with a 10 mm centered bore and 4x M5 corner mounting holes."
)
BOX_VOLUME_MM3 = 20.0 * 30.0 * 40.0
ADR_VOLUME_MM3 = 35_571.416047


def _lbracket_intent() -> DesignIntent:
    return DesignIntent(
        template={
            "template": "l_bracket",
            "width": 60,
            "depth": 45,
            "height": 40,
            "wall": 8,
            "bore_d": 10,
            "hole_d": 5.5,
            "hole_count": 4,
        },
        material="aluminium",
    )


def _lbracket_script() -> ConstructionScript:
    return l_bracket(
        width=60,
        depth=45,
        height=40,
        wall=8,
        bore_d=10,
        hole_d=5.5,
        hole_count=4,
    ).script


def _box_script() -> ConstructionScript:
    return ConstructionScript.model_validate(
        {
            "summary": "20 x 30 x 40 box",
            "components": [
                {
                    "name": "box",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "rectangle",
                            "plane": "XY",
                            "width": 20,
                            "height": 30,
                        },
                        {"op": "extrude", "length": 40},
                    ],
                }
            ],
        }
    )


def _two_part_assembly_script() -> ConstructionScript:
    """A place-only assembly of two boxes: the script declares two solids."""
    return ConstructionScript.model_validate(
        {
            "summary": "two placed 10 x 10 x 5 boxes",
            "components": [
                {
                    "name": "base",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "rectangle",
                            "plane": "XY",
                            "width": 10,
                            "height": 10,
                        },
                        {"op": "extrude", "length": 5},
                    ],
                },
                {
                    "name": "riser",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "rectangle",
                            "plane": "XY",
                            "width": 10,
                            "height": 10,
                        },
                        {"op": "extrude", "length": 5},
                    ],
                },
            ],
            "assembly": [
                {"op": "place", "component": "base", "x": 0, "y": 0, "z": 0},
                {"op": "place", "component": "riser", "x": 20, "y": 0, "z": 0},
            ],
        }
    )


@pytest.fixture(scope="module")
def box_step() -> bytes:
    provider = OpenCascadeProvider()
    script = _box_script()
    return provider.build(script, DesignIntent(script=script)).step_bytes


def _box_build(step_bytes: bytes, *, measured: MeasuredProperties | None = None) -> ProviderBuild:
    return ProviderBuild(
        step_bytes=step_bytes,
        measured=measured
        or MeasuredProperties(
            volume_mm3=BOX_VOLUME_MM3,
            area_mm2=2 * (20 * 30 + 20 * 40 + 30 * 40),
            bounds=MeasuredBounds(min_mm=(0.0, 0.0, 0.0), max_mm=(20.0, 30.0, 40.0)),
            centroid_mm=(10.0, 15.0, 20.0),
        ),
        provider_version="fake-1.0",
        warnings=("a fake provider warning",),
        native_bytes=b"fake native bytes",
        native_kind="fcstd",
        valid=True,
        solid_count=1,
    )


class FakeProvider(CadProvider):
    """A deterministic provider used to prove the service never recompiles."""

    name = "freecad"
    display_name = "Fake FreeCAD"
    measured_by = "Fake FreeCAD"
    mode = "external"

    def __init__(
        self,
        build: ProviderBuild,
        *,
        available: bool = True,
        reason: str | None = None,
    ) -> None:
        self._build = build
        self._available = available
        self._reason = reason
        self.build_calls = 0
        self.probe_calls = 0
        self.seen_scripts: list[ConstructionScript] = []

    def probe(self) -> Availability:
        self.probe_calls += 1
        return Availability(
            available=self._available,
            version="fake-1.0" if self._available else None,
            reason=self._reason,
        )

    def build(self, script: ConstructionScript, intent: DesignIntent) -> ProviderBuild:
        del intent
        self.build_calls += 1
        self.seen_scripts.append(script)
        return self._build


def _service(
    data_dir: Path,
    providers: Mapping[str, CadProvider],
) -> DesignService:
    return DesignService(data_dir, providers=providers, max_step_bytes=8 * 1024 * 1024)


def _client(
    data_dir: Path,
    providers: Mapping[str, CadProvider] | None = None,
) -> TestClient:
    app = create_studio_app(
        data_dir=data_dir,
        api_token=TOKEN,
        max_body_bytes=64 * 1024,
        static_root=None,
        design_providers=providers,
        max_step_bytes=8 * 1024 * 1024,
    )
    return TestClient(
        app,
        base_url="http://127.0.0.1",
        headers=AUTH,
        raise_server_exceptions=False,
    )


@pytest.fixture
def client(tmp_path: Path) -> Iterator[TestClient]:
    with _client(tmp_path) as test_client:
        yield test_client


# ------------------------------------------------------------------ contract


def test_the_provider_set_is_closed_and_has_no_default() -> None:
    assert ALLOWED_PROVIDER_IDS == ("opencascade", "freecad", "solidedge")
    for provider in ALLOWED_PROVIDER_IDS:
        assert provider in {"opencascade", "freecad", "solidedge"}


def test_design_id_binds_the_provider() -> None:
    script = _lbracket_script()
    freecad_id = design_id_for(script, "freecad")
    opencascade_id = design_id_for(script, "opencascade")
    assert freecad_id != opencascade_id
    assert freecad_id == design_id_for(script, "freecad")
    assert len(freecad_id) == 64
    with pytest.raises(ValueError, match="unknown CAD provider"):
        design_id_for(script, "solidworks")


def test_service_refuses_a_missing_or_unknown_provider(tmp_path: Path) -> None:
    service = _service(tmp_path, {"opencascade": OpenCascadeProvider()})
    try:
        with pytest.raises(DesignRejectedError, match="choose one of"):
            service.design(_lbracket_intent(), provider="")
        with pytest.raises(DesignRejectedError, match="unknown CAD provider"):
            service.design(_lbracket_intent(), provider="solidworks")
        with pytest.raises(DesignRejectedError, match="not configured"):
            service.design(_lbracket_intent(), provider="freecad")
    finally:
        service.close()


# --------------------------------------------------------------- opencascade


def test_opencascade_provider_measures_the_lbracket() -> None:
    script = _lbracket_script()
    build = OpenCascadeProvider().build(script, _lbracket_intent())
    assert build.step_bytes.startswith(b"ISO-10303-21")
    assert build.measured.volume_mm3 == pytest.approx(ADR_VOLUME_MM3, rel=1e-6)
    assert build.measured.bounds is not None
    assert build.measured.bounds.min_mm == pytest.approx((-30.0, 0.0, 0.0))
    assert build.measured.bounds.max_mm == pytest.approx((30.0, 45.0, 40.0))
    assert build.valid is True
    assert build.solid_count == 1
    assert build.native_bytes is None
    assert build.provider_version


def test_service_stores_the_provider_step_and_imports_the_view(tmp_path: Path) -> None:
    service = _service(tmp_path, {"opencascade": OpenCascadeProvider()})
    try:
        outcome = service.design(_lbracket_intent(), provider="opencascade")
        assert outcome.provider == "opencascade"
        assert outcome.measured_by == "OpenCascade (opt-in)"
        assert outcome.measurement.volume_mm3 == pytest.approx(ADR_VOLUME_MM3, rel=1e-6)
        assert outcome.view_bounds.min_mm == pytest.approx((-30.0, 0.0, 0.0), abs=0.001)
        assert outcome.view_bounds.max_mm == pytest.approx((30.0, 45.0, 40.0), abs=0.001)
        assert outcome.view_solids == 1
        assert outcome.step_sha256 is not None
        assert service.journal.has_artifact(outcome.step_sha256)
        assert outcome.script_sha256 != outcome.design_id
        assert service.journal.has_artifact(outcome.script_sha256)
        events = service.journal_page(0, 100)
        types = [event.event_type for event in events]
        assert types == [
            "design.intent",
            "provider.build.requested",
            "provider.build.completed",
            "artifact.stored",
            "artifact.stored",
            "view.imported",
            "design.compiled",
            "design.measured",
            "design.checked",
        ]
        completed = events[2].payload
        assert completed is not None
        assert completed["provider"] == "opencascade"
        assert completed["step_sha256"] == outcome.step_sha256
        viewed = events[5].payload
        assert viewed is not None
        assert viewed["provider"] == "opencascade"
        assert viewed["role"] == "view-only"
    finally:
        service.close()


# ------------------------------------------------------------ view fidelity


def test_view_is_the_provider_export_not_a_recompile(tmp_path: Path, box_step: bytes) -> None:
    fake = FakeProvider(_box_build(box_step))
    service = _service(tmp_path, {"freecad": fake})
    try:
        outcome = service.design(_lbracket_intent(), provider="freecad")
    finally:
        service.close()
    # The script is the L-bracket, but the provider's STEP is the 20x30x40 box.
    assert outcome.provider == "freecad"
    assert outcome.measurement.volume_mm3 == pytest.approx(BOX_VOLUME_MM3)
    assert outcome.view_bounds.min_mm == pytest.approx((0.0, 0.0, 0.0), abs=0.001)
    assert outcome.view_bounds.max_mm == pytest.approx((20.0, 30.0, 40.0), abs=0.001)
    assert fake.build_calls == 1
    assert outcome.native_kind == "fcstd"
    assert outcome.native_sha256 == hashlib.sha256(b"fake native bytes").hexdigest()


def test_export_returns_the_stored_provider_bytes(tmp_path: Path, box_step: bytes) -> None:
    fake = FakeProvider(_box_build(box_step))
    service = _service(tmp_path, {"freecad": fake})
    try:
        outcome = service.design(_lbracket_intent(), provider="freecad")
        exported = service.export(outcome.design_id, "step")
    finally:
        service.close()
    assert exported.data == box_step
    assert exported.sha256 == outcome.step_sha256
    assert fake.build_calls == 1


def test_drawing_and_section_use_the_imported_provider_shape(
    tmp_path: Path, box_step: bytes
) -> None:
    fake = FakeProvider(_box_build(box_step))
    service = _service(tmp_path, {"freecad": fake})
    try:
        outcome = service.design(_lbracket_intent(), provider="freecad")
        drawing = service.drawing(outcome.design_id, ("front", "top"))
        section = service.section(outcome.design_id, plane="XZ", offset_mm=15)
        stl = service.export(outcome.design_id, "stl")
    finally:
        service.close()
    assert drawing.view_names == ("front", "top")
    assert all(view.point_count > 0 for view in drawing.views)
    assert section.area_mm2 == pytest.approx(20.0 * 40.0, rel=1e-6)
    assert len(section.outer_loops) == 1
    assert stl.data.startswith(b"kala-studio design mesh")
    assert fake.build_calls == 1


def test_reload_uses_the_stored_step_without_building_again(
    tmp_path: Path, box_step: bytes
) -> None:
    first = FakeProvider(_box_build(box_step))
    service = _service(tmp_path, {"freecad": first})
    try:
        outcome = service.design(_lbracket_intent(), provider="freecad")
    finally:
        service.close()
    second = FakeProvider(_box_build(box_step))
    reloaded = _service(tmp_path, {"freecad": second})
    try:
        loaded = reloaded.load(outcome.design_id)
    finally:
        reloaded.close()
    assert second.build_calls == 0
    assert loaded.cached is True
    assert loaded.measurement.volume_mm3 == pytest.approx(BOX_VOLUME_MM3)
    assert loaded.view_bounds.max_mm == pytest.approx((20.0, 30.0, 40.0), abs=0.001)
    assert loaded.provider == "freecad"


# ------------------------------------------------------------------ honesty


def test_missing_provider_values_are_not_reported(tmp_path: Path, box_step: bytes) -> None:
    fake = FakeProvider(
        ProviderBuild(
            step_bytes=box_step,
            measured=MeasuredProperties(),
            provider_version="fake-1.0",
            valid=None,
            solid_count=None,
        )
    )
    with _client(tmp_path, {"freecad": fake}) as client:
        response = client.post(
            "/api/studio/design",
            json={"provider": "freecad", **_lbracket_body()},
        )
        assert response.status_code == 200, response.text
        body = cast("dict[str, Any]", response.json())
        measured = cast("dict[str, Any]", body["measured"])
        assert measured["volume_mm3"] is None
        assert measured["surface_area_mm2"] is None
        assert measured["bounds"] is None
        assert measured["size_mm"] is None
        assert measured["valid"] is None
        assert measured["solid_count"] is None
        statuses = {row["id"]: row["status"] for row in body["checks"]}
        assert statuses == {
            "brep_valid": "not_reported",
            "single_solid": "not_reported",
            "envelope_x": "not_reported",
            "envelope_y": "not_reported",
            "envelope_z": "not_reported",
        }
        assert body["verified"] is False
        for row in body["checks"]:
            assert row["measured"] is None
            assert row["passed"] is False


def test_checks_compare_provider_bounds_with_the_declared_envelope(
    tmp_path: Path, box_step: bytes
) -> None:
    fake = FakeProvider(_box_build(box_step))
    service = _service(tmp_path, {"freecad": fake})
    try:
        outcome = service.design(_lbracket_intent(), provider="freecad")
    finally:
        service.close()
    statuses = {check.id: check.status for check in outcome.checks}
    assert statuses["envelope_x"] == "failed"
    assert statuses["envelope_y"] == "failed"
    assert statuses["envelope_z"] == "passed"
    assert outcome.verified is False


def test_provider_checks_never_substitute_a_bound() -> None:
    checks = build_provider_checks(
        measured=MeasuredProperties(),
        valid=None,
        solid_count=None,
        expected_envelope=Bounds(min_mm=(-1.0, -1.0, -1.0), max_mm=(1.0, 1.0, 1.0)),
    )
    assert all(check.measured is None for check in checks)
    assert all(check.status == "not_reported" for check in checks)
    assert not any(check.passed for check in checks)


def test_a_provider_solid_count_must_match_a_multi_part_script(
    tmp_path: Path, box_step: bytes
) -> None:
    # The fake build is one solid, but the place-only script declares two.
    fake = FakeProvider(_box_build(box_step))
    service = _service(tmp_path, {"freecad": fake})
    try:
        outcome = service.design(
            DesignIntent(script=_two_part_assembly_script()), provider="freecad"
        )
    finally:
        service.close()
    checks = {check.id: check for check in outcome.checks}
    assert set(checks) == {"brep_valid", "solid_count"}
    row = checks["solid_count"]
    assert row.label == "Solid count"
    assert (row.expected, row.measured) == (2.0, 1.0)
    assert row.status == "failed"
    assert outcome.verified is False


def test_a_missing_assembly_solid_count_is_not_reported(tmp_path: Path, box_step: bytes) -> None:
    fake = FakeProvider(
        ProviderBuild(
            step_bytes=box_step,
            measured=MeasuredProperties(),
            provider_version="fake-1.0",
            valid=True,
            solid_count=None,
        )
    )
    service = _service(tmp_path, {"freecad": fake})
    try:
        outcome = service.design(
            DesignIntent(script=_two_part_assembly_script()), provider="freecad"
        )
    finally:
        service.close()
    checks = {check.id: check for check in outcome.checks}
    assert set(checks) == {"brep_valid", "solid_count"}
    row = checks["solid_count"]
    assert row.expected == 2.0
    assert row.measured is None
    assert row.status == "not_reported"
    assert outcome.verified is False


# ----------------------------------------------------------------- solidedge


def test_solidedge_is_unavailable_on_linux_and_refuses_a_build() -> None:
    provider = SolidEdgeProvider(platform="linux")
    availability = provider.probe()
    assert availability.available is False
    assert availability.reason is not None
    assert "Windows" in availability.reason
    script = _lbracket_script()
    with pytest.raises(CadProviderUnavailableError, match="Windows bridge"):
        provider.build(script, _lbracket_intent())


def test_post_design_with_solidedge_is_a_503(client: TestClient) -> None:
    response = client.post(
        "/api/studio/design",
        json={"provider": "solidedge", **_lbracket_body()},
    )
    assert response.status_code == 503
    body = response.json()
    assert body["code"] == "provider_unavailable"
    joined = " ".join(body["details"])
    assert "Windows bridge" in joined
    assert "available:" in joined


# ---------------------------------------------------------------------- API


def test_provider_endpoint_lists_live_probe_results(client: TestClient) -> None:
    response = client.get("/api/studio/providers/cad")
    assert response.status_code == 200, response.text
    rows = {row["id"]: row for row in response.json()["providers"]}
    assert set(rows) == {"opencascade", "freecad", "solidedge"}
    assert rows["opencascade"]["available"] is True
    assert rows["opencascade"]["mode"] == "opt-in"
    assert rows["solidedge"]["available"] is False
    assert rows["solidedge"]["reason"]
    assert rows["freecad"]["mode"] == "external"
    assert rows["freecad"]["available"] is True
    assert rows["freecad"]["version"]


def test_missing_provider_is_422_naming_allowed_values(client: TestClient) -> None:
    response = client.post("/api/studio/design", json=_lbracket_body())
    assert response.status_code == 422
    body = response.json()
    assert body["code"] == "invalid_request"
    joined = " ".join(body["details"])
    for provider in ALLOWED_PROVIDER_IDS:
        assert provider in joined


def test_chat_without_a_provider_asks_the_user_to_choose(client: TestClient) -> None:
    response = client.post("/api/studio/chat", json={"provider": "local", "prompt": STITCH_PROMPT})
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["design"] is None
    text = body["text"].lower()
    assert "choose a cad provider" in text
    for provider in ALLOWED_PROVIDER_IDS:
        assert provider in text
    assert "no design was built" in text


def test_chat_with_an_unavailable_provider_does_not_build(client: TestClient) -> None:
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "local",
            "prompt": STITCH_PROMPT,
            "cad_provider": "solidedge",
        },
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["design"] is None
    text = body["text"].lower()
    assert "not available" in text
    assert "no design was built" in text


def test_chat_with_a_selected_provider_builds(client: TestClient) -> None:
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "local",
            "prompt": STITCH_PROMPT,
            "cad_provider": "opencascade",
        },
    )
    assert response.status_code == 200, response.text
    body = response.json()
    design = body["design"]
    assert design is not None
    assert design["provider"] == "opencascade"
    assert design["measured"]["volume_mm3"] == pytest.approx(ADR_VOLUME_MM3, rel=1e-6)


# ------------------------------------------------------------------ helpers


def _lbracket_body() -> dict[str, object]:
    return {
        "template": {
            "template": "l_bracket",
            "width": 60,
            "depth": 45,
            "height": 40,
            "wall": 8,
            "bore_d": 10,
            "hole_d": 5.5,
            "hole_count": 4,
        },
        "material": "aluminium",
    }


def test_provider_registry_import_does_not_touch_the_kernel() -> None:
    """The registry and its base contract are importable without OCP/FreeCAD."""
    import ast

    import kala_agent.studio.design.providers.base as base_module

    assert base_module.__file__ is not None
    tree = ast.parse(Path(base_module.__file__).read_text(encoding="utf-8"))
    imported: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            imported.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module:
            imported.add(node.module)
    assert not any(
        name == "OCP" or name.startswith("OCP.") or name.startswith("kala_agent_freecad")
        for name in imported
    )
