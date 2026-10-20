"""Studio and core HTTP reference corpus.

Every response is captured through Starlette's ``TestClient`` against the live
FastAPI application. No socket is opened. The request that produced each case
is recorded alongside the status, stable headers, exact JSON body (when small
enough), and body digest. Binary STL bodies are recorded by digest/length; the
small default files are also stored under ``stl/``. STEP bodies are recorded
structurally because their header carries a write timestamp.
"""

from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import Final

from corpuslib import CORE_TOKEN, STUDIO_TOKEN, SYNTHETIC_PROVIDER_KEY
from corpuslib.common import (
    FixedClock,
    fixed_journal_clock,
    fixed_studio_journal_clock,
    normalise_step_bytes,
    sha256_bytes,
    sha256_text,
    silenced_fds,
    step_normalisation_facts,
)

MAX_STORED_BODY: Final[int] = 65_536
MAX_STORED_REQUEST_BODY: Final[int] = 512
ISO_TIMESTAMP_RE: Final[re.Pattern[str]] = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}")
# The placeholder that replaces a run-specific value in a recorded body.
VOLATILE_PLACEHOLDER: Final[str] = "<VOLATILE>"
STABLE_HEADERS: Final[tuple[str, ...]] = (
    "content-type",
    "content-length",
    "location",
    "content-disposition",
    "x-studio-handoff",
    "www-authenticate",
)
ABSENT_HEADERS: Final[tuple[str, ...]] = ("server", "x-powered-by", "set-cookie")
SECURITY_HEADER_NAMES: Final[tuple[str, ...]] = (
    "cache-control",
    "content-security-policy",
    "cross-origin-opener-policy",
    "cross-origin-resource-policy",
    "referrer-policy",
    "x-content-type-options",
    "x-frame-options",
)

_NO_BODY = object()
_JSON = object()


def _header_record(headers: object) -> dict[str, str]:
    lowered = {str(key).lower(): str(value) for key, value in headers.items()}  # type: ignore[attr-defined]
    record = {name: lowered[name] for name in STABLE_HEADERS if name in lowered}
    for name in SECURITY_HEADER_NAMES:
        if name in lowered:
            record[name] = lowered[name]
    return record


def _normalize_body(body: object, data_dir: Path | None) -> tuple[object, list[str]]:
    normalizations: list[str] = []
    if data_dir is None:
        return body, normalizations
    needles = {str(data_dir), str(data_dir.resolve())}

    def visit(value: object) -> object:
        if isinstance(value, dict):
            result: dict[str, object] = {}
            for key, item in value.items():
                if key == "data_dir" and isinstance(item, str) and item in needles:
                    result[key] = "<DATA_DIR>"
                    if "$.data_dir" not in normalizations:
                        normalizations.append("$.data_dir")
                else:
                    result[key] = visit(item)
            return result
        if isinstance(value, list):
            return [visit(item) for item in value]
        return value

    return visit(body), normalizations


def _compact_digest(values: object) -> str:
    # The protocol canonicalizer is bounded at 2,048 collection items; preview
    # arrays are much larger. Use the same CPython JSON formatting directly.
    text = json.dumps(values, allow_nan=False, ensure_ascii=True, separators=(",", ":"), sort_keys=True)
    return sha256_text(text)


def _array_summary(values: object) -> dict[str, object]:
    materialized = list(values)  # type: ignore[arg-type]
    return {
        "length": len(materialized),
        "sha256_of_compact_json": _compact_digest(materialized),
        "first": materialized[:6],
        "last": materialized[-6:],
    }


def _summarize_preview(body: dict[str, object]) -> dict[str, object]:
    summary: dict[str, object] = {
        "top_level_keys": sorted(body),
        "bounds": body.get("bounds"),
        "engine": body.get("engine"),
        "warnings": body.get("warnings"),
    }
    for name in ("positions", "normals", "indices"):
        if name in body:
            summary[name] = _array_summary(body[name])
    return summary


def _summarize_generic(body: object) -> dict[str, object]:
    if isinstance(body, dict):
        return {"top_level_keys": sorted(body)}
    return {"compact_json_sha256": _compact_digest(body)}


def _step_structure(raw: bytes) -> dict[str, object]:
    normalised = normalise_step_bytes(raw)
    header_text = normalised[:2048].decode("latin-1", errors="replace")
    header_lines = header_text.splitlines()[:12]
    timestamp_line = next(
        (line for line in header_lines if ISO_TIMESTAMP_RE.search(line)), None
    )
    return {
        "length": len(raw),
        "sha256": sha256_bytes(normalised),
        "sha256_role": (
            "digest of the STEP body after the writer's run-specific values are "
            "normalised; the raw digest is informational only"
        ),
        "starts_with_iso_10303_21": raw.startswith(b"ISO-10303-21"),
        "header_lines": header_lines,
        "timestamp_line": timestamp_line,
        "header_timestamp_detected": timestamp_line is not None,
        "manifold_solid_brep_count": raw.count(b"MANIFOLD_SOLID_BREP"),
        "byte_equality_contract": False,
        "normalisation": step_normalisation_facts(),
    }


def _is_sha256_hex(value: object) -> bool:
    text = str(value)
    return len(text) == 64 and all(character in "0123456789abcdef" for character in text)


