#!/usr/bin/env python3
"""Live Python differential oracle for the API schemas.

This is a test oracle, not product code. It builds the **live** Pydantic
models of `kala_agent.api.schemas`, replays a deterministic corpus of payloads
through them, and writes both the corpus and the answers to disk so the Rust
side can replay exactly the same payloads through the native models.

Every payload is also sent to a FastAPI application that carries the **real**
error handlers of `kala_agent.api.errors`, so the 422 the live API answers is
compared with the 422 the native layer answers.

Usage:
    python3 oracle_schemas.py <corpus.json> <results.json>
"""

from __future__ import annotations

import json
import random
import sys
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[4]
SRC = REPO / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

from fastapi import FastAPI  # noqa: E402
from pydantic import BaseModel, ValidationError  # noqa: E402

from kala_agent.api.errors import install_error_handlers  # noqa: E402
from kala_agent.api.schemas import (  # noqa: E402
    CheckpointDecisionRequest,
    CheckpointListResponse,
    CreateRevisionRequest,
    DomainListResponse,
    EvaluateDomainRequest,
    EvaluationResponse,
    HealthResponse,
    PrepareRevisionRequest,
    RevisionListResponse,
    RunMechanicalWorkflowRequest,
    RunResponse,
)

DIGEST = "a" * 64
STAMP = "2026-01-02T03:04:05Z"

MODELS: dict[str, type[BaseModel]] = {
    "EvaluateDomainRequest": EvaluateDomainRequest,
    "CreateRevisionRequest": CreateRevisionRequest,
    "PrepareRevisionRequest": PrepareRevisionRequest,
    "RunMechanicalWorkflowRequest": RunMechanicalWorkflowRequest,
    "CheckpointDecisionRequest": CheckpointDecisionRequest,
    "HealthResponse": HealthResponse,
    "DomainListResponse": DomainListResponse,
    "EvaluationResponse": EvaluationResponse,
    "RevisionListResponse": RevisionListResponse,
    "CheckpointListResponse": CheckpointListResponse,
    "RunResponse": RunResponse,
}


# ---------------------------------------------------------------------------
# The nested documents the response models carry.
# ---------------------------------------------------------------------------


def package_view() -> dict[str, Any]:
    """One registered domain package."""
    return {
        "domain": "mechanical",
        "package_id": "kala.mechanical",
        "version": "1.0.0",
        "digest": DIGEST,
        "description": "The mechanical reference package.",
        "disclaimer": "Not CAD, not CAE, not a certification.",
        "workflow_definition_id": "mechanical-reference",
        "reference_subject_id": "mechanical-reference-subject",
        "schema_digests": [DIGEST],
        "required_capabilities": ["egress.none"],
        "catalog_step_ids": ["mechanical.analyse"],
    }


def workflow_view() -> dict[str, Any]:
    """One declared workflow definition."""
    return {
        "domain": "mechanical",
        "definition_id": "mechanical-reference",
        "version": "1.0.0",
        "name": "Mechanical reference",
        "description": "The mechanical reference workflow.",
        "initial_state": "created",
        "states": ["created", "running"],
        "available_step_ids": ["mechanical.analyse"],
        "transitions": [
            {
                "transition_id": "mechanical.start",
                "from_state": "created",
                "to_state": "running",
                "step_id": "mechanical.analyse",
                "checkpoint_type": "pre_commit",
            }
        ],
        "max_replanning_attempts": 3,
    }


def evaluation_result() -> dict[str, Any]:
    """One contract evaluation result."""
    return {
        "evaluation_id": "mechanical.requirement.1",
        "subject_id": "mechanical-reference-subject",
        "requirement_id": "mechanical.requirement",
        "passed": True,
        "score": 0.5,
        "metrics": [{"name": "mechanical.metric", "value": 1.0, "unit": "mm"}],
        "findings": ["The reference design is within the bounds."],
        "artifact_ids": [],
        "evaluated_at": STAMP,
    }


