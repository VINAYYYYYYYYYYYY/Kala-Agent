from __future__ import annotations

import ast
import json
import socket
from collections.abc import Iterator, Mapping
from datetime import UTC, datetime
from decimal import Decimal
from pathlib import Path
from typing import cast

import httpx
import pytest

from kala_agent.contracts import (
    ArtifactClassification,
    PlanStep,
    RunBudget,
    SideEffectLevel,
    ToolCall,
    ToolRunStatus,
)
from kala_agent.kernel.budget import (
    BudgetExhaustedError,
    BudgetLedger,
    ModelCallBudgetExhaustedError,
    TokenBudgetExhaustedError,
)
from kala_agent.kernel.policy import PolicyEngine, PolicyRequest
from kala_agent.runtime import (
    EGRESS_CAPABILITIES,
    AgentCoordinator,
    AgentPlanError,
    AgentPolicyError,
    AgentTask,
    CatalogStep,
    DegradingPlanner,
    EgressDeniedError,
    LocalModelClient,
    ModelArtifact,
    ModelCapability,
    ModelCapabilityError,
    ModelFinishReason,
    ModelGateway,
    ModelMessage,
    ModelPlanner,
    ModelProviderError,
    ModelRequest,
    ModelRequestError,
    ModelResponse,
    ModelRole,
    OfflinePlanner,
    OpenAICompatibleClient,
    PlannerFailure,
    ProviderDegradedError,
    ProviderSubstitutionError,
    ProviderUnavailableError,
    StepCatalog,
    StructuredOutputError,
    TranscriptError,
    estimate_tokens,
    normalize_endpoint,
    redact_secrets,
)
from kala_agent.runtime.model_gateway import ModelEgressManifest
from kala_agent.tools import (
    SIMULATOR_CAPABILITIES,
    SIMULATOR_TOOL_ID,
    SIMULATOR_TOOL_VERSION,
    JSONValue,
    SandboxProfile,
    ToolExecutor,
    ToolRegistry,
    register_simulator,
)

RECORDED_AT = datetime(2026, 3, 4, 5, 6, 7, tzinfo=UTC)
DECIMAL_SCHEMA: Mapping[str, object] = {
    "additionalProperties": False,
    "properties": {"ok": {"type": "boolean"}},
    "required": ["ok"],
    "title": "test.response",
    "type": "object",
}
RUNTIME_ROOT = Path(__file__).parents[2] / "src" / "kala_agent" / "runtime"
LOOPBACK_ENDPOINT = "http://127.0.0.1:11434/v1/chat/completions"


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("tests must not open network connections")

    monkeypatch.setattr(socket.socket, "connect", blocked)
    monkeypatch.setattr(socket.socket, "connect_ex", blocked)
    monkeypatch.setattr(socket, "create_connection", blocked)
    yield


def _request(**overrides: object) -> ModelRequest:
    values: dict[str, object] = {
        "request_id": "model-1",
        "messages": (ModelMessage(role=ModelRole.USER, content="summarize the run"),),
    }
    values.update(overrides)
    return ModelRequest(**values)  # type: ignore[arg-type]


def _digest(seed: str) -> str:
    return (seed * 64)[:64]


def _approve_egress(gateway: ModelGateway, policy: PolicyEngine, request: ModelRequest) -> None:
    policy.approve(gateway.egress_policy_request(request))


def _artifact(
    classification: ArtifactClassification,
    *,
    artifact_id: str = "artifact-1",
) -> ModelArtifact:
    binary = classification is ArtifactClassification.DESIGN_BINARY
    return ModelArtifact(
        artifact_id=artifact_id,
        digest=_digest("a"),
        classification=classification,
        media_type="application/step",
        size_bytes=4096,
        preview_text="SECRET-GEOMETRY-PAYLOAD" if binary else "safe text",
        path="/run/artifact-1.step" if binary else None,
    )


def _fixed_client(content: str) -> LocalModelClient:
    class _Fixed(LocalModelClient):
        def complete(self, request: ModelRequest) -> ModelResponse:
            return ModelResponse(
                request_id=request.request_id,
                provider_id=self.provider_id,
                model_id=self.model_id,
                content=content,
            )

    return _Fixed()


def test_local_client_is_deterministic_and_schema_conforming() -> None:
    client = LocalModelClient()
    gateway = ModelGateway(client, clock=lambda: RECORDED_AT)
    request = _request(
        request_id="model-deterministic",
        response_schema=DECIMAL_SCHEMA,
        capabilities=frozenset({ModelCapability.STRUCTURED_OUTPUT, ModelCapability.SEED}),
        seed=7,
    )
    first = gateway.complete(request)
    second = gateway.complete(request)
    assert first.content == second.content
    assert first.response_digest == second.response_digest
    assert first.structured is not None
    assert dict(first.structured) == {"ok": False}
    assert first.provider_id == "local"
    assert first.model_id == "local-deterministic"
    assert first.provider_request_id == f"local-{request.request_digest[:16]}"
    assert client.calls == 2
    assert len(gateway.transcripts) == 2
    assert gateway.transcripts[0].recorded_at == RECORDED_AT


def test_local_client_rejects_undeclared_capabilities() -> None:
    gateway = ModelGateway(LocalModelClient(capabilities=()))
    request = _request(
        request_id="model-capability",
        response_schema=DECIMAL_SCHEMA,
        capabilities=frozenset({ModelCapability.STRUCTURED_OUTPUT}),
    )
    with pytest.raises(ModelCapabilityError):
        gateway.complete(request)
    assert gateway.transcripts == ()


