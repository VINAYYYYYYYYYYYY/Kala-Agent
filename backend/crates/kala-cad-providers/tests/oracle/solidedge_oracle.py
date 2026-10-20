"""Live Python oracle for the native Solid Edge provider differential.

Test-only. Runs the repository's own `SolidEdgeProvider` for a list of
constructor/probe/build cases and prints the observed availability and
refusals. The Rust test compares its provider with this document.

The script is run with an isolated interpreter (``python -I``) and inserts the
repository paths itself.

Request (stdin)::

    {"cases": [{"id": "...", "platform": "win32"|null, "environ": {...},
                "url": "..."|null, "token_file": "..."|null,
                "probe": true, "build": false}]}
"""

from __future__ import annotations

import json
import os
import sys
from typing import Any


def _prepare(root: str) -> None:
    path = os.path.join(root, "src")
    if path not in sys.path:
        sys.path.insert(0, path)


def _availability(availability: Any) -> dict[str, Any]:
    return {
        "available": availability.available,
        "version": availability.version,
        "reason": availability.reason,
    }


def _build(provider: Any) -> dict[str, Any]:
    from kala_agent.contracts.construction import ConstructionScript
    from kala_agent.studio.design.intent import DesignIntent
    from kala_agent.studio.design.providers.base import (
        CadProviderBuildError,
        CadProviderUnavailableError,
    )

    script = ConstructionScript.model_validate(
        {
            "summary": "oracle pad",
            "components": [
                {
                    "name": "body",
                    "steps": [
                        {
                            "op": "sketch",
                            "plane": "XY",
                            "profile": "rectangle",
                            "width": 4,
                            "height": 5,
                        },
                        {"op": "extrude", "length": 6},
                    ],
                }
            ],
        }
    )
    try:
        build = provider.build(script, DesignIntent(script=script))
    except CadProviderUnavailableError as error:
        return {
            "kind": "unavailable",
            "message": str(error),
            "reason": error.reason,
            "provider": error.provider,
        }
    except CadProviderBuildError as error:
        return {
            "kind": "build",
            "message": str(error),
            "op": error.op,
        }
    return {
        "kind": "built",
        "step_size": len(build.step_bytes),
        "native_kind": build.native_kind,
    }


def main() -> int:
    root = (
        os.path.abspath(sys.argv[1])
        if len(sys.argv) > 1
        else os.path.abspath(
            os.path.join(os.path.dirname(__file__), "..", "..", "..", "..", "..")
        )
    )
    _prepare(root)
    request = json.loads(sys.stdin.read())

    from kala_agent.studio.design.providers.solidedge import SolidEdgeProvider

    response: dict[str, Any] = {"cases": []}
    for case in request.get("cases") or []:
        provider = SolidEdgeProvider(
            platform=case.get("platform"),
            environ=dict(case.get("environ") or {}),
            url=case.get("url"),
            token_file=case.get("token_file"),
            timeout_seconds=2.0,
            probe_cache_seconds=0,
        )
        result: dict[str, Any] = {"id": case["id"]}
        if case.get("probe"):
            try:
                result["availability"] = _availability(provider.probe())
            except ValueError as error:
                result["exception"] = type(error).__name__ + ": " + str(error)
        if case.get("build"):
            result["build"] = _build(provider)
        response["cases"].append(result)
    print(
        json.dumps(
            response,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=True,
            allow_nan=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
