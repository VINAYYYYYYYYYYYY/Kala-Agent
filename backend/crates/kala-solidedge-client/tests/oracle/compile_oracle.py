"""Live Python oracle for the ConstructionScript lowering differential.

The oracle imports the repository's own compiler and the Python compile
tests, lowers every case, and prints one JSON document describing what the
reference produced. The Rust test compares its own lowering against this
document byte for byte, as canonical JSON.

The script is run with an isolated interpreter (``python -I``). Isolated mode
ignores ``PYTHONPATH``, so the repository paths are inserted here instead.

Usage::

    python -I compile_oracle.py [repository-root]
"""

from __future__ import annotations

import json
import os
import sys
from typing import Any


def _repository_root() -> str:
    if len(sys.argv) > 1:
        return os.path.abspath(sys.argv[1])
    here = os.path.dirname(os.path.abspath(__file__))
    return os.path.abspath(os.path.join(here, "..", "..", "..", "..", ".."))


def _prepare(root: str) -> None:
    """Put the repository packages on the import path."""
    for part in (
        "src",
        os.path.join("adapters", "solidedge", "src"),
        os.path.join("adapters", "solidedge-bridge", "src"),
    ):
        path = os.path.join(root, part)
        if path not in sys.path:
            sys.path.insert(0, path)


def _canonical(operations: tuple[Any, ...]) -> str:
    payload = [operation.model_dump(mode="json") for operation in operations]
    return json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
        allow_nan=False,
    )


def _canonical_script(script: Any) -> str:
    """The canonical JSON text of a validated script."""
    return json.dumps(
        script.model_dump(mode="json"),
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
    )


def _build_cases(test_compile: Any) -> list[dict[str, Any]]:
    """Rebuild the cases of the Python compile tests.

    The helper functions come from the live test module, so the scripts are
    literally the ones the Python suite compiles.
    """
    script = test_compile.script
    rectangle_component = test_compile.rectangle_component

    cases: list[dict[str, Any]] = []

    def add(
        identifier: str,
        parsed: Any,
        analysis: Any = None,
        fillets: dict[str, float] | None = None,
    ) -> None:
        cases.append(
            {
                "id": identifier,
                "raw": None,
                "script": _canonical_script(parsed),
                "analysis": None if analysis is None else _canonical_script(analysis),
                "fillets": fillets,
            }
        )

    add(
        "compile_test:plate_with_hole",
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
        ),
    )

    for plane in ("XY", "XZ", "YZ"):
        add(
            f"compile_test:plane_{plane.lower()}",
            script([rectangle_component(plane=plane, x=2.0, y=3.0)]),
        )

    add(
        "compile_test:reversed_extrusion",
        script([rectangle_component(reversed_=True)]),
    )

    add(
        "compile_test:rectangle_revolve_on_xz_refused",
        script(
            [
                {
                    "name": "wheel",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "rectangle",
                            "plane": "XZ",
                            "width": 4,
                            "height": 5,
                        },
                        {"op": "revolve", "angle": 360},
                    ],
                }
            ]
        ),
    )

    for plane in ("XY", "YZ"):
        add(
            f"compile_test:rectangle_revolve_on_{plane.lower()}",
            script(
                [
                    {
                        "name": "wheel",
                        "steps": [
                            {
                                "op": "sketch",
                                "profile": "rectangle",
                                "plane": plane,
                                "width": 4,
                                "height": 5,
                            },
                            {"op": "revolve"},
                        ],
                    }
                ]
            ),
        )

    for label, length in (("with_length", 12.0), ("without_length", None)):
        step: dict[str, Any] = {"op": "revolve"}
        if length is not None:
            step["length"] = length
        add(
            f"compile_test:circle_revolve_{label}",
            script(
                [
                    {
                        "name": "pin",
                        "steps": [
                            {"op": "sketch", "profile": "circle", "diameter": 6},
                            step,
                        ],
                    }
                ]
            ),
        )

    add(
        "compile_test:multiple_additive_features",
        script(
            [
                {
                    "name": "stack",
                    "steps": [
                        {"op": "sketch", "profile": "rectangle", "width": 10, "height": 10},
                        {"op": "extrude", "length": 2},
                        {
                            "op": "sketch",
                            "profile": "circle",
                            "diameter": 4,
                            "x": 5,
                            "y": 5,
                        },
                        {"op": "extrude", "length": 6},
                    ],
                }
            ]
        ),
    )

    add(
        "compile_test:helix_refused",
        script(
            [
                {
                    "name": "screw",
                    "steps": [
                        {"op": "sketch", "profile": "circle", "diameter": 16},
                        {"op": "extrude", "length": 40},
                        {"op": "helix", "diameter": 16, "pitch": 4, "length": 24},
                    ],
                }
            ]
        ),
    )

    for op in ("fuse", "compound"):
        add(
            f"compile_test:{op}_refused",
            script(
                [rectangle_component("a"), rectangle_component("b", x=12.0)],
                [{"op": op, "components": ["a", "b"]}],
            ),
        )

    analysis = test_compile.AnalysisScript.model_validate(
        {"steps": [{"op": "material", "name": "Steel"}, {"op": "solve"}]}
    )
    add(
        "compile_test:analysis_refused",
        script([rectangle_component()]),
        analysis=analysis,
    )
    add(
        "compile_test:fillets_refused",
        script([rectangle_component()]),
        fillets={"plate": 1.0},
    )

    add(
        "compile_test:duplicate_placement_refused",
        script(
            [rectangle_component("a"), rectangle_component("b")],
            [
                {"op": "place", "component": "a"},
                {"op": "place", "component": "a"},
            ],
        ),
    )

    add("compile_test:single_untranslated", script([rectangle_component()]))
    add(
        "compile_test:single_translated",
        script(
            [rectangle_component()],
            [{"op": "place", "component": "plate", "x": 5, "y": 0, "z": 0}],
        ),
    )
    add(
        "compile_test:every_component_placed_once",
        script(
            [rectangle_component("a"), rectangle_component("b")],
            [{"op": "place", "component": "b", "x": 12}],
        ),
    )

    steps: list[dict[str, Any]] = [
        {"op": "sketch", "profile": "rectangle", "width": 10, "height": 10},
        {"op": "extrude", "length": 1},
    ]
    while len(steps) < 12:
        steps.append({"op": "sketch", "profile": "circle", "diameter": 2, "x": 1, "y": 1})
        steps.append({"op": "extrude", "length": 1})
    add(
        "compile_test:widest_core_script",
        script([{"name": f"part{index}", "steps": steps} for index in range(8)]),
    )

    add(
        "compile_test:pocket_with_a_length",
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
        ),
    )

    add(
        "compile_test:sketch_circle_keeps_plane_and_offset",
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
        ),
    )

    add(
        "compile_test:dimension_parity_biggest",
        script(
            [
                rectangle_component(
                    width=5000.0,
                    height=5000.0,
                    length=5000.0,
                    x=5000.0,
                    y=-5000.0,
                )
            ]
        ),
    )
    add(
        "compile_test:dimension_parity_smallest",
        script([rectangle_component(width=0.001, height=0.001, length=0.001)]),
    )

    add(
        "compile_test:assembly_name_never_collides",
        script([rectangle_component("Assembly"), rectangle_component("b")]),
    )

    return cases