def evaluation_report() -> dict[str, Any]:
    """One deterministic reference design evaluation."""
    return {
        "domain": "mechanical",
        "package_id": "kala.mechanical",
        "package_version": "1.0.0",
        "package_digest": DIGEST,
        "subject_id": "mechanical-reference-subject",
        "status": "completed",
        "passed": True,
        "score": 0.75,
        "parameters_digest": DIGEST,
        "completed_step_ids": ["mechanical.analyse"],
        "blocked_step_id": None,
        "evaluations": [evaluation_result()],
        "findings": ["The reference design is within the bounds."],
        "disclaimer": "Not CAD, not CAE, not a certification.",
    }


def checkpoint_view() -> dict[str, Any]:
    """One policy checkpoint awaiting a decision."""
    return {
        "approval_id": "approval-1",
        "transition_id": "mechanical.start",
        "step_id": "mechanical.analyse",
        "checkpoint_type": "pre_commit",
        "state_digest": DIGEST,
        "action_digest": DIGEST,
        "status": "pending",
        "actor_id": None,
        "decided_at": None,
        "reason": None,
    }


def workflow_state() -> dict[str, Any]:
    """One bounded mirror of the kernel workflow state."""
    return {
        "workflow_id": "mechanical-reference",
        "definition_id": "mechanical-reference",
        "definition_version": "1.0.0",
        "status": "waiting_for_approval",
        "current_state": "running",
        "completed_step_ids": [],
        "attempts": [{"step_id": "mechanical.analyse", "attempt": 1, "succeeded": True, "error_code": None}],
        "pending_approval": checkpoint_view(),
        "replanning_attempts": 0,
        "history": [
            {
                "sequence": 1,
                "transition_id": "mechanical.start",
                "from_state": "created",
                "to_state": "running",
                "step_id": "mechanical.analyse",
                "attempt": 1,
            }
        ],
        "last_error": None,
    }


def budget_view() -> dict[str, Any]:
    """The usage of the budget of one run."""
    return {
        "model_calls": 1,
        "tool_calls": 2,
        "total_tokens": 1234,
        "artifact_bytes": 4096,
        "duration_seconds": 1.5,
        "cost": "0.000000",
        "replanning_attempts": 0,
    }


def design_revision() -> dict[str, Any]:
    """One contract design revision."""
    return {
        "revision_id": "revision-1",
        "design_id": "mechanical-reference",
        "sequence": 1,
        "spec": {
            "id": "mechanical-reference",
            "name": "Mechanical reference",
            "summary": "The reference design of the mechanical package.",
            "parameter_schema_digest": None,
            "attributes_json": {},
            "requirement_ids": ["mechanical.requirement"],
            "constraints": [],
        },
        "parent_revision_id": None,
        "artifacts": [],
        "domain_packages": [],
        "provenance_id": "provenance-1",
        "created_at": STAMP,
    }


def provenance() -> dict[str, Any]:
    """One contract provenance record."""
    return {
        "provenance_id": "provenance-1",
        "subject_ids": ["mechanical-reference-subject"],
        "source_artifact_ids": [],
        "output_artifact_ids": [],
        "tool_call_ids": [],
        "domain_package_digests": [DIGEST],
        "parent_provenance_ids": [],
        "transformation_ids": [],
        "model_transcript_digest": None,
        "seed": None,
        "recorded_at": STAMP,
    }


def revision_record() -> dict[str, Any]:
    """One immutable revision with its provenance."""
    return {
        "revision": design_revision(),
        "provenance": provenance(),
        "artifact_digests": [DIGEST],
    }


def approval_request() -> dict[str, Any]:
    """One contract approval request."""
    return {
        "request_id": "approval-1",
        "checkpoint_type": "pre_commit",
        "status": "pending",
        "workflow_id": "mechanical-reference",
        "step_id": "mechanical.analyse",
        "action_digest": DIGEST,
        "state_digest": DIGEST,
        "options": ["approve", "reject"],
        "selected_option_id": None,
        "requested_capabilities": [],
        "artifact_ids": [],
        "created_at": STAMP,
        "decided_at": None,
        "actor_id": None,
        "modification_digest": None,
    }


def run_result() -> dict[str, Any]:
    """One mechanical reference run."""
    return {
        "workflow_id": "mechanical-reference",
        "design_id": "mechanical-reference",
        "status": "waiting_for_approval",
        "state": workflow_state(),
        "evaluations": [evaluation_result()],
        "checkpoint": checkpoint_view(),
        "policy_status": "allowed",
        "policy_reason": "The reference design is within the bounds.",
        "planned_task_ids": ["mechanical.analyse"],
        "started": True,
        "revision": revision_record(),
        "budget": budget_view(),
    }


