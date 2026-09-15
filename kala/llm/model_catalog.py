"""Fetch available models from LLM provider APIs."""

from __future__ import annotations

import json
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class ModelInfo:
    id: str
    name: str = ""
    owned_by: str = ""

    @property
    def label(self) -> str:
        if self.name and self.name != self.id:
            return f"{self.id} — {self.name}"
        if self.owned_by:
            return f"{self.id} ({self.owned_by})"
        return self.id


class ModelFetchError(RuntimeError):
    pass


def _http_get_json(url: str, headers: dict[str, str], timeout: float = 30.0) -> Any:
    req = urllib.request.Request(url, headers=headers, method="GET")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = resp.read().decode("utf-8")
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")[:300]
        raise ModelFetchError(f"HTTP {exc.code} from {url}: {detail}") from exc
    except urllib.error.URLError as exc:
        raise ModelFetchError(f"Network error contacting {url}: {exc.reason}") from exc
    try:
        return json.loads(body)
    except json.JSONDecodeError as exc:
        raise ModelFetchError(f"Invalid JSON from {url}") from exc


def _normalize_base(base_url: str) -> str:
    return base_url.rstrip("/")


def fetch_openai_compatible_models(base_url: str, api_key: str = "") -> list[ModelInfo]:
    base = _normalize_base(base_url)
    url = f"{base}/models"
    headers = {"Accept": "application/json", "User-Agent": "kala-agent/0.1"}
    if api_key.strip():
        headers["Authorization"] = f"Bearer {api_key.strip()}"
    data = _http_get_json(url, headers)
    rows = data.get("data") if isinstance(data, dict) else None
    if not isinstance(rows, list):
        raise ModelFetchError("Unexpected models payload (missing data[])")
    models: list[ModelInfo] = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        mid = str(row.get("id") or "").strip()
        if not mid:
            continue
        name = str(row.get("name") or row.get("display_name") or "")
        owned = str(row.get("owned_by") or "")
        models.append(ModelInfo(id=mid, name=name, owned_by=owned))
    models.sort(key=lambda m: m.id.lower())
    return models


def fetch_openrouter_models(api_key: str = "", base_url: str = "") -> list[ModelInfo]:
    base = _normalize_base(base_url or "https://openrouter.ai/api/v1")
    url = f"{base}/models"
    headers = {
        "Accept": "application/json",
        "User-Agent": "kala-agent/0.1",
        "HTTP-Referer": "https://github.com/VINAYYYYYYYYYYYY/Kala-Agent",
        "X-Title": "Kala-Agent",
    }
    if api_key.strip():
        headers["Authorization"] = f"Bearer {api_key.strip()}"
    data = _http_get_json(url, headers)
    rows = data.get("data") if isinstance(data, dict) else None
    if not isinstance(rows, list):
        raise ModelFetchError("Unexpected OpenRouter models payload")
    models: list[ModelInfo] = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        mid = str(row.get("id") or "").strip()
        if not mid:
            continue
        name = str(row.get("name") or "")
        arch = row.get("architecture") or {}
        owned = ""
        if isinstance(arch, dict):
            owned = str(arch.get("tokenizer") or arch.get("modality") or "")
        models.append(ModelInfo(id=mid, name=name, owned_by=owned))
    models.sort(key=lambda m: m.id.lower())
    return models


def fetch_anthropic_models(api_key: str, base_url: str = "") -> list[ModelInfo]:
    if not api_key.strip():
        raise ModelFetchError("Anthropic requires an API key to list models")
    base = _normalize_base(base_url or "https://api.anthropic.com")
    url = f"{base}/v1/models"
    headers = {
        "Accept": "application/json",
        "x-api-key": api_key.strip(),
        "anthropic-version": "2023-06-01",
        "User-Agent": "kala-agent/0.1",
    }
    data = _http_get_json(url, headers)
    rows = data.get("data") if isinstance(data, dict) else None
    if not isinstance(rows, list):
        raise ModelFetchError("Unexpected Anthropic models payload")
    models: list[ModelInfo] = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        mid = str(row.get("id") or "").strip()
        if not mid:
            continue
        name = str(row.get("display_name") or row.get("name") or "")
        models.append(ModelInfo(id=mid, name=name))
    models.sort(key=lambda m: m.id.lower())
    return models


def fetch_models(provider: str, *, base_url: str, api_key: str) -> list[ModelInfo]:
    """Dispatch model listing for the selected provider type."""
    key = provider.lower().strip()
    if key == "openrouter":
        return fetch_openrouter_models(api_key=api_key, base_url=base_url)
    if key == "anthropic":
        return fetch_anthropic_models(api_key=api_key, base_url=base_url)
    # OpenAI, Groq, Ollama, custom — OpenAI-compatible /models
    if not base_url.strip():
        raise ModelFetchError("Base URL is required to list models")
    return fetch_openai_compatible_models(base_url=base_url, api_key=api_key)
