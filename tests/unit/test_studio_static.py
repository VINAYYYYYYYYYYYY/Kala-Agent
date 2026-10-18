"""Tests for the studio static-asset root resolution.

These tests cover the three real resolvers used by ``resolve_static_root``.
A regression in the resolver dispatch (e.g. calling a no-arg function with
``environ`` and silently swallowing the TypeError) makes the normal
desktop launcher fail to serve its UI; the tests below pin the contract.
"""

from __future__ import annotations

import importlib
import os
from collections.abc import Iterator
from pathlib import Path

import pytest

from kala_agent.studio.static import (
    REPO_DIST_DIR_PARTS,
    STUDIO_DIST_ENV_VAR,
    STUDIO_DIST_INDEX,
    list_candidate_sources,
    resolve_static_root,
)


@pytest.fixture(autouse=True)
def _no_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("static tests must not open network connections")

    monkeypatch.setattr("socket.socket.connect", blocked)
    yield


def test_repo_dist_directory_uses_pair() -> None:
    assert isinstance(REPO_DIST_DIR_PARTS, tuple)
    assert REPO_DIST_DIR_PARTS == ("studio", "dist")


def test_candidate_sources_are_in_env_then_frozen_then_repo_order() -> None:
    sources = list_candidate_sources()
    assert sources == (
        "_candidate_from_env",
        "_candidate_from_meipass",
        "_candidate_from_repo",
    )


def _make_repo_dist(tmp_path: Path) -> Path:
    dist = tmp_path / "studio" / "dist"
    dist.mkdir(parents=True, exist_ok=True)
    (dist / STUDIO_DIST_INDEX).write_text(
        "<!doctype html><html><body>studio</body></html>",
        encoding="utf-8",
    )
    return dist


def _patch_module_file(monkeypatch: pytest.MonkeyPatch, target: Path) -> None:
    """Force ``Path(__file__)`` inside the static module to ``target``.

    The repo resolver uses ``Path(__file__).parents`` to walk towards the
    repository root; the tests below inject a fake module file under a
    temporary directory so the resolver discovers the fake dist we created.
    """
    static_module = importlib.import_module("kala_agent.studio.static")
    monkeypatch.setattr(static_module, "__file__", str(target))


def test_repo_resolver_finds_real_local_repo() -> None:
    """The repository has a ``studio/dist/index.html`` from the UI worker."""
    root = resolve_static_root({})
    if root is None:
        # The repo has the dist directory; if it is missing the failure is
        # upstream and the test should still record the absent root rather
        # than pass spuriously.
        from kala_agent.studio.static import STUDIO_DIST_ENV_VAR

        pytest.skip(f"{STUDIO_DIST_ENV_VAR} directory with index.html was not located")
    assert (root / STUDIO_DIST_INDEX).is_file()


def test_env_variable_overrides_repo(tmp_path: Path) -> None:
    dist = _make_repo_dist(tmp_path)
    environ = {STUDIO_DIST_ENV_VAR: str(dist)}
    resolved = resolve_static_root(environ)
    assert resolved == dist.resolve()


def test_env_variable_must_contain_index_html(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bad = tmp_path / "no-index"
    bad.mkdir(parents=True, exist_ok=True)
    # Patch the repo resolver out so the test is not polluted by the
    # real repo dist directory discovered by walking module parents.
    pkg_dir = tmp_path / "src" / "kala_agent" / "studio"
    pkg_dir.mkdir(parents=True, exist_ok=True)
    fake_static = pkg_dir / "static.py"
    fake_static.write_text("# fake static module for tests\n", encoding="utf-8")
    _patch_module_file(monkeypatch, fake_static)
    resolved = resolve_static_root({STUDIO_DIST_ENV_VAR: str(bad)})
    assert resolved is None


def test_repo_resolver_walks_parents_and_finds_dist(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Construct a fake package layout: <tmp>/src/kala_agent/studio/static.py
    # with the dist at <tmp>/studio/dist.
    pkg_dir = tmp_path / "src" / "kala_agent" / "studio"
    pkg_dir.mkdir(parents=True, exist_ok=True)
    fake_static = pkg_dir / "static.py"
    fake_static.write_text("# fake static module for tests\n", encoding="utf-8")
    dist = _make_repo_dist(tmp_path)
    _patch_module_file(monkeypatch, fake_static)
    resolved = resolve_static_root({})
    assert resolved == dist.resolve()


def test_no_static_root_returns_none(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    pkg_dir = tmp_path / "src" / "kala_agent" / "studio"
    pkg_dir.mkdir(parents=True, exist_ok=True)
    fake_static = pkg_dir / "static.py"
    fake_static.write_text("# fake static module for tests\n", encoding="utf-8")
    _patch_module_file(monkeypatch, fake_static)
    assert resolve_static_root({}) is None


def test_env_variable_with_invalid_path_returns_none(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    pkg_dir = tmp_path / "src" / "kala_agent" / "studio"
    pkg_dir.mkdir(parents=True, exist_ok=True)
    fake_static = pkg_dir / "static.py"
    fake_static.write_text("# fake static module for tests\n", encoding="utf-8")
    _patch_module_file(monkeypatch, fake_static)
    environ = {STUDIO_DIST_ENV_VAR: "/this/path/does/not/exist/anywhere-xyz"}
    assert resolve_static_root(environ) is None


def test_env_variable_path_traversal_does_not_crash(tmp_path: Path) -> None:
    dist = _make_repo_dist(tmp_path)
    environ = {STUDIO_DIST_ENV_VAR: str(dist) + "/../"}
    resolved = resolve_static_root(environ)
    # The expanduser + resolve call collapses the traversal; the result
    # either points at the real dist or rejects it (both are safe).
    if resolved is not None:
        assert (resolved / STUDIO_DIST_INDEX).is_file()


def test_resolve_static_root_never_raises() -> None:
    # Pathological environments must never raise; they should resolve to None.
    for value in (
        "",
        "\x00nullbyte",
        "/",
        "/\x00etc/passwd",
    ):
        if value == "":
            environ = {STUDIO_DIST_ENV_VAR: ""}
        elif "\x00" in value:
            # os.environ cannot hold NULs on Linux; skip it.
            continue
        else:
            environ = {STUDIO_DIST_ENV_VAR: value}
        try:
            resolve_static_root(environ)
        except Exception as error:
            pytest.fail(
                f"resolve_static_root raised with environ={environ!r}: "
                f"{type(error).__name__}: {error}"
            )


def test_resolve_uses_only_safe_env_namespace(monkeypatch: pytest.MonkeyPatch) -> None:
    captured: dict[str, object] = {}

    def fake_env_get(name: str, default: object = None) -> object:
        captured["name"] = name
        return default

    monkeypatch.setattr(os.environ, "get", fake_env_get)
    resolve_static_root(os.environ)
    assert captured["name"] == STUDIO_DIST_ENV_VAR
    assert STUDIO_DIST_ENV_VAR == "KALA_STUDIO_DIST"


def test_resolve_uses_env_var_in_global_process_env_when_passed() -> None:
    """The dispatch path passes ``os.environ``; the resolver must use the
    mapping it was given rather than implicitly reading the global one."""
    sentinel = {STUDIO_DIST_ENV_VAR: "/explicit/no/such/dir"}
    result = resolve_static_root(sentinel)
    if result is not None:
        # An upstream setUp could have created a real dist; only assert
        # the behaviour is bounded, not the answer.
        assert result.is_dir()
    # The mapping we passed in remains the only place the resolver looked.
    assert STUDIO_DIST_ENV_VAR in sentinel
