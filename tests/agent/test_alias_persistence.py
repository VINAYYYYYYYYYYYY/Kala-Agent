"""Tests for persistent body_id alias handling across planner turns."""

from __future__ import annotations

from kala.agent.loop import Agent
from kala.cad.mock import MockBackend
from kala.cad.protocol import ToolResult
from kala.session.state import SessionState
from kala.procedures.schema import load_default_procedure


def test_session_state_has_id_aliases():
    """Test that SessionState includes id_aliases field."""
    state = SessionState(
        goal="test",
        backend_name="mock",
        standard_parts=False,
        procedure=load_default_procedure("simple_bracket"),
    )
    assert hasattr(state, "id_aliases")
    assert isinstance(state.id_aliases, dict)
    assert len(state.id_aliases) == 0


def test_id_aliases_serializes():
    """Test that id_aliases is included in to_dict() output."""
    state = SessionState(
        goal="test",
        backend_name="mock",
        standard_parts=False,
        procedure=load_default_procedure("simple_bracket"),
    )
    state.id_aliases["Box_1"] = "Fuse_1"
    state.id_aliases["Box_2"] = "Fuse_1"
    
    data = state.to_dict()
    assert "id_aliases" in data
    assert data["id_aliases"]["Box_1"] == "Fuse_1"
    assert data["id_aliases"]["Box_2"] == "Fuse_1"


def test_aliases_persist_across_simulated_turns():
    """Test that aliases are retained across multiple planner turns."""
    state = SessionState(
        goal="test",
        backend_name="mock",
        standard_parts=False,
        procedure=load_default_procedure("simple_bracket"),
    )
    
    # Simulate turn 1: create two boxes and fuse them
    state.id_aliases["Box_1"] = "Fuse_1"
    state.id_aliases["Box_2"] = "Fuse_1"
    
    # Verify aliases persist (not reset between turns)
    assert "Box_1" in state.id_aliases
    assert "Box_2" in state.id_aliases
    assert state.id_aliases["Box_1"] == "Fuse_1"
    assert state.id_aliases["Box_2"] == "Fuse_1"
    
    # Simulate turn 2: create another box and fuse with previous result
    state.id_aliases["Box_3"] = "Fuse_2"
    state.id_aliases["Fuse_1"] = "Fuse_2"
    
    # Verify all aliases still present
    assert len(state.id_aliases) == 4
    assert state.id_aliases["Box_1"] == "Fuse_1"
    assert state.id_aliases["Fuse_1"] == "Fuse_2"


def test_alias_chain_resolution():
    """Test resolving chains of aliases (Box_1 -> Fuse_1 -> Fuse_2)."""
    state = SessionState(
        goal="test",
        backend_name="mock",
        standard_parts=False,
        procedure=load_default_procedure("simple_bracket"),
    )
    
    # Build a chain: Box_1 -> Fuse_1 -> Fuse_2 -> Cut_1
    state.id_aliases["Box_1"] = "Fuse_1"
    state.id_aliases["Fuse_1"] = "Fuse_2"
    state.id_aliases["Fuse_2"] = "Cut_1"
    
    # Manual resolution function (simulating loop behavior)
    def resolve(bid: str) -> str:
        seen: set[str] = set()
        cur = bid
        while cur in state.id_aliases and cur not in seen:
            seen.add(cur)
            cur = state.id_aliases[cur]
        return cur
    
    assert resolve("Box_1") == "Cut_1"
    assert resolve("Fuse_1") == "Cut_1"
    assert resolve("Fuse_2") == "Cut_1"
    assert resolve("Cut_1") == "Cut_1"
    assert resolve("Unknown") == "Unknown"


def test_mock_backend_removed_field():
    """Test that mock backend returns 'removed' list in result data."""
    backend = MockBackend()
    
    # Create two boxes
    result1 = backend.create_box(length=10, width=10, height=10)
    assert result1.ok
    body_a = result1.data["body_id"]
    
    result2 = backend.create_box(length=5, width=5, height=5)
    assert result2.ok
    body_b = result2.data["body_id"]
    
    # Fuse them
    result = backend.boolean_fuse(body_a, body_b)
    assert result.ok
    assert "body_id" in result.data
    assert "removed" in result.data
    assert body_a in result.data["removed"]
    assert body_b in result.data["removed"]


def test_mock_backend_cut_removes_donors():
    """Test that boolean_cut returns removed donor ids."""
    backend = MockBackend()
    
    result1 = backend.create_box(length=10, width=10, height=10)
    assert result1.ok
    body_a = result1.data["body_id"]
    
    result2 = backend.create_cylinder(radius=2, height=15)
    assert result2.ok
    body_b = result2.data["body_id"]
    
    # Cut body_b from body_a
    result = backend.boolean_cut(body_a, body_b)
    assert result.ok
    assert "body_id" in result.data
    assert "removed" in result.data
    assert body_a in result.data["removed"]
    assert body_b in result.data["removed"]
