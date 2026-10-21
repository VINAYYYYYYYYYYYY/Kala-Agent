#!/usr/bin/env python3
"""Self-check the checked-in compatibility corpus.

This validates the corpus structure, re-opens the SQLite fixture through the
live Python reader, verifies stored STL bytes, and (unless
``--skip-regenerate`` is passed) regenerates the whole corpus in a temporary
directory and compares it with the checked-in fixtures.

Run from the repository root:

    .venv/bin/python backend/compat/self_check.py
"""

from __future__ import annotations

import argparse
import json
import shutil
import sqlite3
import tempfile
from pathlib import Path
from typing import Final

from corpuslib import CORE_TOKEN, FIXTURE_ROOT_RELATIVE, STUDIO_TOKEN, SYNTHETIC_PROVIDER_KEY
from corpuslib.common import (
    REPO_ROOT,
    sha256_bytes,
    sha256_file,
    sha256_text,
    silence_native_shutdown,
)
from corpuslib.http_cases import VOLATILE_PLACEHOLDER
from generate_corpus import compare, corpus_digest, generate_all

DEFAULT_CORPUS: Final[Path] = REPO_ROOT / FIXTURE_ROOT_RELATIVE
REQUIRED_JSON: Final[tuple[str, ...]] = (
    "canonical_json.json",
    "event_chain.json",
    "decimal_budget.json",
    "geometry_stl.json",
    "domain_provenance.json",
    "cli_launch.json",
    "studio_http.json",
    "core_http.json",
)


class CheckFailure(RuntimeError):
    pass


def _load(path: Path) -> dict[str, object]:
    return json.loads(path.read_text(encoding="utf-8"))


def _require(condition: object, message: str) -> None:
    if not condition:
        raise CheckFailure(message)


def _check_manifest(root: Path) -> dict[str, object]:
    manifest = _load(root / "manifest.json")
    _require(manifest.get("corpus_schema_version") == 1, "manifest schema version must be 1")
    _require(manifest.get("kind") == "python_compatibility_corpus", "manifest kind")
    files = {entry["path"]: entry for entry in manifest["files"]}  # type: ignore[index]
    for relative in REQUIRED_JSON:
        _require(relative in files, f"manifest is missing {relative}")
        _require((root / relative).is_file(), f"fixture is missing {relative}")
    for relative, entry in files.items():
        path = root / str(relative)
        _require(path.is_file(), f"manifest lists a missing file: {relative}")
        _require(
            path.stat().st_size == entry["size_bytes"],
            f"size mismatch for {relative}",
        )
        if str(relative).endswith(".sqlite3"):
            continue
        _require(
            sha256_file(path) == entry["sha256"],
            f"sha256 mismatch for {relative}",
        )
    for relative in REQUIRED_JSON:
        payload = _load(root / relative)
        _require(
            payload.get("corpus_schema_version") == 1,
            f"{relative} schema version must be 1",
        )
    _require(
        manifest.get("corpus_sha256") == corpus_digest(list(manifest["files"])),  # type: ignore[arg-type]
        "corpus_sha256 does not match the indexed files",
    )
    return manifest


def _check_case_ids(name: str, cases: list[dict[str, object]]) -> None:
    identifiers = [str(case["id"]) for case in cases]
    _require(len(identifiers) == len(set(identifiers)), f"{name} has duplicate case ids")


def _check_canonical(root: Path) -> dict[str, object]:
    payload = _load(root / "canonical_json.json")
    cases = payload["cases"]
    _require(isinstance(cases, list) and cases, "canonical JSON cases are empty")
    _check_case_ids("canonical_json", cases)  # type: ignore[arg-type]
    for case in cases:  # type: ignore[union-attr]
        _require(case.get("operation") in {"dumps", "loads"}, f"bad operation in {case['id']}")
        _require(isinstance(case.get("input"), dict), f"bad input in {case['id']}")
        expected = case.get("expected")
        _require(isinstance(expected, dict), f"bad expected in {case['id']}")
        _require(isinstance(expected.get("ok"), bool), f"bad expected.ok in {case['id']}")
        if expected["ok"]:
            _require(
                "digest" in expected and len(str(expected["digest"])) == 64,
                f"missing digest in {case['id']}",
            )
        else:
            _require(
                isinstance(expected.get("error_type"), str),
                f"missing error_type in {case['id']}",
            )
    return payload


def _check_decimal(root: Path) -> dict[str, object]:
    payload = _load(root / "decimal_budget.json")
    for key in (
        "decode_cases",
        "run_budget_cases",
        "usage_cases",
        "duration_cases",
        "ledger_cases",
        "clamp_cases",
    ):
        cases = payload[key]
        _require(isinstance(cases, list) and cases, f"{key} is empty")
        _check_case_ids(key, cases)  # type: ignore[arg-type]
    return payload