# ---------------------------------------------------------------------------
# The corpus.
# ---------------------------------------------------------------------------


def deep_parameters(depth: int) -> Any:
    """A nested parameter document of the requested depth."""
    value: Any = 1
    for _ in range(depth):
        value = {"a": value}
    return value


def request_cases() -> list[tuple[str, Any]]:
    """The corpus of the request models."""
    cases: list[tuple[str, Any]] = []

    def add(model: str, payload: Any) -> None:
        cases.append((model, payload))

    # The defaults and the minimal bodies.
    add("EvaluateDomainRequest", {})
    add("CreateRevisionRequest", {"design_id": "a-design", "domain": "building"})
    add("PrepareRevisionRequest", {"design_id": "a-design", "domain": "building"})
    add("RunMechanicalWorkflowRequest", {})
    add("CheckpointDecisionRequest", {})

    # Every field of every request model.
    add(
        "EvaluateDomainRequest",
        {"design_id": "a-design", "parameters": {"width": 1.5, "depth": 2, "name": "café"}},
    )
    add(
        "CreateRevisionRequest",
        {
            "design_id": "a-design",
            "domain": "mechanical",
            "summary": "A summary.",
            "parent_revision_id": "revision-1",
            "parameters": {"a": 1},
            "state_digest": DIGEST,
            "action_digest": DIGEST,
        },
    )
    add(
        "PrepareRevisionRequest",
        {
            "design_id": "a-design",
            "domain": "wing",
            "summary": "A summary.",
            "parent_revision_id": "revision-1",
            "parameters": {"a": 1},
        },
    )
    add(
        "RunMechanicalWorkflowRequest",
        {"design_id": "a-design", "workflow_id": "workflow-1", "actor_id": "local-user"},
    )
    add(
        "CheckpointDecisionRequest",
        {
            "actor_id": "local-user",
            "state_digest": DIGEST,
            "action_digest": DIGEST,
            "reason": "Because.",
        },
    )

    # The whitespace, the unicode and the empty values.
    add("EvaluateDomainRequest", {"design_id": "  a-design  "})
    add("EvaluateDomainRequest", {"design_id": "a-design\n"})
    add("EvaluateDomainRequest", {"design_id": "café-设计"})
    add("EvaluateDomainRequest", {"design_id": None})
    add("EvaluateDomainRequest", {"design_id": ""})
    add("CreateRevisionRequest", {"design_id": "  a-design  ", "domain": "  building  "})
    add("CreateRevisionRequest", {"design_id": "café", "domain": "mechanical"})
    add("RunMechanicalWorkflowRequest", {"design_id": "  a-design  ", "actor_id": " local-user "})
    add("CheckpointDecisionRequest", {"reason": "  Because.  "})
    add("EvaluateDomainRequest", {"design_id": "a" * 64})
    add("EvaluateDomainRequest", {"design_id": "a" * 65})
    add("CreateRevisionRequest", {"design_id": "a" * 64, "domain": "building"})
    add("CreateRevisionRequest", {"design_id": "a" * 65, "domain": "building"})
    add("CreateRevisionRequest", {"design_id": "a-design", "domain": "building", "summary": "s" * 512})
    add(
        "CreateRevisionRequest",
        {"design_id": "a-design", "domain": "building", "summary": "s" * 513},
    )
    add(
        "CreateRevisionRequest",
        {
            "design_id": "a-design",
            "domain": "building",
            "parent_revision_id": "i" * 128,
        },
    )
    add(
        "CreateRevisionRequest",
        {
            "design_id": "a-design",
            "domain": "building",
            "parent_revision_id": "i" * 129,
        },
    )
    add("CheckpointDecisionRequest", {"reason": "r" * 512})
    add("CheckpointDecisionRequest", {"reason": "r" * 513})
    add("RunMechanicalWorkflowRequest", {"workflow_id": "i" * 128})
    add("RunMechanicalWorkflowRequest", {"workflow_id": "i" * 129})
    add("RunMechanicalWorkflowRequest", {"actor_id": "a" * 64})
    add("RunMechanicalWorkflowRequest", {"actor_id": "a" * 65})

    # The patterns.
    add("EvaluateDomainRequest", {"design_id": "a-design!"})
    add("EvaluateDomainRequest", {"design_id": "-a-design"})
    add("EvaluateDomainRequest", {"design_id": "a design"})
    add("EvaluateDomainRequest", {"design_id": "a-design\n"})
    add("CreateRevisionRequest", {"design_id": "a design", "domain": "building"})
    add(
        "CreateRevisionRequest",
        {"design_id": "a-design", "domain": "building", "parent_revision_id": "a revision"},
    )
    add(
        "CreateRevisionRequest",
        {"design_id": "a-design", "domain": "building", "state_digest": "A" * 64},
    )
    add(
        "CreateRevisionRequest",
        {"design_id": "a-design", "domain": "building", "state_digest": "a" * 63},
    )
    add(
        "CreateRevisionRequest",
        {"design_id": "a-design", "domain": "building", "action_digest": "g" * 64},
    )
    add("CheckpointDecisionRequest", {"state_digest": "a" * 65})

    # The enums.
    for domain in ("building", "mechanical", "wing"):
        add("CreateRevisionRequest", {"design_id": "a-design", "domain": domain})
    for domain in ("Building", "BUILDING", "", "nope", 5, None, True, ["building"]):
        add("CreateRevisionRequest", {"design_id": "a-design", "domain": domain})

    # The types of every field.
    for value in (5, 5.0, True, [], {}, ["a"], {"a": 1}):
        add("EvaluateDomainRequest", {"design_id": value})
        add("CreateRevisionRequest", {"design_id": "a-design", "domain": value})
        add("CreateRevisionRequest", {"design_id": value, "domain": "building"})
        add("CreateRevisionRequest", {"design_id": "a-design", "domain": "building", "summary": value})
        add("RunMechanicalWorkflowRequest", {"design_id": value})
        add("RunMechanicalWorkflowRequest", {"actor_id": value})
        add("CheckpointDecisionRequest", {"reason": value})

    # The missing fields and the undeclared ones.
    add("CreateRevisionRequest", {"domain": "building"})
    add("CreateRevisionRequest", {"design_id": "a-design"})
    add("CreateRevisionRequest", {})
    add("PrepareRevisionRequest", {"domain": "building"})
    add("PrepareRevisionRequest", {})
    add("EvaluateDomainRequest", {"design_id": "a-design", "extra": 1})
    add("EvaluateDomainRequest", {"extra": 1})
    add("CreateRevisionRequest", {"design_id": "a-design", "domain": "building", "extra": 1})
    add("RunMechanicalWorkflowRequest", {"extra": 1})
    add("CheckpointDecisionRequest", {"extra": 1})
    add("EvaluateDomainRequest", {"design_id": "a-design", "parameters": {}, "extra": {"a": 1}})

    # The parameters.
    add("EvaluateDomainRequest", {"parameters": None})
    add("EvaluateDomainRequest", {"parameters": {}})
    add("EvaluateDomainRequest", {"parameters": {"a": None, "b": True, "c": [1, 2], "d": {"e": "f"}}})
    add("EvaluateDomainRequest", {"parameters": []})
    add("EvaluateDomainRequest", {"parameters": "text"})
    add("EvaluateDomainRequest", {"parameters": 5})
    add("EvaluateDomainRequest", {"parameters": {"": 1}})
    add("EvaluateDomainRequest", {"parameters": {"k" * 64: 1}})
    add("EvaluateDomainRequest", {"parameters": {"k" * 65: 1}})
    add("EvaluateDomainRequest", {"parameters": {f"k{index}": index for index in range(256)}})
    add("EvaluateDomainRequest", {"parameters": {f"k{index}": index for index in range(257)}})
    add("EvaluateDomainRequest", {"parameters": {"a": "x" * 16384}})
    add("EvaluateDomainRequest", {"parameters": {"a": "x" * 32768}})
    add("EvaluateDomainRequest", {"parameters": {"a": "x" * 32769}})
    add("EvaluateDomainRequest", {"parameters": {"a": [1] * 2048}})
    add("EvaluateDomainRequest", {"parameters": {"a": [1] * 2049}})
    add("EvaluateDomainRequest", {"parameters": {"k" * 256: 1}})
    add("EvaluateDomainRequest", {"parameters": {"k" * 257: 1}})
    add("EvaluateDomainRequest", {"parameters": deep_parameters(48)})
    add("EvaluateDomainRequest", {"parameters": deep_parameters(49)})
    add("EvaluateDomainRequest", {"parameters": deep_parameters(64)})
    add("EvaluateDomainRequest", {"parameters": {"a": 1.5, "b": -0.0, "c": 1e300, "d": 10**60}})
    add("EvaluateDomainRequest", {"parameters": {"café": "☕", "": "x"}})
    add("EvaluateDomainRequest", {"parameters": {"a": True}})
    add("EvaluateDomainRequest", {"parameters": {"a": False}})
    add("EvaluateDomainRequest", {"parameters": {"a": None}})

    # The bodies that are not objects at all.
    for payload in (5, 5.5, "text", True, False, None, [], [1, 2], {"a": 1}):
        add("EvaluateDomainRequest", payload)
        add("CreateRevisionRequest", payload)
        add("PrepareRevisionRequest", payload)
        add("RunMechanicalWorkflowRequest", payload)
        add("CheckpointDecisionRequest", payload)

    return cases


