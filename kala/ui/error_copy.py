"""Friendly error copy for UI failures (never leak API keys)."""

from __future__ import annotations


def sanitize_error(raw_message: str) -> str:
    """Map known failure patterns to user-friendly copy; sanitize key material."""
    msg = raw_message.strip()
    
    if not msg:
        return "Run failed"
    
    lower = msg.lower()
    
    if "timeout" in lower or "15 minute" in lower:
        return "Run timed out — hit the 15-minute limit. Try a shorter brief or check if FreeCAD is stuck."
    
    if "api" in lower and ("key" in lower or "auth" in lower or "401" in lower or "403" in lower):
        return "API key issue — check ··· → OpenRouter API… and verify your key is valid."
    
    if "freecad" in lower and ("missing" in lower or "not found" in lower or "import" in lower):
        return "FreeCAD isn't available — install it or switch Backend → Mock to test the flow."
    
    if "json" in lower and "parse" in lower:
        return "Couldn't read the run result — check 'kala logs'. Usually a crashed FreeCAD backend."
    
    if "connection" in lower or "network" in lower or "timeout" in lower:
        return "Network issue — check your connection and the API base URL."
    
    last_line = msg.splitlines()[-1] if "\n" in msg else msg
    
    sanitized = _strip_secrets(last_line)
    
    if len(sanitized) <= 140:
        return sanitized
    
    return sanitized[:137] + "…"


def _strip_secrets(text: str) -> str:
    """Remove API key patterns from error text."""
    import re
    
    redacted = re.sub(r'sk-[a-zA-Z0-9_-]{20,}', '[REDACTED]', text)
    redacted = re.sub(r'Bearer\s+[a-zA-Z0-9_\-\.]+', 'Bearer [REDACTED]', redacted)
    redacted = re.sub(r'"api_key"\s*:\s*"[^"]*"', '"api_key": "[REDACTED]"', redacted)
    redacted = re.sub(r"'api_key'\s*:\s*'[^']*'", "'api_key': '[REDACTED]'", redacted)
    
    return redacted