def _journal_chain_summary(page: object) -> dict[str, object]:
    """Summarise the chain relations of a Studio journal page.

    The chain digests themselves are masked in the recorded body, because the
    chain binds the STEP artifact digest. The relations the chain asserts --
    the genesis link, the link from every event to its predecessor, and the
    shape of every digest -- are reproducible, so they are recorded here.
    """
    events = page.get("events") if isinstance(page, dict) else None
    if not isinstance(events, list):
        return {"available": False}
    previous = "0" * 64
    links: list[dict[str, object]] = []
    for event in events:
        if not isinstance(event, dict):
            continue
        event_digest = str(event.get("event_digest", ""))
        previous_digest = str(event.get("previous_event_digest", ""))
        links.append(
            {
                "sequence": event.get("sequence"),
                "event_type": event.get("event_type"),
                "links_to_previous": previous_digest == previous,
                "digest_is_sha256_hex": _is_sha256_hex(event_digest),
                "payload_digest_is_sha256_hex": _is_sha256_hex(event.get("digest")),
            }
        )
        previous = event_digest
    return {
        "available": True,
        "event_count": len(events),
        "genesis_previous_digest": "0" * 64,
        "event_links": links,
        "chain_is_unbroken": all(bool(link["links_to_previous"]) for link in links),
        "head_digest_is_sha256_hex": _is_sha256_hex(previous),
    }


def _mask_segments(node: object, segments: list[str], sink: list[str]) -> None:
    """Replace the value at a dotted path with the volatile placeholder.

    A segment may end with ``[*]``, which walks every item of the list it
    names: ``$.events[*].digest`` masks the digest of every event.
    """
    if not segments:
        return
    head, rest = segments[0], segments[1:]
    wildcard = head.endswith("[*]")
    if wildcard:
        head = head[: -len("[*]")]
    if head == "*":
        if isinstance(node, list):
            for item in node:
                _mask_segments(item, rest, sink)
        return
    if not isinstance(node, dict) or head not in node:
        return
    if wildcard:
        child = node[head]
        if isinstance(child, list):
            for item in child:
                _mask_segments(item, rest, sink)
        return
    if not rest:
        sink.append(str(node[head]))
        node[head] = VOLATILE_PLACEHOLDER
        return
    _mask_segments(node[head], rest, sink)


def _mask_by_path(body: object, path: str, sink: list[str]) -> None:
    """Replace the value at ``$.a.b`` (or ``$.a[*].b``) with the placeholder."""
    segments = [part for part in path.split(".") if part and part != "$"]
    _mask_segments(body, segments, sink)


def _mask_by_value(node: object, values: tuple[str, ...], sink: list[str]) -> None:
    """Replace every leaf whose value is one of ``values`` with the placeholder."""
    if isinstance(node, dict):
        for key, item in node.items():
            if isinstance(item, str) and item in values:
                sink.append(item)
                node[key] = VOLATILE_PLACEHOLDER
            else:
                _mask_by_value(item, values, sink)
        return
    if isinstance(node, list):
        for index, item in enumerate(node):
            if isinstance(item, str) and item in values:
                sink.append(item)
                node[index] = VOLATILE_PLACEHOLDER
            else:
                _mask_by_value(item, values, sink)


def mask_volatile_fields(
    case: dict[str, object],
    *,
    paths: tuple[tuple[str, str], ...] = (),
    values: tuple[tuple[str, str, str], ...] = (),
) -> list[str]:
    """Replace run-specific values in a captured case with a placeholder.

    A captured response can carry a value that the product itself cannot
    reproduce: the digest of a STEP export that embeds a wall-clock
    timestamp, or a journal chain digest that binds one. Such a value is
    replaced by :data:`VOLATILE_PLACEHOLDER` in the recorded body and in the
    recorded body text, and the case is marked non-deterministic. The reason
    for every masked field is recorded next to it; the masked value itself is
    never recorded, because it is exactly the value that cannot be
    reproduced.

    The masked values are returned so a caller can mask the same value in
    another case (the STEP digest appears in the design result and in the
    journal events that record the artifact).
    """
    expected = case["expected"]
    assert isinstance(expected, dict)
    body = expected.get("body")
    sink: list[str] = []
    for path, _reason in paths:
        _mask_by_path(body, path, sink)
    for value, _label, _reason in values:
        _mask_by_value(body, (value,), sink)
    if not sink:
        return []
    text = expected.get("body_text")
    if not isinstance(text, str):
        raise ValueError("a volatile field needs a stored body_text to mask")
    for value in sink:
        text = text.replace(
            json.dumps(value, ensure_ascii=True),
            json.dumps(VOLATILE_PLACEHOLDER, ensure_ascii=True),
        )
    expected["body_text"] = text
    expected["body_sha256"] = sha256_text(text)
    expected["body_sha256_role"] = (
        "digest of the recorded body text after the run-specific values were "
        "replaced by the placeholder; the raw response bytes carry the real "
        "values and are not reproducible"
    )
    expected["body_deterministic"] = False
    expected["volatile_fields"] = [
        {"path": path, "reason": reason} for path, reason in paths
    ] + [
        {"field": label, "reason": reason} for _value, label, reason in values
    ]
    if isinstance(body, (dict, list)):
        expected["normalized_body_sha256"] = sha256_text(
            json.dumps(body, ensure_ascii=True, separators=(",", ":"), sort_keys=True)
        )
    return sink


def _request_record(
    *,
    method: str,
    path: str,
    json_body: object,
    content: object,
    headers: dict[str, str] | None,
    chunked: bool,
) -> dict[str, object]:
    record_headers = dict(headers or {})
    if json_body is not _NO_BODY and not chunked and "content-type" not in {
        key.lower() for key in record_headers
    }:
        record_headers["content-type"] = "application/json"
    body_record: object = None
    if json_body is not _NO_BODY:
        body_record = {"kind": "json", "value": json_body}
    elif chunked:
        chunks = []
        for chunk in content:  # type: ignore[union-attr]
            chunk_bytes = bytes(chunk)
            chunk_record: dict[str, object] = {
                "length": len(chunk_bytes),
                "sha256": sha256_bytes(chunk_bytes),
            }
            if len(chunk_bytes) <= MAX_STORED_REQUEST_BODY:
                chunk_record["text"] = chunk_bytes.decode("utf-8", errors="replace")
            chunks.append(chunk_record)
        body_record = {"kind": "chunked", "chunks": chunks}
    elif isinstance(content, bytes):
        body_record = {
            "kind": "raw_bytes",
            "length": len(content),
            "sha256": sha256_bytes(content),
        }
        if len(content) <= MAX_STORED_REQUEST_BODY:
            body_record["text"] = content.decode("utf-8", errors="replace")
    return {
        "method": method,
        "path": path,
        "headers": record_headers,
        "body": body_record,
    }


