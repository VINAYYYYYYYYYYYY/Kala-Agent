from datetime import UTC, datetime, timedelta
from decimal import Decimal
from pathlib import Path

import pytest
from pydantic import ValidationError

from kala_agent import Settings
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

DIGEST_A = "a" * 64
DIGEST_B = "b" * 64
DIGEST_C = "c" * 64
RECORDED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)


def _artifact_ref() -> ArtifactRef:
    return ArtifactRef(
        artifact_id="artifact-1",
        uri="artifact://objects/artifact-1",
        digest=DIGEST_A,
        media_type="application/json",
        size_bytes=128,
        classification=ArtifactClassification.DERIVED_DATA,
        provenance_id="provenance-1",
    )


def test_requirement_design_and_revision_are_valid_and_immutable() -> None:
    requirement = Requirement(
        id="req-1",
        title="Preserve design state",
        description="A revision remains reproducible.",
        kind=RequirementKind.FUNCTIONAL,
        priority=RequirementPriority.CRITICAL,
        status=RequirementStatus.APPROVED,
        acceptance_criteria=("A digest is recorded.",),
        tags=("storage", "audit"),
    )
    spec = DesignSpec(
        id="spec-1",
        name="Base specification",
        summary="Initial declarative state.",
        parameter_schema_digest=DIGEST_A,
        attributes_json='{"z": 2, "a": 1}',
        requirement_ids=("req-1",),
        constraints=("Inputs are immutable.",),
    )
    package = DomainPackageManifest(
        package_id="core-domain",
        version="1.0.0",
        digest=DIGEST_B,
        workflow_definition_ids=("workflow-1",),
        tool_ids=("tool-1",),
        schema_digests=(DIGEST_A, DIGEST_C),
        required_capabilities=("storage.write", "tool.execute"),
        signer_id="signer-1",
        signature_digest=DIGEST_C,
    )
    artifact = _artifact_ref()
    revision = DesignRevision(
        revision_id="revision-1",
        design_id="design-1",
        sequence=0,
        spec=spec,
        artifacts=(artifact,),
        domain_packages=(package,),
        provenance_id="provenance-1",
        created_at=RECORDED_AT,
    )

    assert requirement.status is RequirementStatus.APPROVED
    assert spec.attributes_json == '{"a":1,"z":2}'
    assert package.required_capabilities == ("storage.write", "tool.execute")
    assert revision.artifacts[0].uri == "artifact://objects/artifact-1"
    assert isinstance(requirement.acceptance_criteria, tuple)
    mutated_field = "title"
    with pytest.raises(ValidationError):
        setattr(requirement, mutated_field, "Changed")


def test_workflow_plan_and_tool_contracts_are_valid() -> None:
    plan_step = PlanStep(
        step_id="plan-step-1",
        catalog_step_id="analyze",
        intent="Analyze the recorded state.",
        parameters_json='{"resolution": 1.0, "iterations": 2}',
        input_artifact_ids=("artifact-1",),
        required_capabilities=("tool.execute",),
        estimated_duration_seconds=12.5,
    )
    transition = WorkflowTransition(
        transition_id="transition-1",
        from_state="analyze",
        to_state="complete",
        step_id="analyze",
        checkpoint_type=ApprovalKind.PRE_COMMIT,
    )
    workflow = WorkflowDefinition(
        definition_id="workflow-1",
        version="1.0.0",
        name="Reference workflow",
        initial_state="analyze",
        states=("analyze", "complete"),
        transitions=(transition,),
        available_step_ids=("analyze",),
        max_replanning_attempts=2,
    )
    descriptor = ToolDescriptor(
        tool_id="tool-1",
        version="1.2.0",
        name="Bounded evaluator",
        description="Evaluates typed input without ambient authority.",
        input_schema_digest=DIGEST_A,
        output_schema_digest=DIGEST_B,
        side_effect_level=SideEffectLevel.READ,
        required_capabilities=("tool.execute",),
        timeout_seconds=30.0,
    )
    call = ToolCall(
        call_id="call-1",
        tool_id=descriptor.tool_id,
        tool_version=descriptor.version,
        arguments_json='{"value": 2}',
        requested_capabilities=("tool.execute",),
        side_effect_level=descriptor.side_effect_level,
        timeout_seconds=descriptor.timeout_seconds,
        issued_at=RECORDED_AT,
    )
    result = ToolResult(
        call_id=call.call_id,
        tool_id=call.tool_id,
        status=ToolRunStatus.SUCCEEDED,
        result_json='{"value": 4}',
        artifacts=(_artifact_ref(),),
        started_at=RECORDED_AT,
        finished_at=RECORDED_AT + timedelta(milliseconds=25),
        duration_ms=25.0,
    )

    assert plan_step.max_attempts == 1
    assert workflow.transitions[0].checkpoint_type is ApprovalKind.PRE_COMMIT
    assert descriptor.destructive is False
    assert result.finished_at is not None
    assert result.duration_ms == 25.0


