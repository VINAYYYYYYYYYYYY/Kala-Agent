"""Abstract geometry types."""
from dataclasses import dataclass

@dataclass
class Point3D:
    """A point in 3D space."""
    x: float
    y: float
    z: float

@dataclass
class Vector3D:
    """A vector in 3D space."""
    x: float
    y: float
    z: float

@dataclass
class BoundingBox:
    """A bounding box in 3D space."""
    min_pt: Point3D
    max_pt: Point3D
