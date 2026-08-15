"""Core tests."""
import pytest
from kala.core.geometry import Point3D
from kala.core.operations import Operation
from kala.core.agent import KalaAgent
from kala.backends.base import BaseBackend

class MockBackend(BaseBackend):
    def create_box(self, length, width, height): pass
    def create_cylinder(self, radius, height): pass
    def create_sphere(self, radius): pass
    def boolean_fuse(self, body_a, body_b): pass
    def boolean_cut(self, body_a, body_b): pass
    def translate(self, body_id, x, y, z): pass
    def rotate(self, body_id, axis, angle): pass
    def fillet(self, body_id, radius): pass
    def export(self, body_id, format, path): pass
    def list_bodies(self): return []

def test_point3d_creation():
    pt = Point3D(1.0, 2.0, 3.0)
    assert pt.x == 1.0
    assert pt.y == 2.0
    assert pt.z == 3.0

def test_operation_enum():
    assert hasattr(Operation, "CREATE_BOX")
    assert hasattr(Operation, "EXPORT")

def test_agent_instantiation():
    backend = MockBackend()
    agent = KalaAgent(backend)
    assert agent.backend == backend

def test_base_backend_abstract():
    with pytest.raises(TypeError):
        BaseBackend()