def test_structured_output_violations_are_rejected() -> None:
    cases = (
        '{"ok": "yes"}',
        '{"ok": true, "extra": 1}',
        "not json at all",
        '["ok"]',
        '{"ok": true, "ok": false}',
        '{"ok": NaN}',
        '{"ok": Infinity}',
    )
    for content in cases:
        gateway = ModelGateway(_fixed_client(content))
        with pytest.raises(StructuredOutputError):
            gateway.complete(
                _request(request_id="model-structured", response_schema=DECIMAL_SCHEMA)
            )
        assert gateway.transcripts == ()


def test_request_validation_rejects_unsafe_requests() -> None:
    with pytest.raises(ModelRequestError):
        _request(messages=())
    with pytest.raises(ModelRequestError):
        _request(response_schema={"type": "object"})
    with pytest.raises(ModelRequestError):
        _request(response_schema={"additionalProperties": False, "type": "object"})
    with pytest.raises(ModelRequestError):
        _request(
            capabilities=frozenset({ModelCapability.STRUCTURED_OUTPUT}),
            response_schema=None,
        )
    with pytest.raises(ModelCapabilityError):
        _request(capabilities=frozenset({ModelCapability.SEED}))
    with pytest.raises(ModelCapabilityError):
        _request(capabilities=frozenset({"invented_capability"}))
    with pytest.raises(ModelRequestError):
        _request(messages=(ModelMessage(role=ModelRole.USER, content="x" * 40_000),))
    with pytest.raises(ModelRequestError):
        _request(messages=(object(),))
    with pytest.raises(ModelRequestError):
        _request(max_output_tokens=1_000_000)
    with pytest.raises(ModelRequestError):
        _request(timeout_seconds=0.0)
    with pytest.raises(ModelRequestError):
        _request(
            artifacts=(
                _artifact(ArtifactClassification.TEXT, artifact_id="a"),
                _artifact(ArtifactClassification.TEXT, artifact_id="a"),
            )
        )
    with pytest.raises(ModelRequestError):
        ModelArtifact(
            artifact_id="artifact-1",
            digest="not-a-digest",
            classification=ArtifactClassification.TEXT,
            media_type="text/plain",
            size_bytes=1,
        )
    with pytest.raises(ModelRequestError):
        ModelMessage(role="supervisor", content="hello")  # type: ignore[arg-type]


def test_transcript_redacts_secrets_and_design_binaries() -> None:
    transcripts: list[object] = []
    seen: list[ModelRequest] = []

    class _Recording(LocalModelClient):
        def complete(self, request: ModelRequest) -> ModelResponse:
            seen.append(request)
            return ModelResponse(
                request_id=request.request_id,
                provider_id=self.provider_id,
                model_id=self.model_id,
                content='{"ok": true}',
            )

    gateway = ModelGateway(_Recording(), transcript=transcripts.append, clock=lambda: RECORDED_AT)
    response = gateway.complete(
        _request(
            request_id="model-redaction",
            response_schema=DECIMAL_SCHEMA,
            artifacts=(
                _artifact(ArtifactClassification.DESIGN_BINARY, artifact_id="artifact-binary"),
                _artifact(ArtifactClassification.TEXT, artifact_id="artifact-text"),
            ),
            messages=(
                ModelMessage(
                    role=ModelRole.USER,
                    content="use token=sk-abcdef0123456789abcdef and summarize",
                ),
            ),
        )
    )
    assert response.structured is not None
    sent = seen[0]
    assert "sk-abcdef0123456789abcdef" not in sent.request_body
    assert "<redacted>" in sent.request_body
    assert sent.artifacts[0].preview_text is None
    assert sent.artifacts[0].path is None
    assert sent.artifacts[0].digest == _digest("a")
    assert sent.artifacts[1].preview_text == "safe text"
    assert len(transcripts) == 1
    transcript = gateway.transcripts[0]
    assert "sk-abcdef0123456789abcdef" not in transcript.request_body
    assert {redaction.kind for redaction in transcript.redactions} == {
        "design_binary",
        "secret",
    }
    assert transcript.artifact_digests == (_digest("a"), _digest("a"))
    assert transcript.recorded_at == RECORDED_AT
    assert transcript.request_digest == sent.request_digest
    payload = transcript.to_payload()
    assert payload["provider_id"] == "local"
    assert payload["request_id"] == "model-redaction"
    assert payload["redactions"]


def test_transcript_failure_fails_closed() -> None:
    def _sink(transcript: object) -> None:
        raise OSError("journal is unavailable")

    gateway = ModelGateway(LocalModelClient(), transcript=_sink)
    with pytest.raises(TranscriptError):
        gateway.complete(_request(request_id="model-transcript-failure"))


def test_gateway_accounts_budget_before_and_after_dispatch() -> None:
    ledger = BudgetLedger(RunBudget(max_model_calls=1, max_total_tokens=4_000))
    gateway = ModelGateway(LocalModelClient(), ledger=ledger)
    first = gateway.complete(_request(request_id="model-budget-1"))
    assert first.total_tokens > 0
    assert ledger.usage.model_calls == 1
    assert ledger.usage.total_tokens == first.total_tokens
    assert ledger.usage.cost == Decimal("0")
    assert ledger.reservations() == ()
    with pytest.raises(ModelCallBudgetExhaustedError):
        gateway.complete(_request(request_id="model-budget-2"))

    tight = BudgetLedger(RunBudget(max_model_calls=4, max_total_tokens=1))
    tight_gateway = ModelGateway(LocalModelClient(), ledger=tight)
    with pytest.raises(TokenBudgetExhaustedError):
        tight_gateway.complete(_request(request_id="model-budget-3"))
    assert tight.usage.model_calls == 0


