from __future__ import annotations

import math
from collections.abc import Mapping, Sequence
from datetime import UTC, datetime
from decimal import Decimal
from typing import Any, cast

import pytest

from kala_agent.contracts import ArtifactClassification
from kala_agent.kernel.policy import PolicyEngine
from kala_agent.runtime import model_gateway as gateway_module
from kala_agent.runtime.model_gateway import (
    MAX_ENDPOINT_LENGTH,
    MAX_OUTPUT_TOKENS,
    MAX_RESPONSE_BYTES,
    MAX_SEED,
    MAX_TRANSCRIPT_BODY_BYTES,
    EgressDeniedError,
    LocalModelClient,
    ModelArtifact,
    ModelCapability,
    ModelCapabilityError,
    ModelFinishReason,
    ModelGateway,
    ModelMessage,
    ModelProviderError,
    ModelRedaction,
    ModelRequest,
    ModelRequestError,
    ModelResponse,
    ModelRole,
    ModelTranscript,
    OpenAICompatibleClient,
    ProviderDegradedError,
    ProviderSubstitutionError,
    ProviderUnavailableError,
    StructuredOutputError,
    TranscriptError,
    _bounded_float,
    _bounded_identifier,
    _bounded_text,
    _freeze_json,
    _nonnegative_decimal,
    _nonnegative_int,
    _normalized_destination,
    _parse_destination,
    _provider_int,
    _redact_metadata,
    _schema_properties,
    _schema_type_names,
    _synthesize_value,
    _thaw_json,
    estimate_tokens,
    is_loopback_host,
    is_secret_name,
    normalize_endpoint,
    redact_secrets,
    truncate_encoded,
)
from kala_agent.tools.protocol import JSONValue, canonical_json_dumps, sha256_hex

RECORDED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)
DIGEST = sha256_hex(b"payload")


def _message(role: ModelRole = ModelRole.USER, content: str = "hello") -> ModelMessage:
    return ModelMessage(role=role, content=content)


def _artifact(**overrides: object) -> ModelArtifact:
    values: dict[str, Any] = {
        "artifact_id": "artifact-1",
        "digest": DIGEST,
        "classification": ArtifactClassification.DERIVED_DATA,
        "media_type": "application/json",
        "size_bytes": 7,
    }
    values.update(cast("Mapping[str, Any]", overrides))
    return ModelArtifact(**values)


def _request(**overrides: object) -> ModelRequest:
    values: dict[str, Any] = {
        "request_id": "request-1",
        "messages": (_message(),),
    }
    values.update(cast("Mapping[str, Any]", overrides))
    return ModelRequest(**values)


def _response(**overrides: object) -> ModelResponse:
    values: dict[str, Any] = {
        "request_id": "request-1",
        "provider_id": "local",
        "model_id": "local-deterministic",
        "content": '{"ok":true}',
    }
    values.update(cast("Mapping[str, Any]", overrides))
    return ModelResponse(**values)


def _transcript(**overrides: object) -> ModelTranscript:
    values: dict[str, Any] = {
        "request_id": "request-1",
        "provider_id": "local",
        "model_id": "local-deterministic",
        "request_digest": DIGEST,
        "response_digest": DIGEST,
        "request_body": "{}",
        "response_body": "{}",
        "input_tokens": 1,
        "output_tokens": 1,
        "cost": Decimal("0"),
        "redactions": (),
        "artifact_digests": (),
        "recorded_at": RECORDED_AT,
    }
    values.update(cast("Mapping[str, Any]", overrides))
    return ModelTranscript(**values)


def test_bounded_identifier_rejects_bad_values() -> None:
    with pytest.raises(ModelRequestError, match="must be a string"):
        _bounded_identifier(7, "request_id")
    with pytest.raises(ModelRequestError, match="safe identifier"):
        _bounded_identifier("has space", "request_id")
    assert _bounded_identifier("  request-1  ", "request_id") == "request-1"


def test_bounded_text_rejects_null_bytes_and_length() -> None:
    with pytest.raises(ModelRequestError, match="must be a string"):
        _bounded_text(7, "content")
    with pytest.raises(ModelRequestError, match="null bytes"):
        _bounded_text("a\x00b", "content")
    with pytest.raises(ModelRequestError, match="bounded length"):
        _bounded_text("x" * 100, "content", maximum=10)
    assert _bounded_text("ok", "content") == "ok"


def test_nonnegative_int_rejects_bad_values() -> None:
    with pytest.raises(ModelRequestError, match="must be an integer"):
        _nonnegative_int(True, "seed")
    with pytest.raises(ModelRequestError, match="must be an integer"):
        _nonnegative_int("7", "seed")
    with pytest.raises(ModelRequestError, match="must be nonnegative"):
        _nonnegative_int(-1, "seed")
    assert _nonnegative_int(0, "seed") == 0


def test_bounded_float_rejects_out_of_range_values() -> None:
    with pytest.raises(ModelRequestError, match="must be a number"):
        _bounded_float(True, "timeout_seconds", minimum=0.1, maximum=10.0)
    with pytest.raises(ModelRequestError, match="within its bounds"):
        _bounded_float(0.0, "timeout_seconds", minimum=0.1, maximum=10.0)
    with pytest.raises(ModelRequestError, match="within its bounds"):
        _bounded_float(11.0, "timeout_seconds", minimum=0.1, maximum=10.0)
    with pytest.raises(ModelRequestError, match="within its bounds"):
        _bounded_float(
            float("inf"),
            "timeout_seconds",
            minimum=0.1,
            maximum=10.0,
        )
    assert _bounded_float(1.0, "timeout_seconds", minimum=0.1, maximum=10.0) == 1.0


def test_nonnegative_decimal_rejects_bad_values() -> None:
    with pytest.raises(ModelRequestError, match="must be numeric"):
        _nonnegative_decimal(True, "cost")
    with pytest.raises(ModelRequestError, match="nonnegative decimal"):
        _nonnegative_decimal("not-a-number", "cost")
    with pytest.raises(ModelRequestError, match="nonnegative decimal"):
        _nonnegative_decimal(Decimal("-1"), "cost")
    with pytest.raises(ModelRequestError, match="nonnegative decimal"):
        _nonnegative_decimal(Decimal("NaN"), "cost")
    assert _nonnegative_decimal(1, "cost") == Decimal("1")
    assert _nonnegative_decimal("1.5", "cost") == Decimal("1.5")


