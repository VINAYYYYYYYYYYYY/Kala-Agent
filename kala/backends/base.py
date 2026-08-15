"""Base backend module."""
from abc import ABC, abstractmethod
from typing import List, Dict, Any

class BaseBackend(ABC):
    """The interface that every CAD backend must implement."""
    
    @abstractmethod
    def create_box(self, length: float, width: float, height: float) -> str:
        """Create a box."""
        pass
        
    @abstractmethod
    def create_cylinder(self, radius: float, height: float) -> str:
        """Create a cylinder."""
        pass
        
    @abstractmethod
    def create_sphere(self, radius: float) -> str:
        """Create a sphere."""
        pass
        
    @abstractmethod
    def boolean_fuse(self, body_a: str, body_b: str) -> str:
        """Fuse two bodies."""
        pass
        
    @abstractmethod
    def boolean_cut(self, body_a: str, body_b: str) -> str:
        """Cut body_b from body_a."""
        pass
        
    @abstractmethod
    def translate(self, body_id: str, x: float, y: float, z: float) -> str:
        """Translate a body."""
        pass
        
    @abstractmethod
    def rotate(self, body_id: str, axis: str, angle: float) -> str:
        """Rotate a body."""
        pass
        
    @abstractmethod
    def fillet(self, body_id: str, radius: float) -> str:
        """Fillet a body."""
        pass
        
    @abstractmethod
    def export(self, body_id: str, format: str, path: str) -> str:
        """Export a body."""
        pass
        
    @abstractmethod
    def list_bodies(self) -> List[Dict[str, Any]]:
        """List all bodies."""
        pass