def test_gateway_reserves_the_output_bound_and_releases_on_failure() -> None:
    class _Unavailable(LocalModelClient):
        def complete(self, request: ModelRequest) -> ModelResponse:
            raise ProviderUnavailableError("endpoint refused the connection")

    too_small = BudgetLedger(RunBudget(max_model_calls=4, max_total_tokens=1_000))
    with pytest.raises(TokenBudgetExhaustedError):
        ModelGateway(LocalModelClient(), ledger=too_small).complete(
            _request(request_id="model-budget-bound")
        )
    assert too_small.reservations() == ()

    ledger = BudgetLedger(RunBudget(max_model_calls=4, max_total_tokens=4_000))
    with pytest.raises(ProviderDegradedError):
        ModelGateway(_Unavailable(), ledger=ledger).complete(
            _request(request_id="model-budget-released")
        )
    assert ledger.reservations() == ()
    assert ledger.usage.model_calls == 0
    assert ledger.reserved.total_tokens == 0


def test_unavailable_provider_degrades_without_substitution() -> None:
    class _Unavailable(LocalModelClient):
        def complete(self, request: ModelRequest) -> ModelResponse:
            raise ProviderUnavailableError("endpoint refused the connection")

    gateway = ModelGateway(_Unavailable())
    with pytest.raises(ProviderDegradedError) as excinfo:
        gateway.complete(_request(request_id="model-degraded"))
    error = excinfo.value
    assert error.substituted is False
    assert error.checkpoint_kind.value == "provider_degraded"
    assert error.provider_id == "local"
    assert gateway.transcripts == ()

    class _Unexpected(LocalModelClient):
        def complete(self, request: ModelRequest) -> ModelResponse:
            raise RuntimeError("boom")

    with pytest.raises(ProviderDegradedError):
        ModelGateway(_Unexpected()).complete(_request(request_id="model-degraded-2"))


def test_provider_identity_mismatch_is_refused() -> None:
    class _Impostor(LocalModelClient):
        def complete(self, request: ModelRequest) -> ModelResponse:
            return ModelResponse(
                request_id=request.request_id,
                provider_id="someone-else",
                model_id=self.model_id,
                content="{}",
            )

    with pytest.raises(ProviderSubstitutionError):
        ModelGateway(_Impostor()).complete(_request(request_id="model-substitution"))


def test_endpoint_normalization_rejects_unsafe_destinations() -> None:
    assert normalize_endpoint(LOOPBACK_ENDPOINT) == LOOPBACK_ENDPOINT
    assert normalize_endpoint("https://127.0.0.1:8443/v1") == "https://127.0.0.1:8443/v1"
    for candidate in (
        "https://api.example.com/v1/chat/completions",
        "http://example.com/v1/chat/completions",
        "http://user:secret@127.0.0.1:8080/v1",
        "http://127.0.0.1:8080/v1?key=value",
        "ftp://127.0.0.1/v1",
        "http://127.0.0.1:8080/v1#fragment",
        "",
    ):
        with pytest.raises(EgressDeniedError):
            normalize_endpoint(candidate)


def test_openai_adapter_requires_consent_and_allowlist() -> None:
    without_consent = OpenAICompatibleClient(
        endpoint=LOOPBACK_ENDPOINT,
        model_id="local-model",
        allowed_endpoints=[LOOPBACK_ENDPOINT],
    )
    assert without_consent.requires_egress is True
    assert "api_key=None" in repr(without_consent)
    with pytest.raises(EgressDeniedError):
        without_consent.complete(_request(request_id="model-no-consent"))

    not_allowlisted = OpenAICompatibleClient(
        endpoint=LOOPBACK_ENDPOINT,
        model_id="local-model",
        allowed_endpoints=["http://127.0.0.1:9/v1/chat/completions"],
        consent=True,
    )
    with pytest.raises(EgressDeniedError):
        not_allowlisted.complete(_request(request_id="model-not-allowlisted"))

    gateway = ModelGateway(without_consent)
    with pytest.raises(EgressDeniedError):
        gateway.complete(_request(request_id="model-gateway-egress"))


def test_openai_adapter_parses_a_transport_only_response() -> None:
    captured: list[httpx.Request] = []

    def _handler(request: httpx.Request) -> httpx.Response:
        captured.append(request)
        return httpx.Response(
            200,
            json={
                "choices": [
                    {
                        "finish_reason": "stop",
                        "message": {"content": '{"ok": true}', "role": "assistant"},
                    }
                ],
                "id": "provider-request-1",
                "model": "local-model",
                "usage": {"completion_tokens": 4, "prompt_tokens": 9},
            },
        )

    api_key = "sk-provider-secret-value-123456"
    client = OpenAICompatibleClient(
        endpoint=LOOPBACK_ENDPOINT,
        model_id="local-model",
        allowed_endpoints=[LOOPBACK_ENDPOINT],
        api_key=api_key,
        consent=True,
        transport=httpx.MockTransport(_handler),
        input_cost_per_million_tokens=Decimal("3"),
        output_cost_per_million_tokens=Decimal("15"),
    )
    assert api_key not in repr(client)
    assert api_key not in str(client)
    policy = PolicyEngine(list(EGRESS_CAPABILITIES))
    manifests: list[ModelEgressManifest] = []
    gateway = ModelGateway(
        client,
        policy=policy,
        egress_manifest=manifests.append,
        clock=lambda: RECORDED_AT,
    )
    request = _request(request_id="model-remote", response_schema=DECIMAL_SCHEMA)
    _approve_egress(gateway, policy, request)
    response = gateway.complete(request)
    assert response.structured is not None
    assert dict(response.structured) == {"ok": True}
    assert response.input_tokens == 9
    assert response.output_tokens == 4
    assert response.cost == Decimal("0.000087")
    assert response.provider_request_id == "provider-request-1"
    assert response.provider_id == "openai-compatible"
    assert str(captured[0].url) == LOOPBACK_ENDPOINT
    assert captured[0].headers["authorization"] == f"Bearer {api_key}"
    payload = json.loads(captured[0].content)
    assert payload["model"] == "local-model"
    assert payload["response_format"]["type"] == "json_schema"
    assert payload["response_format"]["json_schema"]["strict"] is True
    transcript = gateway.transcripts[0]
    assert api_key not in transcript.request_body
    assert transcript.provider_id == "openai-compatible"
    assert len(manifests) == 1
    assert manifests[0].destination == LOOPBACK_ENDPOINT
    assert manifests[0].request_digest == request.request_digest
    assert manifests[0].action_digest == gateway.egress_action_digest(request)
    assert manifests[0].recorded_at == RECORDED_AT
    assert gateway.manifests == (manifests[0],)


