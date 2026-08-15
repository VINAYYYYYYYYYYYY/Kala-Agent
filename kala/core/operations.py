"""Operations module."""
from enum import Enum, auto
from dataclasses import dataclass
from typing import Dict, Any

class Operation(Enum):
    """Supported CAD operations."""
    CREATE_BOX = auto()
    CREATE_CYLINDER = auto()
    CREATE_SPHERE = auto()
    CREATE_CONE = auto()
    BOOLEAN_FUSE = auto()
    BOOLEAN_CUT = auto()
    BOOLEAN_COMMON = auto()
    TRANSLATE = auto()
    ROTATE = auto()
    FILLET = auto()
    CHAMFER = auto()
    EXPORT = auto()

@dataclass
class OperationRequest:
    """A request to perform an operation."""
    operation: Operation
    params: Dict[str, Any]
