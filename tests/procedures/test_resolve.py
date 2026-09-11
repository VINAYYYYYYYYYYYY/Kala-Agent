"""resolve_procedure_for_send — explicit pick must not be overridden by suggest."""

from __future__ import annotations

import pytest

from kala.procedures import assess_goal, resolve_procedure_for_send


def test_resolve_respects_explicit_pick():
    goal = "stepped shaft OD 20mm"
    assert resolve_procedure_for_send(goal, "simple_bracket", user_picked=True) == "simple_bracket"


def test_resolve_suggests_when_not_picked():
    goal = "stepped shaft OD 20mm"
    assert resolve_procedure_for_send(goal, "simple_bracket", user_picked=False) == "stepped_shaft"


def test_resolve_keeps_pick_when_suggest_would_differ():
    goal = "housing cover with central bore"
    assert resolve_procedure_for_send(goal, "plate_with_holes", user_picked=True) == "plate_with_holes"


def test_resolve_rejects_gated_goal_even_when_user_picked():
    goal = "16 inch laptop"
    assert assess_goal(goal) is not None
    with pytest.raises(ValueError, match="gated goal"):
        resolve_procedure_for_send(goal, "simple_bracket", user_picked=True)


def test_resolve_rejects_part_plan_goal():
    goal = "multi-part housing and cover assembly"
    assert assess_goal(goal) is not None
    with pytest.raises(ValueError, match="gated goal"):
        resolve_procedure_for_send(goal, "housing_cover", user_picked=False)