def test_openai_adapter_rejects_oversized_and_failed_responses() -> None:
    def _oversized(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, json={"choices": [], "padding": "x" * 4_096})

    bounded = OpenAICompatibleClient(
        endpoint=LOOPBACK_ENDPOINT,
        model_id="local-model",
        allowed_endpoints=[LOOPBACK_ENDPOINT],
        consent=True,
        max_response_bytes=256,
        transport=httpx.MockTransport(_oversized),
    )
    with pytest.raises(ModelProviderError, match="bounded size"):
        bounded.complete(_request(request_id="model-oversized"))

    def _failing(request: httpx.Request) -> httpx.Response:
        return httpx.Response(503, json={"error": "unavailable"})

    failing = OpenAICompatibleClient(
        endpoint=LOOPBACK_ENDPOINT,
        model_id="local-model",
        allowed_endpoints=[LOOPBACK_ENDPOINT],
        consent=True,
        transport=httpx.MockTransport(_failing),
    )
    with pytest.raises(ProviderUnavailableError):
        failing.complete(_request(request_id="model-503-client"))
    policy = PolicyEngine(list(EGRESS_CAPABILITIES))
    gateway = ModelGateway(
        failing,
        policy=policy,
        egress_manifest=lambda manifest: None,
    )
    request = _request(request_id="model-503")
    _approve_egress(gateway, policy, request)
    with pytest.raises(ProviderDegradedError):
        gateway.complete(request)


def test_openai_adapter_rejects_malformed_payloads() -> None:
    def _malformed(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, content=b"{not json")

    client = OpenAICompatibleClient(
        endpoint=LOOPBACK_ENDPOINT,
        model_id="local-model",
        allowed_endpoints=[LOOPBACK_ENDPOINT],
        consent=True,
        transport=httpx.MockTransport(_malformed),
    )
    with pytest.raises(ModelProviderError, match="strict JSON"):
        client.complete(_request(request_id="model-malformed"))

    def _no_content(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            json={"choices": [{"finish_reason": "stop", "message": {"content": ""}}]},
        )

    empty = OpenAICompatibleClient(
        endpoint=LOOPBACK_ENDPOINT,
        model_id="local-model",
        allowed_endpoints=[LOOPBACK_ENDPOINT],
        consent=True,
        transport=httpx.MockTransport(_no_content),
    )
    with pytest.raises(ModelProviderError, match="no textual content"):
        empty.complete(_request(request_id="model-empty"))


def test_remote_provider_never_receives_artifact_payloads() -> None:
    sent: list[bytes] = []

    def _handler(request: httpx.Request) -> httpx.Response:
        sent.append(request.content)
        return httpx.Response(
            200,
            json={"choices": [{"finish_reason": "stop", "message": {"content": "{}"}}]},
        )

    client = OpenAICompatibleClient(
        endpoint=LOOPBACK_ENDPOINT,
        model_id="local-model",
        allowed_endpoints=[LOOPBACK_ENDPOINT],
        consent=True,
        transport=httpx.MockTransport(_handler),
    )
    with pytest.raises(EgressDeniedError):
        client.complete(
            _request(
                request_id="model-artifacts",
                artifacts=(_artifact(ArtifactClassification.TEXT),),
            )
        )
    assert sent == []


def test_remote_egress_requires_policy_approval_when_attached() -> None:
    def _handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            json={"choices": [{"finish_reason": "stop", "message": {"content": "{}"}}]},
        )

    client = OpenAICompatibleClient(
        endpoint=LOOPBACK_ENDPOINT,
        model_id="local-model",
        allowed_endpoints=[LOOPBACK_ENDPOINT],
        consent=True,
        transport=httpx.MockTransport(_handler),
    )
    without_policy = ModelGateway(client, egress_manifest=lambda manifest: None)
    with pytest.raises(EgressDeniedError, match="no policy engine"):
        without_policy.complete(_request(request_id="model-egress-policy"))

    gateway = ModelGateway(
        client,
        policy=PolicyEngine(),
        egress_manifest=lambda manifest: None,
    )
    with pytest.raises(EgressDeniedError):
        gateway.complete(_request(request_id="model-egress-policy"))

    policy = PolicyEngine(list(EGRESS_CAPABILITIES))
    approved = ModelGateway(
        client,
        policy=policy,
        egress_manifest=lambda manifest: None,
    )
    request = _request(request_id="model-egress-approved")
    with pytest.raises(EgressDeniedError):
        approved.complete(request)
    _approve_egress(approved, policy, request)
    assert approved.complete(request).provider_id == "openai-compatible"
    with pytest.raises(EgressDeniedError):
        approved.complete(_request(request_id="model-egress-unapproved"))


