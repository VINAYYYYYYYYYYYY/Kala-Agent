from __future__ import annotations

import json
import socket
from collections.abc import Iterator, Mapping
from datetime import UTC, datetime
from typing import cast

import httpx
import pytest

from kala_agent.contracts import ArtifactClassification
from kala_agent.kernel.budget import BudgetDimension, BudgetExhaustedError
from kala_agent.kernel.policy import PolicyDecision, PolicyEngine, PolicyError, PolicyRequest
from kala_agent.runtime.agent import (
    NEVER_DEGRADED_ERRORS,
    AgentPolicyError,
    AgentTask,
    CatalogStep,
    DegradingPlanner,
    OfflinePlanner,
    PlannerFailure,
    StepCatalog,
)
from kala_agent.runtime.model_gateway import (
    EGRESS_CAPABILITIES,
    MAX_TRANSCRIPT_BODY_BYTES,
    EgressDeniedError,
    LocalModelClient,
    ModelArtifact,
    ModelCapabilityError,
    ModelEgressManifest,
    ModelGateway,
    ModelMessage,
    ModelRequest,
    ModelRequestError,
    ModelResponse,
    ModelRole,
    OpenAICompatibleClient,
    ProviderDegradedError,
    normalize_endpoint,
    redact_secrets,
    truncate_encoded,
)
from kala_agent.tools import SIMULATOR_CAPABILITIES, SIMULATOR_TOOL_ID, JSONValue

RECORDED_AT = datetime(2026, 5, 6, 7, 8, 9, tzinfo=UTC)
LOOPBACK_ENDPOINT = "http://127.0.0.1:11434/v1/chat/completions"
HOSTED_ENDPOINT = "https://api.example.com/v1/chat/completions"
AWS_SECRET = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("tests must not open network connections")

    monkeypatch.setattr(socket.socket, "connect", blocked)
    monkeypatch.setattr(socket.socket, "connect_ex", blocked)
    monkeypatch.setattr(socket, "create_connection", blocked)
    yield


def _echo_handler(request: httpx.Request) -> httpx.Response:
    return httpx.Response(
        200,
        json={"choices": [{"finish_reason": "stop", "message": {"content": "{}"}}]},
    )


def _request(
    request_id: str = "model-1",
    *,
    content: str = "summarize the run",
) -> ModelRequest:
    return ModelRequest(
        request_id=request_id,
        messages=(ModelMessage(role=ModelRole.USER, content=content),),
    )


def _remote_client(endpoint: str = LOOPBACK_ENDPOINT) -> OpenAICompatibleClient:
    return OpenAICompatibleClient(
        endpoint=endpoint,
        model_id="local-model",
        allowed_endpoints=[endpoint],
        consent=True,
        transport=httpx.MockTransport(_echo_handler),
    )


def _egress_policy() -> PolicyEngine:
    return PolicyEngine(list(EGRESS_CAPABILITIES))


def _artifact(
    classification: ArtifactClassification,
    *,
    artifact_id: str = "artifact-1",
    preview_text: str | None = None,
    metadata: Mapping[str, JSONValue] | None = None,
) -> ModelArtifact:
    return ModelArtifact(
        artifact_id=artifact_id,
        digest=("a" * 64),
        classification=classification,
        media_type="application/step",
        size_bytes=4096,
        preview_text=preview_text,
        metadata=metadata or {},
    )


def test_egress_without_a_policy_engine_fails_closed() -> None:
    calls: list[str] = []

    class _Remote(LocalModelClient):
        @property
        def requires_egress(self) -> bool:
            return True

        def complete(self, request: ModelRequest) -> ModelResponse:
            calls.append(request.request_id)
            return ModelResponse(
                request_id=request.request_id,
                provider_id=self.provider_id,
                model_id=self.model_id,
                content="{}",
            )

    manifests: list[ModelEgressManifest] = []
    gateway = ModelGateway(_Remote(), egress_manifest=manifests.append)
    assert gateway.requires_egress is True
    with pytest.raises(EgressDeniedError, match="no policy engine is attached"):
        gateway.complete(_request("model-missing-policy"))
    assert calls == []
    assert gateway.transcripts == ()
    assert manifests == []
    assert gateway.manifests == ()


