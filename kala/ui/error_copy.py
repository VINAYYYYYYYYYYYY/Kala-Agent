"""Friendly error copy for UI failures (never leak API keys)."""

from __future__ import annotations

import re


def sanitize_error(raw_message: str) -> str:
    """Map known failure patterns to user-friendly copy; sanitize key material."""
    msg = raw_message.strip()

    if not msg:
        return "Run failed"

    lower = msg.lower()

    if "timeout" in lower or "15 minute" in lower:
        return (
            "Run timed out (15-minute limit). Try a shorter brief or check if FreeCAD is stuck."
        )

    if "api" in lower and ("key" in lower or "auth" in lower or "401" in lower or "403" in lower):
        return provider_fail_copy(msg)

    if "freecad" in lower and ("missing" in lower or "not found" in lower or "import" in lower):
        return "FreeCAD isn't available. Install it or switch Backend → Mock to test the flow."

    if "json" in lower and "parse" in lower:
        return "Couldn't read the run result. Check 'kala logs'. Usually a crashed FreeCAD backend."

    if "connection" in lower or "network" in lower or "timeout" in lower:
        return "Network issue. Check your connection and the API base URL."

    last_line = msg.splitlines()[-1] if "\n" in msg else msg
    sanitized = _strip_secrets(last_line)

    if len(sanitized) <= 140:
        return sanitized

    return sanitized[:137] + "…"


def provider_fail_copy(raw_message: str) -> str:
    """Map OpenRouter / provider catalog failures to short UI copy."""
    msg = raw_message.strip()
    if not msg:
        return "Could not load OpenRouter models."

    lower = msg.lower()

    if "requires an api key" in lower or (
        "api key" in lower
        and ("missing" in lower or "required" in lower or "empty" in lower)
    ):
        return (
            "Paste your OpenRouter API key (sk-or-…) under ··· → OpenRouter API. "
            "Get one at https://openrouter.ai/keys."
        )

    if "rate" in lower and ("limit" in lower or "429" in lower or "too many" in lower):
        return "Rate limited. Wait a moment and try again, or pick a different model."

    if "quota" in lower or "credit" in lower or "balance" in lower or "insufficient" in lower:
        return "Out of credits. Top up at openrouter.ai/credits or pick a free model."

    if "model" in lower and ("not" in lower or "unavail" in lower or "404" in lower):
        return "Model not available. It may be down or removed; try another model."

    if "500" in lower or "502" in lower or "503" in lower:
        return "OpenRouter is having server trouble. Try again in a few minutes."

    if (
        "401" in lower
        or "403" in lower
        or "unauthorized" in lower
        or "forbidden" in lower
        or ("auth" in lower and ("fail" in lower or "invalid" in lower))
    ):
        return (
            "OpenRouter rejected this API key. Open ··· → OpenRouter API and paste a "
            "valid key from https://openrouter.ai/keys."
        )

    if "api" in lower and "key" in lower:
        return (
            "API key problem. Open ··· → OpenRouter API and confirm the key starts with sk-or-…"
        )

    if (
        "network" in lower
        or "connection" in lower
        or "timed out" in lower
        or "timeout" in lower
        or "unreachable" in lower
    ):
        return "Cannot reach OpenRouter. Check your network, then hit Refresh."

    if "json" in lower or "invalid" in lower or "unexpected" in lower:
        return "OpenRouter returned a bad response. Wait a moment and hit Refresh."

    http = re.search(r"\bhttp\s+(\d{3})\b", lower)
    if http:
        return (
            f"OpenRouter request failed (HTTP {http.group(1)}). "
            "Try Refresh, or check openrouter.ai status."
        )

    sanitized = _strip_secrets(msg.splitlines()[-1] if "\n" in msg else msg)
    if len(sanitized) <= 140:
        return sanitized
    return sanitized[:137] + "…"


def _strip_secrets(text: str) -> str:
    """Remove API key patterns from error text."""
    redacted = re.sub(r"sk-[a-zA-Z0-9_-]{20,}", "[REDACTED]", text)
    redacted = re.sub(r"Bearer\s+[a-zA-Z0-9_\-\.]+", "Bearer [REDACTED]", redacted)
    redacted = re.sub(r'"api_key"\s*:\s*"[^"]*"', '"api_key": "[REDACTED]"', redacted)
    redacted = re.sub(r"'api_key'\s*:\s*'[^']*'", "'api_key': '[REDACTED]'", redacted)
    return redacted