def response_cases() -> list[tuple[str, Any]]:
    """The corpus of the response models."""
    cases: list[tuple[str, Any]] = []

    def add(model: str, payload: Any) -> None:
        cases.append((model, payload))

    # The health of the process.
    add(
        "HealthResponse",
        {
            "status": "ok",
            "service": "kala-agent",
            "version": "1.0.0",
            "provider": "local",
            "model": "local",
            "offline": True,
            "egress_configured": False,
            "provider_endpoint_declared": False,
            "data_dir": "/home/operator/.local/share/kala-agent",
            "schema_version": 3,
            "domains": ["building", "mechanical", "wing"],
            "disclaimer": "Bounded surrogate reference arithmetic only.",
        },
    )
    add(
        "HealthResponse",
        {
            "status": "ok",
            "service": "kala-agent",
            "version": "1.0.0",
            "provider": "local",
            "model": "local",
            "offline": False,
            "egress_configured": True,
            "provider_endpoint_declared": True,
            "data_dir": "/tmp/kala",
            "schema_version": 10**30,
            "domains": [],
            "disclaimer": "",
        },
    )
    for field in (
        "status",
        "service",
        "version",
        "provider",
        "model",
        "offline",
        "egress_configured",
        "provider_endpoint_declared",
        "data_dir",
        "schema_version",
        "domains",
        "disclaimer",
    ):
        payload = {
            "status": "ok",
            "service": "kala-agent",
            "version": "1.0.0",
            "provider": "local",
            "model": "local",
            "offline": True,
            "egress_configured": False,
            "provider_endpoint_declared": False,
            "data_dir": "/tmp/kala",
            "schema_version": 3,
            "domains": [],
            "disclaimer": "",
        }
        del payload[field]
        add("HealthResponse", payload)
    for value in (5, "text", True, [], {}, None):
        add(
            "HealthResponse",
            {
                "status": value,
                "service": "kala-agent",
                "version": "1.0.0",
                "provider": "local",
                "model": "local",
                "offline": True,
                "egress_configured": False,
                "provider_endpoint_declared": False,
                "data_dir": "/tmp/kala",
                "schema_version": 3,
                "domains": [],
                "disclaimer": "",
            },
        )
    add(
        "HealthResponse",
        {
            "status": "ok",
            "service": "kala-agent",
            "version": "1.0.0",
            "provider": "local",
            "model": "local",
            "offline": True,
            "egress_configured": False,
            "provider_endpoint_declared": False,
            "data_dir": "/tmp/kala",
            "schema_version": 3,
            "domains": [],
            "disclaimer": "",
            "extra": 1,
        },
    )
    add(
        "HealthResponse",
        {
            "status": "ok",
            "service": "kala-agent",
            "version": "1.0.0",
            "provider": "local",
            "model": "local",
            "offline": "yes",
            "egress_configured": False,
            "provider_endpoint_declared": False,
            "data_dir": "/tmp/kala",
            "schema_version": 3,
            "domains": [],
            "disclaimer": "",
        },
    )

    # The registered packages and workflows.
    add(
        "DomainListResponse",
        {"count": 1, "packages": [package_view()], "workflows": [workflow_view()], "domains": ["mechanical"]},
    )
    add("DomainListResponse", {"count": 0, "packages": [], "workflows": [], "domains": []})
    add("DomainListResponse", {"count": 1, "packages": [], "workflows": [], "domains": ["mechanical"]})
    for value in (5, "text", True, {}, None, [package_view()]):
        add(
            "DomainListResponse",
            {"count": 1, "packages": value, "workflows": [], "domains": []},
        )
    broken = package_view()
    del broken["domain"]
    add("DomainListResponse", {"count": 1, "packages": [broken], "workflows": [], "domains": []})
    broken = package_view()
    broken["extra"] = 1
    add("DomainListResponse", {"count": 1, "packages": [broken], "workflows": [], "domains": []})
    broken = package_view()
    broken["schema_digests"] = "not a tuple"
    add("DomainListResponse", {"count": 1, "packages": [broken], "workflows": [], "domains": []})
    broken = package_view()
    broken["schema_digests"] = [5]
    add("DomainListResponse", {"count": 1, "packages": [broken], "workflows": [], "domains": []})
    broken = workflow_view()
    broken["transitions"] = [{"transition_id": "t"}]
    add("DomainListResponse", {"count": 0, "packages": [], "workflows": [broken], "domains": []})
    broken = workflow_view()
    broken["transitions"] = [5]
    add("DomainListResponse", {"count": 0, "packages": [], "workflows": [broken], "domains": []})
    broken = workflow_view()
    broken["max_replanning_attempts"] = 1.5
    add("DomainListResponse", {"count": 0, "packages": [], "workflows": [broken], "domains": []})
    broken = workflow_view()
    broken["description"] = None
    add("DomainListResponse", {"count": 0, "packages": [], "workflows": [broken], "domains": []})

    # The evaluations.
    add("EvaluationResponse", {"evaluation": evaluation_report()})
    add("EvaluationResponse", {"evaluation": evaluation_report(), "evaluations": []})
    add(
        "EvaluationResponse",
        {"evaluation": evaluation_report(), "evaluations": [evaluation_report()]},
    )
    broken = evaluation_report()
    broken["status"] = "nope"
    add("EvaluationResponse", {"evaluation": broken})
    broken = evaluation_report()
    broken["score"] = "not a number"
    add("EvaluationResponse", {"evaluation": broken})
    broken = evaluation_report()
    broken["score"] = None
    add("EvaluationResponse", {"evaluation": broken})
    broken = evaluation_report()
    del broken["findings"]
    add("EvaluationResponse", {"evaluation": broken})
    broken = evaluation_report()
    broken["evaluations"] = [{"evaluation_id": "e"}]
    add("EvaluationResponse", {"evaluation": broken})
    broken = evaluation_report()
    broken["evaluations"] = [5]
    add("EvaluationResponse", {"evaluation": broken})
    add("EvaluationResponse", {"evaluation": 5})
    add("EvaluationResponse", {"evaluations": []})

    # The revisions.
    add("RevisionListResponse", {"design_id": "a-design", "count": 1, "revisions": [revision_record()]})
    add("RevisionListResponse", {"design_id": "a-design", "count": 0, "revisions": []})
    broken = revision_record()
    broken["revision"] = {"revision_id": "revision-1"}
    add("RevisionListResponse", {"design_id": "a-design", "count": 1, "revisions": [broken]})
    broken = revision_record()
    broken["revision"] = {**design_revision(), "revision_id": "bad id"}
    add("RevisionListResponse", {"design_id": "a-design", "count": 1, "revisions": [broken]})
    broken = revision_record()
    broken["provenance"] = {**provenance(), "provenance_id": "bad id"}
    add("RevisionListResponse", {"design_id": "a-design", "count": 1, "revisions": [broken]})
    broken = revision_record()
    broken["artifact_digests"] = [5]
    add("RevisionListResponse", {"design_id": "a-design", "count": 1, "revisions": [broken]})
    add("RevisionListResponse", {"design_id": "a-design", "count": 1, "revisions": [5]})
    add("RevisionListResponse", {"design_id": 5, "count": 1, "revisions": []})

    # The checkpoints.
    add(
        "CheckpointListResponse",
        {
            "workflow_id": "mechanical-reference",
            "count": 1,
            "pending": checkpoint_view(),
            "approvals": [approval_request()],
        },
    )
    add(
        "CheckpointListResponse",
        {
            "workflow_id": "mechanical-reference",
            "count": 0,
            "pending": None,
            "approvals": [],
        },
    )
    broken = checkpoint_view()
    broken["checkpoint_type"] = "nope"
    add(
        "CheckpointListResponse",
        {"workflow_id": "w", "count": 0, "pending": broken, "approvals": []},
    )
    broken = checkpoint_view()
    broken["decided_at"] = "2026-01-02T03:04:05"
    add(
        "CheckpointListResponse",
        {"workflow_id": "w", "count": 0, "pending": broken, "approvals": []},
    )
    broken = checkpoint_view()
    broken["decided_at"] = "2026-01-02T03:04:05+05:30"
    add(
        "CheckpointListResponse",
        {"workflow_id": "w", "count": 0, "pending": broken, "approvals": []},
    )
    broken = checkpoint_view()
    broken["decided_at"] = "not-a-date"
    add(
        "CheckpointListResponse",
        {"workflow_id": "w", "count": 0, "pending": broken, "approvals": []},
    )
    broken = checkpoint_view()
    broken["status"] = "approved"
    broken["actor_id"] = "local-user"
    broken["decided_at"] = "2026-01-02T03:04:05Z"
    add(
        "CheckpointListResponse",
        {"workflow_id": "w", "count": 0, "pending": broken, "approvals": []},
    )
    broken = approval_request()
    broken["options"] = []
    add(
        "CheckpointListResponse",
        {"workflow_id": "w", "count": 1, "pending": None, "approvals": [broken]},
    )
    add(
        "CheckpointListResponse",
        {"workflow_id": "w", "count": 1, "pending": None, "approvals": [5]},
    )
    add("CheckpointListResponse", {"workflow_id": "w", "count": 0, "approvals": []})

    # The run.
    add(
        "RunResponse",
        {
            "run": run_result(),
            "state": workflow_state(),
            "revision": revision_record(),
            "checkpoint": checkpoint_view(),
        },
    )
    add(
        "RunResponse",
        {"run": run_result(), "state": workflow_state()},
    )
    broken = run_result()
    broken["policy_status"] = "nope"
    add("RunResponse", {"run": broken, "state": workflow_state()})
    broken = run_result()
    broken["started"] = "yes"
    add("RunResponse", {"run": broken, "state": workflow_state()})
    broken = run_result()
    broken["budget"] = {"model_calls": 1}
    add("RunResponse", {"run": broken, "state": workflow_state()})
    broken = workflow_state()
    broken["attempts"] = [{"step_id": "s"}]
    add("RunResponse", {"run": run_result(), "state": broken})
    add("RunResponse", {"run": 5, "state": workflow_state()})

    return cases