def _capture(
    client: object,
    case_id: str,
    method: str,
    path: str,
    *,
    json_body: object = _NO_BODY,
    content: object = None,
    headers: dict[str, str] | None = None,
    chunked: bool = False,
    data_dir: Path | None = None,
    body_contract: str = "exact",
    body_artifact: str | None = None,
    note: str | None = None,
    suppress_stdio: bool = False,
) -> tuple[dict[str, object], object]:
    request_headers = dict(headers or {})
    kwargs: dict[str, object] = {"headers": request_headers}
    if json_body is not _NO_BODY:
        kwargs["json"] = json_body
    elif chunked:
        kwargs["content"] = content
    elif isinstance(content, bytes):
        kwargs["content"] = content
    if suppress_stdio:
        with silenced_fds():
            response = client.request(method, path, **kwargs)  # type: ignore[attr-defined]
    else:
        response = client.request(method, path, **kwargs)  # type: ignore[attr-defined]

    raw = response.content
    lowered = {str(key).lower(): str(value) for key, value in response.headers.items()}
    content_type = lowered.get("content-type", "")
    expected: dict[str, object] = {
        "status": response.status_code,
        "headers": _header_record(response.headers),
        "headers_absent": [name for name in ABSENT_HEADERS if name not in lowered],
        "body_length": len(raw),
        "body_sha256": sha256_bytes(raw),
    }
    parsed: object = None
    deterministic = True
    if body_contract == "structural":
        expected["body_contract"] = "structural"
        expected["body_structural"] = _step_structure(raw)
        expected["body_sha256"] = sha256_bytes(normalise_step_bytes(raw))
        expected["body_sha256_role"] = (
            "digest of the STEP body after the writer's run-specific values were "
            "normalised; the raw response bytes carry a wall-clock timestamp and a "
            "per-process translator counter"
        )
        deterministic = False
    elif content_type.startswith("application/json"):
        parsed = response.json()
        parsed, normalizations = _normalize_body(parsed, data_dir)
        normalised_bytes = raw
        if normalizations:
            deterministic = False
            expected["normalizations"] = normalizations
            if data_dir is not None:
                for needle in (str(data_dir), str(data_dir.resolve())):
                    normalised_bytes = normalised_bytes.replace(
                        needle.encode("utf-8"), b"<DATA_DIR>"
                    )
        expected["body_sha256_role"] = (
            "digest of the normalised response bytes; the raw response embeds the "
            "run-specific data directory path"
            if normalizations
            else "exact response bytes"
        )
        expected["body_sha256"] = sha256_bytes(normalised_bytes)
        expected["normalized_body_sha256"] = sha256_text(
            json.dumps(parsed, ensure_ascii=True, separators=(",", ":"), sort_keys=True)
        )
        if len(raw) <= MAX_STORED_BODY:
            expected["body"] = parsed
            expected["body_text"] = normalised_bytes.decode("utf-8")
        else:
            if isinstance(parsed, dict) and {"positions", "normals", "indices"} <= set(parsed):
                expected["body_summary"] = _summarize_preview(parsed)
            else:
                expected["body_summary"] = _summarize_generic(parsed)
            expected["body_storage"] = (
                f"summarized because the response exceeds {MAX_STORED_BODY} bytes; "
                "body_sha256 is the exact oracle"
            )
    else:
        expected["body_contract"] = "binary"
        expected["body_prefix_hex"] = raw[:96].hex()
        if body_artifact is not None:
            expected["body_artifact"] = body_artifact
        elif len(raw) <= 4096:
            expected["body_hex"] = raw.hex()

    expected["body_deterministic"] = deterministic
    if not deterministic:
        expected["body_sha256_note"] = (
            "informational at generation time; reproduce the stable fields instead"
        )

    case: dict[str, object] = {
        "id": case_id,
        "request": _request_record(
            method=method,
            path=path,
            json_body=json_body,
            content=content,
            headers=headers,
            chunked=chunked,
        ),
        "expected": expected,
    }
    if note:
        case["note"] = note
    return case, parsed


def _dedupe_absent(case: dict[str, object]) -> None:
    expected = case["expected"]  # type: ignore[index]
    assert isinstance(expected, dict)
    expected["headers_absent"] = list(dict.fromkeys(expected["headers_absent"]))  # type: ignore[arg-type]