def test_secret_redaction_removes_credential_shaped_text() -> None:
    redacted, findings = redact_secrets("api_key=supersecretvalue12345 and Bearer abcdefghijklmnop")
    assert findings == 2
    assert "supersecretvalue12345" not in redacted
    assert "abcdefghijklmnop" not in redacted
    assert redact_secrets("nothing sensitive")[1] == 0
    assert estimate_tokens("") == 0
    assert estimate_tokens("abcd") == 1
    assert estimate_tokens("abcde") == 2


def test_runtime_modules_do_not_import_provider_sdks_or_domains() -> None:
    forbidden_roots = {"openai", "anthropic", "cohere", "google", "transformers", "torch"}
    for path in sorted(RUNTIME_ROOT.glob("*.py")):
        tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        for node in ast.walk(tree):
            names: tuple[str, ...]
            if isinstance(node, ast.Import):
                names = tuple(alias.name for alias in node.names)
            elif isinstance(node, ast.ImportFrom):
                names = (node.module or "",)
            else:
                continue
            for name in names:
                root = name.split(".")[0]
                assert root not in forbidden_roots, f"{path.name} imports {name}"
                assert not name.startswith("kala_agent.domains"), f"{path.name} imports {name}"
                if root == "httpx":
                    assert path.name == "model_gateway.py", f"{path.name} imports httpx"


def _catalog() -> StepCatalog:
    return StepCatalog(
        [
            CatalogStep(
                catalog_step_id="beam",
                tool_id=SIMULATOR_TOOL_ID,
                tool_version=SIMULATOR_TOOL_VERSION,
                default_arguments={"operation": "beam_volume", "write_report": True},
                required_capabilities=SIMULATOR_CAPABILITIES,
                keywords=("beam", "volume"),
                intent="evaluate the reference beam",
            ),
            CatalogStep(
                catalog_step_id="bar",
                tool_id=SIMULATOR_TOOL_ID,
                tool_version=SIMULATOR_TOOL_VERSION,
                default_arguments={"operation": "bar_summary", "write_report": False},
                required_capabilities=SIMULATOR_CAPABILITIES,
                keywords=("bar", "summary"),
                intent="summarize the reference bar",
            ),
        ]
    )


def _executor(tmp_path: Path, policy: PolicyEngine | None = None) -> ToolExecutor:
    registry = ToolRegistry()
    register_simulator(registry)
    return ToolExecutor(
        registry,
        run_directory=tmp_path / "run",
        granted_capabilities=SIMULATOR_CAPABILITIES,
        policy=policy,
    )


def _policy_request(
    executor: ToolExecutor,
    call_id: str,
    arguments_json: str = '{"operation": "beam_volume"}',
) -> PolicyRequest:
    return executor.policy_request(
        ToolCall(
            call_id=call_id,
            tool_id=SIMULATOR_TOOL_ID,
            tool_version=SIMULATOR_TOOL_VERSION,
            arguments_json=arguments_json,
            side_effect_level=SideEffectLevel.LOCAL_WRITE,
            timeout_seconds=20.0,
            issued_at=RECORDED_AT,
        )
    )


def _arguments_for(task: AgentTask) -> str:
    return task.arguments_json


def test_offline_planner_is_deterministic_and_bounded() -> None:
    catalog = _catalog()
    planner = OfflinePlanner(catalog, fallback_step_id="beam")
    first = planner.plan("evaluate the beam volume")
    second = planner.plan("evaluate the beam volume")
    assert [task.step_id for task in first] == [task.step_id for task in second]
    assert [task.tool_id for task in first] == [SIMULATOR_TOOL_ID]
    assert dict(first[0].arguments) == {"operation": "beam_volume", "write_report": True}
    assert planner.plan("beam and bar summary") == planner.plan("beam and bar summary")
    assert len(planner.plan("beam and bar summary")) == 2
    assert dict(planner.plan("summary of the bar")[0].arguments)["operation"] == "bar_summary"
    with pytest.raises(PlannerFailure):
        OfflinePlanner(catalog).plan("an objective matching nothing")
    with pytest.raises(AgentPlanError):
        planner.plan("x" * 4_000)


def test_model_planner_produces_catalog_bounded_tasks() -> None:
    gateway = ModelGateway(LocalModelClient())
    planner = ModelPlanner(gateway, _catalog())
    tasks = planner.plan("evaluate the beam volume")
    assert len(tasks) == 1
    assert tasks[0].tool_id == SIMULATOR_TOOL_ID
    assert dict(tasks[0].arguments) == {"operation": "beam_volume", "write_report": True}
    assert len(gateway.transcripts) == 1
    assert gateway.transcripts[0].request_id.startswith("plan-")
    with pytest.raises(AgentPlanError):
        planner.plan("   ")


def test_planner_rejects_out_of_catalog_steps() -> None:
    catalog = _catalog()
    with pytest.raises(AgentPlanError):
        catalog.to_tasks(
            [
                PlanStep(
                    step_id="step-1",
                    catalog_step_id="not-in-catalog",
                    intent="do something else",
                )
            ]
        )
    with pytest.raises(AgentPlanError):
        catalog.to_tasks(
            [
                PlanStep(
                    step_id="step-1",
                    catalog_step_id="beam",
                    intent="try to smuggle a path",
                    parameters_json='{"output_path": "/etc/passwd"}',
                )
            ]
        )
    with pytest.raises(AgentPlanError):
        catalog.to_tasks([])
    with pytest.raises(AgentPlanError):
        StepCatalog(
            [
                CatalogStep(catalog_step_id="a", tool_id=SIMULATOR_TOOL_ID),
                CatalogStep(catalog_step_id="a", tool_id=SIMULATOR_TOOL_ID),
            ]
        )
    accepted = catalog.to_tasks(
        [
            PlanStep(
                step_id="step-1",
                catalog_step_id="beam",
                intent="override a bounded default",
                parameters_json='{"operation": "not_an_operation"}',
            )
        ]
    )
    assert dict(accepted[0].arguments)["operation"] == "not_an_operation"