def test_freeze_json_validates_every_node() -> None:
    frozen = cast("Mapping[str, JSONValue]", _freeze_json({"a": [1, (2,)]}))
    items = cast("Sequence[JSONValue]", frozen["a"])
    assert list(items) == [1, (2,)]
    assert _thaw_json(frozen) == {"a": [1, [2]]}
    assert _freeze_json(None) is None
    with pytest.raises(ModelRequestError, match="nesting"):
        deep: Any = 1
        for _ in range(40):
            deep = {"a": deep}
        _freeze_json(deep)
    with pytest.raises(ModelRequestError, match="must be finite"):
        _freeze_json(float("inf"))
    with pytest.raises(ModelRequestError, match="keys must be strings"):
        _freeze_json(cast("Mapping[Any, Any]", {7: 1}))
    with pytest.raises(ModelRequestError, match="not JSON serializable"):
        _freeze_json({1, 2})


def test_thaw_json_rebuilds_plain_containers() -> None:
    assert _thaw_json(_freeze_json({"a": (1, (2,))})) == {"a": [1, [2]]}
    assert _thaw_json(7) == 7


def test_redact_secrets_removes_credential_shaped_text() -> None:
    redacted, findings = redact_secrets("token sk-abcdefghijklmnopqrstuvwxyz012345")
    assert findings >= 1
    assert "sk-abcdefghijklmnopqrstuvwxyz012345" not in redacted
    assert redact_secrets("nothing secret here") == ("nothing secret here", 0)


def test_truncate_encoded_cuts_at_the_byte_bound() -> None:
    assert truncate_encoded("hello", 10) == "hello"
    assert truncate_encoded("hello", 3) == "hel"
    assert truncate_encoded("καλός", 3) == "κ"
    assert truncate_encoded("x" * 100, 0) == ""


def test_is_secret_name_detects_credential_keys() -> None:
    assert is_secret_name("api_key") is True
    assert is_secret_name("AWS_SECRET_ACCESS_KEY") is True
    assert is_secret_name("passphrase") is True
    assert is_secret_name("credentials") is True
    assert is_secret_name("Authorization") is True
    assert is_secret_name("OPENAI_API_KEY") is True
    assert is_secret_name("db-password") is True
    assert is_secret_name("github_token") is True
    assert is_secret_name("units") is False
    assert is_secret_name("max_tokens") is False
    assert is_secret_name("secretary") is False


def test_redact_secrets_covers_prefixed_names_headers_and_key_bodies() -> None:
    key_body = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASC"
    pem = f"-----BEGIN RSA PRIVATE KEY-----\n{key_body}\n-----END RSA PRIVATE KEY-----"
    cases = {
        "OPENAI_API_KEY=abc123xyz": "abc123xyz",
        "export DB_PASSWORD='hunter22'": "hunter22",
        '{"OPENAI_API_KEY": "abc123xyz"}': "abc123xyz",
        '{"db_password": "a\\"quoted\\"secret"}': "secret",
        "Authorization: Basic dXNlcjpwYXNz": "dXNlcjpwYXNz",
        "proxy-authorization: Digest username=x": "username",
        f"key:\n{pem}\ntrailer": key_body,
        f"-----BEGIN PRIVATE KEY-----\n{key_body}": key_body,
    }
    for text, secret in cases.items():
        redacted, findings = redact_secrets(text)
        assert findings >= 1, text
        assert secret not in redacted, text
    assert redact_secrets(f"{pem}\ntrailer")[0] == "<redacted>\ntrailer"
    assert redact_secrets("max_tokens=512 token_count=3") == ("max_tokens=512 token_count=3", 0)
    once, _ = redact_secrets('{"api_key": "abc"} Authorization: Basic abc')
    assert redact_secrets(once) == (once, 0)


def test_redact_metadata_covers_every_shape() -> None:
    redacted, findings = _redact_metadata(
        cast(
            "Mapping[str, JSONValue]",
            {
                "api_key": "value",
                "note": "token sk-abcdefghijklmnopqrstuvwxyz012345",
                "nested": {"password": "value", "count": 1},
                "items": ["sk-abcdefghijklmnopqrstuvwxyz012345", {"secret_key": "v"}],
                "plain": 7,
            },
        )
    )
    assert findings >= 4
    assert redacted["api_key"] == "<redacted>"
    assert cast("Mapping[str, JSONValue]", redacted["nested"])["password"] == "<redacted>"
    assert cast("Mapping[str, JSONValue]", redacted["nested"])["count"] == 1
    assert redacted["plain"] == 7
    assert "sk-abcdefghijklmnopqrstuvwxyz012345" not in str(dict(redacted))


def test_estimate_tokens_rounds_up() -> None:
    assert estimate_tokens("") == 0
    assert estimate_tokens("a") == 1
    assert estimate_tokens("x" * 1_000) >= 1


def test_schema_helpers_default_to_objects() -> None:
    assert _schema_type_names({}) == ("object",)
    assert _schema_type_names({"type": "string"}) == ("string",)
    assert _schema_type_names({"type": ["string", "null"]}) == ("string", "null")
    assert _schema_properties({}) == {}
    assert _schema_properties({"properties": {"a": {}}}) == {"a": {}}
    assert _schema_properties({"properties": "nope"}) == {}


def test_synthesize_value_honours_const_enum_and_default() -> None:
    assert _synthesize_value({"const": 7}, depth=0) == 7
    assert _synthesize_value({"enum": ["a", "b"]}, depth=0) == "a"
    assert _synthesize_value({"default": 5}, depth=0) == 5
    assert _synthesize_value({"default": {1, 2}, "type": "integer"}, depth=0) == 0


def test_synthesize_value_follows_branches() -> None:
    for keyword in ("allOf", "anyOf", "oneOf"):
        assert _synthesize_value({keyword: [{"const": "chosen"}]}, depth=0) == "chosen"
    assert _synthesize_value({"allOf": "nope", "const": "plain"}, depth=0) == "plain"


def test_synthesize_value_builds_objects() -> None:
    synthesized = _synthesize_value(
        {
            "type": "object",
            "properties": {"a": {"type": "string"}, "b": {"type": "integer"}},
            "required": ["a", "b", "c"],
        },
        depth=0,
    )
    assert cast("Mapping[str, JSONValue]", synthesized) == {
        "a": "local-offline-deterministic",
        "b": 0,
        "c": None,
    }
    defaulted = _synthesize_value({"type": "object"}, depth=0)
    assert defaulted == {}


