#!/usr/bin/env python3
"""Test-only oracle: write a Studio journal with the Python implementation.

This script is *not* part of the product runtime. It lets the native design
service prove that it can **cold reload** a design that the Python
implementation stored: the Python service writes the journal and the content
addressed artifacts, and the native service then loads the design back from
that journal without rebuilding it.

Usage:
    python python_journal_oracle.py <data-dir> <result-file>

The script prints one JSON document with the design id, the digest of the
stored STEP artifact and the measured numbers, so the Rust test can assert
that the native cold reload produced the same facts.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
SRC = REPO / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

from kala_agent.studio.design.intent import DesignIntent  # noqa: E402
from kala_agent.studio.design.service import DesignService  # noqa: E402

BRACKET = {
    "template": "l_bracket",
    "width": 60.0,
    "depth": 45.0,
    "height": 40.0,
    "wall": 6.0,
}


def main() -> int:
    data_dir = Path(sys.argv[1])
    result_file = Path(sys.argv[2])
    service = DesignService(data_dir, environ={})
    intent = DesignIntent.model_validate({"template": BRACKET})
    outcome = service.design(intent, provider="opencascade")
    service.close()
    # The OpenCascade STEP writer prints its transfer statistics to stdout, so
    # the result is written to a file instead of to the standard output.
    result_file.write_text(
        json.dumps(
            {
                "design_id": outcome.design_id,
                "step_sha256": outcome.step_sha256,
                "script_sha256": outcome.script_sha256,
                "volume_mm3": outcome.measurement.volume_mm3,
                "surface_area_mm2": outcome.measurement.area_mm2,
                "solid_count": outcome.solid_count,
                "valid": outcome.valid,
                "verified": outcome.verified,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
