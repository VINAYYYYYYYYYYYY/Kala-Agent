"""Agent module."""
from typing import Any, Dict
from kala.backends.base import BaseBackend

class KalaAgent:
    """The core brain that works with any backend."""
    
    def __init__(self, backend: BaseBackend) -> None:
        """Initialize the agent with a CAD backend.
        
        Args:
            backend (BaseBackend): The CAD backend to use.
        """
        self.backend = backend
        
    def process(self, instruction: str) -> Dict[str, Any]:
        """Process an instruction and dispatch to the backend.
        
        Args:
            instruction (str): The instruction to process.
            
        Returns:
            Dict[str, Any]: A dictionary containing the processing status.
        """
        return {"instruction": instruction, "status": "not_implemented"}
