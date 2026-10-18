"""Static integration registry for the studio."""

from kala_agent.studio.integrations.registry import (
    RegistryEntry,
    build_registry,
    integration_by_id,
    list_capabilities,
)

__all__ = (
    "RegistryEntry",
    "build_registry",
    "integration_by_id",
    "list_capabilities",
)