def test_constructor_boolean_cannot_bypass_egress() -> None:
    client = _remote_client()
    policy = _egress_policy()
    with pytest.raises(TypeError):
        ModelGateway(client, egress_approved=True)  # type: ignore[call-arg]
    with pytest.raises(TypeError):
        ModelGateway(client, egress_approved=True, policy=policy)  # type: ignore[call-arg]
    gateway = ModelGateway(client, policy=policy, egress_manifest=lambda manifest: None)
    with pytest.raises(EgressDeniedError):
        gateway.complete(_request("model-no-boolean-bypass"))
    with pytest.raises(TypeError):
        ModelGateway(client, policy=cast("PolicyEngine", object()))
    with pytest.raises(TypeError):
        ModelGateway(client, egress_manifest="not callable")  # type: ignore[arg-type]


def test_egress_requires_a_per_request_approval() -> None:
    policy = _egress_policy()
    manifests: list[ModelEgressManifest] = []
    gateway = ModelGateway(
        _remote_client(),
        policy=policy,
        egress_manifest=manifests.append,
        clock=lambda: RECORDED_AT,
    )
    approved = _request("model-approved")
    other = _request("model-other")

    with pytest.raises(EgressDeniedError, match="approval_required"):
        gateway.complete(approved)
    policy.approve(gateway.egress_policy_request(other))
    with pytest.raises(EgressDeniedError, match="approval_required"):
        gateway.complete(approved)
    assert not manifests

    policy.approve(gateway.egress_policy_request(approved))
    assert gateway.complete(approved).provider_id == "openai-compatible"
    assert len(manifests) == 1
    assert manifests[0].recorded_at == RECORDED_AT
    assert gateway.complete(approved).provider_id == "openai-compatible"
    assert len(manifests) == 2


def test_stale_egress_approval_is_refused() -> None:
    policy = _egress_policy()
    gateway = ModelGateway(
        _remote_client(),
        policy=policy,
        egress_manifest=lambda manifest: None,
    )
    policy.approve(gateway.egress_policy_request(_request("model-stale", content="approved text")))
    with pytest.raises(EgressDeniedError, match="stale_approval"):
        gateway.complete(_request("model-stale", content="mutated text"))
    with pytest.raises(EgressDeniedError, match="stale_approval"):
        gateway.complete(
            _request(
                "model-stale",
                content="approved text",
            )
        )
    policy.approve(gateway.egress_policy_request(_request("model-stale", content="mutated text")))
    assert (
        gateway.complete(_request("model-stale", content="mutated text")).model_id == "local-model"
    )


def test_egress_action_digest_pins_every_egress_facet() -> None:
    policy = _egress_policy()
    gateway = ModelGateway(
        _remote_client(),
        policy=policy,
        egress_manifest=lambda manifest: None,
    )
    request = _request("model-digest")
    baseline = gateway.egress_action_digest(request)
    state = gateway.egress_state_digest(request)
    assert len(baseline) == 64
    assert len(state) == 64
    assert baseline != state
    assert gateway.egress_action_digest(_request("model-other")) != baseline
    assert gateway.egress_action_digest(_request("model-digest", content="other text")) != baseline
    assert (
        gateway.egress_action_digest(
            _request("model-digest", content="use AWS_SECRET_ACCESS_KEY=abcdefghijklmnop")
        )
        != baseline
    )

    other_destination = ModelGateway(
        _remote_client(HOSTED_ENDPOINT),
        policy=policy,
        egress_manifest=lambda manifest: None,
    )
    assert other_destination.destination == "https://api.example.com:443/v1/chat/completions"
    assert other_destination.egress_action_digest(request) != baseline

    other_model = ModelGateway(
        OpenAICompatibleClient(
            endpoint=LOOPBACK_ENDPOINT,
            model_id="other-model",
            allowed_endpoints=[LOOPBACK_ENDPOINT],
            consent=True,
            transport=httpx.MockTransport(_echo_handler),
        ),
        policy=policy,
        egress_manifest=lambda manifest: None,
    )
    assert other_model.egress_action_digest(request) != baseline

    policy_request = gateway.egress_policy_request(request)
    assert policy_request.request_id == "egress-model-digest"
    assert policy_request.action_digest == baseline
    assert policy_request.state_digest == state
    assert policy_request.requested_capabilities == EGRESS_CAPABILITIES
    assert policy_request.egress is True
    assert policy_request.destructive is False
    assert policy_request.requires_approval is True
    with pytest.raises(TypeError):
        gateway.egress_policy_request("not a request")  # type: ignore[arg-type]


