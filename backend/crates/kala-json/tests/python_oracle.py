#!/usr/bin/env python3
"""Test-only Python oracle for the Rust canonical JSON implementation.

The production crate never invokes Python. This script runs under the
repository virtual environment (or any interpreter with `kala_agent`
importable) and compares against the live product implementation, not a copy.

Modes:
  floats  read one 16-digit hex f64 bit pattern per line, print the product's
          `canonical_json_dumps(value)` representation per line
  cases   read a JSON array of {"text": "<json source>"} objects, strictly parse
          and canonicalize each with the product, print a JSON array of results
"""

from __future__ import annotations

import json
import struct
import sys
from pathlib import Path


def _add_repo_source() -> None:
    repo = Path(__file__).resolve().parents[4]
    source = repo / "src"
    if source.is_dir() and str(source) not in sys.path:
        sys.path.insert(0, str(source))


def main() -> int:
    _add_repo_source()
    from kala_agent.tools.protocol import (  # noqa: PLC0415
        canonical_json_dumps,
        strict_json_loads,
    )

    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode == "floats":
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            bits = int(line, 16)
            value = struct.unpack(">d", bits.to_bytes(8, "big"))[0]
            print(canonical_json_dumps(value))
        return 0

    if mode == "cases":
        payload = json.load(sys.stdin)
        results = []
        for case in payload:
            text = case["text"]
            try:
                value = strict_json_loads(text.encode("utf-8"))
                results.append({"ok": True, "output": canonical_json_dumps(value)})
            except Exception as error:  # noqa: BLE001 - oracle reports exact type/message
                results.append(
                    {
                        "ok": False,
                        "error_type": type(error).__name__,
                        "message": str(error),
                    }
                )
        json.dump(results, sys.stdout, ensure_ascii=True)
        return 0

    print(f"unknown mode: {mode!r}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
