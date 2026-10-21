"""The stored-artifact download surface, shared by the Python reference.

One design can store four kinds of artifact: the canonical construction
``script``, the provider's ``step``, the provider's native document (``fcstd``
for FreeCAD) and a display ``stl`` previously written by the export route. The
download route serves exactly those bytes, under exactly those kinds, and
refuses anything else.

Every rule here mirrors ``kala-studio/src/artifacts.rs`` byte for byte: the
allow-list, the 64 MiB cap and the filename slug. A differential case compares
the two downloads.
"""

from __future__ import annotations

from typing import Final

#: The artifact kinds a stored design can answer for a download.
ARTIFACT_KINDS: Final[tuple[str, ...]] = ("script", "step", "fcstd", "stl")

#: The largest artifact the download route serves.
MAX_ARTIFACT_DOWNLOAD_BYTES: Final[int] = 64 * 1024 * 1024

#: The fallback download filename stem when a design declares no usable label.
DEFAULT_ARTIFACT_SLUG: Final[str] = "kala-studio"

#: The longest label slug the download filename uses.
MAX_ARTIFACT_SLUG_LENGTH: Final[int] = 40

_CONTENT_TYPES: Final[dict[str, str]] = {
    "script": "application/json",
    "step": "application/step",
    "stl": "model/stl",
}
_DEFAULT_CONTENT_TYPE: Final[str] = "application/octet-stream"

_EXTENSIONS: Final[dict[str, str]] = {
    "script": "json",
    "step": "step",
    "stl": "stl",
}
_DEFAULT_EXTENSION: Final[str] = "FCStd"


def content_type(kind: str) -> str:
    """Return the ``content-type`` of one downloadable artifact kind."""
    return _CONTENT_TYPES.get(kind, _DEFAULT_CONTENT_TYPE)


def slug(label: str) -> str:
    """Return a filesystem- and header-safe slug of a design label.

    ASCII alphanumerics are lowercased; every other run becomes one ``-``. The
    result is bounded to :data:`MAX_ARTIFACT_SLUG_LENGTH` characters and falls
    back to :data:`DEFAULT_ARTIFACT_SLUG` when the label has no usable
    character, so a hostile label can never traverse a path or smuggle a
    header.
    """
    out: list[str] = []
    for character in label:
        if character.isascii() and character.isalnum():
            out.append(character.lower())
        elif out and out[-1] != "-":
            out.append("-")
    trimmed = "".join(out).strip("-")
    bounded = trimmed[:MAX_ARTIFACT_SLUG_LENGTH].strip("-")
    return bounded or DEFAULT_ARTIFACT_SLUG


def filename(label: str, digest: str, kind: str) -> str:
    """Return the ``content-disposition`` filename of one artifact."""
    extension = _EXTENSIONS.get(kind, _DEFAULT_EXTENSION)
    return f"{slug(label)}-{digest[:8]}.{extension}"


__all__ = (
    "ARTIFACT_KINDS",
    "DEFAULT_ARTIFACT_SLUG",
    "MAX_ARTIFACT_DOWNLOAD_BYTES",
    "MAX_ARTIFACT_SLUG_LENGTH",
    "content_type",
    "filename",
    "slug",
)