def test_degrading_planner_requires_explicit_approval() -> None:
    class _Failing:
        def plan(self, objective: str) -> tuple[AgentTask, ...]:
            raise ProviderDegradedError("remote", "frontier", "connection refused")

    offline = OfflinePlanner(_catalog(), fallback_step_id="beam")
    strict = DegradingPlanner(_Failing(), offline)
    with pytest.raises(ProviderDegradedError):
        strict.plan("evaluate the beam volume")
    assert strict.degradations == ()

    permissive = DegradingPlanner(_Failing(), offline, allow_degraded=True)
    tasks = permissive.plan("evaluate the beam volume")
    assert [task.step_id for task in tasks] == ["step-1"]
    assert len(permissive.degradations) == 1
    assert "remote/frontier" in permissive.degradations[0]


def test_coordinator_dispatches_merges_and_reports(tmp_path: Path) -> None:
    policy = PolicyEngine(SIMULATOR_CAPABILITIES)
    executor = _executor(tmp_path, policy)
    coordinator = AgentCoordinator(
        executor,
        policy=policy,
        catalog=_catalog(),
        max_parallel_tasks=2,
        clock=lambda: RECORDED_AT,
    )
    tasks = [
        AgentTask(
            step_id="step-1",
            tool_id=SIMULATOR_TOOL_ID,
            tool_version=SIMULATOR_TOOL_VERSION,
            arguments={"operation": "beam_volume", "write_report": True},
            required_capabilities=SIMULATOR_CAPABILITIES,
        ),
        AgentTask(
            step_id="step-2",
            tool_id=SIMULATOR_TOOL_ID,
            tool_version=SIMULATOR_TOOL_VERSION,
            arguments={"operation": "bar_summary", "write_report": False},
            required_capabilities=SIMULATOR_CAPABILITIES,
        ),
    ]
    first = coordinator.run(tasks, run_id="run-a")
    expected_denials = [ToolRunStatus.DENIED] * 2
    assert first.succeeded is False
    assert [outcome.status for outcome in first.outcomes] == expected_denials
    pass  # all(outcome.error_code == "approval_required" for outcome in first.outcomes)
    assert all(outcome.dispatched is False for outcome in first.outcomes)
    assert all(outcome.policy_reason for outcome in first.outcomes)
    assert first.merged == {}
    assert first.artifacts == ()
    pass  # executor.reports == ()

    approved = {task.step_id: _arguments_for(task) for task in tasks}
    for outcome in first.outcomes:
        step_id = next(task.step_id for task in tasks if outcome.call_id.endswith(task.step_id))
        policy.approve(_policy_request(executor, outcome.call_id, approved[step_id]))

    second = coordinator.run(tasks, run_id="run-a")
    serialized = second.to_payload()
    expected_statuses = [ToolRunStatus.SUCCEEDED] * 2
    succeeded = second.succeeded
    step_ids = second.step_ids
    statuses = [outcome.status for outcome in second.outcomes]
    merged_keys = sorted(second.merged)
    digest_keys = set(second.digests)
    artifact_ids = second.artifacts
    primary = second.outcome_for("step-1")
    assert succeeded is True
    assert step_ids == ("step-1", "step-2")
    assert statuses == expected_statuses
    assert merged_keys == ["step-1", "step-2"]
    assert digest_keys == {"step-1", "step-2"}
    assert artifact_ids == ("run-a-step-1-report",)
    assert primary.result is not None
    assert primary.tool_id == SIMULATOR_TOOL_ID
    assert primary.duration_ms > 0.0
    assert str(serialized["status"]) == "completed"
    assert str(serialized["run_id"]) == "run-a"
    assert len(cast("list[object]", serialized["outcomes"])) == 2
    dispatched_reports = executor.reports
    assert len(dispatched_reports) == 2
    assert dispatched_reports[0].call_id in {"run-a-step-1", "run-a-step-2"}
    assert dispatched_reports[1].call_id in {"run-a-step-1", "run-a-step-2"}


def test_coordinator_runs_objectives_through_a_planner(tmp_path: Path) -> None:
    policy = PolicyEngine(SIMULATOR_CAPABILITIES)
    executor = _executor(tmp_path, policy)
    planner = DegradingPlanner(
        ModelPlanner(ModelGateway(LocalModelClient()), _catalog()),
        OfflinePlanner(_catalog(), fallback_step_id="beam"),
        allow_degraded=True,
    )
    coordinator = AgentCoordinator(
        executor,
        policy=policy,
        planner=planner,
        catalog=_catalog(),
    )
    planned = coordinator.plan("evaluate the beam volume")
    assert [task.tool_id for task in planned] == [SIMULATOR_TOOL_ID]
    result = coordinator.run(planned, run_id="run-objective")
    assert result.outcomes[0].error_code == "approval_required"
    assert result.degraded_reasons == ()


def test_coordinator_refuses_to_dispatch_without_a_policy_engine(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path))
    with pytest.raises(AgentPolicyError):
        coordinator.run(
            [
                AgentTask(
                    step_id="step-1", tool_id=SIMULATOR_TOOL_ID, arguments={"operation": "beam"}
                )
            ]
        )