def _core_cases(work_root: Path) -> list[dict[str, object]]:
    from fastapi.testclient import TestClient

    from kala_agent.api import create_app
    from kala_agent.application import KalaApplication
    from kala_agent.config import Settings

    data_dir = work_root / "core-data"
    clock = FixedClock()
    application = KalaApplication(Settings(data_dir=data_dir), clock=clock)
    app = create_app(application, api_token=CORE_TOKEN, max_body_bytes=16_384)
    cases: list[dict[str, object]] = []
    journal_clock = fixed_journal_clock(clock)
    journal_clock.__enter__()

    def add(case_id: str, method: str, path: str, **kwargs: object) -> object:
        headers = dict(kwargs.pop("headers", {}) or {})  # type: ignore[arg-type]
        headers.setdefault("Authorization", f"Bearer {CORE_TOKEN}")
        case, parsed = _capture(
            client,
            case_id,
            method,
            path,
            headers=headers,
            data_dir=data_dir,
            **kwargs,  # type: ignore[arg-type]
        )
        _dedupe_absent(case)
        cases.append(case)
        return parsed

    with TestClient(
        app,
        base_url="http://127.0.0.1",
        headers={"Authorization": f"Bearer {CORE_TOKEN}"},
        raise_server_exceptions=False,
    ) as client, TestClient(
        app, base_url="http://127.0.0.1", raise_server_exceptions=False
    ) as anonymous, TestClient(
        app,
        base_url="http://127.0.0.1",
        headers={"Authorization": "Bearer wrong-token-0123456789abcdef"},
        raise_server_exceptions=False,
    ) as wrong_token:
        add("core.health", "GET", "/health", note="data_dir is normalized to <DATA_DIR>")
        add("core.domains.list", "GET", "/api/v1/domains")
        add("core.domain.read", "GET", "/api/v1/domains/mechanical")
        add(
            "core.domain.evaluate",
            "POST",
            "/api/v1/domains/mechanical/evaluations",
            json_body={
                "parameters": {"length_mm": 140.0, "hole_count": 4},
                "design_id": "corpus-eval",
            },
        )

        revision = add(
            "core.revision.create",
            "POST",
            "/api/v1/revisions",
            json_body={
                "design_id": "corpus-design",
                "domain": "mechanical",
                "summary": "corpus revision",
            },
        )
        revision_id = revision["revision"]["revision_id"]  # type: ignore[index]
        add("core.revision.read", "GET", f"/api/v1/revisions/{revision_id}")
        add("core.revision.list", "GET", "/api/v1/revisions?design_id=corpus-design&limit=10")

        prepared = add(
            "core.revision_commit.prepare",
            "POST",
            "/api/v1/revision-commits",
            json_body={
                "design_id": "corpus-prep",
                "domain": "wing",
                "summary": "prepared commit",
            },
        )
        request_id = prepared["request_id"]  # type: ignore[index]
        add("core.revision_commit.read", "GET", f"/api/v1/revision-commits/{request_id}")
        add(
            "core.revision_commit.approve",
            "POST",
            f"/api/v1/revision-commits/{request_id}/approve",
            json_body={"actor_id": "corpus-actor", "reason": "approved by corpus"},
        )

        rejected_prepared = add(
            "core.revision_commit.prepare_reject",
            "POST",
            "/api/v1/revision-commits",
            json_body={"design_id": "corpus-reject", "domain": "mechanical"},
        )
        rejected_request_id = rejected_prepared["request_id"]  # type: ignore[index]
        add(
            "core.revision_commit.reject",
            "POST",
            f"/api/v1/revision-commits/{rejected_request_id}/reject",
            json_body={"actor_id": "corpus-actor", "reason": "rejected by corpus"},
        )

        run = add(
            "core.workflow.start",
            "POST",
            "/api/v1/workflows/mechanical/reference",
            json_body={"workflow_id": "corpus-flow", "design_id": "corpus-flow-design"},
        )
        workflow_id = run["state"]["workflow_id"]  # type: ignore[index]
        add("core.workflow.read", "GET", f"/api/v1/workflows/{workflow_id}")
        add("core.workflow.checkpoints", "GET", f"/api/v1/workflows/{workflow_id}/checkpoints")
        add(
            "core.workflow.resume",
            "POST",
            "/api/v1/workflows/mechanical/reference",
            json_body={"workflow_id": workflow_id, "design_id": "corpus-flow-design"},
            note="resuming an in-flight run returns 200 rather than 201",
        )
        add(
            "core.workflow.approve",
            "POST",
            f"/api/v1/workflows/{workflow_id}/checkpoints/approve",
            json_body={"actor_id": "corpus-actor"},
        )

        reject_run = add(
            "core.workflow.start_reject",
            "POST",
            "/api/v1/workflows/mechanical/reference",
            json_body={
                "workflow_id": "corpus-flow-reject",
                "design_id": "corpus-flow-reject-design",
            },
        )
        reject_workflow_id = reject_run["state"]["workflow_id"]  # type: ignore[index]
        add(
            "core.workflow.reject",
            "POST",
            f"/api/v1/workflows/{reject_workflow_id}/checkpoints/reject",
            json_body={"actor_id": "corpus-actor", "reason": "corpus rejection"},
        )

        add("core.events.list", "GET", "/api/v1/events?limit=20")
        add(
            "core.events.filtered",
            "GET",
            "/api/v1/events?event_type=domain.evaluated&limit=5",
        )
        add(
            "core.events.by_workflow",
            "GET",
            "/api/v1/events?workflow_id=corpus-flow&limit=5",
        )

        add(
            "core.revision.create_fork_source",
            "POST",
            "/api/v1/revisions",
            json_body={"design_id": "corpus-fork", "domain": "wing"},
        )

        add(
            "core.negative.fork_refused",
            "POST",
            "/api/v1/revisions",
            json_body={
                "design_id": "corpus-design",
                "domain": "mechanical",
                "parent_revision_id": "rev-corpus-fork-r0",
            },
        )
        stale = add(
            "core.revision_commit.prepare_stale",
            "POST",
            "/api/v1/revision-commits",
            json_body={"design_id": "corpus-stale", "domain": "building"},
        )
        stale_request_id = stale["request_id"]  # type: ignore[index]
        add(
            "core.negative.stale_state_digest",
            "POST",
            f"/api/v1/revision-commits/{stale_request_id}/approve",
            json_body={"state_digest": "0" * 64},
        )
        add(
            "core.negative.decided_again",
            "POST",
            f"/api/v1/revision-commits/{request_id}/approve",
            json_body={},
        )
        add(
            "core.negative.finished_workflow",
            "POST",
            "/api/v1/workflows/mechanical/reference",
            json_body={"workflow_id": workflow_id, "design_id": "corpus-flow-design"},
        )

        add("core.negative.unknown_domain", "GET", "/api/v1/domains/nope")
        add("core.negative.unknown_revision", "GET", "/api/v1/revisions/rev-does-not-exist")
        add(
            "core.negative.unknown_prepared",
            "GET",
            "/api/v1/revision-commits/commit-does-not-exist",
        )
        add("core.negative.unknown_workflow", "GET", "/api/v1/workflows/flow-does-not-exist")
        add(
            "core.negative.extra_field",
            "POST",
            "/api/v1/domains/mechanical/evaluations",
            json_body={"parameters": {}, "nope": 1},
        )
        add(
            "core.negative.invalid_parameters",
            "POST",
            "/api/v1/domains/mechanical/evaluations",
            json_body={"parameters": {"length_mm": 1.0}},
        )
        add("core.negative.invalid_query", "GET", "/api/v1/events?limit=0")
        add(
            "core.negative.invalid_json",
            "POST",
            "/api/v1/domains/mechanical/evaluations",
            content=b"{",
            headers={"content-type": "application/json"},
        )
        add(
            "core.negative.wrong_content_type",
            "POST",
            "/api/v1/domains/mechanical/evaluations",
            content=b'{"parameters":{}}',
            headers={"content-type": "text/plain"},
        )
        add(
            "core.negative.invalid_identifier",
            "POST",
            "/api/v1/revisions",
            json_body={"design_id": "contains a space", "domain": "mechanical"},
        )
        add(
            "core.negative.bad_host",
            "GET",
            "/api/v1/domains",
            headers={"Host": "evil.example"},
        )
        add(
            "core.negative.cross_origin",
            "GET",
            "/api/v1/domains",
            headers={"Origin": "http://evil.example"},
        )
        add(
            "core.negative.cross_origin_preflight",
            "OPTIONS",
            "/api/v1/domains",
            headers={"Origin": "http://evil.example", "Access-Control-Request-Method": "GET"},
        )
        add(
            "core.negative.unknown_route",
            "GET",
            "/api/v1/not-a-route",
        )
        add(
            "core.negative.method_not_allowed",
            "DELETE",
            "/api/v1/domains",
        )
        add(
            "core.negative.oversized_declared",
            "POST",
            "/api/v1/revisions",
            content=b"x" * 20_000,
            headers={"content-type": "application/json"},
        )
        add(
            "core.negative.oversized_chunked",
            "POST",
            "/api/v1/revisions",
            content=[b"x" * 10_000, b"x" * 10_000],
            headers={"content-type": "application/json"},
            chunked=True,
        )
        add(
            "core.valid.empty_optional_body",
            "POST",
            "/api/v1/domains/mechanical/evaluations",
        )

        missing_case, _ = _capture(
            anonymous,
            "core.negative.missing_token",
            "POST",
            "/api/v1/revisions",
            json_body={"design_id": "no-token", "domain": "mechanical"},
            data_dir=data_dir,
        )
        _dedupe_absent(missing_case)
        cases.append(missing_case)
        read_case, _ = _capture(
            anonymous,
            "core.valid.read_without_token",
            "GET",
            "/api/v1/domains",
            data_dir=data_dir,
        )
        _dedupe_absent(read_case)
        cases.append(read_case)
        wrong_case, _ = _capture(
            wrong_token,
            "core.negative.wrong_token",
            "POST",
            "/api/v1/revisions",
            json_body={"design_id": "wrong-token", "domain": "mechanical"},
            headers={"Authorization": "Bearer wrong-token-0123456789abcdef"},
            data_dir=data_dir,
        )
        _dedupe_absent(wrong_case)
        cases.append(wrong_case)

    journal_clock.__exit__(None, None, None)
    application.close()
    return cases