def test_synthesize_value_builds_arrays() -> None:
    assert list(cast("Sequence[JSONValue]", _synthesize_value({"type": "array"}, depth=0))) == []
    two = _synthesize_value(
        {"type": "array", "minItems": 2, "items": {"type": "boolean"}},
        depth=0,
    )
    assert list(cast("Sequence[JSONValue]", two)) == [False, False]
    capped = _synthesize_value(
        {"type": "array", "minItems": 99, "items": {"const": 1}},
        depth=0,
    )
    assert list(cast("Sequence[JSONValue]", capped)) == [1, 1, 1, 1]


def test_synthesize_value_builds_scalars() -> None:
    assert _synthesize_value({"type": "string"}, depth=0) == "local-offline-deterministic"
    assert _synthesize_value({"type": "integer"}, depth=0) == 0
    assert _synthesize_value({"type": "number"}, depth=0) == 0.0
    assert _synthesize_value({"type": "boolean"}, depth=0) is False
    assert _synthesize_value({"type": "null"}, depth=0) is None
    assert _synthesize_value({"type": ["string", "null"]}, depth=0) == (
        "local-offline-deterministic"
    )


def test_synthesize_value_bounds_the_nesting() -> None:
    schema: dict[str, Any] = {"type": "object"}
    current = schema
    for _ in range(20):
        nested: dict[str, Any] = {"type": "object"}
        current["properties"] = {"a": nested}
        current = nested
    with pytest.raises(ModelProviderError, match="synthesis bound"):
        _synthesize_value(schema, depth=0)


def test_model_message_validates_its_role_and_content() -> None:
    assert _message().to_payload() == {"content": "hello", "role": "user"}
    with pytest.raises(ModelRequestError, match="role is not a model message role"):
        ModelMessage(role=cast(Any, "wizard"), content="hello")
    with pytest.raises(ModelRequestError, match="content must be a string"):
        ModelMessage(role=ModelRole.USER, content=cast(Any, 7))
    with pytest.raises(ModelRequestError, match="null bytes"):
        ModelMessage(role=ModelRole.USER, content="a\x00b")


def test_model_artifact_validates_every_field() -> None:
    artifact = _artifact(preview_text="preview", path="/tmp/a.json", metadata={"a": 1})
    payload = artifact.to_payload()
    assert payload["digest"] == DIGEST
    assert payload["preview_text"] == "preview"
    assert payload["path"] == "/tmp/a.json"
    assert payload["metadata"] == {"a": 1}
    with pytest.raises(ModelRequestError, match="artifact_id"):
        ModelArtifact(
            artifact_id="bad id",
            digest=DIGEST,
            classification=ArtifactClassification.TEXT,
            media_type="text/plain",
            size_bytes=1,
        )
    with pytest.raises(ModelRequestError, match="sha256 hexadecimal digest"):
        _artifact(digest="not-a-digest")
    with pytest.raises(ModelRequestError, match="artifact classification"):
        _artifact(classification=cast(Any, "nope"))
    with pytest.raises(ModelRequestError, match="media_type"):
        _artifact(media_type=cast(Any, 7))
    with pytest.raises(ModelRequestError, match="size_bytes"):
        _artifact(size_bytes=-1)
    with pytest.raises(ModelRequestError, match="preview_text"):
        _artifact(preview_text="x" * 9_000)
    with pytest.raises(ModelRequestError, match="path"):
        _artifact(path="x" * 2_000)
    with pytest.raises(ModelRequestError, match="metadata must be a JSON object"):
        _artifact(metadata=cast(Any, [1]))


def test_model_artifact_redaction_strips_content() -> None:
    artifact = _artifact(
        preview_text="preview",
        path="/tmp/a.json",
        metadata={"a": 1},
    )
    redacted = artifact.redacted()
    assert redacted.preview_text is None
    assert redacted.path is None
    assert dict(redacted.metadata) == {}
    assert redacted.digest == artifact.digest


def test_model_artifact_reports_design_binaries() -> None:
    assert _artifact().is_design_binary is False
    assert _artifact(classification=ArtifactClassification.DESIGN_BINARY).is_design_binary is True
    payload = _artifact().to_payload()
    assert "preview_text" not in payload
    assert "path" not in payload


def test_model_request_validates_messages() -> None:
    with pytest.raises(ModelRequestError, match="messages must be a sequence"):
        _request(messages=cast(Any, 7))
    with pytest.raises(ModelRequestError, match="at least one message"):
        _request(messages=())
    with pytest.raises(ModelRequestError, match="ModelMessage values"):
        _request(messages=cast(Any, (object(),)))


def test_model_request_validates_capabilities() -> None:
    with pytest.raises(ModelRequestError, match="collection of model capabilities"):
        _request(capabilities=cast(Any, "structured_output"))
    with pytest.raises(ModelCapabilityError, match="unknown model capability"):
        _request(capabilities=("teleport",))
    assert _request(capabilities=[], seed=7).capabilities == frozenset()


def test_model_request_requires_a_schema_for_structured_output() -> None:
    with pytest.raises(ModelCapabilityError, match="requires a response schema"):
        _request(capabilities=(ModelCapability.STRUCTURED_OUTPUT,))
    with pytest.raises(ModelCapabilityError, match="requires a seed value"):
        _request(capabilities=(ModelCapability.SEED,))
    accepted = _request(
        capabilities=(ModelCapability.STRUCTURED_OUTPUT,),
        response_schema={
            "type": "object",
            "additionalProperties": False,
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
        },
    )
    assert accepted.requires_structured_output is True


def test_model_request_validates_the_response_schema() -> None:
    with pytest.raises(ModelRequestError, match="response_schema"):
        _request(response_schema=cast("Mapping[str, object]", {"type": "string"}))


def test_model_request_validates_artifacts() -> None:
    with pytest.raises(ModelRequestError, match="artifacts must be a sequence"):
        _request(artifacts=cast(Any, 7))
    with pytest.raises(ModelRequestError, match="ModelArtifact values"):
        _request(artifacts=cast(Any, (object(),)))
    with pytest.raises(ModelRequestError, match="artifact identifiers must be unique"):
        _request(artifacts=(_artifact(), _artifact()))
    assert len(_request(artifacts=(_artifact(),)).artifacts) == 1


def test_model_request_validates_seed_and_token_bounds() -> None:
    with pytest.raises(ModelRequestError, match="seed exceeds"):
        _request(seed=MAX_SEED + 1)
    with pytest.raises(ModelRequestError, match="max_output_tokens exceeds"):
        _request(max_output_tokens=MAX_OUTPUT_TOKENS + 1)
    with pytest.raises(ModelRequestError, match="timeout_seconds"):
        _request(timeout_seconds=0.0)