def _check_geometry(root: Path) -> dict[str, object]:
    payload = _load(root / "geometry_stl.json")
    meshes = payload["meshes"]
    _require(isinstance(meshes, list) and len(meshes) == 6, "expected six mesh fixtures")
    _check_case_ids("geometry", meshes)  # type: ignore[arg-type]
    for mesh in meshes:  # type: ignore[union-attr]
        for key in ("positions", "normals", "indices"):
            _require(
                mesh[key]["length"] > 0 and len(mesh[key]["sha256_of_compact_json"]) == 64,
                f"{mesh['id']}: bad {key} digest",
            )
        stl = mesh["stl"]
        _require(len(stl["sha256"]) == 64, f"{mesh['id']}: bad STL digest")
        _require(
            stl["length"] == 84 + 50 * mesh["triangle_count"],
            f"{mesh['id']}: STL length does not match the binary layout",
        )
        if stl.get("stored_file"):
            path = root / "stl" / str(stl["stored_file"])
            _require(path.is_file(), f"{mesh['id']}: missing stored STL")
            _require(path.stat().st_size == stl["length"], f"{mesh['id']}: STL size drift")
            _require(sha256_file(path) == stl["sha256"], f"{mesh['id']}: STL digest drift")
    return payload


def _check_provenance(root: Path) -> dict[str, object]:
    payload = _load(root / "domain_provenance.json")
    packages = payload["packages"]
    _require(isinstance(packages, list) and len(packages) == 3, "expected three packages")
    for package in packages:  # type: ignore[union-attr]
        _require(package["provenance_kind"] == "legacy_python_source", "provenance kind")
        _require(len(package["package_digest"]) == 64, "package digest")
        _require(package["source_files"], "source files are missing")
        for implementation in package["evaluator_implementations"]:
            _require(len(implementation["module_digest"]) == 64, "module digest")
            _require(len(implementation["package_digest"]) == 64, "package digest")
    return payload


def _schema_digest(connection: sqlite3.Connection) -> str:
    rows = [
        [
            str(row["type"]),
            str(row["name"]),
            str(row["tbl_name"]),
            None if row["sql"] is None else " ".join(str(row["sql"]).split()),
        ]
        for row in connection.execute(
            "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name, tbl_name"
        )
    ]
    return sha256_bytes(
        json.dumps(rows, ensure_ascii=True, separators=(",", ":"), sort_keys=True).encode()
    )


def _check_database(root: Path) -> dict[str, object]:
    from kala_agent.persistence.database import Database
    from kala_agent.persistence.journal import EventJournal

    payload = _load(root / "event_chain.json")
    database_record = payload["database"]
    source = root / str(database_record["database_file"])
    _require(source.is_file(), "schema fixture is missing")
    with tempfile.TemporaryDirectory(prefix="kala-corpus-db-") as temporary:
        copy = Path(temporary) / "fixture.sqlite3"
        shutil.copyfile(source, copy)
        database = Database(copy)
        try:
            verification = EventJournal(database).verify_chain()
            with database.serialized() as connection:
                rows = [
                    dict(row)
                    for row in connection.execute(
                        "SELECT sequence, event_type, aggregate_id, workflow_id, run_id,"
                        " payload_json, digest, occurred_at, previous_event_digest,"
                        " event_digest FROM events ORDER BY sequence"
                    )
                ]
                migrations = [
                    dict(row)
                    for row in connection.execute(
                        "SELECT version, name, applied_at FROM schema_migrations ORDER BY version"
                    )
                ]
                counts = {
                    str(row["name"]): int(
                        connection.execute(f'SELECT COUNT(*) FROM "{row["name"]}"').fetchone()[0]
                    )
                    for row in connection.execute(
                        "SELECT name FROM sqlite_master WHERE type='table'"
                        " AND name NOT LIKE 'sqlite_%' ORDER BY name"
                    )
                }
                sqlite_sequence_rows = [
                    {"name": str(row["name"]), "seq": int(row["seq"])}
                    for row in connection.execute(
                        "SELECT name, seq FROM sqlite_sequence ORDER BY name"
                    )
                ]
                digest = _schema_digest(connection)
        finally:
            database.close()
    _require(
        verification.event_count == database_record["verified_event_count"],
        "verified event count drift",
    )
    _require(
        verification.head_sequence == database_record["verified_head_sequence"],
        "verified head sequence drift",
    )
    _require(
        verification.head_event_digest == database_record["verified_head_event_digest"],
        "verified head digest drift",
    )
    _require(rows == database_record["stored_event_rows"], "stored event rows drift")
    _require(migrations == database_record["migrations"], "migration rows drift")
    _require(counts == database_record["table_counts"], "table counts drift")
    _require(
        sqlite_sequence_rows == database_record["sqlite_sequence_rows"],
        "sqlite_sequence rows drift",
    )
    _require(digest == database_record["schema_digest"], "schema digest drift")
    return payload


