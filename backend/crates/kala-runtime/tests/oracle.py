#!/usr/bin/env python3
"""Live Python oracle for the Kala model gateway.

The script is test-only. It drives the repository's own
`kala_agent.runtime.model_gateway` implementation so the native Rust crate can
be compared against it byte for byte.

Usage:

    python oracle.py gateway < scenario.json

The scenario document is canonical JSON on stdin. The result document is
canonical JSON on stdout.

Scenario shape::

    {
      "cases": [
        {
          "name": "local-deterministic",
          "kind": "local" | "remote",
          "request": {...},
          "policy": {"grants": [...], "approve": true},
          "ledger": {"max_model_calls": 4, "max_total_tokens": 4000} | null,
          "endpoint": "http://127.0.0.1:1234/case/success" | null,
          "api_key": "secret" | null,
          "consent": true,
          "max_response_bytes": 1048576,
          "input_cost": "0",
          "output_cost": "0",
          "model_id": "local-model",
          "provider_id": "local",
          "capabilities": ["structured_output", "seed"]
        }
      ]
    }
"""

from __future__ import annotations

import json
import os
import sys
from datetime import UTC, datetime
from decimal import Decimal
from typing import Any

# The oracle runs in CPython's isolated mode (`-I`), so the repository
# sources are put on the path explicitly instead of through `PYTHONPATH`.
_SOURCE_ROOT = os.environ.get("KALA_SRC")
if _SOURCE_ROOT:
    sys.path.insert(0, _SOURCE_ROOT)

from kala_agent.contracts import ArtifactClassification
from kala_agent.kernel.budget import BudgetLedger, RunBudget
from kala_agent.kernel.policy import PolicyEngine
from kala_agent.runtime.model_gateway import (
    EGRESS_CAPABILITIES,
    EgressDeniedError,
    LocalModelClient,
    ModelArtifact,
    ModelCapability,
    ModelGateway,
    ModelMessage,
    ModelRequest,
    OpenAICompatibleClient,
)

RECORDED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)