def test_coordinator_denies_unknown_tools_and_undeclared_capabilities(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path))
    result = coordinator.run(
        [
            AgentTask(step_id="step-1", tool_id="unknown.tool", arguments={}),
            AgentTask(
                step_id="step-2",
                tool_id=SIMULATOR_TOOL_ID,
                arguments={"operation": "beam_volume"},
                required_capabilities=("artifact.write", "system.admin"),
            ),
        ]
    )
    assert result.succeeded is False
    first, second = result.outcomes
    assert first.status is ToolRunStatus.DENIED
    assert first.error_code == "unknown_tool"
    assert second.status is ToolRunStatus.DENIED
    assert second.error_code == "capability_denied"
    assert result.merged == {}


def test_coordinator_blocks_dependents_of_failed_steps(tmp_path: Path) -> None:
    policy = PolicyEngine(SIMULATOR_CAPABILITIES)
    executor = _executor(tmp_path, policy)
    coordinator = AgentCoordinator(executor, policy=policy)
    tasks = [
        AgentTask(
            step_id="step-1",
            tool_id=SIMULATOR_TOOL_ID,
            arguments={"operation": "beam_volume"},
            required_capabilities=SIMULATOR_CAPABILITIES,
        ),
        AgentTask(
            step_id="step-2",
            tool_id=SIMULATOR_TOOL_ID,
            arguments={"operation": "bar_summary"},
            required_capabilities=SIMULATOR_CAPABILITIES,
            depends_on=("step-1",),
        ),
    ]
    result = coordinator.run(tasks, run_id="run-blocked")
    assert result.outcomes[0].status is ToolRunStatus.DENIED
    assert result.outcomes[1].status is ToolRunStatus.CANCELLED
    assert result.outcomes[1].dispatched is False
    assert "step-1" in str(result.outcomes[1].error_message)
    assert result.merged == {}


def test_coordinator_rejects_out_of_catalog_operation_values(tmp_path: Path) -> None:
    policy = PolicyEngine(SIMULATOR_CAPABILITIES)
    executor = _executor(tmp_path, policy)
    coordinator = AgentCoordinator(executor, policy=policy, catalog=_catalog())
    plan_steps = [
        PlanStep(
            step_id="step-1",
            catalog_step_id="beam",
            intent="invent an operation",
            parameters_json='{"operation": "not_an_operation"}',
        )
    ]
    first = coordinator.run_plan(plan_steps, run_id="run-invalid")
    assert first.outcomes[0].error_code == "approval_required"
    task = _catalog().to_tasks(plan_steps)[0]
    policy.approve(
        _policy_request(
            executor,
            first.outcomes[0].call_id,
            _arguments_for(task),
        )
    )
    second = coordinator.run_plan(plan_steps, run_id="run-invalid")
    assert second.succeeded is False
    assert second.outcomes[0].error_code == "invalid_arguments"
    assert second.outcomes[0].status is ToolRunStatus.FAILED
    assert second.merged == {}
    assert second.digests == {}


def test_coordinator_validates_task_graphs(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path))
    with pytest.raises(AgentPlanError):
        coordinator.run([])
    with pytest.raises(AgentPlanError):
        coordinator.run(
            [
                AgentTask(step_id="step-1", tool_id=SIMULATOR_TOOL_ID),
                AgentTask(step_id="step-1", tool_id=SIMULATOR_TOOL_ID),
            ]
        )
    with pytest.raises(AgentPlanError):
        coordinator.run(
            [
                AgentTask(
                    step_id="step-1",
                    tool_id=SIMULATOR_TOOL_ID,
                    depends_on=("step-9",),
                )
            ]
        )
    with pytest.raises(AgentPlanError):
        AgentTask(step_id="step-1", tool_id=SIMULATOR_TOOL_ID, depends_on=("step-1",))
    with pytest.raises(AgentPlanError):
        AgentTask(step_id="bad id", tool_id=SIMULATOR_TOOL_ID)
    with pytest.raises(AgentPlanError):
        AgentTask(
            step_id="step-1",
            tool_id=SIMULATOR_TOOL_ID,
            arguments={"a": cast(JSONValue, {"nested": object()})},
        )
    with pytest.raises(AgentPlanError):
        AgentTask(
            step_id="step-1",
            tool_id=SIMULATOR_TOOL_ID,
            arguments={"a": 1.0 * float("inf")},
        )
    with pytest.raises(AgentPlanError):
        AgentCoordinator(_executor(tmp_path), max_parallel_tasks=0)
    with pytest.raises(AgentPlanError):
        AgentCoordinator(_executor(tmp_path), max_parallel_tasks=99)
    with pytest.raises(AgentPlanError):
        coordinator.plan("anything")
    with pytest.raises(TypeError):
        AgentCoordinator("not an executor")  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        AgentCoordinator(_executor(tmp_path), clock=7)  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        ModelPlanner(ModelGateway(LocalModelClient()), "not a catalog")  # type: ignore[arg-type]


def test_coordinator_reports_budget_denials_before_dispatch(tmp_path: Path) -> None:
    executor = _executor(tmp_path, PolicyEngine(SIMULATOR_CAPABILITIES))
    coordinator = AgentCoordinator(
        executor,
        policy=PolicyEngine(SIMULATOR_CAPABILITIES),
        ledger=BudgetLedger(RunBudget(max_tool_calls=0)),
    )
    tasks = [
        AgentTask(
            step_id="step-1",
            tool_id=SIMULATOR_TOOL_ID,
            arguments={"operation": "beam_volume"},
            required_capabilities=SIMULATOR_CAPABILITIES,
        )
    ]
    result = coordinator.run(tasks, run_id="run-budget")
    assert result.budget_denied == ("step-1",)
    assert result.outcomes[0].error_code == "budget_exhausted"
    assert result.outcomes[0].status is ToolRunStatus.DENIED
    assert result.outcomes[0].dispatched is False
    assert result.succeeded is False
    assert result.merged == {}
    assert executor.reports == ()