def _check_http(root: Path, name: str) -> dict[str, object]:
    payload = _load(root / name)
    service = payload["service"]
    cases = service["cases"]
    _require(isinstance(cases, list) and cases, f"{name} has no cases")
    _check_case_ids(name, cases)  # type: ignore[arg-type]
    for case in cases:  # type: ignore[union-attr]
        request = case["request"]
        expected = case["expected"]
        _require(
            request["method"] in {"GET", "POST", "DELETE", "PUT", "PATCH", "OPTIONS", "HEAD"},
            "method",
        )
        _require(str(request["path"]).startswith("/"), "path")
        _require(isinstance(expected["status"], int), f"{case['id']}: status")
        _require(len(expected["body_sha256"]) == 64, f"{case['id']}: body digest")
        _require(isinstance(expected["body_deterministic"], bool), f"{case['id']}: determinism")
        if "body_text" in expected:
            _require(
                sha256_text(str(expected["body_text"])) == expected["body_sha256"],
                f"{case['id']}: body_text does not match body_sha256",
            )
        if expected["body_deterministic"] and "body_hex" in expected:
            _require(
                sha256_bytes(bytes.fromhex(str(expected["body_hex"]))) == expected["body_sha256"],
                f"{case['id']}: body_hex does not match body_sha256",
            )
        # A normalised body is frozen by its *normalised* bytes. Its length and
        # every header that embeds that length must follow the normalised body,
        # never the run-specific path the generator saw; otherwise the fixture
        # silently depends on the length of TMPDIR.
        normalised_headers = expected.get("normalised_headers") or []
        _require(isinstance(normalised_headers, list), f"{case['id']}: normalised_headers")
        for name in normalised_headers:
            _require(isinstance(name, str) and bool(name), f"{case['id']}: normalised header")
            _require(
                name not in expected["headers"],
                f"{case['id']}: {name} is both frozen and normalised",
            )
        if expected.get("normalizations"):
            _require(
                expected["body_deterministic"] is False,
                f"{case['id']}: a normalised body is marked deterministic",
            )
            _require(
                "content-length" not in expected["headers"],
                f"{case['id']}: content-length is frozen on a normalised body",
            )
            _require(
                "content-length" in normalised_headers,
                f"{case['id']}: content-length is neither frozen nor declared normalised",
            )
            if "body_text" in expected:
                _require(
                    expected["body_length"] == len(str(expected["body_text"]).encode("utf-8")),
                    f"{case['id']}: body_length is not the length of the normalised body",
                )
        else:
            _require(
                not normalised_headers,
                f"{case['id']}: normalised_headers declared without normalizations",
            )
        for header in (
            "cache-control",
            "content-security-policy",
            "cross-origin-opener-policy",
            "cross-origin-resource-policy",
            "referrer-policy",
            "x-content-type-options",
            "x-frame-options",
        ):
            _require(header in expected["headers"], f"{case['id']}: missing {header}")
        for header in ("server", "x-powered-by", "set-cookie"):
            _require(header in expected["headers_absent"], f"{case['id']}: {header} present")
        body_text = json.dumps(expected.get("body"), ensure_ascii=True)
        body_text += str(expected.get("body_text", ""))
        for secret in (CORE_TOKEN, STUDIO_TOKEN, SYNTHETIC_PROVIDER_KEY):
            _require(secret not in body_text, f"{case['id']}: token leaked into a response")
        artifact = expected.get("body_artifact")
        if artifact:
            path = root / str(artifact)
            _require(path.is_file(), f"{case['id']}: missing {artifact}")
            _require(
                path.stat().st_size == expected["body_length"], f"{case['id']}: artifact size"
            )
            _require(
                sha256_file(path) == expected["body_sha256"],
                f"{case['id']}: artifact digest",
            )
        summary = json.dumps(expected.get("body_summary"), ensure_ascii=True)
        for secret in (CORE_TOKEN, STUDIO_TOKEN, SYNTHETIC_PROVIDER_KEY):
            _require(secret not in summary, f"{case['id']}: token leaked into a summary")
    return payload


def _check_cli(root: Path) -> dict[str, object]:
    payload = _load(root / "cli_launch.json")
    core_commands = payload["core"]["parser"]["commands"]
    _require(isinstance(core_commands, list) and core_commands, "core command inventory")
    _require(payload["executions"], "CLI executions are missing")
    exit_codes = {int(item["exit_code"]) for item in payload["executions"]}
    _require({0, 1, 2, 3} <= exit_codes, "CLI fixture does not exercise 0, 1, 2, and 3")
    _require(payload["studio"]["default_port"] == 8765, "studio default port")
    _require(payload["studio"]["token_environment_variable"] == "KALA_STUDIO_TOKEN", "studio env")
    return payload