def test_egress_manifest_is_recorded_before_dispatch() -> None:
    order: list[str] = []
    manifests: list[ModelEgressManifest] = []

    def _record(manifest: ModelEgressManifest) -> None:
        manifests.append(manifest)
        order.append("manifest")

    def _handler(request: httpx.Request) -> httpx.Response:
        order.append("dispatch")
        return _echo_handler(request)

    client = OpenAICompatibleClient(
        endpoint=LOOPBACK_ENDPOINT,
        model_id="local-model",
        allowed_endpoints=[LOOPBACK_ENDPOINT],
        consent=True,
        transport=httpx.MockTransport(_handler),
    )
    policy = _egress_policy()
    gateway = ModelGateway(client, policy=policy, egress_manifest=_record)
    request = _request("model-manifest")
    policy.approve(gateway.egress_policy_request(request))
    gateway.complete(request)
    assert order == ["manifest", "dispatch"]
    manifest = manifests[0]
    assert manifest.destination == LOOPBACK_ENDPOINT
    assert manifest.request_digest == request.request_digest
    assert manifest.egress_request_id == "egress-model-manifest"
    assert manifest.capabilities == EGRESS_CAPABILITIES
    assert manifest.provider_id == "openai-compatible"
    assert manifest.model_id == "local-model"
    payload = manifest.to_payload()
    assert payload["destination"] == LOOPBACK_ENDPOINT
    assert payload["request_digest"] == request.request_digest


def test_missing_or_failing_egress_manifest_denies_the_call() -> None:
    dispatched: list[str] = []

    def _handler(request: httpx.Request) -> httpx.Response:
        dispatched.append(request.url.host)
        return _echo_handler(request)

    client = OpenAICompatibleClient(
        endpoint=LOOPBACK_ENDPOINT,
        model_id="local-model",
        allowed_endpoints=[LOOPBACK_ENDPOINT],
        consent=True,
        transport=httpx.MockTransport(_handler),
    )
    without_manifest_policy = _egress_policy()
    without_manifest = ModelGateway(client, policy=without_manifest_policy)
    request = _request("model-no-manifest")
    with pytest.raises(EgressDeniedError, match="approval_required"):
        without_manifest.complete(request)
    assert dispatched == []

    approved = _request("model-manifest-approved")
    without_manifest_policy.approve(without_manifest.egress_policy_request(approved))
    with pytest.raises(EgressDeniedError, match="no egress manifest is attached"):
        without_manifest.complete(approved)
    assert dispatched == []

    def _unavailable(manifest: ModelEgressManifest) -> None:
        raise OSError("audit journal is unavailable")

    policy = _egress_policy()
    failing = ModelGateway(client, policy=policy, egress_manifest=_unavailable)
    failing_request = _request("model-failed-manifest")
    policy.approve(failing.egress_policy_request(failing_request))
    with pytest.raises(EgressDeniedError, match="egress manifest could not be recorded"):
        failing.complete(failing_request)
    assert dispatched == []
    assert failing.transcripts == ()