def _thaw(value: Any) -> Any:
    if isinstance(value, dict) or hasattr(value, "items"):
        return {key: _thaw(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_thaw(item) for item in value]
    return value


def _request(payload: dict[str, Any]) -> ModelRequest:
    messages = tuple(
        ModelMessage(role=message["role"], content=message["content"])
        for message in payload.get("messages", ())
    )
    artifacts = tuple(
        ModelArtifact(
            artifact_id=artifact["artifact_id"],
            digest=artifact["digest"],
            classification=ArtifactClassification(artifact["classification"]),
            media_type=artifact["media_type"],
            size_bytes=artifact["size_bytes"],
            preview_text=artifact.get("preview_text"),
            path=artifact.get("path"),
            metadata=artifact.get("metadata") or {},
        )
        for artifact in payload.get("artifacts", ())
    )
    capabilities = payload.get("capabilities")
    return ModelRequest(
        request_id=payload["request_id"],
        messages=messages,
        capabilities=frozenset(ModelCapability(item) for item in capabilities)
        if capabilities is not None
        else frozenset(),
        response_schema=payload.get("response_schema"),
        artifacts=artifacts,
        seed=payload.get("seed"),
        max_output_tokens=payload.get("max_output_tokens", 1024),
        timeout_seconds=payload.get("timeout_seconds", 60.0),
    )


def _build_client(case: dict[str, Any]) -> Any:
    if case["kind"] == "local":
        return LocalModelClient(
            model_id=case.get("model_id", "local-deterministic"),
            provider_id=case.get("provider_id", "local"),
            capabilities=tuple(
                ModelCapability(item) for item in case.get("capabilities", ())
            ),
        )
    endpoint = case["endpoint"]
    return OpenAICompatibleClient(
        endpoint=endpoint,
        model_id=case.get("model_id", "local-model"),
        allowed_endpoints=[endpoint],
        api_key=case.get("api_key"),
        consent=bool(case.get("consent", False)),
        timeout_seconds=case.get("client_timeout_seconds", 30.0),
        max_response_bytes=case.get("max_response_bytes", 1_048_576),
        input_cost_per_million_tokens=Decimal(case.get("input_cost", "0")),
        output_cost_per_million_tokens=Decimal(case.get("output_cost", "0")),
    )


def _run_case(case: dict[str, Any]) -> dict[str, Any]:
    client = _build_client(case)
    ledger = None
    if case.get("ledger"):
        ledger = BudgetLedger(
            RunBudget(
                max_model_calls=case["ledger"].get("max_model_calls", 64),
                max_total_tokens=case["ledger"].get("max_total_tokens", 1_000_000),
                max_cost=Decimal(case["ledger"].get("max_cost", "0")),
            )
        )
    policy = None
    policy_grants = case.get("policy", {}).get("grants") if case.get("policy") else None
    if policy_grants is not None:
        policy = PolicyEngine(list(policy_grants))
    transcripts: list[Any] = []
    manifests: list[Any] = []
    gateway = ModelGateway(
        client,
        ledger=ledger,
        transcript=transcripts.append if case.get("collect_transcript", True) else None,
        policy=policy,
        egress_manifest=manifests.append if case.get("collect_manifest", True) else None,
        clock=lambda: RECORDED_AT,
    )
    result: dict[str, Any] = {"name": case["name"]}
    request: ModelRequest | None = None
    try:
        request = _request(case["request"])
        if policy is not None and case.get("policy", {}).get("approve"):
            policy.approve(gateway.egress_policy_request(request))
        response = gateway.complete(request)
    except BaseException as error:  # noqa: BLE001 - the oracle reports every class
        result["outcome"] = "error"
        result["error"] = {
            "class": type(error).__name__,
            "message": str(error),
        }
    else:
        result["outcome"] = "ok"
        result["response"] = {
            "content": response.content,
            "cost": str(response.cost),
            "finish_reason": response.finish_reason.value,
            "input_tokens": response.input_tokens,
            "model_id": response.model_id,
            "output_tokens": response.output_tokens,
            "provider_id": response.provider_id,
            "provider_request_id": response.provider_request_id,
            "request_id": response.request_id,
            "structured": _thaw(response.structured) if response.structured is not None else None,
            "total_tokens": response.total_tokens,
        }
    result["transcripts"] = [transcript.to_payload() for transcript in gateway.transcripts]
    result["manifests"] = [manifest.to_payload() for manifest in gateway.manifests]
    result["ledger"] = ledger.export_state() if ledger is not None else None
    if request is None:
        result["digests"] = {"action": None, "state": None}
    else:
        result["digests"] = {
            "action": _safe_digest(lambda: gateway.egress_action_digest(request)),
            "state": _safe_digest(lambda: gateway.egress_state_digest(request)),
        }
    return result


def _safe_digest(action: Any) -> str | None:
    try:
        return action()
    except Exception:  # noqa: BLE001 - a refusal is reported as "no digest"
        return None


def _corpus(document: dict[str, Any]) -> dict[str, Any]:
    """Redaction, secret-name and endpoint results for a deterministic corpus."""
    from kala_agent.runtime.model_gateway import (
        is_loopback_host,
        is_secret_name,
        normalize_endpoint,
        redact_secrets,
        truncate_encoded,
    )

    redactions = []
    for text in document.get("texts", []):
        redacted, findings = redact_secrets(text)
        redactions.append(
            {
                "text": text,
                "redacted": redacted,
                "findings": findings,
                "truncated": truncate_encoded(text, 64),
            }
        )
    names = [
        {"name": name, "secret": is_secret_name(name)} for name in document.get("names", [])
    ]
    endpoints = []
    for value in document.get("endpoints", []):
        try:
            normalized = normalize_endpoint(value)
        except EgressDeniedError as error:
            endpoints.append({"value": value, "error": str(error)})
        else:
            endpoints.append(
                {
                    "value": value,
                    "normalized": normalized,
                    "loopback": is_loopback_host(normalized.split("://")[1].split(":")[0]),
                }
            )
    decimals = []
    for pair in document.get("decimals", []):
        left = Decimal(pair["left"])
        right = Decimal(pair["right"])
        try:
            product = str(left * right)
        except ArithmeticError as error:
            product = type(error).__name__
        try:
            quotient = str(left / right)
        except ArithmeticError as error:
            quotient = type(error).__name__
        decimals.append(
            {"left": pair["left"], "right": pair["right"], "product": product, "quotient": quotient}
        )
    return {
        "redactions": redactions,
        "names": names,
        "endpoints": endpoints,
        "decimals": decimals,
    }


def main() -> int:
    if len(sys.argv) != 2 or sys.argv[1] not in ("gateway", "corpus"):
        print("usage: oracle.py gateway|corpus < scenario.json", file=sys.stderr)
        return 2
    document = json.load(sys.stdin)
    if sys.argv[1] == "corpus":
        payload = _corpus(document)
    else:
        results = [_run_case(case) for case in document.get("cases", [])]
        payload = {"results": results}
    json.dump(payload, sys.stdout, sort_keys=True, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
