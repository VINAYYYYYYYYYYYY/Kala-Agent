"""Explicitly gated live Windows test. Never enabled by default.

Set ``KALA_SOLIDEDGE_LIVE=1`` in a logged-in Windows desktop session with Solid
Edge installed to run it. It only attaches and reads ``Application.Version``; it
creates no document and performs no feature operation.
"""

from __future__ import annotations

import os
import sys
from typing import Any

import pytest

from kala_solidedge_bridge.session import Session

pytestmark = pytest.mark.solidedge

_LIVE = os.environ.get("KALA_SOLIDEDGE_LIVE") == "1"


@pytest.mark.skipif(
    sys.platform != "win32" or not _LIVE,
    reason="live Solid Edge tests require Windows and KALA_SOLIDEDGE_LIVE=1",
)
def test_live_attach_reports_a_version_without_creating_a_document() -> None:
    session = Session()
    session.start()

    def attach_and_read(binding: Any) -> str:  # noqa: ANN401
        application = binding.attach(False)
        return str(binding.version(application))

    try:
        version = session.run(attach_and_read, timeout=30.0)
    finally:
        assert session.stop() is True
    assert version and version != "unknown"