def test_transcript_truncation_is_byte_safe() -> None:
    unit = "κόσμε-πληροφορία-"
    request_text = unit * 1_000
    response_text = unit * 3_800
    assert len(response_text) < MAX_TRANSCRIPT_BODY_BYTES
    assert len(response_text.encode("utf-8")) > MAX_TRANSCRIPT_BODY_BYTES

    class _Chatty(LocalModelClient):
        def complete(self, request: ModelRequest) -> ModelResponse:
            return ModelResponse(
                request_id=request.request_id,
                provider_id=self.provider_id,
                model_id=self.model_id,
                content=response_text,
            )

    gateway = ModelGateway(_Chatty(), clock=lambda: RECORDED_AT)
    request = ModelRequest(
        request_id="model-bytes",
        messages=(ModelMessage(role=ModelRole.USER, content=request_text),),
    )
    assert len(request.request_body.encode("utf-8")) > MAX_TRANSCRIPT_BODY_BYTES
    response = gateway.complete(request)
    transcript = gateway.transcripts[0]
    assert len(transcript.request_body.encode("utf-8")) == MAX_TRANSCRIPT_BODY_BYTES
    assert len(transcript.response_body.encode("utf-8")) <= MAX_TRANSCRIPT_BODY_BYTES
    assert len(transcript.response_body) < len(response_text)
    assert transcript.request_body.encode("utf-8").decode("utf-8") == transcript.request_body
    assert transcript.response_body.encode("utf-8").decode("utf-8") == transcript.response_body
    assert {redaction.kind for redaction in transcript.redactions} == {"truncated"}
    assert {redaction.subject for redaction in transcript.redactions} == {"request", "response"}
    assert transcript.request_digest == request.request_digest
    assert response.canonical_content() == response_text
    assert truncate_encoded("κόσμε" * 10, 0) == ""
    assert truncate_encoded("short", 64) == "short"
    assert len(truncate_encoded("😀" * 100, 10).encode("utf-8")) <= 10


def test_artifact_preview_and_metadata_text_is_redacted() -> None:
    seen: list[ModelRequest] = []

    class _Recording(LocalModelClient):
        def complete(self, request: ModelRequest) -> ModelResponse:
            seen.append(request)
            return ModelResponse(
                request_id=request.request_id,
                provider_id=self.provider_id,
                model_id=self.model_id,
                content="{}",
            )

    gateway = ModelGateway(_Recording(), clock=lambda: RECORDED_AT)
    gateway.complete(
        _request("model-artifacts").with_artifacts(
            (
                _artifact(
                    ArtifactClassification.DESIGN_BINARY,
                    artifact_id="artifact-binary",
                    preview_text="SECRET-GEOMETRY",
                ),
                _artifact(
                    ArtifactClassification.TEXT,
                    artifact_id="artifact-text",
                    preview_text=f"preview with AWS_SECRET_ACCESS_KEY={AWS_SECRET}",
                    metadata={
                        "source": "inbox",
                        "nested": {"api_key": "abcdefghijklmnop"},
                        "tags": ["safe", "Bearer abcdefghijklmnop"],
                        "size": 12,
                    },
                ),
                _artifact(
                    ArtifactClassification.METADATA,
                    artifact_id="artifact-metadata",
                    preview_text="no credentials here",
                ),
            )
        )
    )
    sent = seen[0]
    binary, text, metadata_only = sent.artifacts
    assert binary.preview_text is None
    assert binary.path is None
    assert dict(binary.metadata) == {}
    assert text.preview_text == "preview with <redacted>"
    assert AWS_SECRET not in (text.preview_text or "")
    assert dict(text.metadata)["source"] == "inbox"
    nested = cast("dict[str, str]", text.metadata["nested"])
    assert nested["api_key"] == "<redacted>"
    tags = list(cast("list[str]", text.metadata["tags"]))
    assert tags[0] == "safe"
    assert tags[1] == "<redacted>"
    assert text.metadata["size"] == 12
    assert metadata_only.preview_text == "no credentials here"
    assert AWS_SECRET not in sent.request_body
    assert "abcdefghijklmnop" not in sent.request_body
    redactions = gateway.transcripts[0].redactions
    subjects = {redaction.subject for redaction in redactions}
    assert "artifact-text.preview_text" in subjects
    assert "artifact-text.metadata" in subjects
    assert "artifact-binary" in subjects
    assert {redaction.kind for redaction in redactions} == {"secret", "design_binary"}
    assert "artifact-metadata.preview_text" not in subjects


