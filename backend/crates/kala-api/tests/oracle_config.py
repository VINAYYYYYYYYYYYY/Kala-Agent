#!/usr/bin/env python3
"""Live Python differential oracle for the Kala local configuration.

This is a test oracle, not product code. It exercises the **live**
`kala_agent.config.Settings` model of the repository and records, for every
case, either the validated settings or the exact validation failures: the
Pydantic error kind, the field, and the message.

The corpus is written to disk so the Rust side can validate the very same
values and compare the answers.

Usage:
    python3 oracle_config.py <corpus.json> <results.json>
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[4]
SRC = REPO / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

from kala_agent.config import Settings  # noqa: E402
from pydantic import ValidationError  # noqa: E402


def text(value: str) -> dict[str, Any]:
    """A string value."""
    return {"kind": "text", "value": value}


def integer(value: int) -> dict[str, Any]:
    """An integer value."""
    return {"kind": "integer", "value": value}


def number(value: str) -> dict[str, Any]:
    """A floating point value, carried as text so the corpus stays valid JSON.

    The text is what CPython's `float()` reads, so `inf` and `nan` are part of
    the corpus without making the document unreadable.
    """
    return {"kind": "float", "value": value}


def boolean(value: bool) -> dict[str, Any]:
    """A boolean value."""
    return {"kind": "boolean", "value": value}


def null() -> dict[str, Any]:
    """An explicit null."""
    return {"kind": "null"}


def to_python(value: dict[str, Any]) -> Any:
    """Convert a tagged corpus value into the Python value it stands for."""
    kind = value["kind"]
    if kind == "text":
        return value["value"]
    if kind == "integer":
        return value["value"]
    if kind == "float":
        return float(value["value"])
    if kind == "boolean":
        return value["value"]
    if kind == "null":
        return None
    raise ValueError(f"unknown value kind {kind!r}")


def build_cases(work: Path) -> list[dict[str, Any]]:
    """Build the deterministic configuration corpus."""
    existing = work / "data"
    existing.mkdir(parents=True, exist_ok=True)
    not_a_directory = work / "not-a-directory"
    not_a_directory.write_text("data", encoding="utf-8")
    link = work / "link"
    if not link.exists():
        link.symlink_to(existing, target_is_directory=True)

    cases: list[dict[str, Any]] = []

    def add(name: str, values: dict[str, Any]) -> None:
        cases.append({"name": name, "values": values})

    add("default", {})
    add("data_dir.existing", {"data_dir": text(str(existing))})
    add("data_dir.missing.child", {"data_dir": text(str(work / "child"))})
    add("data_dir.trailing.dot", {"data_dir": text(str(existing) + "/.")})
    add("data_dir.parent", {"data_dir": text(str(work / ".." / existing.name))})
    add("data_dir.symlink", {"data_dir": text(str(link))})
    add("data_dir.relative", {"data_dir": text("relative/path")})
    add("data_dir.root", {"data_dir": text("/")})
    add("data_dir.file", {"data_dir": text(str(not_a_directory))})
    add("data_dir.nul", {"data_dir": text("/tmp/a\u0000b")})
    add("data_dir.integer", {"data_dir": integer(5)})
    add("data_dir.boolean", {"data_dir": boolean(True)})
    add("data_dir.null", {"data_dir": null()})
    add("data_dir.home", {"data_dir": text("~")})
    add("data_dir.tilde.child", {"data_dir": text("~/kala-data")})
    add("data_dir.double.slash", {"data_dir": text("//tmp//x")})

    add("model_provider.default", {"model_provider": text("local")})
    add("model_provider.upper", {"model_provider": text("LOCAL")})
    add("model_provider.padded", {"model_provider": text("  local  ")})
    add("model_provider.punycode", {"model_provider": text("xn--r8jz45g")})
    add("model_provider.dots", {"model_provider": text("a.b_c-d")})
    add("model_provider.digit.first", {"model_provider": text("1local")})
    add("model_provider.dot.first", {"model_provider": text(".local")})
    add("model_provider.empty", {"model_provider": text("")})
    add("model_provider.spaces", {"model_provider": text("   ")})
    add("model_provider.space.inside", {"model_provider": text("a b")})
    add("model_provider.too.long", {"model_provider": text("a" * 65)})
    add("model_provider.exactly.long", {"model_provider": text("a" * 64)})
    add("model_provider.integer", {"model_provider": integer(5)})
    add("model_provider.null", {"model_provider": null()})

    add("model_id.default", {"model_id": text("local")})
    add("model_id.padded", {"model_id": text("  local  ")})
    add("model_id.empty", {"model_id": text("")})
    add("model_id.spaces", {"model_id": text("   ")})
    add("model_id.too.long", {"model_id": text("a" * 257)})
    add("model_id.exactly.long", {"model_id": text("a" * 256)})
    add("model_id.integer", {"model_id": integer(5)})

    add("model_endpoint.none", {"model_endpoint": null()})
    add("model_endpoint.http", {"model_endpoint": text("http://example.com")})
    add("model_endpoint.https", {"model_endpoint": text("https://example.com/v1")})
    add("model_endpoint.port", {"model_endpoint": text("https://example.com:8443/v1")})
    add("model_endpoint.upper", {"model_endpoint": text("HTTP://EXAMPLE.COM/V1")})
    add("model_endpoint.trailing.space", {"model_endpoint": text("http://example.com ")})
    add("model_endpoint.path", {"model_endpoint": text("http://example.com/a%20b")})
    add("model_endpoint.unicode", {"model_endpoint": text("http://example.com/\u00fcn\u00ef")})
    add("model_endpoint.idna", {"model_endpoint": text("http://\u4f8b\u3048.\u30c6\u30b9\u30c8/")})
    add("model_endpoint.punycode", {"model_endpoint": text("http://xn--r8jz45g.xn--zckzah/")})
    add("model_endpoint.credentials", {"model_endpoint": text("https://user:password@example.com/v1")})
    add("model_endpoint.user", {"model_endpoint": text("http://user@example.com")})
    add("model_endpoint.password", {"model_endpoint": text("http://:pass@example.com")})
    add("model_endpoint.empty.user", {"model_endpoint": text("http://@example.com")})
    add("model_endpoint.query", {"model_endpoint": text("https://example.com/v1?api_key=secret")})
    add("model_endpoint.fragment", {"model_endpoint": text("https://example.com/v1#frag")})
    add("model_endpoint.empty.fragment", {"model_endpoint": text("http://example.com#")})
    add("model_endpoint.empty.query", {"model_endpoint": text("http://example.com/?")})
    add("model_endpoint.relative", {"model_endpoint": text("/relative")})
    add("model_endpoint.scheme", {"model_endpoint": text("ftp://example.com/v1")})
    add("model_endpoint.no.host", {"model_endpoint": text("http://")})
    add("model_endpoint.space.host", {"model_endpoint": text("http://exa mple.com")})
    add("model_endpoint.percent.host", {"model_endpoint": text("http://ex%ample.com/")})
    add("model_endpoint.ipv4", {"model_endpoint": text("http://999.1.1.1/")})
    add("model_endpoint.ipv6", {"model_endpoint": text("http://[::1]/x")})
    add("model_endpoint.ipv6.broken", {"model_endpoint": text("http://[::1")})
    add("model_endpoint.port.text", {"model_endpoint": text("http://example.com:abc/")})
    add("model_endpoint.port.high", {"model_endpoint": text("http://example.com:65536/")})
    add("model_endpoint.port.zero", {"model_endpoint": text("http://example.com:0/")})
    add("model_endpoint.integer", {"model_endpoint": integer(5)})
    add("model_endpoint.boolean", {"model_endpoint": boolean(True)})

    add("request_timeout.default", {"request_timeout_seconds": number("120.0")})
    add("request_timeout.text", {"request_timeout_seconds": text("12.5")})
    add("request_timeout.integer", {"request_timeout_seconds": integer(30)})
    add("request_timeout.zero", {"request_timeout_seconds": number("0.0")})
    add("request_timeout.infinity", {"request_timeout_seconds": number("inf")})
    add("request_timeout.nan", {"request_timeout_seconds": number("nan")})
    add("request_timeout.negative", {"request_timeout_seconds": number("-1.0")})
    add("request_timeout.text.invalid", {"request_timeout_seconds": text("abc")})
    add("request_timeout.text.empty", {"request_timeout_seconds": text("")})
    add("request_timeout.text.infinity", {"request_timeout_seconds": text("inf")})
    add("request_timeout.text.nan", {"request_timeout_seconds": text("nan")})
    add("request_timeout.text.underscore", {"request_timeout_seconds": text("1_000")})
    add("request_timeout.text.padded", {"request_timeout_seconds": text(" 1.5 ")})
    add("request_timeout.boolean", {"request_timeout_seconds": boolean(True)})
    add("request_timeout.null", {"request_timeout_seconds": null()})

    add("tool_timeout.default", {"tool_timeout_seconds": number("300.0")})
    add("tool_timeout.text", {"tool_timeout_seconds": text("30")})
    add("tool_timeout.infinity", {"tool_timeout_seconds": number("inf")})
    add("tool_timeout.negative", {"tool_timeout_seconds": number("-0.5")})

    add("worker_count.default", {"worker_count": integer(4)})
    add("worker_count.text", {"worker_count": text("4")})
    add("worker_count.text.padded", {"worker_count": text(" 4 ")})
    add("worker_count.text.zeros", {"worker_count": text("0004")})
    add("worker_count.minimum", {"worker_count": integer(1)})
    add("worker_count.maximum", {"worker_count": integer(8)})
    add("worker_count.high", {"worker_count": integer(9)})
    add("worker_count.low", {"worker_count": integer(0)})
    add("worker_count.negative", {"worker_count": integer(-1)})
    add("worker_count.float", {"worker_count": number("4.0")})
    add("worker_count.boolean", {"worker_count": boolean(True)})
    add("worker_count.text.invalid", {"worker_count": text("four")})
    add("worker_count.text.negative", {"worker_count": text("-1")})
    add("worker_count.null", {"worker_count": null()})

    add("extra.forbidden", {"nope": integer(1)})
    add("extra.two", {"nope": integer(1), "other": text("x")})
    add(
        "multiple.errors",
        {"model_provider": text(""), "worker_count": integer(9), "nope": integer(1)},
    )
    add(
        "multiple.errors.order",
        {"worker_count": integer(9), "model_provider": text("")},
    )
    add(
        "multiple.timeouts",
        {
            "request_timeout_seconds": text("abc"),
            "tool_timeout_seconds": text("xyz"),
        },
    )
    return cases


def evaluate(case: dict[str, Any]) -> dict[str, Any]:
    """Run one case through the live settings model."""
    values = {name: to_python(value) for name, value in case["values"].items()}
    try:
        settings = Settings.model_validate(values)
    except ValidationError as error:
        return {
            "name": case["name"],
            "ok": False,
            "errors": [
                {
                    "kind": failure["type"],
                    "location": ".".join(str(part) for part in failure["loc"]),
                    "message": failure["msg"],
                }
                for failure in error.errors()
            ],
        }
    return {
        "name": case["name"],
        "ok": True,
        "settings": {
            "data_dir": str(settings.data_dir),
            "model_provider": settings.model_provider,
            "model_id": settings.model_id,
            "model_endpoint": (
                None if settings.model_endpoint is None else str(settings.model_endpoint)
            ),
            "request_timeout_seconds": settings.request_timeout_seconds,
            "tool_timeout_seconds": settings.tool_timeout_seconds,
            "worker_count": settings.worker_count,
            "artifact_dir": str(settings.artifact_dir),
            "journal_path": str(settings.journal_path),
            "database_path": str(settings.database_path),
            "transcript_dir": str(settings.transcript_dir),
        },
    }


def environment_cases() -> list[dict[str, Any]]:
    """The environment corpus: the only source the Python model reads."""
    return [
        {"name": "env.empty", "environ": {}},
        {"name": "env.worker", "environ": {"KALA_AGENT_WORKER_COUNT": "2"}},
        {
            "name": "env.full",
            "environ": {
                "KALA_AGENT_WORKER_COUNT": "8",
                "KALA_AGENT_MODEL_PROVIDER": "LOCAL",
                "KALA_AGENT_MODEL_ID": "local-model",
                "KALA_AGENT_MODEL_ENDPOINT": "http://127.0.0.1:8080/v1",
                "KALA_AGENT_REQUEST_TIMEOUT_SECONDS": "12.5",
                "KALA_AGENT_TOOL_TIMEOUT_SECONDS": "30",
                "KALA_AGENT_MODEL_API_KEY": "do-not-retain-this-secret",
                "OPENAI_API_KEY": "do-not-retain-this-secret",
            },
        },
        {"name": "env.worker.invalid", "environ": {"KALA_AGENT_WORKER_COUNT": "nine"}},
        {"name": "env.worker.high", "environ": {"KALA_AGENT_WORKER_COUNT": "9"}},
        {"name": "env.timeout.invalid", "environ": {"KALA_AGENT_REQUEST_TIMEOUT_SECONDS": "abc"}},
        {
            "name": "env.endpoint.credentials",
            "environ": {"KALA_AGENT_MODEL_ENDPOINT": "https://user:pass@example.com/v1"},
        },
    ]


def evaluate_environment(case: dict[str, Any]) -> dict[str, Any]:
    """Run one environment case through the live settings model."""
    try:
        settings = Settings.from_env(case["environ"])
    except ValidationError as error:
        return {
            "name": case["name"],
            "ok": False,
            "errors": [
                {
                    "kind": failure["type"],
                    "location": ".".join(str(part) for part in failure["loc"]),
                    "message": failure["msg"],
                }
                for failure in error.errors()
            ],
        }
    return {
        "name": case["name"],
        "ok": True,
        "settings": {
            "data_dir": str(settings.data_dir),
            "model_provider": settings.model_provider,
            "model_id": settings.model_id,
            "model_endpoint": (
                None if settings.model_endpoint is None else str(settings.model_endpoint)
            ),
            "request_timeout_seconds": settings.request_timeout_seconds,
            "tool_timeout_seconds": settings.tool_timeout_seconds,
            "worker_count": settings.worker_count,
        },
    }


def main(argv: list[str]) -> int:
    if len(argv) not in (3, 4):
        print(
            "usage: oracle_config.py <corpus.json> <results.json> [work-directory]",
            file=sys.stderr,
        )
        return 2
    if len(argv) == 4:
        work = Path(argv[3])
        work.mkdir(parents=True, exist_ok=True)
        cases = build_cases(work)
        corpus = {
            "cases": cases,
            "environment_cases": environment_cases(),
            "data_dir": str(work / "data"),
        }
        results = [evaluate(case) for case in cases]
        results.extend(evaluate_environment(case) for case in corpus["environment_cases"])
    else:
        with tempfile.TemporaryDirectory(prefix="kala-api-config-") as workdir:
            work = Path(workdir)
            cases = build_cases(work)
            corpus = {
                "cases": cases,
                "environment_cases": environment_cases(),
                "data_dir": str(work / "data"),
            }
            results = [evaluate(case) for case in cases]
            results.extend(
                evaluate_environment(case) for case in corpus["environment_cases"]
            )
    corpus_path = Path(argv[1])
    results_path = Path(argv[2])
    corpus_path.parent.mkdir(parents=True, exist_ok=True)
    corpus_path.write_text(json.dumps(corpus, indent=1, sort_keys=True), encoding="utf-8")
    results_path.write_text(json.dumps(results, indent=1, sort_keys=True), encoding="utf-8")
    print(f"oracle configuration cases: {len(results)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
