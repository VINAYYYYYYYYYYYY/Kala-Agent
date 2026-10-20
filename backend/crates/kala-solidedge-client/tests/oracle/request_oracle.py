"""Live Python oracle for the build request bytes and digest.

The request digest is the journal's identity for one semantic submission: a
retry reuses a request id only when the whole canonical request is identical.
This oracle prints the canonical bytes, the digest, and the wire body the
Python client would send, so the Rust side can prove the three agree.

Usage::

    python -I request_oracle.py <request-json-path>
"""

from __future__ import annotations

import json
import os
import sys


def _repository_root() -> str:
    if len(sys.argv) > 2:
        return os.path.abspath(sys.argv[2])
    here = os.path.dirname(os.path.abspath(__file__))
    return os.path.abspath(os.path.join(here, "..", "..", "..", "..", ".."))


def _prepare(root: str) -> None:
    for part in (
        "src",
        os.path.join("adapters", "solidedge", "src"),
        os.path.join("adapters", "solidedge-bridge", "src"),
    ):
        path = os.path.join(root, part)
        if path not in sys.path:
            sys.path.insert(0, path)


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: request_oracle.py <request-json-path> [repository-root]", file=sys.stderr)
        return 2
    root = _repository_root()
    _prepare(root)

    from kala_solidedge_bridge.protocol import BuildRequest

    with open(sys.argv[1], encoding="utf-8") as handle:
        text = handle.read()
    try:
        request = BuildRequest.model_validate_json(text)
    except Exception as error:  # noqa: BLE001 - the message is the evidence
        json.dump({"error": str(error)}, sys.stdout, sort_keys=True)
        sys.stdout.write("\n")
        return 0
    json.dump(
        {
            "canonical": request.canonical_request_bytes().decode("utf-8"),
            "digest": request.request_digest(),
            "wire": request.model_dump_json(),
        },
        sys.stdout,
        sort_keys=True,
    )
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
