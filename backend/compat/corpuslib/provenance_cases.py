"""Domain package provenance frozen as a LEGACY Python reference.

These digests are hashes of the Python source tree that currently implements
each domain package. They are recorded so a Rust implementation can pin them
or version a visible replacement; they must never be presented as the
identity of Rust source files.
"""

from __future__ import annotations

from typing import Final

from corpuslib.common import REPO_ROOT, SOURCE_ROOT, sha256_file

PROVENANCE_KIND: Final[str] = "legacy_python_source"


def _source_files_for(domain: str) -> list[dict[str, object]]:
    directory = SOURCE_ROOT / "domains" / domain
    paths = sorted(
        path
        for path in directory.rglob("*.py")
        if "__pycache__" not in path.parts
    )
    ancestors = [SOURCE_ROOT / "__init__.py", SOURCE_ROOT / "domains" / "__init__.py"]
    records: list[dict[str, object]] = []
    for path in [*paths, *ancestors]:
        records.append(
            {
                "path": path.relative_to(REPO_ROOT).as_posix(),
                "sha256": sha256_file(path),
                "size_bytes": path.stat().st_size,
            }
        )
    return records


def _package_record(domain: str, package: object) -> dict[str, object]:
    manifest = package.manifest()  # type: ignore[attr-defined]
    implementations = [
        implementation.as_dict()
        for implementation in package.evaluator_implementations  # type: ignore[attr-defined]
    ]
    return {
        "domain": domain,
        "provenance_kind": PROVENANCE_KIND,
        "package_id": package.package_id,  # type: ignore[attr-defined]
        "version": package.version,  # type: ignore[attr-defined]
        "package_digest": package.digest(),  # type: ignore[attr-defined]
        "manifest": manifest.model_dump(mode="json"),
        "schema_digests": list(package.schema_digests),  # type: ignore[attr-defined]
        "workflow_definition_id": package.workflow.definition_id,  # type: ignore[attr-defined]
        "workflow_version": package.workflow.version,  # type: ignore[attr-defined]
        "catalog_step_ids": list(package.step_ids),  # type: ignore[attr-defined]
        "evaluator_implementations": implementations,
        "source_files": _source_files_for(domain),
    }


def build() -> dict[str, object]:
    from kala_agent.domains import IMPLEMENTATION_DIGEST_VERSION
    from kala_agent.domains.building import BUILDING_PACKAGE
    from kala_agent.domains.mechanical import MECHANICAL_PACKAGE
    from kala_agent.domains.wing import WING_PACKAGE

    packages = [
        _package_record("building", BUILDING_PACKAGE),
        _package_record("mechanical", MECHANICAL_PACKAGE),
        _package_record("wing", WING_PACKAGE),
    ]
    return {
        "corpus_schema_version": 1,
        "kind": "domain_provenance",
        "provenance_kind": PROVENANCE_KIND,
        "implementation_digest_version": IMPLEMENTATION_DIGEST_VERSION,
        "warning": (
            "These digests identify the current Python implementation and its source "
            "files. A Rust implementation cannot reproduce them from Rust sources. "
            "Pin them as explicit versioned legacy constants, or bump "
            "implementation_digest_version and accept a visible digest change. "
            "Silent drift is not acceptable."
        ),
        "rust_identity_policy_required": True,
        "packages": packages,
    }


__all__ = ("PROVENANCE_KIND", "build")