def test_aws_style_secrets_are_redacted() -> None:
    cases = (
        f"AWS_SECRET_ACCESS_KEY={AWS_SECRET}",
        f"aws-secret-access-key: '{AWS_SECRET}'",
        json.dumps({"AWS_SECRET_ACCESS_KEY": AWS_SECRET, "region": "us-east-1"}),
        "export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE",
        "AWS_SESSION_TOKEN=FwoGZXIvYXdzEBYaDF",
        "ghp_abcdefghijklmnopqrstuvwxyz0123",
        "client_secret=abcdefghijklmnop",
        "private_key=-----BEGIN RSA PRIVATE KEY-----",
    )
    for candidate in cases:
        redacted, findings = redact_secrets(candidate)
        assert findings >= 1, candidate
        assert "<redacted>" in redacted, candidate
        assert AWS_SECRET not in redacted
        assert "AKIAIOSFODNN7EXAMPLE" not in redacted
        assert "abcdefghijklmnop" not in redacted
        assert "FwoGZXIvYXdzEBYaDF" not in redacted
        assert "ghp_abcdefghijklmnopqrstuvwxyz0123" not in redacted
        assert "BEGIN RSA PRIVATE KEY" not in redacted
    structured = redact_secrets(json.dumps({"AWS_SECRET_ACCESS_KEY": AWS_SECRET}))[0]
    assert json.loads(structured) == {"AWS_SECRET_ACCESS_KEY": "<redacted>"}
    assert redact_secrets("nothing sensitive")[1] == 0
    assert redact_secrets(AWS_SECRET)[1] == 0


def test_hosted_https_endpoints_require_allowlist_and_consent() -> None:
    assert normalize_endpoint(LOOPBACK_ENDPOINT) == LOOPBACK_ENDPOINT
    assert normalize_endpoint("https://127.0.0.1:8443/v1") == "https://127.0.0.1:8443/v1"
    assert (
        normalize_endpoint(
            HOSTED_ENDPOINT,
            allowed_endpoints=[HOSTED_ENDPOINT],
            consent=True,
        )
        == "https://api.example.com:443/v1/chat/completions"
    )
    for kwargs in (
        {},
        {"consent": False},
        {"consent": True},
        {"allowed_endpoints": [HOSTED_ENDPOINT]},
        {"allowed_endpoints": ["https://api.other.example.com/v1/chat/completions"]},
    ):
        with pytest.raises(EgressDeniedError):
            normalize_endpoint(HOSTED_ENDPOINT, **kwargs)
    for candidate in (
        "http://api.example.com/v1/chat/completions",
        "https://user:secret@api.example.com/v1",
        "https://api.example.com/v1?key=value",
        "ftp://127.0.0.1/v1",
        "",
    ):
        with pytest.raises(EgressDeniedError):
            normalize_endpoint(
                candidate,
                allowed_endpoints=[candidate],
                consent=True,
            )
    with pytest.raises(EgressDeniedError, match="explicit egress consent"):
        OpenAICompatibleClient(
            endpoint=HOSTED_ENDPOINT,
            model_id="hosted-model",
            allowed_endpoints=[HOSTED_ENDPOINT],
            transport=httpx.MockTransport(_echo_handler),
        )
    with pytest.raises(EgressDeniedError, match="egress allowlist"):
        OpenAICompatibleClient(
            endpoint=HOSTED_ENDPOINT,
            model_id="hosted-model",
            allowed_endpoints=["https://api.other.example.com/v1/chat/completions"],
            consent=True,
            transport=httpx.MockTransport(_echo_handler),
        )
    client = _remote_client(HOSTED_ENDPOINT)
    assert client.endpoint == "https://api.example.com:443/v1/chat/completions"
    assert client.requires_egress is True
    assert client.complete(_request("model-hosted")).provider_id == "openai-compatible"


