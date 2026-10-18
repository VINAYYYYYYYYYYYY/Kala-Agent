"""``python -m kala_agent.studio`` entry point."""

from __future__ import annotations

import sys

from kala_agent.studio.cli import main

if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))


__all__ = ("main",)
