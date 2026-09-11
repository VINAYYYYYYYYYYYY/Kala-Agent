"""Procedure exit_criteria gate."""

from __future__ import annotations

from kala.agent.loop import _exit_criteria_met
from kala.procedures import load_default_procedure
from kala.session.state import SessionState, ToolEvent


def _state(**kwargs):
    p = load_default_procedure("simple_bracket")
    return SessionState(
        goal="test",
        backend_name="mock",
        standard_parts=False,
        procedure=p,
        **kwargs,
    )


def test_envelope_blocked_without_create():
    st = _state()
    assert _exit_criteria_met(st) is False


def test_envelope_met_after_create_box():
    st = _state()
    st.history.append(ToolEvent("create_box", {}, True, "ok"))
    assert _exit_criteria_met(st) is True
