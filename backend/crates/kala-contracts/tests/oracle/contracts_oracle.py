#!/usr/bin/env python3
"""Live Python oracle for the native `kala-contracts` crate.

The production crate never invokes Python. This script imports the live
repository package, validates the same cases as the Rust tests and prints the
Python wire, canonical JSON and digest results. Run it through
`tests/python_differential.rs`.
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

DIGEST_A = "a" * 64
DIGEST_B = "b" * 64
DIGEST_C = "c" * 64
AT = "2026-01-02T03:04:05Z"


def _add_repo_source() -> None:
    repo = Path(__file__).resolve().parents[5]
    source = repo / "src"
    if source.is_dir() and str(source) not in sys.path:
        sys.path.insert(0, str(source))


def main() -> int:
    _add_repo_source()
    from pydantic import ValidationError

    from kala_agent.contracts import (
        ApprovalKind,
        ApprovalRequest,
        ApprovalStatus,
        ArtifactClassification,
        ArtifactRef,
        DesignRevision,
        DesignSpec,
        DomainPackageManifest,
        EvaluationMetric,
        EvaluationResult,
        PlanStep,
        Provenance,
        Requirement,
        RequirementKind,
        RequirementPriority,
        RequirementStatus,
        RunBudget,
        SideEffectLevel,
        ToolCall,
        ToolDescriptor,
        ToolResult,
        ToolRunStatus,
        WorkflowDefinition,
        WorkflowTransition,
    )
    from kala_agent.contracts.construction import (
        AnalysisReading,
        AnalysisScript,
        AnalysisStep,
        AssemblyStep,
        ComponentScript,
        ComponentStep,
        ConstructionError,
        ConstructionScript,
        EditStep,
        load_analysis,
        load_construction,
        load_reading,
    )

    models = {
        cls.__name__: cls
        for cls in [
            Requirement,
            DesignSpec,
            DomainPackageManifest,
            ArtifactRef,
            DesignRevision,
            PlanStep,
            WorkflowTransition,
            WorkflowDefinition,
            ToolDescriptor,
            ToolCall,
            ToolResult,
            EvaluationMetric,
            EvaluationResult,
            ApprovalRequest,
            RunBudget,
            Provenance,
            ComponentStep,
            ComponentScript,
            AssemblyStep,
            ConstructionScript,
            AnalysisStep,
            AnalysisScript,
            EditStep,
            AnalysisReading,
        ]
    }
    loaders = {
        "load_construction": load_construction,
        "load_analysis": load_analysis,
        "load_reading": load_reading,
    }

    cases: list[dict[str, object]] = []

    def add_text(model: str, text: str, loader: str | None = None, entry: str = "str") -> None:
        case: dict[str, object] = {
            "id": f"{loader or model}.{len(cases)}",
            "model": model,
            "loader": loader,
            "entry": entry,
            "input_json": text,
        }
        try:
            # FastAPI's JSON pipeline decodes the body with Python json.loads
            # (last-wins duplicate keys, insertion order, lone-surrogate
            # escapes preserved) and then validates the resulting mapping.
            raw = json.loads(text)
            if loader is not None:
                value = loaders[loader](raw)
            else:
                value = models[model].model_validate(raw)
            dump = value.model_dump(mode="json")
            canonical = json.dumps(
                dump, allow_nan=False, ensure_ascii=True, sort_keys=True, separators=(",", ":")
            )
            case["ok"] = True
            case["wire_ascii"] = json.dumps(
                dump, allow_nan=False, ensure_ascii=True, separators=(",", ":")
            )
            try:
                wire_utf8 = json.dumps(
                    dump, allow_nan=False, ensure_ascii=False, separators=(",", ":")
                )
                wire_utf8.encode("utf-8")
                case["wire_utf8"] = wire_utf8
            except UnicodeEncodeError:
                pass
            case["canonical"] = canonical
            case["digest"] = hashlib.sha256(canonical.encode("utf-8")).hexdigest()
        except ConstructionError as error:
            case["ok"] = False
            case["error_type"] = "ConstructionError"
            case["message"] = str(error)
        except ValidationError as error:
            case["ok"] = False
            case["error_type"] = "ValidationError"
            case["errors"] = [
                {"loc": list(detail["loc"]), "type": detail["type"], "msg": detail["msg"]}
                for detail in error.errors()
            ]
        cases.append(case)

    def add(model: str, raw: object, loader: str | None = None) -> None:
        add_text(model, json.dumps(raw, ensure_ascii=True, allow_nan=False), loader)

    # --- Requirement -------------------------------------------------------
    add("Requirement", {"id": "req-1", "title": "  Preserve  ", "kind": "functional"})
    add(
        "Requirement",
        {
            "id": " req-2 ",
            "title": "\u3000 spaced \u0085",
            "kind": "non_functional",
            "description": "A description.",
            "priority": "critical",
            "status": "approved",
            "acceptance_criteria": ["One.", "Two."],
            "tags": ["storage", "audit", "storage" if False else "b"],
        },
    )
    add("Requirement", {"id": "a" * 128, "title": "x", "kind": "constraint"})
    add("Requirement", {"id": "   ", "title": "x", "kind": "constraint"})
    add("Requirement", {"id": "r", "title": " \u001c ", "kind": "constraint"})
    add("Requirement", {"id": "r", "title": "x", "kind": "constraint", "administrative": True})
    add("Requirement", {"id": "r", "title": "x", "kind": "unknown"})
    add("Requirement", {"id": "r", "title": "x", "kind": "constraint", "tags": ["a", "a"]})

    # --- DesignSpec --------------------------------------------------------
    add("DesignSpec", {"id": "spec", "name": "Spec", "summary": "Summary"})
    add(
        "DesignSpec",
        {
            "id": "spec",
            "name": " Spec ",
            "summary": " Summary ",
            "parameter_schema_digest": DIGEST_A.upper(),
            "attributes_json": '{"z": 2, "a": [1, {"b": -0.0}], "big": 1180591620717411303424}',
            "requirement_ids": ["z", "a"],
            "constraints": ["Two", "One"],
        },
    )
    add(
        "DesignSpec",
        {
            "id": "spec",
            "name": "Spec",
            "summary": "Summary",
            "attributes_json": {"b": 2, "a": 1},
        },
    )
    add(
        "DesignSpec",
        {"id": "spec", "name": "Spec", "summary": "Summary", "attributes_json": "[]"},
    )
    add(
        "DesignSpec",
        {"id": "spec", "name": "Spec", "summary": "Summary", "attributes_json": '{"a":1,"a":2}'},
    )
    add(
        "DesignSpec",
        {"id": "spec", "name": "Spec", "summary": "Summary", "constraints": ["x", "x"]},
    )

    # --- DomainPackageManifest --------------------------------------------
    add(
        "DomainPackageManifest",
        {"package_id": "core", "version": "1.0.0", "digest": DIGEST_A},
    )
    add(
        "DomainPackageManifest",
        {
            "package_id": "core",
            "version": "1.0.0",
            "digest": DIGEST_A.upper(),
            "description": "A package.",
            "workflow_definition_ids": ["w2", "w1"],
            "tool_ids": ["t2", "t1"],
            "schema_digests": [DIGEST_C, DIGEST_A],
            "required_capabilities": ["storage.write", "tool.execute"],
            "policy_overlay_digest": DIGEST_B,
            "signer_id": "signer",
            "signature_digest": DIGEST_C,
        },
    )
    add(
        "DomainPackageManifest",
        {"package_id": "core", "version": "1", "digest": DIGEST_A, "signer_id": "s"},
    )

    # --- ArtifactRef -------------------------------------------------------
    add(
        "ArtifactRef",
        {
            "artifact_id": "artifact-1",
            "uri": "HTTPS://Example.COM:443/objects//item%20one",
            "digest": DIGEST_A.upper(),
            "media_type": "APPLICATION/JSON",
            "size_bytes": 0,
        },
    )
    add(
        "ArtifactRef",
        {
            "artifact_id": "artifact-1",
            "uri": "artifact://objects/artifact-1",
            "digest": DIGEST_A,
            "media_type": "application/json",
            "size_bytes": 2**70,
            "classification": "design_binary",
            "preview_uri": "file:///tmp/a//b/",
            "provenance_id": "prov",
        },
    )
    for uri in [
        "relative/path",
        "javascript:alert(1)",
        "https://user:password@example.com/object",
        "https://example.com/object?token=secret",
        "https://example.com/object#fragment",
        "file://remote-host/object",
        "https://example.com/a/%2e%2e/b",
        "https://example.com/a/%00",
        "https://example.com/a\\b",
    ]:
        add(
            "ArtifactRef",
            {
                "artifact_id": "artifact-1",
                "uri": uri,
                "digest": DIGEST_A,
                "media_type": "application/json",
                "size_bytes": 1,
            },
        )
    add(
        "ArtifactRef",
        {
            "artifact_id": "a",
            "uri": "https://bücher.example/a",
            "digest": DIGEST_A,
            "media_type": "text/plain",
            "size_bytes": 1,
        },
    )

    # --- DesignRevision ----------------------------------------------------
    add(
        "DesignRevision",
        {
            "revision_id": "revision-1",
            "design_id": "design-1",
            "sequence": 0,
            "spec": {"id": "spec", "name": "Spec", "summary": "Summary"},
            "created_at": AT,
        },
    )
    add(
        "DesignRevision",
        {
            "revision_id": "revision-1",
            "design_id": "design-1",
            "sequence": 7,
            "spec": {"id": "spec", "name": "Spec", "summary": "Summary"},
            "parent_revision_id": "revision-0",
            "artifacts": [
                {
                    "artifact_id": "artifact-1",
                    "uri": "artifact://objects/a",
                    "digest": DIGEST_A,
                    "media_type": "application/json",
                    "size_bytes": 1,
                }
            ],
            "domain_packages": [
                {"package_id": "core", "version": "1.0.0", "digest": DIGEST_B}
            ],
            "provenance_id": "prov",
            "created_at": "2026-01-02 08:34:05.123456789+05:30",
        },
    )
    add(
        "DesignRevision",
        {
            "revision_id": "revision-1",
            "design_id": "design-1",
            "sequence": 1,
            "spec": {"id": "spec", "name": "Spec", "summary": "Summary"},
            "parent_revision_id": "revision-1",
            "created_at": AT,
        },
    )

    # --- PlanStep ----------------------------------------------------------
    add(
        "PlanStep",
        {"step_id": "step", "catalog_step_id": "catalog", "intent": "Do the thing."},
    )
    add(
        "PlanStep",
        {
            "step_id": "step",
            "catalog_step_id": "catalog",
            "intent": "Do the thing.",
            "parameters_json": '{"b":2,"a":1}',
            "input_artifact_ids": ["b", "a"],
            "required_capabilities": ["tool.execute"],
            "depends_on": ["other"],
            "estimated_duration_seconds": -0.0,
            "max_attempts": 2**70,
        },
    )
    add(
        "PlanStep",
        {
            "step_id": "step",
            "catalog_step_id": "catalog",
            "intent": "x",
            "depends_on": ["step"],
        },
    )

    # --- Workflow ----------------------------------------------------------
    add(
        "WorkflowTransition",
        {
            "transition_id": "t1",
            "from_state": "start",
            "to_state": "done",
            "step_id": "s1",
            "checkpoint_type": "pre_commit",
        },
    )
    add(
        "WorkflowDefinition",
        {
            "definition_id": "workflow",
            "version": "1.0.0",
            "name": "Workflow",
            "initial_state": "start",
            "states": ["done", "start"],
            "transitions": [
                {
                    "transition_id": "t1",
                    "from_state": "start",
                    "to_state": "done",
                    "step_id": "s1",
                }
            ],
            "available_step_ids": ["s1"],
            "max_replanning_attempts": 2,
        },
    )
    add(
        "WorkflowDefinition",
        {
            "definition_id": "workflow",
            "version": "1",
            "name": "Workflow",
            "initial_state": "start",
            "states": ["start", "unreachable"],
            "available_step_ids": ["s1"],
        },
    )

    # --- Tools -------------------------------------------------------------
    add(
        "ToolDescriptor",
        {
            "tool_id": "tool",
            "version": "1.2.0",
            "name": "Bounded evaluator",
            "description": "Evaluates typed input without ambient authority.",
            "input_schema_digest": DIGEST_A,
            "output_schema_digest": DIGEST_B,
            "side_effect_level": "read",
        },
    )
    add(
        "ToolDescriptor",
        {
            "tool_id": "tool",
            "version": "1",
            "name": "Tool",
            "description": "Description",
            "input_schema_digest": DIGEST_A,
            "output_schema_digest": DIGEST_B,
            "side_effect_level": "none",
            "timeout_seconds": -0.0,
            "destructive": False,
            "supports_structured_output": False,
            "idempotent": False,
        },
    )
    add(
        "ToolDescriptor",
        {
            "tool_id": "tool",
            "version": "1",
            "name": "Tool",
            "description": "Description",
            "input_schema_digest": DIGEST_A,
            "output_schema_digest": DIGEST_B,
            "side_effect_level": "none",
            "destructive": True,
        },
    )
    add(
        "ToolCall",
        {
            "call_id": "call",
            "tool_id": "tool",
            "tool_version": "1",
            "side_effect_level": "read",
            "timeout_seconds": 30,
            "issued_at": AT,
        },
    )
    add(
        "ToolCall",
        {
            "call_id": "call",
            "tool_id": "tool",
            "tool_version": "1",
            "arguments_json": '{"value": 2}',
            "side_effect_level": "read",
            "timeout_seconds": 30.0,
            "issued_at": 1767323045,
        },
    )
    add(
        "ToolResult",
        {
            "call_id": "call",
            "tool_id": "tool",
            "status": "succeeded",
            "result_json": '{"value": 4}',
            "started_at": AT,
            "finished_at": "2026-01-02T03:04:05.025000Z",
            "duration_ms": 25.0,
        },
    )
    add(
        "ToolResult",
        {
            "call_id": "call",
            "tool_id": "tool",
            "status": "failed",
            "result_json": '{"partial": true}',
        },
    )
    add(
        "ToolResult",
        {
            "call_id": "call",
            "tool_id": "tool",
            "status": "failed",
            "error_code": "tool.failed",
        },
    )

    # --- Evaluation --------------------------------------------------------
    add("EvaluationMetric", {"name": "margin", "value": 1.25, "unit": "ratio", "passed": True})
    add(
        "EvaluationResult",
        {
            "evaluation_id": "eval",
            "subject_id": "revision",
            "passed": True,
            "score": 0.95,
            "evaluated_at": AT,
        },
    )
    add(
        "EvaluationResult",
        {
            "evaluation_id": "eval",
            "subject_id": "revision",
            "requirement_id": "req",
            "passed": False,
            "score": 0.0,
            "metrics": [{"name": "b", "value": 2}, {"name": "a", "value": 1}],
            "findings": ["Two", "One"],
            "artifact_ids": ["b", "a"],
            "evaluated_at": AT,
        },
    )
    add("EvaluationResult", {"evaluation_id": "e", "subject_id": "s", "passed": True, "score": 1.5, "evaluated_at": AT})
    add("EvaluationResult", {"evaluation_id": "e", "subject_id": "s", "passed": 1, "evaluated_at": AT})

    # --- Approval ----------------------------------------------------------
    add(
        "ApprovalRequest",
        {
            "request_id": "approval",
            "checkpoint_type": "egress",
            "status": "pending",
            "workflow_id": "workflow",
            "action_digest": DIGEST_A,
            "state_digest": DIGEST_B,
            "options": ["deny", "approve"],
            "requested_capabilities": ["network.egress"],
            "artifact_ids": ["artifact"],
            "created_at": AT,
        },
    )
    add(
        "ApprovalRequest",
        {
            "request_id": "approval",
            "checkpoint_type": "egress",
            "status": "approved",
            "workflow_id": "workflow",
            "action_digest": DIGEST_A,
            "state_digest": DIGEST_B,
            "options": ["approve", "deny"],
            "selected_option_id": "approve",
            "modification_digest": DIGEST_C,
            "created_at": AT,
            "decided_at": "2026-01-02T04:04:05Z",
            "actor_id": "actor",
        },
    )
    add(
        "ApprovalRequest",
        {
            "request_id": "approval",
            "checkpoint_type": "plan_approval",
            "status": "pending",
            "workflow_id": "workflow",
            "action_digest": DIGEST_A,
            "state_digest": DIGEST_B,
            "options": ["approve", "reject"],
            "selected_option_id": "approve",
            "created_at": AT,
        },
    )
    add(
        "ApprovalRequest",
        {
            "request_id": "approval",
            "checkpoint_type": "plan_approval",
            "status": "pending",
            "workflow_id": "workflow",
            "action_digest": DIGEST_A,
            "state_digest": DIGEST_B,
            "options": [],
            "created_at": AT,
        },
    )

    # --- Budget ------------------------------------------------------------
    add("RunBudget", {})
    add(
        "RunBudget",
        {
            "max_model_calls": 2**70,
            "max_tool_calls": 0,
            "max_total_tokens": 0,
            "max_artifact_bytes": 0,
            "max_duration_seconds": -0.0,
            "max_cost": "1.00",
            "max_replanning_attempts": 0,
        },
    )
    add("RunBudget", {"max_cost": "0.10"})
    add("RunBudget", {"max_cost": "-0.0"})
    add("RunBudget", {"max_cost": "NaN"})
    add("RunBudget", {"max_duration_seconds": -1.0})
    add("RunBudget", {"max_model_calls": True})
    add("RunBudget", {"max_cost": "Infinity"})

    # --- Provenance --------------------------------------------------------
    add("Provenance", {"provenance_id": "prov", "subject_ids": ["revision"], "recorded_at": AT})
    add(
        "Provenance",
        {
            "provenance_id": "prov",
            "subject_ids": ["b", "a"],
            "source_artifact_ids": ["b", "a"],
            "output_artifact_ids": ["b", "a"],
            "tool_call_ids": ["b", "a"],
            "domain_package_digests": [DIGEST_B, DIGEST_A],
            "parent_provenance_ids": ["b", "a"],
            "transformation_ids": ["b", "a"],
            "model_transcript_digest": DIGEST_C,
            "seed": 7,
            "recorded_at": AT,
        },
    )
    add("Provenance", {"provenance_id": "p", "subject_ids": [], "recorded_at": AT})
    add(
        "Provenance",
        {
            "provenance_id": "p",
            "subject_ids": ["s"],
            "parent_provenance_ids": ["p"],
            "recorded_at": AT,
        },
    )

    # --- Canonical JSON objects -------------------------------------------
    for attributes in [
        '{"x": "\\ud800"}',
        '{"x": "\\u001c"}',
        '{"deep": ' + '{"a":' * 100 + "1" + "}" * 100 + "}",
        '{"big": ' + "9" * 500 + "}",
        '{"float": -0.0, "small": 1e-05, "large": 1e+16}',
    ]:
        add(
            "DesignSpec",
            {
                "id": "spec",
                "name": "Spec",
                "summary": "Summary",
                "attributes_json": attributes,
            },
        )

    # --- Construction ------------------------------------------------------
    def plate(name: str = "plate", width: float = 80, height: float = 40, length: float = 8):
        return {
            "name": name,
            "steps": [
                {"op": "sketch", "profile": "rectangle", "width": width, "height": height},
                {"op": "extrude", "length": length},
            ],
        }

    add(
        "ConstructionScript",
        {
            "summary": "  A plate.  ",
            "components": [plate()],
            "assembly": [{"op": "place", "component": "plate", "z": 5}],
        },
        "load_construction",
    )
    add(
        "ConstructionScript",
        {
            "summary": "A screw jack.",
            "components": [
                {
                    "name": "Screw",
                    "steps": [
                        {"op": "sketch", "profile": {"circle": {"diameter": 20}, "x": 1, "y": 2}},
                        {"op": "extrude", "length": 100},
                        {"op": "helix", "diameter": 20, "length": 100, "pitch": 5},
                    ],
                }
            ],
            "assembly": [],
        },
        "load_construction",
    )
    add(
        "ConstructionScript",
        {
            "summary": "Inference.",
            "components": [
                {
                    "name": "shaft",
                    "steps": [
                        {"op": "sketch", "diameter": 16, "profile": "circular"},
                        {"op": "extrude", "length": 40},
                        {"op": "sketch", "profile": "flange", "width": 40, "height": 10},
                        {"op": "extrude", "length": 8},
                    ],
                }
            ],
        },
        "load_construction",
    )
    add(
        "ConstructionScript",
        {"summary": "Revolve.", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 2, "height": 2}, {"op": "revolve"}]}]},
        "load_construction",
    )
    for raw in [
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "bogus"}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 2}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "extrude", "length": 2}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch"}, {"op": "extrude", "length": 2}, {"op": "sketch"}]}]},
        {"summary": "hi", "components": [plate("B"), plate("B")]},
        {"summary": "hi", "components": [plate("B")], "assembly": [{"op": "place", "component": "X"}]},
        {"summary": "hi", "components": [plate("B")], "extra": 1},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": True, "height": 2}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 0.0009, "height": 2}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 2, "height": 2, "plane": "xy"}]}]},
    ]:
        add("ConstructionScript", raw, "load_construction")
    for raw in [[], 1, "x", None]:
        add("ConstructionScript", raw, "load_construction")

    add(
        "AnalysisScript",
        {
            "steps": [
                {"op": "material", "name": "steel"},
                {"op": "fix", "face": 1},
                {"op": "force", "face": 2, "force_n": 10},
                {"op": "mesh"},
                {"op": "solve"},
            ]
        },
        "load_analysis",
    )
    for raw in [
        {"steps": [{"op": "solve"}, {"op": "material"}]},
        {"steps": [{"op": "material"}]},
        {"steps": [{"op": "force"}]},
        {"steps": [{"op": "fix", "face": 0}]},
        {"steps": [{"op": "fix", "face": 65}]},
        {"steps": [{"op": "fix", "face": True}]},
        {"steps": [{"op": "force", "force_n": 0}]},
        {"steps": [{"op": "force", "force_n": 1_000_000}]},
        {"steps": [{"op": "force", "force_n": 1_000_000.1}]},
        {"steps": [{"op": "pressure", "pressure_mpa": 0.1}]},
        {"steps": [{"op": "solve"}, {"op": "solve"}]},
        {"steps": []},
        {"steps": [{"op": "solve"}] * 9},
    ]:
        add("AnalysisScript", raw, "load_analysis")

    add("AnalysisReading", {"verdict": "  Looks stable.  "}, "load_reading")
    add(
        "AnalysisReading",
        {
            "verdict": "Needs a smaller fillet.",
            "tool_edit": [{"op": "fillet", "component": "plate", "radius": 2}],
            "rewrite": plate("plate"),
        },
        "load_reading",
    )
    for raw in [
        {"verdict": "x" * 501},
        {"verdict": "ok", "tool_edit": [{"op": "move", "component": "B"}] * 5},
        {"verdict": "ok", "tool_edit": [{"op": "fillet", "component": "B"}]},
        {"verdict": "ok", "tool_edit": [{"op": "resize", "component": "B"}]},
        {"verdict": "ok", "rewrite": {"name": "B", "steps": []}},
    ]:
        add("AnalysisReading", raw, "load_reading")


    # --- Expanded invalid/edge coverage -----------------------------------
    for kind in ["functional", "non_functional", "constraint", "safety", "quality", "performance"]:
        add("Requirement", {"id": "r", "title": "t", "kind": kind})
    for priority in ["low", "medium", "high", "critical"]:
        add("Requirement", {"id": "r", "title": "t", "kind": "functional", "priority": priority})
    for status in ["draft", "proposed", "approved", "rejected", "implemented", "verified"]:
        add("Requirement", {"id": "r", "title": "t", "kind": "functional", "status": status})
    for effect in ["none", "read", "local_write", "network", "egress"]:
        add(
            "ToolDescriptor",
            {
                "tool_id": "t",
                "version": "1",
                "name": "n",
                "description": "d",
                "input_schema_digest": DIGEST_A,
                "output_schema_digest": DIGEST_B,
                "side_effect_level": effect,
            },
        )
    for status in ["requested", "running", "succeeded", "failed", "denied", "timed_out", "cancelled"]:
        result = {"call_id": "c", "tool_id": "t", "status": status}
        if status in {"failed", "denied", "timed_out"}:
            result["error_code"] = "tool.failed"
        add("ToolResult", result)
    for classification in ["metadata", "text", "derived_data", "design_binary"]:
        add(
            "ArtifactRef",
            {
                "artifact_id": "a",
                "uri": "artifact://objects/a",
                "digest": DIGEST_A,
                "media_type": "application/json",
                "size_bytes": 1,
                "classification": classification,
            },
        )

    # Missing, extra, and type errors.
    add("Requirement", {"title": "t", "kind": "functional"})
    add("Requirement", {"id": "r", "title": "t", "kind": "functional", "extra": 1})
    add("Requirement", {"id": 1, "title": "t", "kind": "functional"})
    add("Requirement", {"id": "r", "title": "t", "kind": "functional", "description": ""})
    add("Requirement", {"id": "r", "title": "\u001c", "kind": "functional"})
    add("Requirement", {"id": "r", "title": "x" * 16385, "kind": "functional"})
    add(
        "DesignSpec",
        {
            "id": "s",
            "name": "n",
            "summary": "z",
            "attributes_json": '{"x":"\\ud800"}',
        },
    )
    add(
        "DesignSpec",
        {
            "id": "s",
            "name": "n",
            "summary": "z",
            "attributes_json": '{"x":' + "9" * 4301 + "}",
        },
    )
    add(
        "DesignSpec",
        {
            "id": "s",
            "name": "n",
            "summary": "z",
            "attributes_json": '{"x":"' + "y" * 1_100_000 + '"}',
        },
    )
    add("DesignSpec", {"id": "s", "name": "n", "summary": "z", "attributes_json": '{"x":1e999}'})
    add("DesignSpec", {"id": "s", "name": "n", "summary": "z", "attributes_json": "{\"x\": NaN}"})
    add("DesignSpec", {"id": "s", "name": "n", "summary": "z", "attributes_json": '{"x":"\\ud800","y":2}'})
    add(
        "DesignPackageManifest" if False else "DomainPackageManifest",
        {
            "package_id": "p",
            "version": "1",
            "digest": DIGEST_A,
            "workflow_definition_ids": ["a", "a"],
        },
    )
    add(
        "DomainPackageManifest",
        {"package_id": "p", "version": "1", "digest": DIGEST_A, "schema_digests": [DIGEST_A, DIGEST_A]},
    )
    add(
        "DomainPackageManifest",
        {"package_id": "p", "version": "1", "digest": DIGEST_A, "signature_digest": DIGEST_B},
    )
    add(
        "ArtifactRef",
        {
            "artifact_id": "a",
            "uri": "artifact://objects/a",
            "digest": "g" * 64,
            "media_type": "application/json",
            "size_bytes": 1,
        },
    )
    add(
        "ArtifactRef",
        {
            "artifact_id": "a",
            "uri": "artifact://objects/a",
            "digest": DIGEST_A,
            "media_type": "application/json; charset=utf-8",
            "size_bytes": 1,
        },
    )
    add(
        "ArtifactRef",
        {
            "artifact_id": "a",
            "uri": "artifact://objects/a",
            "digest": DIGEST_A,
            "media_type": "application/json",
            "size_bytes": -1,
        },
    )
    add(
        "ArtifactRef",
        {
            "artifact_id": "a",
            "uri": "artifact://objects/a",
            "digest": DIGEST_A,
            "media_type": "application/json",
            "size_bytes": 1.0,
        },
    )
    add(
        "ArtifactRef",
        {
            "artifact_id": "a",
            "uri": "artifact://objects/a",
            "digest": DIGEST_A,
            "media_type": "application/json",
            "size_bytes": True,
        },
    )

    # Workflow invariants.
    add(
        "WorkflowDefinition",
        {
            "definition_id": "w",
            "version": "1",
            "name": "n",
            "initial_state": "start",
            "states": ["start", "start"],
            "available_step_ids": ["s"],
        },
    )
    add(
        "WorkflowDefinition",
        {
            "definition_id": "w",
            "version": "1",
            "name": "n",
            "initial_state": "missing",
            "states": ["start"],
            "available_step_ids": ["s"],
        },
    )
    add(
        "WorkflowDefinition",
        {
            "definition_id": "w",
            "version": "1",
            "name": "n",
            "initial_state": "start",
            "states": ["start", "done"],
            "transitions": [
                {"transition_id": "t1", "from_state": "start", "to_state": "done", "step_id": "s"},
                {"transition_id": "t1", "from_state": "start", "to_state": "done", "step_id": "s"},
            ],
            "available_step_ids": ["s"],
        },
    )
    add(
        "WorkflowDefinition",
        {
            "definition_id": "w",
            "version": "1",
            "name": "n",
            "initial_state": "start",
            "states": ["start", "done"],
            "transitions": [
                {"transition_id": "t1", "from_state": "start", "to_state": "done", "step_id": "missing"},
            ],
            "available_step_ids": ["s"],
        },
    )

    # Revision duplicate artifact/package ids and timestamp ordering.
    artifact = {
        "artifact_id": "a",
        "uri": "artifact://objects/a",
        "digest": DIGEST_A,
        "media_type": "application/json",
        "size_bytes": 1,
    }
    add(
        "DesignRevision",
        {
            "revision_id": "r",
            "design_id": "d",
            "sequence": 1,
            "spec": {"id": "s", "name": "n", "summary": "z"},
            "artifacts": [artifact, artifact],
            "created_at": AT,
        },
    )
    add(
        "ToolResult",
        {
            "call_id": "c",
            "tool_id": "t",
            "status": "succeeded",
            "started_at": "2026-01-02T03:04:05Z",
            "finished_at": "2026-01-02T08:34:04+05:30",
        },
    )
    add(
        "ToolResult",
        {
            "call_id": "c",
            "tool_id": "t",
            "status": "succeeded",
            "started_at": "2026-01-02T03:04:05Z",
            "finished_at": "2026-01-02T03:04:04Z",
        },
    )
    add(
        "ToolResult",
        {"call_id": "c", "tool_id": "t", "status": "succeeded", "finished_at": AT},
    )

    # Approval decision invariants.
    add(
        "ApprovalRequest",
        {
            "request_id": "a",
            "checkpoint_type": "egress",
            "status": "rejected",
            "workflow_id": "w",
            "action_digest": DIGEST_A,
            "state_digest": DIGEST_B,
            "options": ["approve", "reject"],
            "selected_option_id": "reject",
            "modification_digest": DIGEST_C,
            "created_at": AT,
        },
    )
    add(
        "ApprovalRequest",
        {
            "request_id": "a",
            "checkpoint_type": "egress",
            "status": "approved",
            "workflow_id": "w",
            "action_digest": DIGEST_A,
            "state_digest": DIGEST_B,
            "options": ["approve", "reject"],
            "created_at": AT,
        },
    )
    add(
        "ApprovalRequest",
        {
            "request_id": "a",
            "checkpoint_type": "egress",
            "status": "approved",
            "workflow_id": "w",
            "action_digest": DIGEST_A,
            "state_digest": DIGEST_B,
            "options": ["approve", "reject"],
            "selected_option_id": "approve",
            "created_at": "2026-01-02T03:04:05Z",
            "decided_at": "2026-01-02T03:04:04Z",
        },
    )
    add(
        "ApprovalRequest",
        {
            "request_id": "a",
            "checkpoint_type": "egress",
            "status": "pending",
            "workflow_id": "w",
            "action_digest": DIGEST_A,
            "state_digest": DIGEST_B,
            "options": ["approve"],
            "created_at": AT,
            "actor_id": "actor",
        },
    )

    # Evaluation duplicates and metric strictness.
    add(
        "EvaluationResult",
        {
            "evaluation_id": "e",
            "subject_id": "s",
            "passed": True,
            "metrics": [{"name": "x", "value": 1}, {"name": "x", "value": 2}],
            "evaluated_at": AT,
        },
    )
    add(
        "EvaluationResult",
        {
            "evaluation_id": "e",
            "subject_id": "s",
            "passed": True,
            "findings": ["x", "x"],
            "evaluated_at": AT,
        },
    )
    add(
        "EvaluationResult",
        {
            "evaluation_id": "e",
            "subject_id": "s",
            "passed": True,
            "score": -0.0,
            "evaluated_at": AT,
        },
    )
    add("EvaluationMetric", {"name": "m", "value": -0.0})
    add("EvaluationMetric", {"name": "m", "value": "-1.5"})
    add("EvaluationMetric", {"name": "m", "value": "-0.0"})
    add("EvaluationMetric", {"name": "m", "value": " 1_0.5 "})
    add("EvaluationMetric", {"name": "m", "value": "inf"})
    add("EvaluationMetric", {"name": "m", "value": True})
    add("EvaluationMetric", {"name": "m", "value": 1, "passed": 1})
    add("EvaluationMetric", {"name": "m", "value": 1, "passed": None})

    # Provenance duplicates.
    add(
        "Provenance",
        {
            "provenance_id": "p",
            "subject_ids": ["a", "a"],
            "recorded_at": AT,
        },
    )
    add(
        "Provenance",
        {
            "provenance_id": "p",
            "subject_ids": ["a"],
            "domain_package_digests": [DIGEST_A, DIGEST_A],
            "recorded_at": AT,
        },
    )

    # Timestamp parsing surface.
    for timestamp in [
        "2026-01-02T03:04:05z",
        "2026-01-02t03:04:05.0000001Z",
        "2026-01-02_03:04:05Z",
        "2026-01-02T03:04:05\u221205:30",
        "2026-01-02T03:04:05\u221200:00",
        "1.",
        ".5",
        "+.5",
        "2026-01-02 08:34:05.123456789+05:30",
        "2026-01-02T03:04Z",
        "2026-01-02T03:04:05-05:30",
        "2026-01-02T03:04:05+23:59",
        "2026-01-02T03:04:05-00:00",
        "001767323045",
        "+2026",
        "-1.5",
        0,
        -1,
        1_767_323_045,
        1_767_323_045.5,
        "0001-01-01T00:00:00Z",
        "9999-12-31T23:59:59Z",
    ]:
        add("EvaluationResult", {"evaluation_id": "e", "subject_id": "s", "passed": True, "evaluated_at": timestamp})
    for timestamp in [
        "2026-01-02T03:04:05",
        "2026-02-29T03:04:05Z",
        "2026-01-02T24:00:00Z",
        "2026-01-02T03:04:05+24:00",
        "2026-01-02T03:04:05+05",
        "2026-01-02T03:04:05.5",
        "10000-01-01T00:00:00Z",
        "20260102T030405Z",
        " 2026-01-02T03:04:05Z",
    ]:
        add("EvaluationResult", {"evaluation_id": "e", "subject_id": "s", "passed": True, "evaluated_at": timestamp})

    # URI normalization surface.
    for uri in [
        "https://example.com:8443/a",
        "https://example.com:80/a",
        "https://example.com:/a",
        "https://example.com:0443/a",
        "file:///tmp/a//b/",
        "file://localhost:123/a",
        "file:////server/share/a",
        "artifact://Objects/foo",
        "https://[::1]:8443/a",
        "https://[::ffff:c000:201]:443/a",
        "https://[::192.0.2.1]/a",
        "https://[2001:db8::1:0:0:0]/a",
        "https://[fe80::1%25eth0]/a",
        "https://bücher.example/a",
        "https://faß.de/a",
        "https://straße.de/a",
        "https://ÖBB.at/a",
        "https://①.com/a",
        "https://xn--bcher-kva.example/a",
        "https://example.com./a",
        "https://example.com/a%2Fb",
        "https://example.com/%C3%A9",
        "https://example.com/a;b",
        "https://example.com/a%3Fb",
        "https://example.com/a//b",
        "https://example.com/a/",
        "https://example.com/",
        "https://example.com/a|b",
    ]:
        add(
            "ArtifactRef",
            {
                "artifact_id": "a",
                "uri": uri,
                "digest": DIGEST_A,
                "media_type": "application/json",
                "size_bytes": 0,
            },
        )
    for uri in [
        "https://example.com:65536/a",
        "https://example.com:notaport/a",
        "https://[::1]x/a",
        "https://[fe80::1%eth0/a",
        "https://exa[mple.com/a",
        "https://example.com/%ff",
        "https://example.com/a/./b",
        "https://example.com/..",
        "https://example.com/a/%2E%2E%2Fb",
        "artifact:/a",
        "artifact:a",
        "https:///a",
    ]:
        add(
            "ArtifactRef",
            {
                "artifact_id": "a",
                "uri": uri,
                "digest": DIGEST_A,
                "media_type": "application/json",
                "size_bytes": 0,
            },
        )

    # Decimal scales and arbitrary integers.
    for cost in ["1.00", "0.10", "1E+2", "-0.0", "0", "0001.2300", "1e-7"]:
        add("RunBudget", {"max_cost": cost})
    add(
        "RunBudget",
        {
            "max_model_calls": 2**70 + 1,
            "max_tool_calls": 0,
            "max_total_tokens": 0,
            "max_artifact_bytes": 2**70,
            "max_duration_seconds": 1e308,
            "max_cost": "1.25",
            "max_replanning_attempts": 2**64,
        },
    )

    # Construction: every operation and flattening branch.
    add(
        "ComponentStep",
        {"op": "sketch", "profile": {"box": {"width": 2, "height": 3}}},
    )
    add(
        "ComponentStep",
        {"op": "sketch", "profile": {"type": "circle", "diameter": 3}},
    )
    add(
        "ComponentStep",
        {"op": "sketch", "profile": {"kind": "CIRCLE", "diameter": 3}},
    )
    add(
        "ComponentStep",
        {"op": "sketch", "profile": {"shape": "round", "diameter": 3}},
    )
    add(
        "ComponentStep",
        {"op": "sketch", "profile": {"circle": {"diameter": 3}}, "width": None},
    )
    add(
        "ComponentStep",
        {"op": "sketch", "profile": {"circle": {"diameter": 3}}, "x": None},
    )
    add(
        "ComponentStep",
        {"op": "sketch", "profile": {"type": "triangle", "diameter": 3}},
    )
    add(
        "ComponentStep",
        {"op": "sketch", "profile": {"circle": {}}, "diameter": 3},
    )
    add("ComponentStep", {"op": "sketch", "profile": {"rectangle": {"width": 3}}, "height": 4})
    add("ComponentStep", {"op": "sketch", "profile": {"rect": {"width": 3, "height": 4}}, "angle": 10})
    add("ComponentStep", {"op": "sketch", "width": 2, "height": 3, "plane": "XZ"})
    add("ComponentStep", {"op": "extrude", "length": 2, "plane": "YZ", "reversed": True})
    add("ComponentStep", {"op": "extrude", "length": 2, "reversed": 1})
    add("ComponentStep", {"op": "revolve", "length": 2})
    add("ComponentStep", {"op": "helix", "diameter": 2, "pitch": 1, "length": 4})
    add("ComponentStep", {"op": "helix", "diameter": 2, "length": 4})
    add("ComponentStep", {"op": "pocket", "length": 2})
    add("ComponentStep", {"op": "sketch", "width": 0.001, "height": 5000})
    add("ComponentStep", {"op": "sketch", "width": 5000.1, "height": 2})
    add("ComponentStep", {"op": "sketch", "width": True, "height": 2})
    add("ComponentStep", {"op": "sketch", "width": "2", "height": 2})
    add("ComponentStep", {"op": "sketch", "width": 2, "height": 2, "x": -5000})
    add("ComponentStep", {"op": "sketch", "width": 2, "height": 2, "x": 5000.1})
    add("ComponentStep", {"op": "sketch", "width": 2, "height": 2, "profile": "unknown"})
    add("ComponentStep", {"op": "extrude", "length": 2, "profile": "unknown"})
    add("ComponentStep", {"op": "sketch", "width": 2, "height": 2, "profile": None})
    add("ComponentStep", {"op": "pocket"})

    add("AssemblyStep", {"op": "place", "component": "plate"})
    add("AssemblyStep", {"op": "fuse", "components": ["a", "b"]})
    add("AssemblyStep", {"op": "compound", "components": ["a", "b"], "x": -1})
    add("AssemblyStep", {"op": "fuse", "components": ["a"]})
    add("AssemblyStep", {"op": "place"})
    add("AssemblyStep", {"op": "weld"})
    add("AssemblyStep", {"op": "place", "component": "9bad"})

    add(
        "ConstructionScript",
        {
            "summary": "Compound.",
            "components": [plate("a"), plate("b")],
            "assembly": [
                {"op": "place", "component": "a"},
                {"op": "fuse", "components": ["a", "b"]},
                {"op": "compound", "components": ["a", "b"]},
            ],
        },
        "load_construction",
    )
    add(
        "ConstructionScript",
        {
            "summary": "Pocket and helix.",
            "components": [
                {
                    "name": "part",
                    "steps": [
                        {"op": "sketch", "width": 10, "height": 10},
                        {"op": "extrude", "length": 10},
                        {"op": "sketch", "diameter": 2},
                        {"op": "pocket", "length": 2},
                        {"op": "helix", "diameter": 2, "pitch": 1, "length": 3},
                    ],
                }
            ],
        },
        "load_construction",
    )
    for raw in [
        {"summary": "x" * 100, "components": [plate()]},
        {"summary": "hi", "components": []},
        {"summary": "hi", "components": [plate()], "assembly": [{"op": "place", "component": "plate"}] * 17},
        {"summary": "hi", "components": [{"name": "bad name", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "revolve", "angle": 361}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1, "reversed": True}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "diameter": 1}, {"op": "extrude", "length": 1}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch"}, {"op": "helix", "diameter": 1, "pitch": 1, "length": 1}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}]}], "assembly": [{"op": "fuse", "components": ["B", "missing"]}]},
    ]:
        add("ConstructionScript", raw, "load_construction")

    add(
        "AnalysisScript",
        {"steps": [{"op": "material", "name": "steel"}, {"op": "fix", "face": 64}, {"op": "pressure", "pressure_mpa": 5000}, {"op": "mesh"}, {"op": "solve"}, {"op": "solve"}]},
        "load_analysis",
    )
    for raw in [
        {"steps": [{"op": "material", "name": ""}, {"op": "solve"}]},
        {"steps": [{"op": "fix", "face": 1}, {"op": "force", "force_n": 1_000_000}, {"op": "pressure", "pressure_mpa": 0.1}, {"op": "solve"}]},
        {"steps": [{"op": "fix", "face": 1}, {"op": "material"}, {"op": "solve"}]},
        {"steps": [{"op": "solve"}, {"op": "mesh"}]},
        {"steps": [{"op": "force", "force_n": "2"}]},
        {"steps": [{"op": "force", "force_n": True}]},
        {"steps": [{"op": "material", "name": 7}, {"op": "solve"}]},
    ]:
        add("AnalysisScript", raw, "load_analysis")

    add("EditStep", {"op": "move", "component": "B"})
    add("EditStep", {"op": "resize", "component": "B", "length": 2})
    add("EditStep", {"op": "resize", "component": "B", "diameter": 2})
    add("EditStep", {"op": "fillet", "component": "B", "radius": 1})
    add("EditStep", {"op": "fillet", "component": "B"})
    add("EditStep", {"op": "move", "component": "B", "x": -5000})
    add("EditStep", {"op": "move", "component": "bad name"})
    add("EditStep", {"op": "twist", "component": "B"})
    add(
        "AnalysisReading",
        {"verdict": "x" * 500, "tool_edit": [{"op": "move", "component": "B"}] * 4},
        "load_reading",
    )
    add("AnalysisReading", {"verdict": ""}, "load_reading")
    add("AnalysisReading", {"verdict": 7}, "load_reading")
    add("AnalysisReading", {"verdict": "ok", "rewrite": None}, "load_reading")


    # Remaining model invalid branches and numeric timestamp rounding.
    add("WorkflowTransition", {"transition_id": "t", "from_state": "a", "to_state": "b"})
    add("WorkflowTransition", {"transition_id": "t", "from_state": "a", "to_state": "b", "step_id": "s", "checkpoint_type": "bad"})
    add("PlanStep", {"step_id": "s", "catalog_step_id": "c"})
    add("PlanStep", {"step_id": "s", "catalog_step_id": "c", "intent": "i", "parameters_json": "[]"})
    add("PlanStep", {"step_id": "s", "catalog_step_id": "c", "intent": "i", "estimated_duration_seconds": True})
    add("PlanStep", {"step_id": "s", "catalog_step_id": "c", "intent": "i", "depends_on": ["s"]})
    add("ToolCall", {"call_id": "c", "tool_id": "t", "tool_version": "1"})
    add("ToolCall", {"call_id": "c", "tool_id": "t", "tool_version": "1", "side_effect_level": "read", "timeout_seconds": -1, "issued_at": AT})
    add("ToolCall", {"call_id": "c", "tool_id": "t", "tool_version": "1", "side_effect_level": "read", "timeout_seconds": True, "issued_at": AT})
    add("DesignRevision", {"revision_id": "r", "design_id": "d", "sequence": 1, "created_at": AT})
    add("DesignRevision", {"revision_id": "r", "design_id": "d", "sequence": 1, "spec": {"id": "s", "name": "n", "summary": "z"}, "created_at": "not-a-time"})
    add("DomainPackageManifest", {"package_id": "p", "version": "1", "digest": DIGEST_A, "extra": 1})
    add("DomainPackageManifest", {"package_id": "p", "version": "1", "digest": "nope"})
    add("Provenance", {"provenance_id": "p", "subject_ids": ["a"], "seed": -1, "recorded_at": AT})
    add("Provenance", {"provenance_id": "p", "subject_ids": ["a"], "seed": 1.0, "recorded_at": AT})
    add("ApprovalRequest", {"request_id": "a", "checkpoint_type": "egress", "status": "pending", "workflow_id": "w", "action_digest": DIGEST_A, "state_digest": DIGEST_B, "options": ["a", "a"], "created_at": AT})
    add("RunBudget", {"bogus": 1})
    add("RunBudget", {"max_cost": True})
    add("RunBudget", {"max_cost": []})
    add("RunBudget", {"max_total_tokens": 2**70})
    add("EvaluationResult", {"evaluation_id": "e", "subject_id": "s", "passed": True, "metrics": [{"name": "m", "value": 1, "extra": 1}], "evaluated_at": AT})

    for timestamp in [
        0.0000005, 0.0000015, 0.0000025, 1.9999995, -0.0000005, -0.0000015,
        -1.0000005, -1.9999995, 1_767_323_045_000, 1_767_323_045_000.0,
        20_000_000_000, 20_000_000_001, 20_000_000_000.0, 20_000_000_001.0,
        1_654_619_320_123.123, -62_135_596_800, 253_402_300_799,
        253_402_300_799_000, 253_402_300_799_000.0,
    ]:
        add("EvaluationResult", {"evaluation_id": "e", "subject_id": "s", "passed": True, "evaluated_at": timestamp})

    # More construction branches.
    for raw in [
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}, {"op": "sketch"}, {"op": "extrude", "length": 1}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}, {"op": "sketch"}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "sketch"}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "helix", "diameter": 1, "pitch": 1, "length": 1}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "revolve", "angle": 0.001}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}, {"op": "revolve"}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}, {"op": "helix", "diameter": 1, "length": 1}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}, {"op": "helix", "diameter": 1, "pitch": 1}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}, {"op": "revolve", "reversed": True}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}, {"op": "pocket"}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}, {"op": "pocket", "length": 1}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}]}, {"name": "C", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}]}], "assembly": [{"op": "compound", "components": ["B"]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}]}], "assembly": [{"op": "place", "component": "B", "x": 5000.1}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}]}, {"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}]}]},
        {"summary": "hi", "components": [{"name": "a" * 41, "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}]}], "assembly": [{"op": "fuse", "components": ["B"]}]},
    ]:
        add("ConstructionScript", raw, "load_construction")

    for raw in [
        {"steps": [{"op": "material"}, {"op": "fix"}, {"op": "mesh"}, {"op": "solve"}]},
        {"steps": [{"op": "fix"}, {"op": "material"}, {"op": "solve"}]},
        {"steps": [{"op": "material"}, {"op": "solve"}, {"op": "mesh"}]},
        {"steps": [{"op": "material"}, {"op": "mesh"}, {"op": "force", "force_n": 1}, {"op": "solve"}]},
        {"steps": [{"op": "material"}, {"op": "bogus"}]},
        {"steps": [{"op": "force", "force_n": -1}]},
        {"steps": [{"op": "force", "force_n": 0.001}]},
        {"steps": [{"op": "pressure"}]},
        {"steps": [{"op": "fix", "face": 1.0}]},
        {"steps": [{"op": "fix", "face": "1"}]},
    ]:
        add("AnalysisScript", raw, "load_analysis")

    add("ComponentScript", {"name": "B", "steps": []})
    add("ComponentScript", {"name": "B"})
    add("ComponentScript", {"name": "B", "steps": [{"op": "extrude", "length": 1}]})
    add("ComponentScript", {"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}]})
    add("ComponentScript", {"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}, {"op": "extrude", "length": 1}] * 7})


    # Required arrays with no default.
    add("WorkflowDefinition", {"definition_id": "w", "version": "1", "name": "n", "initial_state": "a", "available_step_ids": []})
    add("WorkflowDefinition", {"definition_id": "w", "version": "1", "name": "n", "initial_state": "a", "states": []})
    add("ApprovalRequest", {"request_id": "a", "checkpoint_type": "egress", "status": "pending", "workflow_id": "w", "action_digest": DIGEST_A, "state_digest": DIGEST_B, "created_at": AT})
    add("Provenance", {"provenance_id": "p", "recorded_at": AT})


    for raw in [
        {"summary": "hi", "components": 1},
        {"summary": "hi", "components": None},
        {"summary": "hi", "components": [{"name": "B", "steps": 1}]},
        {"summary": "hi", "components": [{"name": 1, "steps": []}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [1]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": 1}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1, "x": None}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1, "reversed": None}]}]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}]}], "assembly": 1},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}]}], "assembly": [1]},
        {"summary": "hi", "components": [{"name": "B", "steps": [{"op": "sketch", "width": 1, "height": 1}]}], "assembly": [{"op": "place", "component": None}]},
    ]:
        add("ConstructionScript", raw, "load_construction")
    for raw in [{"steps": 1}, {"steps": None}, {"steps": [1]}, {"steps": [{"op": 1}]}]:
        add("AnalysisScript", raw, "load_analysis")
    for raw in [{"verdict": "ok", "tool_edit": 1}, {"verdict": "ok", "tool_edit": None}, {"verdict": "ok", "rewrite": 1}, {"verdict": "ok", "rewrite": []}]:
        add("AnalysisReading", raw, "load_reading")


    add("Requirement", {"id": "r", "title": "a\ud800b", "kind": "functional"})
    add("Requirement", {"id": "r", "title": "a\ud800b", "kind": "functional", "description": "c\udfff"})
    add(
        "ConstructionScript",
        {
            "summary": "a\ud800b",
            "components": [
                {
                    "name": "B",
                    "steps": [
                        {"op": "sketch", "width": 1, "height": 1},
                        {"op": "extrude", "length": 1},
                    ],
                }
            ],
        },
        "load_construction",
    )
    add("AnalysisReading", {"verdict": "a\ud800b"}, "load_reading")


    add("AnalysisScript", {"steps": [{"op": "material", "name": "a\ud800b"}, {"op": "solve"}]}, "load_analysis")
    add("ComponentStep", {"op": "sketch", "width": 1, "height": 1, "plane": "a\ud800b"})
    add("ComponentStep", {"op": "sketch", "width": 1, "height": 1, "profile": "a\ud800b"})
    add("ComponentStep", {"op": "extrude", "length": 1, "profile": "a\ud800b"})


    add("ArtifactRef", {"artifact_id": "a", "uri": "file://:80/a", "digest": DIGEST_A, "media_type": "application/json", "size_bytes": 0})
    add("ArtifactRef", {"artifact_id": "a", "uri": "https://:80/a", "digest": DIGEST_A, "media_type": "application/json", "size_bytes": 0})
    add("ArtifactRef", {"artifact_id": "a", "uri": "file:///", "digest": DIGEST_A, "media_type": "application/json", "size_bytes": 0})
    add("ArtifactRef", {"artifact_id": "a", "uri": "file://localhost", "digest": DIGEST_A, "media_type": "application/json", "size_bytes": 0})


    # Pydantic last-wins duplicate keys and construction insertion order.
    add_text("Requirement", '{"id":"req-a","id":"req-b","title":"probe","kind":"functional"}')
    add_text("Requirement", '{"id":"req","title":"first","title":"second","kind":"functional"}')
    add_text(
        "ConstructionScript",
        '{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":10,"height":4},"circle":{"diameter":8}}},{"op":"extrude","length":2}]}]}',
        "load_construction",
    )
    add_text(
        "ConstructionScript",
        '{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","profile":{"circle":{"diameter":8},"rectangle":{"width":10,"height":4}}},{"op":"extrude","length":2}]}]}',
        "load_construction",
    )
    add_text(
        "ConstructionScript",
        '{"summary":"hi","summary":"last","components":[{"name":"B","steps":[{"op":"sketch","width":1,"height":2,"width":3},{"op":"extrude","length":4}]}]}',
        "load_construction",
    )
    add_text(
        "ComponentStep",
        '{"op":"sketch","profile":{"rectangle":{"width":10,"height":4},"circle":{"diameter":8}}}',
    )
    add_text(
        "AnalysisReading",
        '{"verdict":"ok","rewrite":{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":10,"height":4},"circle":{"diameter":8}}},{"op":"extrude","length":2}]}}',
        "load_reading",
    )


    add_text(
        "ConstructionScript",
        '{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":1,"height":1},"circle":{"diameter":2}},"profile":{"circle":{"diameter":3}}},{"op":"extrude","length":2}]}]}',
        "load_construction",
    )
    add_text(
        "ComponentScript",
        '{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":1,"height":1},"circle":{"diameter":2}}},{"op":"extrude","length":2}]}',
    )
    add_text(
        "AnalysisScript",
        '{"steps":[{"op":"material"},{"op":"material"},{"op":"solve"}]}',
        "load_analysis",
    )
    add_text(
        "RunBudget",
        '{"max_cost":"1.00","max_cost":"2.00","max_model_calls":1,"max_model_calls":2}',
    )

    add_text(
        "ConstructionScript",
        '{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":1,"height":1},"rectangle":{"width":5,"height":6}}},{"op":"extrude","length":2}]}]}',
        "load_construction",
    )

    # --- R2 regressions: full duplicate uniqueness ------------------------
    # Python's _reject_duplicate_strings uses set equality, so separated and
    # whitespace-normalized duplicates are refused while original order is
    # retained on accepted values.
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","constraints":["a","b","a"]}',
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","constraints":["a","b","a"]}',
        entry="bytes",
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","constraints":[" a ","b","a"]}',
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","constraints":["a","a","b"]}',
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","constraints":["b","a","c"]}',
        entry="bytes",
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","requirement_ids":["a","b","a"]}',
    )
    add_text(
        "EvaluationResult",
        r'{"evaluation_id":"e","subject_id":"s","passed":true,"findings":["x","y","x"],"evaluated_at":"2026-01-02T03:04:05Z"}',
    )
    add_text(
        "EvaluationResult",
        r'{"evaluation_id":"e","subject_id":"s","passed":true,"findings":[" x ","y","x"],"evaluated_at":"2026-01-02T03:04:05Z"}',
        entry="bytes",
    )
    add_text(
        "EvaluationResult",
        r'{"evaluation_id":"e","subject_id":"s","passed":true,"findings":["b","a","c"],"evaluated_at":"2026-01-02T03:04:05Z"}',
    )
    add_text(
        "EvaluationResult",
        r'{"evaluation_id":"e","subject_id":"s","passed":true,"artifact_ids":["a","b","a"],"evaluated_at":"2026-01-02T03:04:05Z"}',
    )

    # --- R2 regressions: ordinary JSON keys must remain data ---------------
    # serde's arbitrary-precision Number used to reinterpret this ordinary
    # object. These are valid user dictionaries, at several depths, with
    # siblings, escaped key spelling and a surrogate value.
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","attributes_json":{"$serde_json::private::Number":"42"}}',
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","attributes_json":{"$serde_json::private::Number":42}}',
        entry="bytes",
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","attributes_json":{"\u0024serde_json::private::Number":"42","sibling":1}}',
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","attributes_json":{"outer":{"$serde_json::private::Number":"7"},"items":[{"$serde_json::private::Number":8,"x":true}]}}',
        entry="bytes",
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","attributes_json":{"$serde_json::private::Number":"\ud800"}}',
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","attributes_json":{"$serde_json::private::Number":"first","$serde_json::private::Number":"last"}}',
    )
    add_text(
        "PlanStep",
        r'{"step_id":"s","catalog_step_id":"c","intent":"i","parameters_json":{"nested":{"$serde_json::private::Number":"42"},"sibling":[1,2]}}',
        entry="bytes",
    )
    add_text(
        "ToolCall",
        r'{"call_id":"c","tool_id":"t","tool_version":"1","side_effect_level":"read","timeout_seconds":1,"issued_at":"2026-01-02T03:04:05Z","arguments_json":{"$serde_json::private::Number":42}}',
    )
    add_text(
        "ToolResult",
        r'{"call_id":"c","tool_id":"t","status":"succeeded","result_json":{"nested":{"$serde_json::private::Number":"42"}}}',
        entry="bytes",
    )

    # --- R2 regressions: duplicates + surrogates + arbitrary integers ------
    # Exact REVIEW-2 reproduction, then the same shape with a marker key and a
    # >100-bit integer to prove those domains coexist.
    add_text(
        "DesignSpec",
        r'{"id":"spec","id":"last","name":"probe","summary":"probe","attributes_json":{"text":"\ud800"}}',
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","id":"last","name":"probe","summary":"probe","attributes_json":{"text":"\ud800"}}',
        entry="bytes",
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","id":"last","name":"probe","summary":"probe","attributes_json":{"text":"\ud800","big":1180591620717411303424,"$serde_json::private::Number":"42"}}',
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","id":"last","name":"probe","summary":"probe","attributes_json":{"text":"\ud800","big":1180591620717411303424,"$serde_json::private::Number":"42"}}',
        entry="bytes",
    )
    add_text(
        "DesignSpec",
        r'{"i\u0064":"first","id":"last","name":"probe","summary":"probe","attributes_json":{"text":"a\ud800b"}}',
    )
    # A persisted canonical object supplied as JSON *text* must still refuse
    # duplicate keys, while an ordinary parsed dict has already collapsed them.
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","attributes_json":"{\"a\":1,\"a\":2}"}',
    )
    add_text(
        "DesignSpec",
        r'{"id":"spec","name":"probe","summary":"probe","attributes_json":{"a":1,"a":2}}',
        entry="bytes",
    )

    # --- R2 regressions: duplicate/surrogate + ordered construction --------
    add_text(
        "ConstructionScript",
        r'{"summary":"first","summary":"last\ud800","components":[{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":10,"height":4},"circle":{"diameter":8}}},{"op":"extrude","length":2}]}]}',
        "load_construction",
    )
    add_text(
        "ConstructionScript",
        r'{"summary":"first","summary":"last\ud800","components":[{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":10,"height":4},"circle":{"diameter":8}}},{"op":"extrude","length":2}]}]}',
        "load_construction",
        entry="bytes",
    )
    add_text(
        "ConstructionScript",
        r'{"summary":"first","summary":"last","components":[{"name":"B","steps":[{"op":"sketch","profile":{"circle":{"diameter":8},"rectangle":{"width":10,"height":4}}},{"op":"extrude","length":2}]}]}',
        "load_construction",
        entry="bytes",
    )
    add_text(
        "ComponentStep",
        r'{"op":"bogus","op":"sketch","profile":{"rec\u0074angle":{"width":10,"height":4},"circle":{"diameter":8}}}',
        entry="bytes",
    )
    add_text(
        "ComponentStep",
        r'{"op":"sketch","profile":{"rectangle":{"width":1,"width":5,"height":2}}}',
        entry="bytes",
    )
    add_text(
        "ComponentStep",
        r'{"op":"sketch","profile":{"circle":{"diameter":1},"rectangle":{"width":2,"height":3},"circle":{"diameter":8}}}',
        entry="bytes",
    )
    add_text(
        "ComponentScript",
        r'{"name":"first","name":"B","steps":[{"op":"sketch","profile":{"circle":{"diameter":8},"rectangle":{"width":10,"height":4}}},{"op":"extrude","length":2}]}',
        entry="bytes",
    )
    add_text(
        "AnalysisReading",
        r'{"verdict":"first","verdict":"last\ud800","rewrite":{"name":"B","steps":[{"op":"sketch","profile":{"circle":{"diameter":8},"rectangle":{"width":10,"height":4}}},{"op":"extrude","length":2}]}}',
        "load_reading",
        entry="bytes",
    )
    add_text(
        "AnalysisScript",
        r'{"steps":[{"op":"material"},{"op":"solve"}]}',
        "load_analysis",
        entry="bytes",
    )
    add_text(
        "AnalysisReading",
        r'{"verdict":"first","verdict":"last","rewrite":{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":10,"height":4},"circle":{"diameter":8}}},{"op":"extrude","length":2}]}}',
        "load_reading",
    )

    json.dump({"cases": cases}, sys.stdout, ensure_ascii=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
