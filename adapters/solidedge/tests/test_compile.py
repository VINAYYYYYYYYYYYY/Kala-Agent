"""The compiler refuses the unsupported subset before any socket exists."""

from __future__ import annotations

from collections.abc import Mapping, Sequence

import pytest
from pydantic import ValidationError

from kala_agent.contracts.construction import (
    AnalysisScript,
    ConstructionError,
    ConstructionScript,
    load_construction,
)
from kala_agent_solidedge.compile import (
    ASSEMBLY_NAME,
    build_request,
    compile_operations,
    new_request_id,
)
from kala_agent_solidedge.errors import CompileError
from kala_solidedge_bridge.protocol import (
    MAX_OPERATIONS,
    AssemblyNew,
    AssemblyPlace,
    BuildRequest,
    PartNew,
    SketchCircle,
    SketchRectangle,
    SolidCut,
    SolidExtrude,
    SolidRevolve,
)


def script(
    components: Sequence[Mapping[str, object]],
    assembly: Sequence[Mapping[str, object]] | None = None,
) -> ConstructionScript:
    return load_construction(
        {
            "summary": "test script",
            "components": components,
            "assembly": assembly or [],
        }
    )


def rectangle_component(
    name: str = "plate",
    *,
    plane: str = "XY",
    width: float = 20.0,
    height: float = 20.0,
    length: float = 10.0,
    x: float = 0.0,
    y: float = 0.0,
    reversed_: bool = False,
) -> dict[str, object]:
    return {
        "name": name,
        "steps": [
            {
                "op": "sketch",
                "profile": "rectangle",
                "plane": plane,
                "width": width,
                "height": height,
                "x": x,
                "y": y,
            },
            {"op": "extrude", "length": length, "reversed": reversed_},
        ],
    }


def test_plate_with_hole_compiles_to_the_documented_procedure() -> None:
    operations = compile_operations(
        script(
            [
                {
                    "name": "plate",
                    "steps": [
                        {"op": "sketch", "profile": "rectangle", "width": 20, "height": 20},
                        {"op": "extrude", "length": 10},
                        {
                            "op": "sketch",
                            "profile": "circle",
                            "diameter": 2,
                            "x": 5,
                            "y": 5,
                        },
                        {"op": "pocket"},
                    ],
                }
            ]
        )
    )

    assert [operation.op for operation in operations] == [
        "part.new",
        "sketch.rectangle",
        "solid.extrude",
        "sketch.circle",
        "solid.cut",
        "part.measure",
        "part.save",
        "part.export_step",
    ]
    cut = operations[4]
    assert isinstance(cut, SolidCut)
    assert cut.length is None


def test_every_plane_keeps_its_offsets_and_the_canonical_name() -> None:
    for plane in ("XY", "XZ", "YZ"):
        operations = compile_operations(script([rectangle_component(plane=plane, x=2.0, y=3.0)]))
        sketch = operations[1]
        assert isinstance(sketch, SketchRectangle)
        assert sketch.plane.value == plane
        assert (sketch.x, sketch.y) == (2.0, 3.0)


def test_reversed_extrusion_is_preserved_not_dropped() -> None:
    operations = compile_operations(script([rectangle_component(reversed_=True)]))
    extrude = operations[2]
    assert isinstance(extrude, SolidExtrude)
    assert extrude.reversed is True


def test_rectangle_revolve_on_xz_is_refused_before_submission() -> None:
    component = {
        "name": "wheel",
        "steps": [
            {"op": "sketch", "profile": "rectangle", "plane": "XZ", "width": 4, "height": 5},
            {"op": "revolve", "angle": 360},
        ],
    }

    with pytest.raises(CompileError, match="XZ"):
        compile_operations(script([component]))


def test_rectangle_revolve_on_xy_and_yz_becomes_solid_revolve() -> None:
    for plane in ("XY", "YZ"):
        component = {
            "name": "wheel",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": plane, "width": 4, "height": 5},
                {"op": "revolve"},
            ],
        }
        revolve = compile_operations(script([component]))[2]
        assert isinstance(revolve, SolidRevolve)
        assert revolve.angle == 360.0
        assert revolve.axis == "Y"


def test_circle_revolve_is_lowered_to_a_pad_with_length_then_diameter() -> None:
    def component(length: float | None) -> dict[str, object]:
        step: dict[str, object] = {"op": "revolve"}
        if length is not None:
            step["length"] = length
        return {
            "name": "pin",
            "steps": [
                {"op": "sketch", "profile": "circle", "diameter": 6},
                step,
            ],
        }

    with_length = compile_operations(script([component(12.0)]))[2]
    assert isinstance(with_length, SolidExtrude)
    assert with_length.length == 12.0

    without_length = compile_operations(script([component(None)]))[2]
    assert isinstance(without_length, SolidExtrude)
    assert without_length.length == 6.0