def test_policy_failures_propagate_without_degradation() -> None:
    seen: list[PolicyRequest] = []

    class _BrokenPolicy(PolicyEngine):
        def decide(
            self,
            request: PolicyRequest,
            *,
            now: datetime | None = None,
        ) -> PolicyDecision:
            seen.append(request)
            raise PolicyError("the policy backend is unreachable")

    for request_id in ("model-policy-error", "model-policy-error-second"):
        gateway = ModelGateway(
            _remote_client(),
            policy=_BrokenPolicy(),
            egress_manifest=lambda manifest: None,
        )
        with pytest.raises(PolicyError, match="policy backend is unreachable"):
            gateway.complete(_request(request_id))
        assert gateway.transcripts == ()
        assert gateway.manifests == ()
    assert [request.request_id for request in seen] == [
        "egress-model-policy-error",
        "egress-model-policy-error-second",
    ]
    assert all(request.egress is True for request in seen)

    denied = ModelGateway(
        _remote_client(),
        policy=PolicyEngine(),
        egress_manifest=lambda manifest: None,
    )
    with pytest.raises(EgressDeniedError, match="capability_not_granted"):
        denied.complete(_request("model-policy-denied"))
    assert denied.transcripts == ()
    assert denied.manifests == ()


def _catalog() -> StepCatalog:
    return StepCatalog(
        [
            CatalogStep(
                catalog_step_id="beam",
                tool_id=SIMULATOR_TOOL_ID,
                default_arguments={"operation": "beam_volume"},
                required_capabilities=SIMULATOR_CAPABILITIES,
                keywords=("beam",),
                intent="evaluate the reference beam",
            )
        ]
    )


def _offline_planner() -> OfflinePlanner:
    return OfflinePlanner(_catalog(), fallback_step_id="beam")


def test_degrading_planner_never_downgrades_denials() -> None:
    offline = _offline_planner()

    class _Denying:
        def __init__(self, error: BaseException) -> None:
            self._error = error

        def plan(self, objective: str) -> tuple[AgentTask, ...]:
            raise self._error

    denials = (
        AgentPolicyError("tool needs a policy decision and no policy engine is attached"),
        EgressDeniedError("provider egress is denied"),
        BudgetExhaustedError(BudgetDimension.TOTAL_TOKENS, 10, 0, 10),
        ModelCapabilityError("provider local does not declare structured_output"),
    )
    assert set(NEVER_DEGRADED_ERRORS) == {
        AgentPolicyError,
        BudgetExhaustedError,
        EgressDeniedError,
        ModelCapabilityError,
    }
    for error in denials:
        for allow_degraded in (False, True):
            planner = DegradingPlanner(_Denying(error), offline, allow_degraded=allow_degraded)
            with pytest.raises(type(error)):
                planner.plan("evaluate the beam volume")
            assert planner.degradations == ()

    class _Failing:
        def plan(self, objective: str) -> tuple[AgentTask, ...]:
            raise PlannerFailure("no catalog step matches the objective")

    degradable = DegradingPlanner(_Failing(), offline, allow_degraded=True)
    assert [task.step_id for task in degradable.plan("evaluate the beam volume")] == ["step-1"]
    assert len(degradable.degradations) == 1
    with pytest.raises(ProviderDegradedError):
        DegradingPlanner(_Failing(), offline).plan("evaluate the beam volume")


def test_model_requests_reject_oversized_content() -> None:
    with pytest.raises(ModelRequestError):
        _request("model-oversized", content="x" * 40_000)
