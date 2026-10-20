"""DesignService orchestration, journal, cache, and export tests."""

from __future__ import annotations

import hashlib
import json
import math
import sqlite3
import tempfile
from collections.abc import Iterator
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

from kala_agent.contracts.construction import ConstructionScript
from kala_agent.studio.design.intent import DesignIntent
from kala_agent.studio.design.service import (
    DesignOutcome,
    DesignRejectedError,
    DesignService,
    canonical_script_json,
    design_id_for,
)
from kala_agent.studio.design.templates import l_bracket

ADR_VOLUME_MM3 = 35_571.416047
ADR_AREA_MM2 = 11_889.101731


def _intent() -> DesignIntent:
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


@pytest.fixture
def data_dir() -> Iterator[Path]:
    with tempfile.TemporaryDirectory(prefix="kala-design-service-") as handle:
        yield Path(handle)


@pytest.fixture
def service(data_dir: Path) -> Iterator[DesignService]:
    instance = DesignService(data_dir)
    try:
        yield instance
    finally:
        instance.close()


def _event_types(service: DesignService) -> list[str]:
    return [event.event_type for event in service.journal_page(0, 500)]


def test_design_measures_the_adr_lbracket(service: DesignService) -> None:
    outcome = service.design(_intent(), provider="opencascade")
    assert outcome.measurement.volume_mm3 == pytest.approx(ADR_VOLUME_MM3, rel=1e-6)
    assert outcome.measurement.bounds is not None
    assert outcome.measurement.bounds.min_mm == (-30.0, 0.0, 0.0)
    assert outcome.measurement.bounds.max_mm == (30.0, 45.0, 40.0)
    assert outcome.verified is True
    assert {check.id for check in outcome.checks} == {
        "brep_valid",
        "single_solid",
        "envelope_x",
        "envelope_y",
        "envelope_z",
    }
    assert outcome.design_id == design_id_for(_script_for(outcome), "opencascade")
    assert outcome.script_sha256 != outcome.design_id
    assert outcome.view.triangle_count > 0
    assert outcome.declared_envelope is not None


def _script_for(outcome: DesignOutcome) -> ConstructionScript:
    return ConstructionScript.model_validate(outcome.script)