def test_multiple_additive_features_extend_one_part() -> None:
    operations = compile_operations(
        script(
            [
                {
                    "name": "stack",
                    "steps": [
                        {"op": "sketch", "profile": "rectangle", "width": 10, "height": 10},
                        {"op": "extrude", "length": 2},
                        {"op": "sketch", "profile": "circle", "diameter": 4, "x": 5, "y": 5},
                        {"op": "extrude", "length": 6},
                    ],
                }
            ]
        )
    )

    assert [operation.op for operation in operations] == [
        "part.new",
        "sketch.rectangle",
        "solid.extrude",
        "sketch.circle",
        "solid.extrude",
        "part.measure",
        "part.save",
        "part.export_step",
    ]
    assert sum(1 for operation in operations if isinstance(operation, PartNew)) == 1


def test_helix_is_refused() -> None:
    component = {
        "name": "screw",
        "steps": [
            {"op": "sketch", "profile": "circle", "diameter": 16},
            {"op": "extrude", "length": 40},
            {"op": "helix", "diameter": 16, "pitch": 4, "length": 24},
        ],
    }

    with pytest.raises(CompileError, match="helix"):
        compile_operations(script([component]))


def test_fuse_and_compound_are_refused_not_reinterpreted() -> None:
    components = [rectangle_component("a"), rectangle_component("b", x=12.0)]
    for op in ("fuse", "compound"):
        with pytest.raises(CompileError, match="not supported"):
            compile_operations(
                script(
                    components,
                    [{"op": op, "components": ["a", "b"]}],
                )
            )


def test_analysis_and_fillets_are_refused() -> None:
    parsed = script([rectangle_component()])
    analysis = AnalysisScript.model_validate(
        {"steps": [{"op": "material", "name": "Steel"}, {"op": "solve"}]}
    )

    with pytest.raises(CompileError, match="analysis"):
        compile_operations(parsed, analysis=analysis)
    with pytest.raises(CompileError, match="fillets"):
        compile_operations(parsed, fillets={"plate": 1.0})


def test_duplicate_placement_is_refused() -> None:
    with pytest.raises(CompileError, match="more than once"):
        compile_operations(
            script(
                [rectangle_component("a"), rectangle_component("b")],
                [
                    {"op": "place", "component": "a"},
                    {"op": "place", "component": "a"},
                ],
            )
        )


def test_a_single_untranslated_component_needs_no_assembly() -> None:
    operations = compile_operations(script([rectangle_component()]))
    assert not any(operation.op.startswith("assembly.") for operation in operations)


def test_a_single_translated_component_still_needs_an_assembly() -> None:
    operations = compile_operations(
        script(
            [rectangle_component()],
            [{"op": "place", "component": "plate", "x": 5, "y": 0, "z": 0}],
        )
    )

    assert isinstance(operations[-1], type(operations[-1]))
    assert operations[-4].op == "assembly.new"
    assert operations[-3].op == "assembly.place"
    assert operations[-2].op == "assembly.save"
    assert operations[-1].op == "assembly.export_step"


def test_every_component_is_placed_once_with_defaults_filled() -> None:
    operations = compile_operations(
        script(
            [rectangle_component("a"), rectangle_component("b")],
            [{"op": "place", "component": "b", "x": 12}],
        )
    )

    assembly_new = [operation for operation in operations if operation.op == "assembly.new"]
    assert len(assembly_new) == 1
    assert (
        any(
            isinstance(operation, PartNew) and operation.name == ASSEMBLY_NAME
            for operation in operations
        )
        is False
    )
    placements = [operation for operation in operations if isinstance(operation, AssemblyPlace)]
    assert [(place.component, place.x, place.y, place.z) for place in placements] == [
        ("a", 0.0, 0.0, 0.0),
        ("b", 12.0, 0.0, 0.0),
    ]


def test_the_request_factory_uses_public_types_and_keeps_identity() -> None:
    parsed = script([rectangle_component()])
    request_id = new_request_id()
    request = build_request(parsed, request_id=request_id, calibration_id="cal-1")

    assert isinstance(request, BuildRequest)
    assert request.request_id == request_id
    assert request.calibration_id == "cal-1"
    assert (
        request.request_digest()
        == build_request(parsed, request_id="other-id", calibration_id="cal-1").request_digest()
    )


def test_the_wire_cap_is_the_protocol_cap() -> None:
    parsed = script([rectangle_component()])
    request = build_request(parsed, request_id="cap-1")

    assert len(request.procedure) <= MAX_OPERATIONS
    assert BuildRequest.model_validate_json(request.model_dump_json()) == request