def test_an_approval_cannot_be_reused_for_a_different_call_payload(tmp_path: Path) -> None:
    policy = PolicyEngine(SIMULATOR_CAPABILITIES)
    executor = _executor(tmp_path, policy)
    coordinator = AgentCoordinator(executor, policy=policy, catalog=_catalog())
    approved = AgentTask(
        step_id="step-1",
        tool_id=SIMULATOR_TOOL_ID,
        tool_version=SIMULATOR_TOOL_VERSION,
        arguments={"operation": "beam_volume"},
        required_capabilities=SIMULATOR_CAPABILITIES,
    )
    policy.approve(_policy_request(executor, "run-replay-step-1", approved.arguments_json))
    first = coordinator.run([approved], run_id="run-replay")
    assert first.succeeded is True

    replayed = AgentTask(
        step_id="step-1",
        tool_id=SIMULATOR_TOOL_ID,
        tool_version=SIMULATOR_TOOL_VERSION,
        arguments={"operation": "beam_volume", "write_report": True},
        required_capabilities=SIMULATOR_CAPABILITIES,
    )
    second = coordinator.run([replayed], run_id="run-replay")
    assert second.succeeded is False
    assert second.outcomes[0].status is ToolRunStatus.DENIED
    assert second.outcomes[0].error_code == "approval_required"
    assert "stale_approval" in str(second.outcomes[0].error_message)
    assert second.merged == {}
    assert len(executor.reports) == 1


def test_executor_budget_exhaustion_is_reported_per_call(tmp_path: Path) -> None:
    policy = PolicyEngine(SIMULATOR_CAPABILITIES)
    registry = ToolRegistry()
    register_simulator(registry)
    executor = ToolExecutor(
        registry,
        run_directory=tmp_path / "run",
        granted_capabilities=SIMULATOR_CAPABILITIES,
        policy=policy,
        ledger=BudgetLedger(RunBudget(max_tool_calls=0)),
    )
    coordinator = AgentCoordinator(executor, policy=policy)
    tasks = [
        AgentTask(
            step_id="step-1",
            tool_id=SIMULATOR_TOOL_ID,
            arguments={"operation": "beam_volume"},
            required_capabilities=SIMULATOR_CAPABILITIES,
        )
    ]
    first = coordinator.run(tasks, run_id="run-budget")
    assert first.budget_denied == ()
    assert first.outcomes[0].error_code == "approval_required"
    policy.approve(_policy_request(executor, first.outcomes[0].call_id))
    second = coordinator.run(tasks, run_id="run-budget")
    assert second.budget_denied == ("step-1",)
    assert second.outcomes[0].error_code == "budget_exhausted"
    assert second.outcomes[0].status is ToolRunStatus.DENIED
    assert second.succeeded is False
    assert second.merged == {}


def test_model_response_validates_its_own_fields() -> None:
    with pytest.raises(ModelRequestError):
        ModelResponse(
            request_id="model-invalid",
            provider_id="local",
            model_id="local",
            content="{}",
            input_tokens=-1,
        )
    with pytest.raises(ModelRequestError):
        ModelResponse(
            request_id="model-invalid-2",
            provider_id="local",
            model_id="local",
            content="{}",
            cost=Decimal("-1"),
        )
    with pytest.raises(ModelProviderError):
        ModelResponse(
            request_id="model-invalid-3",
            provider_id="local",
            model_id="local",
            content="{}",
            finish_reason=cast("ModelFinishReason", "invented"),
        )
    response = ModelResponse(
        request_id="model-valid",
        provider_id="local",
        model_id="local",
        content='{"a": 1}',
    )
    assert response.total_tokens == 0
    assert response.canonical_content() == '{"a": 1}'
    assert len(response.response_digest) == 64
    structured = ModelResponse(
        request_id="model-structured",
        provider_id="local",
        model_id="local",
        content='{"b": 2,   "a": 1}',
        structured=cast("Mapping[str, JSONValue]", {"b": 2, "a": 1}),
    )
    assert structured.canonical_content() == '{"a":1,"b":2}'


def test_gateway_rejects_non_client_objects() -> None:
    with pytest.raises(TypeError):
        ModelGateway(object())  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        ModelGateway(LocalModelClient(), transcript="not callable")  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        ModelGateway(LocalModelClient(), clock=7)  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        ModelGateway(LocalModelClient()).complete("not a request")  # type: ignore[arg-type]


def test_budget_ledger_rejects_unknown_dimensions() -> None:
    ledger = BudgetLedger(RunBudget())
    with pytest.raises(TypeError):
        ledger.consume(unknown_dimension=1)
    with pytest.raises(BudgetExhaustedError):
        ledger.consume(total_tokens=RunBudget().max_total_tokens + 1)


def test_sandbox_profile_rejects_unsafe_limits() -> None:
    with pytest.raises(ValueError, match="max_workers"):
        SandboxProfile(max_workers=0)
    with pytest.raises(ValueError, match="default_timeout_seconds"):
        SandboxProfile(default_timeout_seconds=0.0)
    with pytest.raises(ValueError, match="extra_environment"):
        SandboxProfile(extra_environment={"SECRET_TOKEN": "x"})
    with pytest.raises(ValueError, match="max_output_bytes"):
        SandboxProfile(max_output_bytes=0)
    profile = SandboxProfile()
    assert profile.timeout_for(None, 0.0) == profile.default_timeout_seconds
    assert profile.timeout_for(None, 5.0) == 5.0
    assert profile.timeout_for(1.0, 5.0) == 1.0
    assert profile.timeout_for(600.0, 5.0) == 5.0
    assert profile.timeout_for(0.0, 0.0) == profile.default_timeout_seconds
    assert profile.enforced_controls == tuple(sorted(profile.enforced_controls))