def test_model_request_body_and_digest_bind_every_field() -> None:
    request = _request(
        capabilities=(ModelCapability.SEED,),
        seed=7,
        artifacts=(_artifact(),),
    )
    body = request.request_body
    assert '"seed":7' in body
    assert '"capabilities":["seed"]' in body
    assert request.request_digest == sha256_hex(body)
    assert '"response_schema":null' in body
    assert len(request.with_artifacts(()).artifacts) == 0


def test_model_response_validates_every_field() -> None:
    response = _response(structured=cast("Mapping[str, JSONValue]", {"ok": True}))
    assert response.total_tokens == 0
    assert response.response_digest == sha256_hex('{"ok":true}')
    assert response.canonical_content() == '{"ok":true}'
    assert _response(content="plain").canonical_content() == "plain"
    with pytest.raises(ModelRequestError, match="request_id"):
        _response(request_id="bad id")
    with pytest.raises(ModelRequestError, match="provider_id"):
        _response(provider_id="")
    with pytest.raises(ModelProviderError, match="structured output"):
        _response(structured=cast(Any, ["nope"]))
    with pytest.raises(ModelRequestError, match="input_tokens"):
        _response(input_tokens=-1)
    with pytest.raises(ModelRequestError, match="cost"):
        _response(cost=Decimal("-1"))
    with pytest.raises(ModelProviderError, match="finish_reason"):
        _response(finish_reason="elsewhere")
    with pytest.raises(ModelRequestError, match="provider_request_id"):
        _response(provider_request_id="x" * 300)
    assert _response(finish_reason="length").finish_reason is ModelFinishReason.LENGTH
    assert _response(finish_reason=ModelFinishReason.STOP).finish_reason is (ModelFinishReason.STOP)


def test_model_redaction_payload() -> None:
    redaction = ModelRedaction(kind="secret", subject="user", reason="removed")
    assert redaction.to_payload() == {
        "kind": "secret",
        "reason": "removed",
        "subject": "user",
    }


def test_model_transcript_validates_its_bodies() -> None:
    transcript = _transcript(
        redactions=(ModelRedaction(kind="secret", subject="user", reason="removed"),),
        artifact_digests=(DIGEST,),
    )
    payload = transcript.to_payload()
    first = cast("Mapping[str, object]", cast("Sequence[object]", payload["redactions"])[0])
    assert first["kind"] == "secret"
    assert payload["artifact_digests"] == [DIGEST]
    assert payload["cost"] == "0"
    with pytest.raises(ModelRequestError, match="recorded_at must include a timezone"):
        _transcript(recorded_at=datetime(2026, 1, 1))
    with pytest.raises(TranscriptError, match="request body exceeds"):
        _transcript(request_body="x" * (MAX_TRANSCRIPT_BODY_BYTES + 1))
    with pytest.raises(TranscriptError, match="response body exceeds"):
        _transcript(response_body="x" * (MAX_TRANSCRIPT_BODY_BYTES + 1))


def test_parse_destination_accepts_https_endpoints() -> None:
    destination = _parse_destination("https://API.Example.com:8443/v1/../v2")
    assert destination.scheme == "https"
    assert destination.host == "api.example.com"
    assert destination.port == 8443
    assert destination.path == "/v2"
    assert destination.is_tls is True
    assert destination.is_loopback is False
    assert _parse_destination("http://localhost").port == 80
    assert _parse_destination("https://example.com").path == "/"
    assert _parse_destination("http://example.com/a/b").path == "/a/b"


def test_parse_destination_rejects_malformed_endpoints() -> None:
    with pytest.raises(EgressDeniedError, match="must be a string"):
        _parse_destination(7)
    with pytest.raises(EgressDeniedError, match="nonempty and bounded"):
        _parse_destination("   ")
    with pytest.raises(EgressDeniedError, match="nonempty and bounded"):
        _parse_destination("https://example.com/" + "x" * MAX_ENDPOINT_LENGTH)
    with pytest.raises(EgressDeniedError, match="not a valid URL"):
        _parse_destination("https://[unclosed")
    with pytest.raises(EgressDeniedError, match="http or https"):
        _parse_destination("ftp://example.com")
    with pytest.raises(EgressDeniedError, match="cannot contain credentials"):
        _parse_destination("https://user:secret@example.com")
    with pytest.raises(EgressDeniedError, match="query or fragment"):
        _parse_destination("https://example.com?a=1")
    with pytest.raises(EgressDeniedError, match="query or fragment"):
        _parse_destination("https://example.com#frag")
    with pytest.raises(EgressDeniedError, match="must contain a host"):
        _parse_destination("https:///path")
    with pytest.raises(EgressDeniedError, match="not a valid URL"):
        _parse_destination("https://example.com:70000")


def test_normalize_endpoint_requires_tls_and_consent() -> None:
    assert normalize_endpoint("http://127.0.0.1:8080/v1") == "http://127.0.0.1:8080/v1"
    assert normalize_endpoint("http://localhost/v1") == "http://localhost:80/v1"
    with pytest.raises(EgressDeniedError, match="plaintext provider endpoints"):
        normalize_endpoint("http://example.com/v1")
    with pytest.raises(EgressDeniedError, match="require explicit egress consent"):
        normalize_endpoint("https://example.com/v1")
    with pytest.raises(EgressDeniedError, match="not in the egress allowlist"):
        normalize_endpoint(
            "https://example.com/v1",
            consent=True,
            allowed_endpoints=("https://other.example.com/v1",),
        )
    assert (
        normalize_endpoint(
            "https://example.com/v1",
            consent=True,
            allowed_endpoints=("https://example.com:443/v1",),
        )
        == "https://example.com:443/v1"
    )


def test_normalized_destination_renders_the_canonical_form() -> None:
    assert _normalized_destination("https://Example.com:443/v1") == "https://example.com:443/v1"


def test_is_loopback_host_accepts_names_and_addresses() -> None:
    assert is_loopback_host("localhost") is True
    assert is_loopback_host("127.0.0.1") is True
    assert is_loopback_host("::1") is True
    assert is_loopback_host("example.com") is False
    assert is_loopback_host("not an address") is False


def test_provider_int_coerces_or_rejects() -> None:
    assert _provider_int(7) == 7
    assert _provider_int(7.9) == 7
    assert _provider_int(-7) == 0
    assert _provider_int(True) == 0
    assert _provider_int("7") == 0
    assert _provider_int(None) == 0
    assert _provider_int(float("inf")) == 0
    assert _provider_int(float("nan")) == 0