def _studio_cases(work_root: Path) -> list[dict[str, object]]:
    from fastapi.testclient import TestClient

    from kala_agent.studio.server import create_studio_app

    data_dir = work_root / "studio-data"
    static_dir = work_root / "static"
    static_dir.mkdir(parents=True, exist_ok=True)
    (static_dir / "index.html").write_text(
        "<!doctype html><html><body>corpus</body></html>", encoding="utf-8"
    )
    (static_dir / "asset.txt").write_text("asset\n", encoding="utf-8")
    step_workdir = work_root / "step-work"

    app = create_studio_app(
        data_dir=data_dir,
        api_token=STUDIO_TOKEN,
        max_body_bytes=16_384,
        static_root=static_dir,
        environ={},
    )
    cases: list[dict[str, object]] = []

    def add(
        case_id: str, method: str, path: str, **kwargs: object
    ) -> tuple[dict[str, object], object]:
        headers = dict(kwargs.pop("headers", {}) or {})  # type: ignore[arg-type]
        headers.setdefault("Authorization", f"Bearer {STUDIO_TOKEN}")
        kwargs.setdefault("suppress_stdio", True)
        case, parsed = _capture(
            client,
            case_id,
            method,
            path,
            headers=headers,
            **kwargs,  # type: ignore[arg-type]
        )
        _dedupe_absent(case)
        cases.append(case)
        return case, parsed

    previous_workdir = os.environ.get("KALA_STUDIO_DATA_DIR")
    os.environ["KALA_STUDIO_DATA_DIR"] = str(step_workdir)
    studio_clock = FixedClock()
    try:
        with fixed_studio_journal_clock(studio_clock), TestClient(
            app,
            base_url="http://127.0.0.1",
            headers={"Authorization": f"Bearer {STUDIO_TOKEN}"},
            raise_server_exceptions=False,
        ) as client, TestClient(
            app, base_url="http://127.0.0.1", raise_server_exceptions=False
        ) as anonymous, TestClient(
            app,
            base_url="http://127.0.0.1",
            headers={"Authorization": "Bearer wrong-token-0123456789abcdef"},
            raise_server_exceptions=False,
        ) as wrong_token:
            add("studio.health", "GET", "/api/studio/health")
            add("studio.integrations", "GET", "/api/studio/integrations")
            add("studio.providers", "GET", "/api/studio/providers")
            add("studio.settings.read", "GET", "/api/studio/settings")
            add(
                "studio.settings.store_key",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openai", "api_key": SYNTHETIC_PROVIDER_KEY},
                note="synthetic key only; never a live credential",
            )
            add("studio.settings.read_stored", "GET", "/api/studio/settings")
            add("studio.providers.configured", "GET", "/api/studio/providers")
            add("studio.health.online", "GET", "/api/studio/health")
            add(
                "studio.settings.clear_key",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openai", "api_key": None},
            )
            add("studio.health.offline_again", "GET", "/api/studio/health")

            add("studio.preview.box_default", "POST", "/api/studio/preview", json_body={"kind": "box"})
            add(
                "studio.preview.box_custom",
                "POST",
                "/api/studio/preview",
                json_body={
                    "kind": "box",
                    "parameters": {
                        "size_x": 20,
                        "size_y": 10,
                        "size_z": 5,
                        "subdivisions": 2,
                    },
                },
            )
            add(
                "studio.preview.cylinder_max",
                "POST",
                "/api/studio/preview",
                json_body={
                    "kind": "cylinder",
                    "parameters": {"radial_segments": 256, "height_segments": 64},
                },
            )
            add(
                "studio.preview.sphere_max",
                "POST",
                "/api/studio/preview",
                json_body={"kind": "sphere", "parameters": {"rings": 96, "sectors": 128}},
                note="response exceeds the stored-body threshold; digests and counts are frozen",
            )
            add(
                "studio.export.stl_box_default",
                "POST",
                "/api/studio/export",
                json_body={"kind": "box", "format": "stl"},
                body_artifact="stl/box_default.stl",
            )
            add(
                "studio.export.stl_sphere_max",
                "POST",
                "/api/studio/export",
                json_body={
                    "kind": "sphere",
                    "format": "stl",
                    "parameters": {"rings": 96, "sectors": 128},
                },
            )
            add(
                "studio.export.step_box",
                "POST",
                "/api/studio/export",
                json_body={"kind": "box", "format": "step"},
                body_contract="structural",
                suppress_stdio=True,
            )
            add(
                "studio.handoff.stl_freecad",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "freecad", "kind": "box", "format": "stl"},
                body_artifact="stl/box_default.stl",
            )
            add(
                "studio.handoff.step_solidworks",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "solidworks", "kind": "box", "format": "step"},
                body_contract="structural",
                suppress_stdio=True,
            )
            add(
                "studio.chat.local",
                "POST",
                "/api/studio/chat",
                json_body={
                    "provider": "local",
                    "prompt": "A box 10x20x30 mm for FreeCAD. Show the envelope.",
                },
            )
            add(
                "studio.chat.openai_model_required",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "openai", "prompt": "hello"},
            )
            add(
                "studio.chat.openai_remote_not_allowed",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "openai", "model": "gpt-4o-mini", "prompt": "hello"},
            )
            add(
                "studio.chat.openai_missing_key",
                "POST",
                "/api/studio/chat",
                json_body={
                    "provider": "openai",
                    "model": "gpt-4o-mini",
                    "prompt": "hello",
                    "allow_remote": True,
                },
            )
            add(
                "studio.chat.codex_unavailable",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "codex", "prompt": "hello", "allow_remote": True},
            )
            add(
                "studio.chat.codex_requires_remote",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "codex", "prompt": "hello"},
            )

            add(
                "studio.negative.invalid_kind",
                "POST",
                "/api/studio/preview",
                json_body={"kind": "torus"},
            )
            add(
                "studio.negative.extra_field",
                "POST",
                "/api/studio/preview",
                json_body={"kind": "box", "nope": 1},
            )
            add(
                "studio.negative.parameters_list",
                "POST",
                "/api/studio/preview",
                json_body={"kind": "box", "parameters": {"size_x": [1, 2]}},
            )
            add(
                "studio.negative.dimension_out_of_range",
                "POST",
                "/api/studio/preview",
                json_body={"kind": "box", "parameters": {"size_x": 0.01}},
            )
            add(
                "studio.negative.empty_prompt",
                "POST",
                "/api/studio/chat",
                json_body={"provider": "local", "prompt": ""},
            )
            add(
                "studio.negative.unknown_integration",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "nope", "kind": "box", "format": "step"},
            )
            add(
                "studio.negative.unsupported_handoff_format",
                "POST",
                "/api/studio/handoff",
                json_body={"integration_id": "openscad", "kind": "box", "format": "step"},
            )
            add(
                "studio.negative.invalid_api_key",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "openai", "api_key": "not a key"},
            )
            add(
                "studio.negative.invalid_settings_provider",
                "POST",
                "/api/studio/settings",
                json_body={"provider": "codex", "api_key": "x"},
            )
            add(
                "studio.negative.bad_host",
                "GET",
                "/api/studio/health",
                headers={"Host": "evil.example"},
            )
            add(
                "studio.negative.cross_origin",
                "GET",
                "/api/studio/health",
                headers={"Origin": "http://evil.example"},
            )
            add(
                "studio.negative.cross_origin_preflight",
                "OPTIONS",
                "/api/studio/preview",
                headers={
                    "Origin": "http://evil.example",
                    "Access-Control-Request-Method": "POST",
                },
            )
            add(
                "studio.negative.unknown_route",
                "GET",
                "/api/studio/not-a-route",
            )
            add(
                "studio.negative.method_not_allowed",
                "DELETE",
                "/api/studio/preview",
            )
            add(
                "studio.negative.oversized_declared",
                "POST",
                "/api/studio/preview",
                content=b"x" * 20_000,
                headers={"content-type": "application/json"},
            )
            add(
                "studio.negative.oversized_chunked",
                "POST",
                "/api/studio/preview",
                content=[b"x" * 10_000, b"x" * 10_000],
                headers={"content-type": "application/json"},
                chunked=True,
            )
            add(
                "studio.valid.same_origin",
                "GET",
                "/api/studio/health",
                headers={"Origin": "http://127.0.0.1"},
            )
            add("studio.static.index", "GET", "/")
            add("studio.static.asset", "GET", "/asset.txt")
            add("studio.static.spa_fallback", "GET", "/missing")
            add("studio.static.api_prefix_reserved", "GET", "/api/not-a-route")

            # --- ADR-0009 and ADR-0011 routes ------------------------------
            # These are the design-turn, journal and CAD-provider routes. The
            # Rust front door proxies them to the Python worker (ADR-0010), so
            # the corpus freezes the contract the proxy must reproduce.
            add(
                "studio.cad_providers",
                "GET",
                "/api/studio/providers/cad",
                note=(
                    "ADR-0011: availability is a live probe of this host, so the "
                    "refusal reasons are environment facts recorded in the manifest"
                ),
            )

            design_case, design_body = add(
                "studio.design.l_bracket",
                "POST",
                "/api/studio/design",
                json_body={
                    "provider": "opencascade",
                    "template": {
                        "template": "l_bracket",
                        "width": 60.0,
                        "depth": 45.0,
                        "height": 30.0,
                        "wall": 4.0,
                    },
                    "label": "corpus bracket",
                    "material": "steel",
                },
                note=(
                    "ADR-0009/ADR-0011: the provider is mandatory; the measured "
                    "values, the check rows and the imported view are the "
                    "product's own numbers"
                ),
            )
            design_id = str(design_body["design_id"])  # type: ignore[index]
            step_digest = str(design_body["artifacts"]["step_sha256"])  # type: ignore[index]
            mask_volatile_fields(
                design_case,
                paths=(
                    (
                        "$.artifacts.step_sha256",
                        "the OpenCascade STEP export embeds the wall clock of the "
                        "write, so its content digest is not reproducible",
                    ),
                    (
                        "$.journal_head",
                        "the Studio journal chain digest binds the journalled STEP "
                        "artifact digest, so it is not reproducible either",
                    ),
                ),
            )

            add(
                "studio.design.drawing",
                "GET",
                f"/api/studio/design/{design_id}/drawing",
                note="ADR-0009 hidden-line views of a stored design",
            )
            add(
                "studio.design.section",
                "GET",
                f"/api/studio/design/{design_id}/section",
                note="ADR-0009 section loops of a stored design",
            )
            add(
                "studio.design.provider_required",
                "POST",
                "/api/studio/design",
                json_body={
                    "template": {
                        "template": "l_bracket",
                        "width": 60.0,
                        "depth": 45.0,
                        "height": 30.0,
                        "wall": 4.0,
                    }
                },
                note="ADR-0011: the provider is required, not defaulted",
            )
            add(
                "studio.design.drawing.unknown",
                "GET",
                f"/api/studio/design/{'0' * 64}/drawing",
                note="a design id that was never stored is a 404, not an empty drawing",
            )

            journal_case, journal_body = add(
                "studio.journal.page",
                "GET",
                "/api/studio/journal",
                note=(
                    "the Studio session journal is its own hash chain; the chain "
                    "digests are masked because they bind the STEP artifact digest"
                ),
            )
            # The chain relations are read before the masking, because they are
            # relations between the real digests.
            journal_chain = _journal_chain_summary(journal_body)
            page_events = journal_body.get("events") if isinstance(journal_body, dict) else None
            last_event_digest = ""
            if isinstance(page_events, list) and page_events:
                last = page_events[-1]
                if isinstance(last, dict):
                    last_event_digest = str(last.get("event_digest", ""))
            mask_volatile_fields(
                journal_case,
                paths=(
                    (
                        "$.events[*].digest",
                        "the payload digest of the event that records the STEP "
                        "artifact is not reproducible",
                    ),
                    (
                        "$.events[*].event_digest",
                        "the chain digest binds the STEP artifact digest, so it is "
                        "not reproducible",
                    ),
                    (
                        "$.events[*].previous_event_digest",
                        "the chain digest binds the STEP artifact digest, so it is "
                        "not reproducible",
                    ),
                ),
                values=(
                    (
                        step_digest,
                        "the STEP artifact digest",
                        "the OpenCascade STEP export embeds the wall clock of the "
                        "write, so its content digest is not reproducible",
                    ),
                ),
            )
            journal_case["expected"]["journal_chain"] = journal_chain

            verify_case, verify_body = add(
                "studio.journal.verify",
                "GET",
                "/api/studio/journal/verify",
                note="the chain verdict for the Studio session journal",
            )
            # The verdict is compared with the page before either is masked.
            verify_chain = {
                "head_digest_matches_last_event": (
                    str(verify_body.get("head_digest")) == last_event_digest  # type: ignore[union-attr]
                ),
                "event_count_matches_page": (
                    verify_body.get("event_count")  # type: ignore[union-attr]
                    == len(page_events)  # type: ignore[arg-type]
                ),
                "chain_ok": verify_body.get("ok"),  # type: ignore[union-attr]
                "first_bad_sequence": verify_body.get("first_bad_sequence"),  # type: ignore[union-attr]
            }
            mask_volatile_fields(
                verify_case,
                paths=(
                    (
                        "$.head_digest",
                        "the chain head binds the STEP artifact digest, so it is "
                        "not reproducible",
                    ),
                ),
            )
            verify_case["expected"]["journal_chain"] = verify_chain

            missing_case, _ = _capture(
                anonymous,
                "studio.negative.missing_token",
                "POST",
                "/api/studio/preview",
                json_body={"kind": "box"},
            )
            _dedupe_absent(missing_case)
            cases.append(missing_case)
            wrong_case, _ = _capture(
                wrong_token,
                "studio.negative.wrong_token",
                "POST",
                "/api/studio/preview",
                json_body={"kind": "box"},
                headers={"Authorization": "Bearer wrong-token-0123456789abcdef"},
            )
            _dedupe_absent(wrong_case)
            cases.append(wrong_case)
    finally:
        if previous_workdir is None:
            os.environ.pop("KALA_STUDIO_DATA_DIR", None)
        else:
            os.environ["KALA_STUDIO_DATA_DIR"] = previous_workdir
    return cases


