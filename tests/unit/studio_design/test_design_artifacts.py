"""The stored-artifact download rules (parity with the Rust `artifacts` module)."""

from __future__ import annotations

import pytest

from kala_agent.studio.design.artifacts import (
    ARTIFACT_KINDS,
    DEFAULT_ARTIFACT_SLUG,
    MAX_ARTIFACT_SLUG_LENGTH,
    content_type,
    filename,
    slug,
)


def test_artifact_kinds_and_content_types_match_the_native_route() -> None:
    assert ARTIFACT_KINDS == ("script", "step", "fcstd", "stl")
    assert content_type("script") == "application/json"
    assert content_type("step") == "application/step"
    assert content_type("fcstd") == "application/octet-stream"
    assert content_type("stl") == "model/stl"
    assert content_type("anything-else") == "application/octet-stream"


def test_the_download_filename_is_the_slug_plus_eight_digest_characters() -> None:
    assert (
        filename("L-bracket 60 x 45 x 40", "a9d04f8f0000", "fcstd")
        == "l-bracket-60-x-45-x-40-a9d04f8f.FCStd"
    )
    assert (
        filename("corpus bracket", "7a50e9c4ffff", "step")
        == "corpus-bracket-7a50e9c4.step"
    )
    assert filename("", "abc", "script") == "kala-studio-abc.json"
    assert filename("part", "d" * 64, "stl") == "part-dddddddd.stl"


@pytest.mark.parametrize(
    ("label", "expected"),
    [
        ("../../etc/passwd", "etc-passwd"),
        ('a"b\r\nc', "a-b-c"),
        ("---", DEFAULT_ARTIFACT_SLUG),
        ("Ünïcode part", "n-code-part"),
        ("Plate (v2) — final", "plate-v2-final"),
    ],
)
def test_a_hostile_label_cannot_escape_the_filename(label: str, expected: str) -> None:
    assert slug(label) == expected


def test_the_slug_is_bounded() -> None:
    assert len(slug("x" * 200)) <= MAX_ARTIFACT_SLUG_LENGTH