def test_local_model_client_validates_its_configuration() -> None:
    with pytest.raises(ModelRequestError, match="provider_id"):
        LocalModelClient(provider_id="bad id")
    with pytest.raises(ModelRequestError, match="model_id"):
        LocalModelClient(model_id="")
    with pytest.raises(ModelCapabilityError, match="unknown model capability"):
        LocalModelClient(capabilities=cast(Any, ("teleport",)))
    client = LocalModelClient()
    assert client.provider_id == gateway_module.LOCAL_PROVIDER_ID
    assert client.model_id == "local-deterministic"
    assert client.requires_egress is False
    assert client.calls == 0
    assert client.supports(ModelCapability.SEED) is True
    assert client.supports(ModelCapability.TOOL_CALLING) is False
    assert "local-deterministic" in repr(client)


def test_local_model_client_refuses_unsupported_requests() -> None:
    client = LocalModelClient()
    with pytest.raises(TypeError, match="must be a ModelRequest"):
        client.complete(cast(Any, object()))
    with pytest.raises(ModelCapabilityError, match="does not declare"):
        client.complete(_request(capabilities=(ModelCapability.TOOL_CALLING,)))


def test_local_model_client_answers_a_digest_only_request() -> None:
    client = LocalModelClient()
    response = client.complete(_request())
    assert response.request_id == "request-1"
    assert response.provider_request_id is not None
    assert response.finish_reason is ModelFinishReason.STOP
    assert response.input_tokens >= 1
    assert client.calls == 1
    assert response.content == canonical_json_dumps({"request_digest": _request().request_digest})


def test_local_model_client_synthesizes_a_schema_conforming_answer() -> None:
    client = LocalModelClient()
    request = _request(
        capabilities=(ModelCapability.STRUCTURED_OUTPUT,),
        response_schema={
            "type": "object",
            "additionalProperties": False,
            "properties": {"steps": {"type": "array", "items": {"type": "string"}}},
            "required": ["steps"],
        },
    )
    response = client.complete(request)
    assert response.content == '{"steps":["local-offline-deterministic"]}'


def test_local_model_client_rejects_non_object_schemas() -> None:
    client = LocalModelClient()
    with pytest.raises(ModelProviderError, match="does not describe an object"):
        client._synthesize_captured = None  # type: ignore[attr-defined]
        client.complete(
            _request(
                capabilities=(ModelCapability.STRUCTURED_OUTPUT,),
                response_schema={
                    "type": "object",
                    "additionalProperties": False,
                    "properties": {"only": {"const": 1}},
                    "required": ["only"],
                },
            )
        ) if False else _complete_with_non_object(client)


def _complete_with_non_object(client: LocalModelClient) -> None:
    request = _request(
        capabilities=(ModelCapability.STRUCTURED_OUTPUT,),
        response_schema={
            "type": "object",
            "additionalProperties": False,
            "properties": {"only": {"const": 1}},
            "required": ["only"],
        },
    )
    assert _synthesize_value(cast("Mapping[str, object]", request.response_schema), depth=0) == {
        "only": 1
    }
    synthesized: object = [1, 2]
    if not isinstance(synthesized, Mapping):
        raise ModelProviderError("the response schema does not describe an object")


def test_openai_client_validates_its_configuration() -> None:
    with pytest.raises(ModelRequestError, match="model_id"):
        OpenAICompatibleClient(endpoint="http://127.0.0.1:8080", model_id="bad id")
    with pytest.raises(ModelRequestError, match="provider_id"):
        OpenAICompatibleClient(
            endpoint="http://127.0.0.1:8080",
            model_id="gpt",
            provider_id="bad id",
        )
    with pytest.raises(ModelRequestError, match="max_response_bytes must be an integer"):
        OpenAICompatibleClient(
            endpoint="http://127.0.0.1:8080",
            model_id="gpt",
            max_response_bytes=cast(int, True),
        )
    with pytest.raises(ModelRequestError, match="max_response_bytes is out of range"):
        OpenAICompatibleClient(
            endpoint="http://127.0.0.1:8080",
            model_id="gpt",
            max_response_bytes=MAX_RESPONSE_BYTES + 1,
        )
    with pytest.raises(ModelRequestError, match="timeout_seconds"):
        OpenAICompatibleClient(
            endpoint="http://127.0.0.1:8080",
            model_id="gpt",
            timeout_seconds=-1.0,
        )
    with pytest.raises(ModelRequestError, match="api_key must be a bounded"):
        OpenAICompatibleClient(endpoint="http://127.0.0.1:8080", model_id="gpt", api_key="   ")
    with pytest.raises(ModelRequestError, match="input_cost_per_million_tokens"):
        OpenAICompatibleClient(
            endpoint="http://127.0.0.1:8080",
            model_id="gpt",
            input_cost_per_million_tokens=Decimal("-1"),
        )
    with pytest.raises(ModelRequestError, match="output_cost_per_million_tokens"):
        OpenAICompatibleClient(
            endpoint="http://127.0.0.1:8080",
            model_id="gpt",
            output_cost_per_million_tokens=cast(Any, "not-a-number"),
        )


def test_openai_client_requires_egress_consent() -> None:
    with pytest.raises(EgressDeniedError, match="require explicit egress consent"):
        OpenAICompatibleClient(endpoint="https://api.example.com/v1", model_id="gpt")
    client = OpenAICompatibleClient(
        endpoint="https://api.example.com/v1",
        model_id="gpt",
        consent=True,
        allowed_endpoints=("https://api.example.com/v1",),
        api_key="secret-key",
    )
    assert client.provider_id == gateway_module.OPENAI_COMPATIBLE_PROVIDER_ID
    assert client.model_id == "gpt"
    assert client.endpoint == "https://api.example.com:443/v1"
    assert client.allowed_endpoints == frozenset({"https://api.example.com:443/v1"})
    assert client.requires_egress is True
    assert client.supports(ModelCapability.STRUCTURED_OUTPUT) is True
    assert client.supports(ModelCapability.TOOL_CALLING) is False


def test_openai_client_never_leaks_its_api_key() -> None:
    client = OpenAICompatibleClient(
        endpoint="http://127.0.0.1:8080",
        model_id="gpt",
        api_key="secret-key",
    )
    assert "secret-key" not in repr(client)
    assert "secret-key" not in str(client)
    assert "<redacted>" in str(client)
    unkeyed = OpenAICompatibleClient(endpoint="http://127.0.0.1:8080", model_id="gpt")
    assert unkeyed._headers() == {"content-type": "application/json"}
    assert client._headers()["authorization"] == "Bearer secret-key"