def build(work_root: Path) -> dict[str, object]:
    studio = _studio_cases(work_root)
    core = _core_cases(work_root)
    return {
        "corpus_schema_version": 1,
        "kind": "http",
        "capture": {
            "client": "fastapi.testclient.TestClient (in-process ASGI, no socket)",
            "body_storage_threshold_bytes": MAX_STORED_BODY,
            "stable_headers": list(STABLE_HEADERS),
            "security_headers": list(SECURITY_HEADER_NAMES),
            "absent_headers": list(ABSENT_HEADERS),
            "json_body_serialization": "exact response bytes, with body_text for small bodies",
            "binary_body_serialization": "sha256/length; small bodies may also store hex",
            "step_body_serialization": "structural fields only; the writer's wall-clock timestamp and per-process translator counter are normalised before the digest is taken",
            "authorization_fixture_tokens": {
                "core": CORE_TOKEN,
                "studio": STUDIO_TOKEN,
                "synthetic_provider_key": SYNTHETIC_PROVIDER_KEY,
                "live_credentials": False,
            },
            "journal_timestamp_override": (
                "EventJournal uses datetime.now(UTC) in production, and so does "
                "StudioSessionJournal. The fixture harness substitutes a fixed "
                "clock for kala_agent.persistence.journal.datetime and for "
                "kala_agent.studio.session_journal.datetime so event timestamps "
                "and event digests are reproducible. The hashing and serialization "
                "code is unchanged."
            ),
            "replay": (
                "Each case records the exact method, path, explicit request headers "
                "(including the synthetic bearer token where used), and a tagged body. "
                "The Host used by the default client is 127.0.0.1."
            ),
            "generated_identifiers": (
                "Revision ids are rev-<design>-r<sequence>; commit request ids are "
                "derived from the state and action digests; workflow ids are supplied "
                "by the request. No generated identifier required normalization."
            ),
            "timestamps": (
                "Application timestamps use a fixed one-second sequence from "
                "2026-01-01T00:00:00Z. Journal timestamps use the fixture-only fixed "
                "journal clock described above."
            ),
            "envelopes": {
                "studio": "flat {code,message,details}",
                "core": "nested {error:{code,message},details}",
                "unified": False,
            },
        },
        "studio": {
            "prefix": "/api/studio",
            "app": "kala_agent.studio.server.create_studio_app",
            "max_body_bytes": 16_384,
            "cases": studio,
        },
        "core": {
            "prefix": "/api/v1",
            "app": "kala_agent.api.app.create_app",
            "max_body_bytes": 16_384,
            "cases": core,
        },
        "normalizations": [
            {
                "service": "core",
                "field": "$.data_dir in GET /health",
                "value": "<DATA_DIR>",
                "reason": "fixtures never record the operator's filesystem paths",
            },
            {
                "service": "core",
                "field": "GET /health body digest",
                "value": "digest of the normalised response bytes",
                "reason": "the raw response embeds the temporary data directory path",
            },
            {
                "service": "studio",
                "field": "STEP body bytes, header timestamp and translator counter",
                "value": "normalised before the digest is taken",
                "reason": "the OCP writer stamps the wall clock and numbers the translated product per process",
            },
            {
                "service": "studio",
                "field": "design result step digest, journal head and journal chain digests",
                "value": "<VOLATILE>",
                "reason": "the chain binds the STEP artifact digest, which embeds the wall clock of the write",
            },
        ],
        "nondeterministic_fields": [
            {
                "service": "core",
                "field": "GET /health data_dir",
                "treatment": "normalized to <DATA_DIR>",
            },
            {
                "service": "core",
                "field": "journal event occurred_at and dependent event digests",
                "treatment": (
                    "deterministic fixed clock injected into the journal module for "
                    "fixture generation; production uses the wall clock"
                ),
            },
            {
                "service": "studio",
                "field": "STEP body bytes and header timestamp",
                "treatment": (
                    "structural capture; the writer's wall-clock timestamp and its "
                    "per-process translator counter are normalised before the digest "
                    "is taken"
                ),
            },
            {
                "service": "studio",
                "field": "studio journal event occurred_at and dependent event digests",
                "treatment": (
                    "deterministic fixed clock injected into "
                    "kala_agent.studio.session_journal for fixture generation; "
                    "production uses the wall clock"
                ),
            },
            {
                "service": "studio",
                "field": "design result step digest, journal head and journal chain digests",
                "treatment": (
                    "masked with <VOLATILE>; the chain binds the STEP artifact digest, "
                    "which embeds the wall clock of the write"
                ),
            },
            {
                "service": "studio",
                "field": "CAD provider probe results",
                "treatment": (
                    "recorded as they are; ADR-0011 makes availability a live probe "
                    "of the host, and the manifest records the same probe as an "
                    "environment fact"
                ),
            },
            {
                "service": "studio",
                "field": "OCP writer transfer statistics",
                "treatment": "captured and discarded; never part of an HTTP response",
            },
        ],
    }


__all__ = ("build",)