def test_design_journals_the_whole_turn(service: DesignService) -> None:
    outcome = service.design(_intent(), provider="opencascade")
    events = service.journal_page(0, 50)
    assert [event.event_type for event in events] == [
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
    assert events[3].payload is not None
    assert events[3].payload["sha256"] == outcome.script_sha256
    artifact = service.journal.get_artifact_text(outcome.script_sha256)
    assert hashlib.sha256(artifact.encode("utf-8")).hexdigest() == outcome.script_sha256
    assert service.verify().ok is True


def test_same_script_twice_is_a_cache_hit_not_a_second_compile(
    service: DesignService,
) -> None:
    first = service.design(_intent(), provider="opencascade")
    second = service.design(_intent(), provider="opencascade")
    assert first.design_id == second.design_id
    assert first.cached is False
    assert second.cached is True
    events = service.journal_page(0, 50)
    compiled = [event for event in events if event.event_type == "design.compiled"]
    intents = [event for event in events if event.event_type == "design.intent"]
    assert len(compiled) == 1
    assert len(intents) == 2
    assert intents[1].payload is not None
    assert intents[1].payload["cache_hit"] is True


def test_a_cold_cache_recompiles_from_the_stored_script(
    data_dir: Path,
) -> None:
    with DesignService(data_dir) as first_service:
        first = first_service.design(_intent(), provider="opencascade")
    with DesignService(data_dir) as second_service:
        second = second_service.load(first.design_id)
    assert second.design_id == first.design_id
    assert second.measurement.volume_mm3 == pytest.approx(first.measurement.volume_mm3, rel=1e-12)
    assert second.verified is True
    assert second.declared_envelope is not None
    assert [check.id for check in second.checks] == [check.id for check in first.checks]


def test_a_cold_reload_restores_the_surface_area_from_the_journal_bytes(
    data_dir: Path,
) -> None:
    """The area survives a restart: the reader reads the key the writer writes.

    The ``provider.build.completed`` payload is the record a cold reload reads
    its measurement from. It writes ``surface_area_mm2``; an earlier reader
    looked for ``area_mm2`` instead, so a design restored from the journal
    lost its area. This asserts against the journal's own canonical bytes and
    then against the restored outcome.
    """
    with DesignService(data_dir) as first_service:
        first = first_service.design(_intent(), provider="opencascade")
        completed = next(
            event
            for event in first_service.journal_page(0, 500)
            if event.event_type == "provider.build.completed"
        )
    assert first.measurement.area_mm2 is not None
    assert first.measurement.area_mm2 == pytest.approx(ADR_AREA_MM2, rel=1e-6)
    stored = json.loads(completed.payload_json)["measured"]
    assert "area_mm2" not in stored
    assert stored["surface_area_mm2"] == pytest.approx(first.measurement.area_mm2, rel=1e-12)
    with DesignService(data_dir) as second_service:
        second = second_service.load(first.design_id)
    assert second.cached is True
    assert second.measurement.area_mm2 == pytest.approx(first.measurement.area_mm2, rel=1e-12)
    assert second.measurement.volume_mm3 == pytest.approx(first.measurement.volume_mm3, rel=1e-12)
    assert second.verified is True


def test_the_measured_reader_keeps_the_legacy_area_key_as_a_fallback() -> None:
    from kala_agent.studio.design.service import _measured_from_payload

    assert _measured_from_payload(
        {"volume_mm3": 1.0, "surface_area_mm2": 2.5}
    ).area_mm2 == pytest.approx(2.5)
    assert _measured_from_payload({"volume_mm3": 1.0, "area_mm2": 3.5}).area_mm2 == pytest.approx(
        3.5
    )
    assert _measured_from_payload(
        {"surface_area_mm2": None, "area_mm2": 4.5}
    ).area_mm2 == pytest.approx(4.5)
    assert _measured_from_payload({}).area_mm2 is None


def test_a_compile_refusal_writes_design_rejected_and_no_artifact(
    service: DesignService,
) -> None:
    script = {
        "summary": "unsupported helix",
        "components": [
            {
                "name": "shaft",
                "steps": [
                    {
                        "op": "sketch",
                        "profile": "circle",
                        "plane": "XY",
                        "diameter": 10,
                    },
                    {"op": "extrude", "length": 20},
                    {"op": "helix", "diameter": 10, "pitch": 2, "length": 10},
                ],
            }
        ],
    }
    with pytest.raises(DesignRejectedError, match="helix"):
        service.design(DesignIntent.model_validate({"script": script}), provider="opencascade")
    design_id = design_id_for(ConstructionScript.model_validate(script), "opencascade")
    assert service.journal.has_artifact(design_id) is False
    types = _event_types(service)
    assert "design.rejected" in types
    assert "design.compiled" not in types
    assert service.verify().ok is True


def test_infeasible_template_values_are_rejected_before_any_event(
    service: DesignService,
) -> None:
    intent = DesignIntent.model_validate(
        {
            "template": {
                "template": "l_bracket",
                "width": 10,
                "depth": 45,
                "height": 40,
                "wall": 8,
                "hole_d": 5.5,
                "hole_count": 4,
            }
        }
    )
    with pytest.raises(DesignRejectedError, match="edge distance"):
        service.design(intent, provider="opencascade")
    assert _event_types(service) == []


def test_raw_script_has_validity_checks_but_no_envelope_checks(
    service: DesignService,
) -> None:
    script = {
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
                        "x": -10,
                        "y": -15,
                    },
                    {"op": "extrude", "length": 40},
                ],
            }
        ],
    }
    outcome = service.design(
        DesignIntent.model_validate({"script": script}), provider="opencascade"
    )
    assert [check.id for check in outcome.checks] == ["brep_valid", "single_solid"]
    assert outcome.declared_envelope is None
    assert outcome.measurement.volume_mm3 == pytest.approx(20 * 30 * 40, rel=1e-12)
    assert outcome.verified is True


def test_section_matches_criterion_4(service: DesignService) -> None:
    outcome = service.design(_intent(), provider="opencascade")
    section = service.section(outcome.design_id, plane="XZ", offset_mm=4)
    assert len(section.outer_loops) == 1
    assert len(section.inner_loops) == 1
    assert section.area_mm2 == pytest.approx(2400 - 25 * math.pi, rel=1e-3)
    assert section.area_exact is True
    outer = section.outer_loops[0]
    assert outer.bounds.width == pytest.approx(60, abs=0.05)
    assert outer.bounds.height == pytest.approx(40, abs=0.05)
    inner = section.inner_loops[0]
    assert inner.bounds.width == pytest.approx(10, abs=0.05)
    assert inner.centroid[0] == pytest.approx(0, abs=0.05)
    assert inner.centroid[1] == pytest.approx(24, abs=0.05)