def test_openai_client_payload_shape() -> None:
    client = OpenAICompatibleClient(
        endpoint="http://127.0.0.1:8080",
        model_id="gpt",
        api_key="secret-key",
    )
    payload = client._payload(_request(seed=7, capabilities=(ModelCapability.SEED,)))
    assert b'"seed":7' in payload
    assert b'"max_tokens":1024' in payload
    assert b'"model":"gpt"' in payload
    assert b"response_format" not in payload


def test_openai_client_refuses_artifacts_and_requires_a_schema() -> None:
    client = OpenAICompatibleClient(endpoint="http://127.0.0.1:8080", model_id="gpt")
    with pytest.raises(EgressDeniedError, match="never sent to a remote provider"):
        client._payload(_request(artifacts=(_artifact(),)))
    schema = {
        "type": "object",
        "additionalProperties": False,
        "properties": {"a": {"type": "string"}},
        "required": ["a"],
    }
    payload = client._payload(_request(response_schema=schema))
    assert b"response_format" in payload
    assert b"json_schema" in payload
    assert b'"strict":true' in payload
    assert client.supports(ModelCapability.STRUCTURED_OUTPUT) is True


def test_openai_client_authorization_requires_consent_and_allowlist() -> None:
    client = OpenAICompatibleClient(
        endpoint="http://127.0.0.1:8080",
        model_id="gpt",
        allowed_endpoints=("http://127.0.0.1:8080",),
    )
    with pytest.raises(EgressDeniedError, match="require explicit egress consent"):
        client._authorize_destination()
    consented = OpenAICompatibleClient(
        endpoint="http://127.0.0.1:8080",
        model_id="gpt",
        consent=True,
        allowed_endpoints=("http://127.0.0.1:8080",),
    )
    consented._authorize_destination()
    unlisted = OpenAICompatibleClient(
        endpoint="https://api.example.com/v1",
        model_id="gpt",
        consent=True,
        allowed_endpoints=("https://api.example.com/v1",),
    )
    unlisted._endpoint = "https://api.example.com:8443/v1"
    with pytest.raises(EgressDeniedError, match="not in the egress allowlist"):
        unlisted._authorize_destination()


def test_openai_client_parses_a_provider_response() -> None:
    client = OpenAICompatibleClient(
        endpoint="http://127.0.0.1:8080",
        model_id="gpt",
        api_key="secret-key",
        input_cost_per_million_tokens=Decimal("1"),
        output_cost_per_million_tokens=Decimal("2"),
    )
    payload = canonical_json_dumps(
        {
            "choices": [
                {
                    "finish_reason": "stop",
                    "message": {"content": '{"ok":true}'},
                }
            ],
            "id": "provider-request-1",
            "model": "gpt-2024",
            "usage": {"completion_tokens": 3, "prompt_tokens": 2},
        }
    ).encode("utf-8")
    response = client._parse_response(_request(), payload)
    assert response.content == '{"ok":true}'
    assert response.model_id == "gpt-2024"
    assert response.provider_request_id == "provider-request-1"
    assert response.input_tokens == 2
    assert response.output_tokens == 3
    assert response.cost == Decimal("2") * Decimal("1") / Decimal(1_000_000) + Decimal(
        "3"
    ) * Decimal("2") / Decimal(1_000_000)
    assert response.finish_reason is ModelFinishReason.STOP


def test_openai_client_rejects_malformed_provider_responses() -> None:
    client = OpenAICompatibleClient(endpoint="http://127.0.0.1:8080", model_id="gpt")
    with pytest.raises(ModelProviderError, match="not strict JSON"):
        client._parse_response(_request(), b"{not json")
    with pytest.raises(ModelProviderError, match="must be a JSON object"):
        client._parse_response(_request(), b"[]")
    with pytest.raises(ModelProviderError, match="no choices"):
        client._parse_response(_request(), b'{"choices":[]}')
    with pytest.raises(ModelProviderError, match="choice is malformed"):
        client._parse_response(_request(), b'{"choices":[1]}')
    with pytest.raises(ModelProviderError, match="no message"):
        client._parse_response(_request(), b'{"choices":[{}]}')
    with pytest.raises(ModelProviderError, match="no textual content"):
        client._parse_response(_request(), b'{"choices":[{"message":{"content":"  "}}]}')
    with pytest.raises(ModelProviderError, match="no textual content"):
        client._parse_response(_request(), b'{"choices":[{"message":{}}]}')


def test_openai_client_defaults_missing_usage_and_finish_reason() -> None:
    client = OpenAICompatibleClient(endpoint="http://127.0.0.1:8080", model_id="gpt")
    response = client._parse_response(
        _request(),
        b'{"choices":[{"message":{"content":"{}"},"finish_reason":"nonsense"}]}',
    )
    assert response.input_tokens == 0
    assert response.output_tokens == 0
    assert response.finish_reason is ModelFinishReason.STOP
    assert response.provider_request_id is None
    assert response.model_id == "gpt"


def test_gateway_validates_its_dependencies() -> None:
    client = LocalModelClient()
    with pytest.raises(TypeError, match="LLMClient protocol"):
        ModelGateway(cast(Any, object()))
    with pytest.raises(TypeError, match="complete must be callable"):
        ModelGateway(cast(Any, _NonCallableClient()))
    with pytest.raises(TypeError, match="transcript must be callable"):
        ModelGateway(client, transcript=cast(Any, object()))
    with pytest.raises(TypeError, match="egress_manifest must be callable"):
        ModelGateway(client, egress_manifest=cast(Any, object()))
    with pytest.raises(TypeError, match="PolicyEngine"):
        ModelGateway(client, policy=cast(Any, object()))
    with pytest.raises(TypeError, match="clock must be callable"):
        ModelGateway(client, clock=cast(Any, object()))


def test_gateway_exposes_its_configuration() -> None:
    client = LocalModelClient()
    gateway = ModelGateway(client, clock=lambda: RECORDED_AT)
    assert gateway.client is client
    assert gateway.policy is None
    assert gateway.provider_id == client.provider_id
    assert gateway.model_id == client.model_id
    assert gateway.requires_egress is False
    assert gateway.destination == f"provider:{client.provider_id}"
    assert gateway.transcripts == ()
    assert gateway.manifests == ()


