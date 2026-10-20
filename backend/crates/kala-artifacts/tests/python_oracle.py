"""Live Python oracle used by the Rust cross-reader integration tests.

This script only drives the real
``kala_agent.persistence.artifacts.ContentAddressedArtifactStore`` so the Rust
crate can be checked against the actual Python implementation in both
directions.  It is test-only and is never imported or shipped by the library.

Usage:
    python_oracle.py create <root> <max> <payload-hex>...
    python_oracle.py read   <root> <max> <digest>...
    python_oracle.py key    <root> <max> <digest>...

Output is one line per request item (fields are separated by `|` so an empty
payload still yields a complete record):
    create -> "ok|<digest>|<size>|<key>"
    read   -> "ok|<payload-hex>|<size>|<contains>" | "missing"
    key    -> "ok|<key>"
"""

from __future__ import annotations

import sys
from pathlib import Path

from kala_agent.persistence.artifacts import ContentAddressedArtifactStore


def main(argv: list[str]) -> int:
    if len(argv) < 4:
        print("usage: python_oracle.py <create|read|key> <root> <max> <args...>", file=sys.stderr)
        return 2
    mode = argv[1]
    root = Path(argv[2])
    max_object_size = int(argv[3])
    store = ContentAddressedArtifactStore(root, max_object_size=max_object_size)
    lines: list[str] = []
    if mode == "create":
        for payload_hex in argv[4:]:
            payload = bytes.fromhex(payload_hex)
            digest = store.put_bytes(payload)
            lines.append(f"ok|{digest}|{len(payload)}|{store.key_for(digest)}")
    elif mode == "read":
        for digest in argv[4:]:
            try:
                payload = store.get_bytes(digest)
            except FileNotFoundError:
                lines.append("missing")
                continue
            lines.append(
                f"ok|{payload.hex()}|{store.get_size(digest)}|{store.contains(digest)}"
            )
    elif mode == "key":
        for digest in argv[4:]:
            lines.append(f"ok|{store.key_for(digest)}")
    else:
        print(f"unknown mode {mode}", file=sys.stderr)
        return 2
    sys.stdout.write("\n".join(lines))
    if lines:
        sys.stdout.write("\n")
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