def _fixture_cases(root: str) -> list[dict[str, Any]]:
    path = os.path.join(root, "tests", "fixtures", "construction_parity.json")
    with open(path, encoding="utf-8") as handle:
        document = json.load(handle)
    cases: list[dict[str, Any]] = []
    for case in document["cases"]:
        cases.append(
            {
                "id": f"fixture:{case['id']}",
                "raw": case["script"],
                "script": None,
                "analysis": None,
                "fillets": None,
            }
        )
    return cases


def main() -> int:
    root = _repository_root()
    _prepare(root)

    from kala_agent_solidedge.compile import compile_operations
    from kala_agent_solidedge.errors import CompileError

    tests_root = os.path.join(root, "adapters", "solidedge", "tests")
    if tests_root not in sys.path:
        sys.path.insert(0, tests_root)
    try:
        import test_compile
    except ImportError as error:  # pragma: no cover - the venv always has pytest
        print(f"the compile oracle could not import the Python compile tests: {error}", file=sys.stderr)
        return 2

    cases = _fixture_cases(root) + _build_cases(test_compile)
    results: list[dict[str, Any]] = []
    for case in cases:
        from kala_agent.contracts.construction import load_construction

        raw = case["raw"] if case["raw"] is not None else json.loads(case["script"])
        parsed = load_construction(raw)
        analysis = case["analysis"]
        if analysis is not None:
            from kala_agent.contracts.construction import AnalysisScript

            analysis = AnalysisScript.model_validate(json.loads(analysis))
        fillets = case["fillets"]
        try:
            operations = compile_operations(parsed, analysis=analysis, fillets=fillets)
        except CompileError as error:
            results.append(
                {
                    "id": case["id"],
                    "ok": False,
                    "message": str(error),
                    "script": _canonical_script(parsed),
                    "analysis": case["analysis"],
                    "fillets": case["fillets"],
                }
            )
        else:
            results.append(
                {
                    "id": case["id"],
                    "ok": True,
                    "canonical": _canonical(operations),
                    "script": _canonical_script(parsed),
                    "analysis": case["analysis"],
                    "fillets": case["fillets"],
                }
            )

    json.dump({"cases": results}, sys.stdout, ensure_ascii=True, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