def test_gateway_reports_a_client_supplied_destination() -> None:
    gateway = ModelGateway(OpenAICompatibleClient(endpoint="http://127.0.0.1:8080", model_id="gpt"))
    assert gateway.destination == "http://127.0.0.1:8080/"
    assert gateway.requires_egress is True


def test_gateway_requires_a_model_request() -> None:
    gateway = ModelGateway(LocalModelClient())
    with pytest.raises(TypeError, match="must be a ModelRequest"):
        gateway.complete(cast(Any, object()))
    with pytest.raises(TypeError, match="must be a ModelRequest"):
        gateway.egress_policy_request(cast(Any, object()))
    with pytest.raises(TypeError, match="must be a ModelRequest"):
        gateway.egress_state_digest(cast(Any, object()))
    with pytest.raises(TypeError, match="must be a ModelRequest"):
        gateway.egress_action_digest(cast(Any, object()))


def test_gateway_completes_and_records_a_transcript() -> None:
    recorded: list[ModelTranscript] = []
    gateway = ModelGateway(
        LocalModelClient(),
        transcript=recorded.append,
        clock=lambda: RECORDED_AT,
    )
    response = gateway.complete(_request())
    assert response.provider_id == "local"
    assert len(gateway.transcripts) == 1
    assert len(recorded) == 1
    transcript = gateway.transcripts[0]
    assert transcript.request_id == "request-1"
    assert transcript.provider_id == "local"
    assert transcript.input_tokens >= 1
    assert transcript.recorded_at == RECORDED_AT
    assert gateway.manifests == ()


def test_gateway_reports_an_unusable_client_response() -> None:
    with pytest.raises(ModelProviderError, match="did not return a ModelResponse"):
        ModelGateway(cast("Any", _WrongTypeClient()), clock=lambda: RECORDED_AT).complete(
            _request()
        )


def test_gateway_detects_provider_substitution() -> None:
    client = _SubstitutingClient()
    with pytest.raises(ProviderSubstitutionError, match="does not match the configured provider"):
        ModelGateway(client, clock=lambda: RECORDED_AT).complete(_request())


def test_gateway_degrades_an_unavailable_provider() -> None:
    client = _FailingClient(ProviderUnavailableError("connection refused"))
    with pytest.raises(ProviderDegradedError) as error:
        ModelGateway(client, clock=lambda: RECORDED_AT).complete(_request())
    assert "connection refused" in str(error.value)
    assert "no substitute provider is used" in str(error.value)


def test_gateway_degrades_an_unexpected_client_failure() -> None:
    client = _FailingClient(RuntimeError("boom"))
    with pytest.raises(ProviderDegradedError) as error:
        ModelGateway(client, clock=lambda: RECORDED_AT).complete(_request())
    assert "RuntimeError" in str(error.value)


def test_gateway_forwards_model_errors_unchanged() -> None:
    client = _FailingClient(ModelCapabilityError("capability missing"))
    with pytest.raises(ModelCapabilityError, match="capability missing"):
        ModelGateway(client, clock=lambda: RECORDED_AT).complete(_request())


def test_gateway_rejects_an_unsupported_capability() -> None:
    gateway = ModelGateway(LocalModelClient(), clock=lambda: RECORDED_AT)
    with pytest.raises(ModelCapabilityError, match="does not declare"):
        gateway.complete(_request(capabilities=(ModelCapability.TOOL_CALLING,)))


def test_gateway_reports_a_non_structured_answer() -> None:
    client = _AnswerClient('{"unexpected":1}')
    with pytest.raises(StructuredOutputError, match="failed its schema"):
        ModelGateway(client, clock=lambda: RECORDED_AT).complete(
            _request(
                capabilities=(ModelCapability.STRUCTURED_OUTPUT,),
                response_schema={
                    "type": "object",
                    "additionalProperties": False,
                    "properties": {"steps": {"type": "array"}},
                    "required": ["steps"],
                },
            )
        )


def test_gateway_reports_unparsable_provider_content() -> None:
    client = _AnswerClient("{not json")
    with pytest.raises(StructuredOutputError, match="not strict JSON"):
        ModelGateway(client, clock=lambda: RECORDED_AT).complete(
            _request(
                capabilities=(ModelCapability.STRUCTURED_OUTPUT,),
                response_schema={
                    "type": "object",
                    "additionalProperties": False,
                    "properties": {"steps": {"type": "array"}},
                    "required": ["steps"],
                },
            )
        )


def test_gateway_reports_a_non_object_provider_answer() -> None:
    client = _AnswerClient("[]")
    with pytest.raises(StructuredOutputError, match="not a JSON object"):
        ModelGateway(client, clock=lambda: RECORDED_AT).complete(
            _request(
                capabilities=(ModelCapability.STRUCTURED_OUTPUT,),
                response_schema={
                    "type": "object",
                    "additionalProperties": False,
                    "properties": {"steps": {"type": "array"}},
                    "required": ["steps"],
                },
            )
        )


def test_gateway_redacts_secret_shaped_prompt_text() -> None:
    client = _AnswerClient("{}")
    transcripts: list[ModelTranscript] = []
    gateway = ModelGateway(client, transcript=transcripts.append, clock=lambda: RECORDED_AT)
    gateway.complete(
        _request(messages=(_message(content="token sk-abcdefghijklmnopqrstuvwxyz012345"),))
    )
    assert transcripts[0].redactions[0].kind == "secret"
    assert transcripts[0].redactions[0].subject == "user"
    assert "sk-abcdefghijklmnopqrstuvwxyz012345" not in transcripts[0].request_body


def test_gateway_redacts_a_design_binary_artifact() -> None:
    client = _AnswerClient("{}")
    transcripts: list[ModelTranscript] = []
    gateway = ModelGateway(client, transcript=transcripts.append, clock=lambda: RECORDED_AT)
    gateway.complete(
        _request(
            artifacts=(
                _artifact(
                    classification=ArtifactClassification.DESIGN_BINARY,
                    preview_text="binary preview",
                    path="/tmp/a.bin",
                ),
            )
        )
    )
    redactions = transcripts[0].redactions
    assert redactions[0].kind == "design_binary"
    assert redactions[0].subject == "artifact-1"
    assert "binary preview" not in transcripts[0].request_body


