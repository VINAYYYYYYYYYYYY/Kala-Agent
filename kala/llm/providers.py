"""LLM API provider configs (stored locally; keys never logged)."""

from __future__ import annotations

import json
import os
import uuid
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class ProviderPreset:
    id: str
    label: str
    default_base_url: str
    default_model: str
    needs_api_key: bool = True


PRESETS: dict[str, ProviderPreset] = {
    "openai": ProviderPreset(
        "openai", "OpenAI", "https://api.openai.com/v1", "gpt-4.1-mini"
    ),
    "anthropic": ProviderPreset(
        "anthropic", "Anthropic", "https://api.anthropic.com", "claude-sonnet-4-20250514"
    ),
    "openrouter": ProviderPreset(
        "openrouter",
        "OpenRouter",
        "https://openrouter.ai/api/v1",
        "anthropic/claude-sonnet-4",
    ),
    "groq": ProviderPreset(
        "groq", "Groq", "https://api.groq.com/openai/v1", "llama-3.3-70b-versatile"
    ),
    "ollama": ProviderPreset(
        "ollama",
        "Ollama (local)",
        "http://127.0.0.1:11434/v1",
        "llama3.2",
        needs_api_key=False,
    ),
    "custom": ProviderPreset(
        "custom", "Custom OpenAI-compatible", "https://example.com/v1", "model-name"
    ),
}


def looks_like_api_key(value: str, provider: str = "") -> bool:
    """Reject empty keys and pasted website URLs mistaken for API keys."""
    key = (value or "").strip()
    if not key:
        return provider in {"ollama"}
    if key.startswith(("http://", "https://")):
        return False
    if "openrouter.ai" in key.lower() and not key.startswith("sk-"):
        return False
    if provider in {"openrouter", "openai", "groq", "custom"}:
        return key.startswith("sk-") or len(key) >= 20
    if provider == "anthropic":
        return key.startswith("sk-ant-") or len(key) >= 20
    return len(key) >= 8


@dataclass
class ProviderConfig:
    id: str
    name: str
    provider: str
    api_key: str = ""
    base_url: str = ""
    model: str = ""
    enabled: bool = True

    def to_public_dict(self) -> dict[str, Any]:
        """Serialize without exposing the full API key."""
        key = self.api_key.strip()
        masked = ""
        if key:
            masked = ("*" * max(0, len(key) - 4)) + key[-4:]
        return {
            "id": self.id,
            "name": self.name,
            "provider": self.provider,
            "api_key_set": bool(key),
            "api_key_masked": masked,
            "base_url": self.base_url,
            "model": self.model,
            "enabled": self.enabled,
        }


@dataclass
class ProviderStore:
    active_id: str | None = None
    providers: list[ProviderConfig] = field(default_factory=list)

    def get(self, provider_id: str) -> ProviderConfig | None:
        for p in self.providers:
            if p.id == provider_id:
                return p
        return None

    def active(self) -> ProviderConfig | None:
        if not self.active_id:
            return None
        return self.get(self.active_id)

    def upsert(self, cfg: ProviderConfig) -> None:
        for i, existing in enumerate(self.providers):
            if existing.id == cfg.id:
                self.providers[i] = cfg
                return
        self.providers.append(cfg)

    def remove(self, provider_id: str) -> None:
        self.providers = [p for p in self.providers if p.id != provider_id]
        if self.active_id == provider_id:
            self.active_id = self.providers[0].id if self.providers else None


def config_path() -> Path:
    override = os.environ.get("KALA_CONFIG_DIR")
    if override:
        base = Path(override)
    else:
        xdg = os.environ.get("XDG_CONFIG_HOME")
        base = Path(xdg) if xdg else Path.home() / ".config"
        base = base / "kala"
    base.mkdir(parents=True, exist_ok=True)
    return base / "providers.json"


def load_store() -> ProviderStore:
    path = config_path()
    if not path.exists():
        return ProviderStore()
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return ProviderStore()
    providers = [
        ProviderConfig(
            id=str(item.get("id") or uuid.uuid4()),
            name=str(item.get("name") or "Provider"),
            provider=str(item.get("provider") or "custom"),
            api_key=str(item.get("api_key") or ""),
            base_url=str(item.get("base_url") or ""),
            model=str(item.get("model") or ""),
            enabled=bool(item.get("enabled", True)),
        )
        for item in raw.get("providers", [])
    ]
    active_id = raw.get("active_id")
    if active_id and not any(p.id == active_id for p in providers):
        active_id = None
    return ProviderStore(active_id=active_id, providers=providers)


def save_store(store: ProviderStore) -> Path:
    path = config_path()
    payload = {
        "active_id": store.active_id,
        "providers": [asdict(p) for p in store.providers],
    }
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    try:
        path.chmod(0o600)
    except OSError:
        pass
    return path


def new_from_preset(provider_id: str, name: str | None = None) -> ProviderConfig:
    preset = PRESETS.get(provider_id) or PRESETS["custom"]
    return ProviderConfig(
        id=str(uuid.uuid4()),
        name=name or preset.label,
        provider=preset.id,
        api_key="",
        base_url=preset.default_base_url,
        model=preset.default_model,
        enabled=True,
    )