def randomized_cases(seed: int) -> list[tuple[str, Any]]:
    """A deterministic corpus of mutated payloads."""
    generator = random.Random(seed)
    cases: list[tuple[str, Any]] = []
    bases = [
        ("EvaluateDomainRequest", {"design_id": "a-design", "parameters": {"a": 1, "b": "two"}}),
        (
            "CreateRevisionRequest",
            {
                "design_id": "a-design",
                "domain": "mechanical",
                "summary": "A summary.",
                "parent_revision_id": "revision-1",
                "parameters": {"a": 1},
                "state_digest": DIGEST,
                "action_digest": DIGEST,
            },
        ),
        (
            "PrepareRevisionRequest",
            {
                "design_id": "a-design",
                "domain": "building",
                "summary": "A summary.",
                "parent_revision_id": "revision-1",
                "parameters": {"a": 1},
            },
        ),
        (
            "RunMechanicalWorkflowRequest",
            {"design_id": "a-design", "workflow_id": "workflow-1", "actor_id": "local-user"},
        ),
        (
            "CheckpointDecisionRequest",
            {
                "actor_id": "local-user",
                "state_digest": DIGEST,
                "action_digest": DIGEST,
                "reason": "Because.",
            },
        ),
    ]
    values: list[Any] = [
        None,
        True,
        False,
        0,
        1,
        -1,
        5,
        5.5,
        -0.0,
        1e300,
        10**40,
        "",
        " ",
        "text",
        "café ☕",
        "a" * 64,
        "a" * 65,
        "a" * 512,
        "a" * 513,
        [],
        [1],
        ["text"],
        {},
        {"a": 1},
        "building",
        "nope",
        DIGEST,
        "A" * 64,
        "2026-01-02T03:04:05Z",
        "2026-01-02T03:04:05",
        "not-a-date",
    ]
    fields = [
        "design_id",
        "domain",
        "summary",
        "parent_revision_id",
        "parameters",
        "state_digest",
        "action_digest",
        "workflow_id",
        "actor_id",
        "reason",
        "extra",
    ]
    for model, base in bases:
        for _ in range(60):
            payload = dict(base)
            action = generator.randrange(4)
            if action == 0:
                payload[generator.choice(fields)] = generator.choice(values)
            elif action == 1:
                payload.pop(generator.choice(fields), None)
            elif action == 2:
                payload[generator.choice(fields)] = generator.choice(values)
                payload[generator.choice(fields)] = generator.choice(values)
            else:
                payload["extra"] = generator.choice(values)
            cases.append((model, payload))
    return cases