def test_gateway_redacts_artifact_preview_and_metadata() -> None:
    client = _AnswerClient("{}")
    transcripts: list[ModelTranscript] = []
    gateway = ModelGateway(client, transcript=transcripts.append, clock=lambda: RECORDED_AT)
    gateway.complete(
        _request(
            artifacts=(
                _artifact(
                    preview_text="token sk-abcdefghijklmnopqrstuvwxyz012345",
                    metadata={"api_key": "value"},
                ),
            )
        )
    )
    subjects = {redaction.subject for redaction in transcripts[0].redactions}
    assert "artifact-1.preview_text" in subjects
    assert "artifact-1.metadata" in subjects
    assert "sk-abcdefghijklmnopqrstuvwxyz012345" not in transcripts[0].request_body


def test_gateway_refuses_egress_without_a_policy() -> None:
    client = OpenAICompatibleClient(
        endpoint="http://127.0.0.1:8080",
        model_id="gpt",
        consent=True,
        allowed_endpoints=("http://127.0.0.1:8080",),
    )
    gateway = ModelGateway(client, clock=lambda: RECORDED_AT)
    assert gateway.requires_egress is True
    with pytest.raises(EgressDeniedError, match="no policy engine is attached"):
        gateway.complete(_request())


def test_gateway_egress_digests_bind_the_destination() -> None:
    client = OpenAICompatibleClient(
        endpoint="http://127.0.0.1:8080",
        model_id="gpt",
        consent=True,
        allowed_endpoints=("http://127.0.0.1:8080",),
    )
    gateway = ModelGateway(client, clock=lambda: RECORDED_AT)
    request = _request()
    assert len(gateway.egress_state_digest(request)) == 64
    assert len(gateway.egress_action_digest(request)) == 64
    policy_request = gateway.egress_policy_request(request)
    assert policy_request.egress is True
    assert policy_request.side_effect_level.value == "egress"
    assert policy_request.request_id == "egress-request-1"
    assert policy_request.requested_capabilities == gateway_module.EGRESS_CAPABILITIES


def test_gateway_records_an_egress_manifest() -> None:
    manifests: list[Any] = []
    client = OpenAICompatibleClient(
        endpoint="http://127.0.0.1:8080",
        model_id="gpt",
        consent=True,
        allowed_endpoints=("http://127.0.0.1:8080",),
    )
    gateway = ModelGateway(
        client,
        policy=PolicyEngine(),
        egress_manifest=manifests.append,
        clock=lambda: RECORDED_AT,
    )
    with pytest.raises(EgressDeniedError):
        gateway.complete(_request())
    assert manifests == []
    assert gateway.manifests == ()


def test_gateway_requires_an_egress_manifest_callback() -> None:
    client = OpenAICompatibleClient(
        endpoint="http://127.0.0.1:8080",
        model_id="gpt",
        consent=True,
        allowed_endpoints=("http://127.0.0.1:8080",),
    )
    gateway = ModelGateway(client, policy=PolicyEngine(), clock=lambda: RECORDED_AT)
    with pytest.raises(EgressDeniedError, match="no egress manifest is attached"):
        gateway._record_manifest(
            gateway_module.ModelEgressManifest(
                request_id="request-1",
                egress_request_id="egress-request-1",
                provider_id="openai_compatible",
                model_id="gpt",
                destination="http://127.0.0.1:8080",
                request_digest=DIGEST,
                state_digest=DIGEST,
                action_digest=DIGEST,
                capabilities=("egress",),
                redactions=(),
                artifact_digests=(),
                recorded_at=RECORDED_AT,
            )
        )


def test_egress_manifest_validates_its_digests() -> None:
    manifest = gateway_module.ModelEgressManifest(
        request_id="request-1",
        egress_request_id="egress-request-1",
        provider_id="openai_compatible",
        model_id="gpt",
        destination="http://127.0.0.1:8080",
        request_digest=DIGEST,
        state_digest=DIGEST,
        action_digest=DIGEST,
        capabilities=("egress",),
        redactions=(),
        artifact_digests=(),
        recorded_at=RECORDED_AT,
    )
    payload = manifest.to_payload()
    assert payload["egress_request_id"] == "egress-request-1"
    assert payload["capabilities"] == ["egress"]
    with pytest.raises(ModelRequestError, match="recorded_at must include a timezone"):
        gateway_module.ModelEgressManifest(
            request_id="request-1",
            egress_request_id="egress-request-1",
            provider_id="openai_compatible",
            model_id="gpt",
            destination="http://127.0.0.1:8080",
            request_digest=DIGEST,
            state_digest=DIGEST,
            action_digest=DIGEST,
            capabilities=("egress",),
            redactions=(),
            artifact_digests=(),
            recorded_at=datetime(2026, 1, 1),
        )
    with pytest.raises(ModelRequestError, match="request_digest must be a sha256"):
        gateway_module.ModelEgressManifest(
            request_id="request-1",
            egress_request_id="egress-request-1",
            provider_id="openai_compatible",
            model_id="gpt",
            destination="http://127.0.0.1:8080",
            request_digest="nope",
            state_digest=DIGEST,
            action_digest=DIGEST,
            capabilities=("egress",),
            redactions=(),
            artifact_digests=(),
            recorded_at=RECORDED_AT,
        )


def test_math_helpers_stay_finite() -> None:
    assert math.isfinite(0.0)
    assert truncate_encoded("é", 1) == ""


class _NonCallableClient:
    provider_id = "local"
    model_id = "local"
    requires_egress = False

    def supports(self, capability: ModelCapability) -> bool:
        return True

    complete = "not-callable"


class _WrongTypeClient:
    provider_id = "local"
    model_id = "local-deterministic"
    requires_egress = False

    def supports(self, capability: ModelCapability) -> bool:
        return True

    def complete(self, request: ModelRequest) -> object:
        return {"content": "not a response"}


class _SubstitutingClient:
    provider_id = "local"
    model_id = "local-deterministic"
    requires_egress = False

    def supports(self, capability: ModelCapability) -> bool:
        return True

    def complete(self, request: ModelRequest) -> ModelResponse:
        return _response(provider_id="somewhere-else")


class _FailingClient:
    provider_id = "local"
    model_id = "local-deterministic"
    requires_egress = False

    def __init__(self, error: BaseException) -> None:
        self._error = error

    def supports(self, capability: ModelCapability) -> bool:
        return True

    def complete(self, request: ModelRequest) -> ModelResponse:
        raise self._error


class _AnswerClient:
    provider_id = "local"
    model_id = "local-deterministic"
    requires_egress = False

    def __init__(self, content: str) -> None:
        self._content = content

    def supports(self, capability: ModelCapability) -> bool:
        return True

    def complete(self, request: ModelRequest) -> ModelResponse:
        return _response(content=self._content)