def test_a_maximum_core_ir_script_fits_the_wire_request() -> None:
    # Eight components, each at the core IR's twelve-step cap: a pad and then
    # four additive features, which is the widest legal core script.
    steps: list[Mapping[str, object]] = [
        {"op": "sketch", "profile": "rectangle", "width": 10, "height": 10},
        {"op": "extrude", "length": 1},
    ]
    while len(steps) < 12:
        steps.append({"op": "sketch", "profile": "circle", "diameter": 2, "x": 1, "y": 1})
        steps.append({"op": "extrude", "length": 1})
    components = [{"name": f"part{i}", "steps": steps} for i in range(8)]

    request = build_request(script(components), request_id="wide-1")
    encoded = request.model_dump_json().encode("utf-8")

    assert len(encoded) < 1024 * 1024
    assert len(request.procedure) <= MAX_OPERATIONS
    assert request.procedure[-1].op == "assembly.export_step"
    assert sum(1 for op in request.procedure if op.op == "part.export_step") == 8


def test_a_pocket_with_a_length_is_a_finite_cut() -> None:
    operations = compile_operations(
        script(
            [
                {
                    "name": "plate",
                    "steps": [
                        {"op": "sketch", "profile": "rectangle", "width": 20, "height": 20},
                        {"op": "extrude", "length": 10},
                        {
                            "op": "sketch",
                            "profile": "circle",
                            "diameter": 2,
                            "x": 5,
                            "y": 5,
                        },
                        {"op": "pocket", "length": 4},
                    ],
                }
            ]
        )
    )

    cut = operations[4]
    assert isinstance(cut, SolidCut)
    assert cut.length == 4.0


def test_an_empty_script_is_refused_by_the_contract() -> None:
    with pytest.raises(ConstructionError):
        load_construction({"summary": "empty", "components": []})


def test_a_bad_request_id_is_a_compile_error_not_a_raw_validation_error() -> None:
    with pytest.raises(CompileError, match="request_id"):
        build_request(script([rectangle_component()]), request_id="not safe!")


def test_the_protocol_rejects_a_duplicate_part_name_independently() -> None:
    with pytest.raises(ValidationError):
        BuildRequest(
            request_id="dup-1",
            procedure=(PartNew(name="a"), PartNew(name="a")),
        )


def test_a_sketch_circle_keeps_its_plane_and_offset() -> None:
    operations = compile_operations(
        script(
            [
                {
                    "name": "pin",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "circle",
                            "plane": "YZ",
                            "diameter": 6,
                            "x": 1.5,
                            "y": -2.5,
                        },
                        {"op": "extrude", "length": 9},
                    ],
                }
            ]
        )
    )

    sketch = operations[1]
    assert isinstance(sketch, SketchCircle)
    assert (sketch.plane.value, sketch.diameter, sketch.x, sketch.y) == (
        "YZ",
        6.0,
        1.5,
        -2.5,
    )


def test_dimension_parity_between_the_core_ir_and_the_wire() -> None:
    """The core bounds and the wire bounds must agree, not merely overlap."""
    from kala_agent.contracts.construction import MAX_DIMENSION_MM
    from kala_solidedge_bridge.protocol import MAX_OFFSET_MM, MAX_SIZE_MM, MIN_SIZE_MM

    assert MAX_DIMENSION_MM == MAX_SIZE_MM == 5000.0
    assert MIN_SIZE_MM == 0.001
    assert MAX_OFFSET_MM == 5000.0

    biggest = build_request(
        script(
            [
                rectangle_component(
                    width=MAX_SIZE_MM,
                    height=MAX_SIZE_MM,
                    length=MAX_SIZE_MM,
                    x=MAX_OFFSET_MM,
                    y=-MAX_OFFSET_MM,
                )
            ]
        ),
        request_id="limit-1",
    )
    sketch = biggest.procedure[1]
    assert isinstance(sketch, SketchRectangle)
    assert (sketch.width, sketch.height) == (5000.0, 5000.0)
    assert (sketch.x, sketch.y) == (5000.0, -5000.0)

    smallest = build_request(
        script([rectangle_component(width=0.001, height=0.001, length=0.001)]),
        request_id="limit-2",
    )
    small_sketch = smallest.procedure[1]
    assert isinstance(small_sketch, SketchRectangle)
    assert (small_sketch.width, small_sketch.height) == (0.001, 0.001)


def test_an_assembly_name_never_collides_with_a_component_name() -> None:
    operations = compile_operations(
        script(
            [rectangle_component("Assembly"), rectangle_component("b")],
        )
    )

    names = [operation.name for operation in operations if isinstance(operation, AssemblyNew)]
    assert names == ["Assembly1"]