def build_corpus() -> list[dict[str, Any]]:
    """The whole corpus, in a stable order."""
    corpus: list[dict[str, Any]] = []
    for index, (model, payload) in enumerate(
        request_cases() + response_cases() + randomized_cases(20261020)
    ):
        corpus.append(
            {
                "name": f"{model}.{index:04d}",
                "model": model,
                "payload": payload,
            }
        )
    return corpus


# ---------------------------------------------------------------------------
# The answers.
# ---------------------------------------------------------------------------


def render_error(error: dict[str, Any]) -> dict[str, Any]:
    """One failure, in the shape FastAPI renders."""
    rendered = {
        "type": error["type"],
        "loc": [str(part) for part in error["loc"]],
        "msg": error["msg"],
        "input": error["input"],
        "input_canonical": json.dumps(
            error["input"],
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=True,
            default=str,
        ),
    }
    if "ctx" in error:
        rendered["ctx"] = error["ctx"]
    return rendered


def evaluate(model: type[BaseModel], payload: Any) -> dict[str, Any]:
    """Validate one payload with the live model."""
    try:
        instance = model.model_validate(payload)
    except ValidationError as error:
        return {
            "ok": False,
            "errors": [render_error(failure) for failure in error.errors()],
        }
    dumped = instance.model_dump(mode="json")
    return {
        "ok": True,
        "canonical": json.dumps(dumped, sort_keys=True, separators=(",", ":"), ensure_ascii=True),
        "wire": json.dumps(dumped, separators=(",", ":"), ensure_ascii=False),
    }


