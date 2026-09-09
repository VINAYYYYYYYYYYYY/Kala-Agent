"""Provider configuration and API key management."""
import json
import os
from pathlib import Path
from typing import Optional


def looks_like_api_key(key: Optional[str]) -> bool:
    """Check if a string looks like a valid API key.
    
    Args:
        key: The string to check
        
    Returns:
        True if the key looks valid (non-empty, reasonable format)
    """
    if not key or not isinstance(key, str):
        return False
    
    key = key.strip()
    if not key:
        return False
    
    # Reject if it looks like a URL
    if key.startswith(("http://", "https://", "www.")):
        return False
    
    # Accept keys that look like OpenAI format (sk-...) or similar provider formats
    # Minimum reasonable length
    if len(key) < 20:
        return False
    
    return True


def load_store() -> dict:
    """Load the provider configuration store.
    
    Returns:
        Dictionary with provider configuration including API keys
    """
    store_path = Path.home() / ".kala" / "providers.json"
    
    if not store_path.exists():
        return {}
    
    try:
        with open(store_path, "r") as f:
            return json.load(f)
    except (json.JSONDecodeError, OSError):
        return {}


def save_store(store: dict) -> None:
    """Save the provider configuration store.
    
    Args:
        store: Dictionary with provider configuration
    """
    store_path = Path.home() / ".kala" / "providers.json"
    store_path.parent.mkdir(parents=True, exist_ok=True)
    
    with open(store_path, "w") as f:
        json.dump(store, f, indent=2)


def get_planner_key() -> Optional[str]:
    """Get the planner API key from the store.
    
    Returns:
        The API key if configured, None otherwise
    """
    store = load_store()
    return store.get("planner_api_key")