def test_evaluation_approval_budget_and_provenance_are_valid() -> None:
    metric = EvaluationMetric(
        name="margin",
        value=1.25,
        unit="ratio",
        passed=True,
    )
    evaluation = EvaluationResult(
        evaluation_id="evaluation-1",
        subject_id="revision-1",
        requirement_id="req-1",
        passed=True,
        score=0.95,
        metrics=(metric,),
        findings=("All checks passed.",),
        artifact_ids=("artifact-1",),
        evaluated_at=RECORDED_AT,
    )
    approval = ApprovalRequest(
        request_id="approval-1",
        checkpoint_type=ApprovalKind.EGRESS,
        status=ApprovalStatus.PENDING,
        workflow_id="workflow-1",
        action_digest=DIGEST_A,
        state_digest=DIGEST_B,
        options=("deny", "approve"),
        requested_capabilities=("network.egress",),
        artifact_ids=("artifact-1",),
        created_at=RECORDED_AT,
    )
    budget = RunBudget(
        max_model_calls=4,
        max_tool_calls=8,
        max_total_tokens=10_000,
        max_artifact_bytes=1_000_000,
        max_duration_seconds=600.0,
        max_cost=Decimal("1.25"),
        max_replanning_attempts=1,
    )
    provenance = Provenance(
        provenance_id="provenance-1",
        subject_ids=("revision-1",),
        source_artifact_ids=("artifact-1",),
        output_artifact_ids=("artifact-2",),
        tool_call_ids=("call-1",),
        domain_package_digests=(DIGEST_B,),
        transformation_ids=("transform-1",),
        model_transcript_digest=DIGEST_C,
        seed=7,
        recorded_at=RECORDED_AT,
    )

    assert evaluation.metrics[0].value == 1.25
    assert approval.options == ("approve", "deny")
    assert budget.max_cost == Decimal("1.25")
    assert provenance.subject_ids == ("revision-1",)


def test_models_reject_blank_identifiers_and_unknown_fields() -> None:
    with pytest.raises(ValidationError):
        Requirement(id="   ", title="Invalid", kind=RequirementKind.CONSTRAINT)
    with pytest.raises(ValidationError):
        Requirement.model_validate(
            {
                "id": "req-1",
                "title": "Valid",
                "kind": "functional",
                "administrative": True,
            }
        )


def test_json_arguments_are_canonical_and_reject_ambiguous_numbers() -> None:
    normalized = PlanStep(
        step_id="step-2",
        catalog_step_id="catalog-step",
        intent="Use typed arguments.",
        parameters_json='{"b": 2, "a": 1}',
    )
    assert normalized.parameters_json == '{"a":1,"b":2}'

    invalid_values = (
        "[]",
        '{"value":NaN}',
        '{"value":1,"value":2}',
        '{"value":Infinity}',
        '{"value":1e309}',
    )
    for value in invalid_values:
        with pytest.raises(ValidationError):
            PlanStep(
                step_id="step-3",
                catalog_step_id="catalog-step",
                intent="Reject ambiguous arguments.",
                parameters_json=value,
            )


@pytest.mark.parametrize("value", [-1.0, float("nan"), float("inf")])
def test_metrics_must_be_finite_and_nonnegative(value: float) -> None:
    with pytest.raises(ValidationError):
        EvaluationMetric(name="margin", value=value)


def test_timeouts_and_budget_metrics_are_finite_and_nonnegative() -> None:
    with pytest.raises(ValidationError):
        ToolDescriptor(
            tool_id="tool-1",
            version="1.0.0",
            name="Tool",
            description="A bounded tool.",
            input_schema_digest=DIGEST_A,
            output_schema_digest=DIGEST_B,
            side_effect_level=SideEffectLevel.READ,
            timeout_seconds=float("nan"),
        )
    with pytest.raises(ValidationError):
        RunBudget(max_duration_seconds=-1.0)
    with pytest.raises(ValidationError):
        RunBudget(max_cost=Decimal("NaN"))


def test_artifact_uris_are_normalized() -> None:
    artifact = ArtifactRef(
        artifact_id="artifact-1",
        uri="HTTPS://Example.COM:443/objects//item%20one",
        digest=DIGEST_A.upper(),
        media_type="APPLICATION/JSON",
        size_bytes=0,
    )
    assert artifact.uri == "https://example.com/objects/item%20one"
    assert artifact.digest == DIGEST_A
    assert artifact.media_type == "application/json"