def build_app() -> FastAPI:
    """Build an application that carries the real error handlers."""
    app = FastAPI()

    for name, model in MODELS.items():

        def make(model: type[BaseModel]):
            async def endpoint(payload):
                return payload

            endpoint.__annotations__["payload"] = model
            endpoint.__name__ = f"endpoint_{model.__name__}"
            return endpoint

        app.post(f"/model/{name}")(make(model))

    install_error_handlers(app)
    return app


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print("usage: oracle_schemas.py <corpus.json> <results.json>", file=sys.stderr)
        return 2
    corpus_path = Path(argv[1])
    results_path = Path(argv[2])

    corpus = build_corpus()
    app = build_app()
    from starlette.testclient import TestClient

    client = TestClient(app, raise_server_exceptions=False)
    results: list[dict[str, Any]] = []
    for case in corpus:
        model = MODELS[case["model"]]
        answer = evaluate(model, case["payload"])
        response = client.post(
            f"/model/{case['model']}",
            json=case["payload"],
        )
        answer["status"] = response.status_code
        answer["body"] = response.text
        results.append(answer)

    corpus_path.write_text(json.dumps({"cases": corpus}, ensure_ascii=False), encoding="utf-8")
    results_path.write_text(
        json.dumps({"results": results}, ensure_ascii=False, default=str),
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