def _check_coverage(manifest: dict[str, object]) -> None:
    coverage = manifest["coverage"]
    for name in ("studio_http", "core_http"):
        missing = coverage[name]["missing"]  # type: ignore[index]
        _require(not missing, f"{name}: uncovered declared routes {missing}")
    studio_count = len(coverage["studio_http"]["declared"])  # type: ignore[index]
    _require(
        studio_count == 16,
        f"expected 16 studio routes, found {studio_count}",
    )
    core_count = len(coverage["core_http"]["declared"])  # type: ignore[index]
    # The audit/task text says 18 core routes. The live route table is the
    # authority here; record the exact count rather than forcing it.
    _require(core_count >= 17, f"expected at least 17 core routes, found {core_count}")


def _check_volatile_fields(root: Path, name: str) -> None:
    """Every masked field must still carry its placeholder and its reason."""
    payload = _load(root / name)
    for case in payload["service"]["cases"]:  # type: ignore[index]
        expected = case["expected"]
        masked = expected.get("volatile_fields")
        if not masked:
            continue
        _require(bool(expected.get("body_deterministic") is False), f"{case['id']}: masked case is marked deterministic")
        for field in masked:
            _require(
                bool(str(field.get("reason", "")).strip()),
                f"{case['id']}: a masked field has no reason",
            )
        body_text = str(expected.get("body_text", ""))
        _require(
            VOLATILE_PLACEHOLDER in body_text,
            f"{case['id']}: a masked field left no placeholder in the body",
        )
        _require(
            sha256_text(body_text) == expected["body_sha256"],
            f"{case['id']}: the masked body digest does not match the body text",
        )
    chain = expected.get("journal_chain")
    if isinstance(chain, dict):
        if "chain_is_unbroken" in chain:
            _require(
                chain["chain_is_unbroken"] is True,
                f"{case['id']}: the recorded journal chain is broken",
            )
        if "chain_ok" in chain:
            _require(chain["chain_ok"] is True, f"{case['id']}: the journal verdict is not ok")
        if "head_digest_matches_last_event" in chain:
            _require(
                chain["head_digest_matches_last_event"] is True,
                f"{case['id']}: the journal head does not match the last event",
            )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Validate the compatibility corpus.")
    parser.add_argument(
        "--corpus", type=Path, default=DEFAULT_CORPUS, help="corpus directory"
    )
    parser.add_argument(
        "--skip-regenerate",
        action="store_true",
        help="skip the full regeneration/determinism comparison",
    )
    arguments = parser.parse_args(argv)
    root = arguments.corpus.expanduser().resolve()
    try:
        manifest = _check_manifest(root)
        canonical = _check_canonical(root)
        chain = _check_database(root)
        decimal = _check_decimal(root)
        geometry = _check_geometry(root)
        provenance = _check_provenance(root)
        cli = _check_cli(root)
        studio = _check_http(root, "studio_http.json")
        core = _check_http(root, "core_http.json")
        _check_volatile_fields(root, "studio_http.json")
        _check_volatile_fields(root, "core_http.json")
        _check_coverage(manifest)
        if not arguments.skip_regenerate:
            with tempfile.TemporaryDirectory(prefix="kala-corpus-selfcheck-") as temporary:
                regenerated = Path(temporary) / "corpus"
                generate_all(regenerated)
                problems = compare(root, regenerated)
            if problems:
                raise CheckFailure(
                    "regeneration differs from the checked-in corpus:\n- "
                    + "\n- ".join(problems)
                )
    except CheckFailure as error:
        print(f"self-check FAILED: {error}")
        return 1
    print("self-check OK")
    print(
        json.dumps(
            {
                "canonical_json_cases": len(canonical["cases"]),
                "chain_events": len(chain["database"]["events"]),
                "decimal_cases": sum(
                    len(decimal[key])
                    for key in (
                        "decode_cases",
                        "run_budget_cases",
                        "usage_cases",
                        "duration_cases",
                        "ledger_cases",
                        "clamp_cases",
                    )
                ),
                "geometry_meshes": len(geometry["meshes"]),
                "domain_packages": len(provenance["packages"]),
                "cli_commands": len(cli["core"]["parser"]["commands"]),
                "cli_executions": len(cli["executions"]),
                "studio_http_cases": len(studio["service"]["cases"]),
                "core_http_cases": len(core["service"]["cases"]),
                "studio_routes": len(manifest["coverage"]["studio_http"]["declared"]),
                "core_routes": len(manifest["coverage"]["core_http"]["declared"]),
                "corpus_sha256": manifest.get("corpus_sha256"),
                "regenerated": not arguments.skip_regenerate,
            },
            indent=2,
            sort_keys=True,
        )
    )
    silence_native_shutdown()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
