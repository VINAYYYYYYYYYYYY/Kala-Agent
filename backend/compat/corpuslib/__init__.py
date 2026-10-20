"""Reference corpus generation support for the Rust backend migration.

This package is Python-only and read-only with respect to the product
sources. It imports the live Python implementation and records what that
implementation actually does; nothing here reimplements a hashing,
formatting, or arithmetic rule by hand.
"""

from __future__ import annotations

from typing import Final

CORPUS_SCHEMA_VERSION: Final[int] = 1
FIXTURE_ROOT_RELATIVE: Final[str] = "backend/fixtures/corpus/v1"
GENERATOR_RELATIVE: Final[str] = "backend/compat/generate_corpus.py"

# Synthetic credentials used only by the fixtures. They are not live tokens:
# they are never written to a real data directory and never grant access to
# anything but the in-process test client built by the generator.
CORE_TOKEN: Final[str] = "corpus-launch-token-0123456789abcdef"
STUDIO_TOKEN: Final[str] = "corpus-studio-token-0123456789abcdef"
SYNTHETIC_PROVIDER_KEY: Final[str] = "sk-corpus-synthetic-not-a-live-credential-0001"

# The fixed clock makes every generated timestamp a pure function of request
# order. The start value is arbitrary; only its determinism is meaningful.
FIXED_CLOCK_START_ISO: Final[str] = "2026-01-01T00:00:00+00:00"

__all__ = (
    "CORE_TOKEN",
    "CORPUS_SCHEMA_VERSION",
    "FIXED_CLOCK_START_ISO",
    "FIXTURE_ROOT_RELATIVE",
    "GENERATOR_RELATIVE",
    "STUDIO_TOKEN",
    "SYNTHETIC_PROVIDER_KEY",
)