def test_drawing_has_all_four_views(service: DesignService) -> None:
    outcome = service.design(_intent(), provider="opencascade")
    drawing = service.drawing(outcome.design_id, ("front", "top", "right", "iso"))
    assert drawing.view_names == ("front", "top", "right", "iso")
    assert all(view.point_count > 0 for view in drawing.views)
    assert drawing.total_point_count > 0


def test_export_step_is_digested_and_stored(service: DesignService) -> None:
    outcome = service.design(_intent(), provider="opencascade")
    exported = service.export(outcome.design_id, "step")
    assert exported.data.startswith(b"ISO-10303-21")
    assert exported.sha256 == hashlib.sha256(exported.data).hexdigest()
    assert service.journal.has_artifact(exported.sha256) is True
    assert service.step_digest(outcome.design_id) == exported.sha256
    types = _event_types(service)
    assert types[-2:] == ["artifact.stored", "export.completed"]


def test_export_stl_starts_with_the_design_header(service: DesignService) -> None:
    outcome = service.design(_intent(), provider="opencascade")
    exported = service.export(outcome.design_id, "stl")
    assert exported.data.startswith(b"kala-studio design mesh")
    assert exported.sha256 == hashlib.sha256(exported.data).hexdigest()


def test_export_refuses_any_other_format(service: DesignService) -> None:
    outcome = service.design(_intent(), provider="opencascade")
    with pytest.raises(ValueError):
        service.export(outcome.design_id, "iges")


def test_tampering_with_a_payload_breaks_the_chain(data_dir: Path) -> None:
    with DesignService(data_dir) as service:
        service.design(_intent(), provider="opencascade")
        assert service.verify().ok is True
        connection = sqlite3.connect(data_dir / "studio-journal.sqlite3")
        connection.execute(
            "UPDATE studio_events SET payload_json = payload_json || ' ' WHERE sequence = 4"
        )
        connection.commit()
        connection.close()
        verification = service.verify()
        assert verification.ok is False
        assert verification.first_bad_sequence == 4


def test_concurrent_designs_all_succeed_and_verify(service: DesignService) -> None:
    with ThreadPoolExecutor(max_workers=8) as pool:
        outcomes = list(
            pool.map(lambda _index: service.design(_intent(), provider="opencascade"), range(8))
        )
    assert len({outcome.design_id for outcome in outcomes}) == 1
    assert all(
        outcome.measurement.volume_mm3 == pytest.approx(ADR_VOLUME_MM3, rel=1e-6)
        for outcome in outcomes
    )
    compiled = [
        event for event in service.journal_page(0, 500) if event.event_type == "design.compiled"
    ]
    assert len(compiled) == 1
    assert service.verify().ok is True


def test_chat_turn_payload_never_stores_prompt_text(service: DesignService) -> None:
    secret = "operator note: my token is not a credential, but this text is private"
    event = service.record_chat_turn(
        provider="local",
        model="deterministic-paraphrase-v1",
        offline=True,
        prompt=secret,
        design_id=None,
        design_requested=False,
    )
    assert event.payload is not None
    assert "private" not in event.payload_json
    assert event.payload["prompt_sha256"] == hashlib.sha256(secret.encode()).hexdigest()


def test_journal_lives_in_the_supplied_studio_data_dir(
    data_dir: Path,
) -> None:
    with DesignService(data_dir) as service:
        assert service.journal.database_path == data_dir / "studio-journal.sqlite3"
        assert service.journal.artifact_root == data_dir / "artifacts"
        service.design(_intent(), provider="opencascade")
    assert (data_dir / "studio-journal.sqlite3").is_file()


def test_unknown_design_is_not_found(service: DesignService) -> None:
    from kala_agent.studio.design.service import DesignNotFoundError

    with pytest.raises(DesignNotFoundError):
        service.load("0" * 64)


def test_template_script_is_the_one_cached_for_the_design(service: DesignService) -> None:
    outcome = service.design(_intent(), provider="opencascade")
    template_script = l_bracket(
        width=60,
        depth=45,
        height=40,
        wall=8,
        bore_d=10,
        hole_d=5.5,
        hole_count=4,
    ).script
    assert canonical_script_json(template_script) == service.journal.get_artifact_text(
        outcome.script_sha256
    ).encode("utf-8")