@pytest.mark.parametrize(
    "uri",
    [
        "relative/path",
        "javascript:alert(1)",
        "https://user:password@example.com/object",
        "https://example.com/object?token=secret",
        "https://example.com/object#fragment",
        "file://remote-host/object",
        "https://example.com/a/%2e%2e/b",
        "https://example.com/a/%00",
        "https://example.com/a\\b",
    ],
)
def test_artifact_uris_reject_unsafe_inputs(uri: str) -> None:
    with pytest.raises(ValidationError):
        ArtifactRef(
            artifact_id="artifact-1",
            uri=uri,
            digest=DIGEST_A,
            media_type="application/json",
            size_bytes=1,
        )


def test_cross_field_contracts_reject_inconsistent_state() -> None:
    spec = DesignSpec(id="spec-1", name="Spec", summary="Summary")
    with pytest.raises(ValidationError):
        DesignRevision(
            revision_id="revision-1",
            design_id="design-1",
            sequence=1,
            spec=spec,
            parent_revision_id="revision-1",
            created_at=RECORDED_AT,
        )

    with pytest.raises(ValidationError):
        WorkflowDefinition(
            definition_id="workflow-1",
            version="1.0.0",
            name="Workflow",
            initial_state="start",
            states=("start", "unreachable"),
            available_step_ids=("analyze",),
        )

    with pytest.raises(ValidationError):
        ApprovalRequest(
            request_id="approval-1",
            checkpoint_type=ApprovalKind.PLAN_APPROVAL,
            status=ApprovalStatus.PENDING,
            workflow_id="workflow-1",
            action_digest=DIGEST_A,
            state_digest=DIGEST_B,
            options=("approve", "reject"),
            selected_option_id="approve",
            created_at=RECORDED_AT,
        )

    with pytest.raises(ValidationError):
        DomainPackageManifest(
            package_id="domain-1",
            version="1.0.0",
            digest=DIGEST_A,
            signer_id="signer-1",
        )


def test_failed_tool_result_requires_error_and_disallows_result_data() -> None:
    with pytest.raises(ValidationError):
        ToolResult(
            call_id="call-1",
            tool_id="tool-1",
            status=ToolRunStatus.FAILED,
            result_json='{"partial": true}',
        )


def test_settings_parse_allowlisted_environment_without_retaining_secrets(
    tmp_path: Path,
    caplog: pytest.LogCaptureFixture,
) -> None:
    secret = "do-not-retain-this-secret"
    settings = Settings.from_env(
        {
            "KALA_AGENT_DATA_DIR": str(tmp_path),
            "KALA_AGENT_MODEL_PROVIDER": "LOCAL",
            "KALA_AGENT_MODEL_ID": "local-model",
            "KALA_AGENT_MODEL_ENDPOINT": "http://127.0.0.1:8080/v1",
            "KALA_AGENT_REQUEST_TIMEOUT_SECONDS": "12.5",
            "KALA_AGENT_TOOL_TIMEOUT_SECONDS": "30",
            "KALA_AGENT_WORKER_COUNT": "8",
            "KALA_AGENT_MODEL_API_KEY": secret,
            "OPENAI_API_KEY": secret,
        }
    )

    assert settings.data_dir == tmp_path.resolve()
    assert settings.model_provider == "local"
    assert settings.worker_count == 8
    assert settings.artifact_dir == tmp_path.resolve() / "artifacts"
    assert settings.journal_path == tmp_path.resolve() / "journal.sqlite3"
    assert secret not in repr(settings)
    assert secret not in str(settings.model_dump(mode="json"))
    assert all(secret not in record.getMessage() for record in caplog.records)


def test_settings_reject_unsafe_paths_endpoints_and_worker_counts(tmp_path: Path) -> None:
    with pytest.raises(ValidationError):
        Settings.model_validate({"data_dir": "relative/path"})
    with pytest.raises(ValidationError):
        Settings(data_dir=Path(tmp_path.anchor))
    with pytest.raises(ValidationError):
        Settings(worker_count=9)
    with pytest.raises(ValidationError):
        Settings(request_timeout_seconds=float("inf"))
    with pytest.raises(ValidationError):
        Settings.model_validate({"model_endpoint": "https://user:password@example.com/v1"})
    with pytest.raises(ValidationError):
        Settings.model_validate({"model_endpoint": "https://example.com/v1?api_key=secret"})

    file_path = tmp_path / "not-a-directory"
    file_path.write_text("data", encoding="utf-8")
    with pytest.raises(ValidationError):
        Settings(data_dir=file_path)
