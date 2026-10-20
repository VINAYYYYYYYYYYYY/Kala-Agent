"""Test-only oracle for kala-process compatibility.

This script imports the legacy Python implementation from ``src`` and returns
observable values for comparison. It is never used by the Rust product path.
"""

from __future__ import annotations

import base64
import json
import subprocess
import sys
from pathlib import Path

from kala_agent.tools import sandbox
from kala_agent.tools.protocol import build_child_environment, resolve_workspace_path


def main() -> int:
    request = json.load(sys.stdin)
    workspace = Path(request["workspace"])
    import_roots = [Path(item) for item in request.get("import_roots", [])]
    result: dict[str, object] = {}
    result["platform"] = sandbox.platform_name()

    probes = []
    for item in request.get("probes", []):
        probe = sandbox.SandboxProbe(
            available=bool(item["available"]),
            platform=str(item["platform"]),
            controls=tuple(item.get("controls", [])),
            detail=str(item.get("detail", "")),
            elapsed_ms=float(item.get("elapsed_ms", 0.0)),
        )
        probes.append(probe.to_payload())
    result["probes"] = probes

    spec = sandbox.SandboxSpec.from_payload(request["spec"])
    result["spec_payload"] = spec.to_payload()

    verdicts = []
    for item in request.get("judgements", []):
        profile = sandbox.IsolationProfile(item["profile"])
        probe_payload = item.get("probe")
        probe = None
        if probe_payload is not None:
            probe = sandbox.SandboxProbe(
                available=bool(probe_payload["available"]),
                platform=str(probe_payload["platform"]),
                controls=tuple(probe_payload.get("controls", [])),
                detail=str(probe_payload.get("detail", "")),
                elapsed_ms=float(probe_payload.get("elapsed_ms", 0.0)),
            )
        verdict = sandbox.judge_sandbox(profile, probe)
        verdicts.append(verdict.to_payload())
    result["judgements"] = verdicts

    environment = build_child_environment(workspace=workspace, import_roots=import_roots)
    result["environment"] = environment

    sanitized = []
    for item in request.get("sanitize", []):
        payload = base64.b64decode(item["payload"])
        from kala_agent.tools.protocol import sanitize_child_stderr

        sanitized.append(
            sanitize_child_stderr(payload, replacement=item.get("replacement"))
        )
    result["sanitized"] = sanitized

    workspace_paths = []
    for candidate in request.get("workspace_paths", []):
        try:
            resolved = resolve_workspace_path(workspace, candidate)
        except Exception as error:  # noqa: BLE001 - oracle reports the class only
            workspace_paths.append({"ok": False, "error": type(error).__name__})
        else:
            workspace_paths.append({"ok": True, "path": resolved.as_posix()})
    result["workspace_paths"] = workspace_paths

    signal_fixture = request.get("signal_fixture")
    if signal_fixture:
        completed = subprocess.run(
            [signal_fixture, "self-signal", "9"],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        result["signal_returncode"] = completed.returncode

    json.dump(result, sys.stdout, sort_keys=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
