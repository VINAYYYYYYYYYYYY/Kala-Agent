#!/usr/bin/env python3
"""Generate or verify the Python compatibility corpus.

Usage, from the repository root:

    .venv/bin/python backend/compat/generate_corpus.py
    .venv/bin/python backend/compat/generate_corpus.py --check

``--check`` generates into a temporary directory and compares it with the
checked-in corpus without writing to ``backend/fixtures``. It treats the
SQLite file bytes and declared-informational STEP digests as volatile; every
JSON fixture, every documented logical row, and every STL digest must match.
"""

from __future__ import annotations

import argparse
import copy
import json
import re
import shutil
import sys
import tempfile
from pathlib import Path
from typing import Final

from corpuslib import (
    CORPUS_SCHEMA_VERSION,
    FIXTURE_ROOT_RELATIVE,
    GENERATOR_RELATIVE,
    canonical_json_cases,
    chain_cases,
    cli_cases,
    decimal_cases,
    geometry_cases,
    http_cases,
    provenance_cases,
)
from corpuslib.common import (
    REPO_ROOT,
    environment_facts,
    git_state,
    sha256_file,
    sha256_text,
    silence_native_shutdown,
    source_tree_state,
    step_normalisation_facts,
    write_json,
)
from corpuslib.http_cases import VOLATILE_PLACEHOLDER

FIXTURES_ROOT: Final[Path] = REPO_ROOT / "backend" / "fixtures" / "corpus"
DEFAULT_OUTPUT: Final[Path] = REPO_ROOT / FIXTURE_ROOT_RELATIVE
VOLATILE_JSON_PATHS: Final[tuple[tuple[str, ...], ...]] = (
    ("database", "database_sha256"),
    ("database", "database_size_bytes"),
)
# The manifest parts that are recorded for provenance but are not part of the
# comparison. The file list is covered by the per-file comparison and by the
# corpus digest; the git working-tree state lists the paths of every worker
# in the repository, so it changes whenever an unrelated file changes; the CAD
# provider probe is a live fact about the host (ADR-0011), so it is recorded
# for the reader and never compared.
EXCLUDED_FROM_COMPARISON: Final[tuple[str, ...]] = (
    "files",
    "source_state.git",
    "oracle_environment.cad_providers",
)

# --------------------------------------------------------------------------
# The body comparison contract
# --------------------------------------------------------------------------
#
# A captured body is not always reproducible byte for byte. The corpus says
# which kind of comparison every case carries, so that a consumer never has to
# guess and never has to loosen a case that *is* reproducible:
#
# * ``exact``: the live body must equal the recorded body byte for byte. The
#   corpus marks the case deterministic, so the bytes are the contract.
# * ``structural``: the body is a STEP export; the corpus records the stable
#   structural fields (an ISO 10303-21 file with one manifold solid) instead
#   of the bytes, because the writer stamps the wall clock into its header.
# * ``fields``: the body carries values that no run can reproduce. The corpus
#   declares every one of them by path, together with the JSON types and the
#   shape the live value must have. A consumer masks those paths and compares
#   the rest of the document exactly.
#
# The paths are declared with a small JSON-path dialect: ``$.a.b``,
# ``$.a[0].b`` and ``$.a[*].b``.

# The JSON types a live value may take at a volatile path, and the shape it
# must have. The key is the path with every index folded to ``[*]``.
VOLATILE_PATH_SHAPES: Final[dict[str, tuple[tuple[str, ...], str | None]]] = {
    "$.artifacts.step_sha256": (("string",), "sha256-hex"),
    "$.journal_head": (("string",), "sha256-hex"),
    "$.head_digest": (("string",), "sha256-hex"),
    "$.events[*].digest": (("string",), "sha256-hex"),
    "$.events[*].event_digest": (("string",), "sha256-hex"),
    "$.events[*].previous_event_digest": (("string",), "sha256-hex"),
    "$.events[*].payload.step_sha256": (("string",), "sha256-hex"),
    "$.events[*].payload.sha256": (("string",), "sha256-hex"),
    "$.events[*].occurred_at": (("string",), "iso8601-utc"),
    "$.providers[*].available": (("boolean",), None),
    "$.providers[*].version": (("string", "null"), None),
    "$.providers[*].reason": (("string", "null"), None),
}

# The volatile paths the capture itself does not mask, because the recorded
# value is reproducible for the oracle but not for a running service.
EXTRA_VOLATILE_PATHS: Final[dict[str, tuple[tuple[str, str], ...]]] = {
    "studio.journal.page": (
        (
            "$.events[*].occurred_at",
            "the Studio session journal stamps the wall clock; the fixture "
            "harness substitutes a fixed clock, so a running service reports "
            "a different timestamp for the same event",
        ),
    ),
}

# The cases whose body is a live probe of the host rather than a product
# fact. The recorded body keeps the host-independent shape and the static
# provider metadata; the probe result itself is masked and is compared
# against the current host probe by the consumer.
HOST_DEPENDENT_CASES: Final[dict[str, dict[str, object]]] = {
    "studio.cad_providers": {
        "rationale": (
            "ADR-0011 makes CAD provider availability a live probe of the "
            "host: whether FreeCAD answers, which version it reports and why "
            "it refuses are facts about this host and its installation, not "
            "about the product. The corpus therefore freezes the provider "
            "set, the display metadata and the document shape, and records "
            "the probe result as host evidence."
        ),
        "probe": (
            "GET /api/studio/providers/cad probes every configured provider on "
            "every request; the same probe is recorded as an environment fact "
            "in manifest.oracle_environment.cad_providers"
        ),
        "paths": (
            (
                "$.providers[*].available",
                "the probe result of this host",
            ),
            (
                "$.providers[*].version",
                "the version this host's installation reports",
            ),
            (
                "$.providers[*].reason",
                "the refusal text this host's installation produces",
            ),
        ),
    },
}


def corpus_digest(files: list[dict[str, object]]) -> str:
    """Return one digest that identifies the whole corpus.

    The digest covers the indexed file list and the bytes of every file whose
    bytes are asserted. The SQLite fixture's file bytes are not asserted --
    the SQLite library version changes them -- so its entry is recorded as
    ``not-asserted`` instead of a digest. Its logical rows are asserted
    separately in ``event_chain.json``, which is covered.
    """
    entries = [
        [
            str(entry["path"]),
            "not-asserted"
            if str(entry["path"]).endswith(".sqlite3")
            else str(entry["sha256"]),
        ]
        for entry in sorted(files, key=lambda item: str(item["path"]))
    ]
    return sha256_text(
        json.dumps(entries, ensure_ascii=True, separators=(",", ":"), sort_keys=True)
    )


# --------------------------------------------------------------------------
# The body comparison contract
# --------------------------------------------------------------------------

_PATH_SEGMENT = re.compile(r"\.([A-Za-z_][A-Za-z0-9_]*)|\[(\*|\d+)\]")


def _path_segments(path: str) -> list[str]:
    """Split a JSON path into its segments.

    ``$.events[*].digest`` becomes ``["events", "[*]", "digest"]`` and
    ``$.events[3].digest`` becomes ``["events", "[3]", "digest"]``.
    """
    segments: list[str] = []
    for match in _PATH_SEGMENT.finditer(path):
        name, index = match.group(1), match.group(2)
        segments.append(name if name is not None else f"[{index}]")
    return segments


def _fold_indices(path: str) -> str:
    """Return the path with every concrete index folded to ``[*]``."""
    folded: list[str] = []
    for segment in _path_segments(path):
        if segment.startswith("[") and segment != "[*]":
            folded.append("[*]")
        else:
            folded.append(segment)
    text = "$"
    for segment in folded:
        if segment.startswith("["):
            text += segment
        else:
            text += f".{segment}"
    return text


def _mask_at(document: object, segments: list[str], index: int, replacement: str) -> bool:
    """Replace the value at the resolved path. Whether it was found."""
    if index >= len(segments):
        return False
    segment = segments[index]
    if segment == "[*]":
        if not isinstance(document, list):
            return False
        found = False
        for item in document:
            found = _mask_at(item, segments, index + 1, replacement) or found
        return found
    if segment.startswith("["):
        position = int(segment[1:-1])
        if not isinstance(document, list) or position >= len(document):
            return False
        return _mask_at(document[position], segments, index + 1, replacement)
    if not isinstance(document, dict) or segment not in document:
        return False
    if index + 1 == len(segments):
        document[segment] = replacement
        return True
    return _mask_at(document[segment], segments, index + 1, replacement)


def _mask_path(document: object, path: str, replacement: str) -> bool:
    """Replace the value at a JSON path, if the path is present."""
    return _mask_at(document, _path_segments(path), 0, replacement)


def _leaf_paths(value: object, prefix: str = "$") -> list[str]:
    """Return the JSON path of every leaf of a document."""
    paths: list[str] = []
    if isinstance(value, dict):
        for key, item in value.items():
            paths.extend(_leaf_paths(item, f"{prefix}.{key}"))
    elif isinstance(value, list):
        for index, item in enumerate(value):
            paths.extend(_leaf_paths(item, f"{prefix}[{index}]"))
    else:
        paths.append(prefix)
    return paths


def _compact_json(value: object) -> str:
    """Serialize a document the way the product's JSON response does."""
    return json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":"))


def _shape_of(path: str) -> tuple[tuple[str, ...], str | None]:
    """Return the JSON types and the shape a live value must have."""
    return VOLATILE_PATH_SHAPES.get(_fold_indices(path), (("string",), None))


def _host_dependent_mask(case: dict[str, object], host: dict[str, object]) -> None:
    """Mask the host-dependent values of a live-probe case.

    The recorded body keeps the host-independent shape and the static
    provider metadata. The probe result itself is replaced by the
    placeholder, so the corpus bytes do not depend on the host and the
    consumer compares the rest of the document exactly.
    """
    expected = case["expected"]
    body = expected.get("body")
    text = expected.get("body_text")
    if not isinstance(body, dict) or not isinstance(text, str):
        raise ValueError(f"{case['id']}: a host-dependent case needs a stored body")
    if _compact_json(json.loads(text)) != _compact_json(body):
        # The recorded text and the recorded body must describe the same
        # document; otherwise masking one of them would silently change the
        # other.
        raise ValueError(f"{case['id']}: the recorded body and body text differ")
    masked_fields: list[dict[str, str]] = []
    for path, reason in host["paths"]:  # type: ignore[union-attr]
        if not _mask_path(body, path, VOLATILE_PLACEHOLDER):
            raise ValueError(f"{case['id']}: the host-dependent path {path} is absent")
        masked_fields.append({"path": path, "reason": str(reason)})
    masked_text = _compact_json(body)
    expected["body"] = body
    expected["body_text"] = masked_text
    expected["body_sha256"] = sha256_text(masked_text)
    expected["body_sha256_role"] = (
        "digest of the recorded body text after the host-dependent values were "
        "replaced by the placeholder; the raw response bytes carry the live "
        "probe result and are not reproducible"
    )
    expected["body_length"] = len(masked_text)
    expected["body_length_role"] = (
        "length of the recorded body text; the raw response length is host "
        "evidence and is not a contract"
    )
    expected["body_deterministic"] = False
    expected["body_sha256_note"] = (
        "informational at generation time; reproduce the stable fields instead"
    )
    expected["volatile_fields"] = masked_fields
    expected["host_dependent"] = True
    expected["host_dependence"] = {
        "rationale": str(host["rationale"]),
        "probe": str(host["probe"]),
        "dependent_fields": [path for path, _reason in host["paths"]],  # type: ignore[union-attr]
    }
    headers = expected.get("headers")
    if isinstance(headers, dict) and "content-length" in headers:
        # The raw length follows the host-dependent body, so the header is
        # host evidence too. The consumer checks it against the body it
        # received instead of against a recorded number.
        headers.pop("content-length")
        expected["host_dependent_headers"] = ["content-length"]


def _volatile_declarations(case: dict[str, object]) -> list[dict[str, object]]:
    """Return the declared volatile paths of a case.

    The declarations come from three places: the fields the capture masked,
    the leaves the capture masked by value (whose path it could not know), and
    the paths this generator adds because a running service reports a
    different value than the fixture clock does.
    """
    expected = case["expected"]
    body = expected.get("body")
    declarations: dict[str, dict[str, object]] = {}

    def declare(path: str, reason: str) -> None:
        if path in declarations:
            return
        types, shape = _shape_of(path)
        declarations[path] = {
            "path": path,
            "types": list(types),
            "reason": reason,
            **({"format": shape} if shape is not None else {}),
        }

    for field in expected.get("volatile_fields", []) or []:  # type: ignore[union-attr]
        path = str(field.get("path", ""))
        if path:
            declare(path, str(field.get("reason", "the value is not reproducible")))
    for path, reason in EXTRA_VOLATILE_PATHS.get(str(case["id"]), ()):  # type: ignore[arg-type]
        declare(path, reason)
    if isinstance(body, dict) or isinstance(body, list):
        # The capture masked a value it could not name by path (a digest that
        # appears in more than one place). Every leaf that still holds the
        # placeholder and that no declaration covers is named here.
        for path in _leaf_paths(body):
            value = _value_at(body, path)
            if value != VOLATILE_PLACEHOLDER:
                continue
            if any(_covers(declaration["path"], path) for declaration in declarations.values()):
                continue
            declare(path, "the value is not reproducible")
    return [declarations[key] for key in sorted(declarations)]


def _value_at(document: object, path: str) -> object:
    """Return the value at a JSON path, or ``None``."""
    cursor: object = document
    for segment in _path_segments(path):
        if segment == "[*]":
            return None
        if segment.startswith("["):
            position = int(segment[1:-1])
            if not isinstance(cursor, list) or position >= len(cursor):
                return None
            cursor = cursor[position]
            continue
        if not isinstance(cursor, dict) or segment not in cursor:
            return None
        cursor = cursor[segment]
    return cursor


def _covers(declaration: str, path: str) -> bool:
    """Whether a declared path covers a concrete path."""
    declared = _path_segments(declaration)
    concrete = _path_segments(path)
    if len(declared) != len(concrete):
        return False
    for left, right in zip(declared, concrete, strict=True):
        if left == "[*]":
            continue
        if left != right:
            return False
    return True


def _body_comparison_counts(cases: list[dict[str, object]]) -> dict[str, int]:
    """Count the captured cases by their body comparison contract."""
    counts: dict[str, int] = {}
    for case in cases:
        contract = str(case["expected"].get("body_comparison", "exact"))  # type: ignore[index]
        counts[contract] = counts.get(contract, 0) + 1
    return dict(sorted(counts.items()))


def _apply_body_contracts(cases: list[dict[str, object]]) -> None:
    """Give every captured Studio case an explicit body comparison contract."""
    for case in cases:
        case_id = str(case["id"])
        expected = case["expected"]
        host = HOST_DEPENDENT_CASES.get(case_id)
        if host is not None:
            _host_dependent_mask(case, host)
        if expected.get("body_contract") == "structural":
            expected["body_comparison"] = "structural"
            expected.setdefault(
                "body_length_role",
                "length of the raw response bytes; a STEP body is compared "
                "structurally",
            )
            continue
        declarations = _volatile_declarations(case)
        if declarations:
            expected["body_comparison"] = "fields"
            expected["volatile_paths"] = declarations
        else:
            expected["body_comparison"] = "exact"
        if isinstance(expected.get("body_text"), str):
            expected["body_length"] = len(str(expected["body_text"]))
            expected["body_length_role"] = (
                "length of the recorded body text; the recorded digest is the "
                "digest of the same text"
            )


def _route_inventory() -> dict[str, list[dict[str, str]]]:
    from kala_agent.api.app import API_PREFIX, root_router
    from kala_agent.api.app import router as core_router
    from kala_agent.studio.server import STUDIO_PREFIX
    from kala_agent.studio.server import router as studio_router

    def rows(router: object, prefix: str) -> list[dict[str, str]]:
        found: list[dict[str, str]] = []
        for route in router.routes:  # type: ignore[attr-defined]
            path = getattr(route, "path", None)
            methods = getattr(route, "methods", None)
            if not isinstance(path, str) or not methods:
                continue
            for method in sorted(str(method) for method in methods):
                if method in {"HEAD", "OPTIONS"}:
                    continue
                found.append({"method": method, "path": f"{prefix}{path}"})
        return sorted(found, key=lambda row: (row["path"], row["method"]))

    return {
        "studio": rows(studio_router, STUDIO_PREFIX),
        "core": rows(root_router, "") + rows(core_router, API_PREFIX),
    }


def _route_pattern(path: str) -> str:
    import re

    return "^" + re.sub(r"\{[^/]+\}", r"[^/]+", path) + "$"


def _coverage(
    declared: list[dict[str, str]], cases: list[dict[str, object]]
) -> dict[str, object]:
    import re

    covered: list[dict[str, str]] = []
    missing: list[dict[str, str]] = []
    for row in declared:
        pattern = re.compile(_route_pattern(row["path"]))
        hit = any(
            str(case["request"]["method"]) == row["method"]  # type: ignore[index]
            and pattern.match(str(case["request"]["path"]).split("?", 1)[0])  # type: ignore[index]
            for case in cases
        )
        (covered if hit else missing).append(row)
    return {"declared": declared, "covered": covered, "missing": missing}


def generate_all(output: Path) -> dict[str, object]:
    output.mkdir(parents=True, exist_ok=True)
    stl_dir = output / "stl"
    if stl_dir.exists():
        shutil.rmtree(stl_dir)
    stl_dir.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="kala-corpus-gen-") as temporary:
        work_root = Path(temporary)
        schema_path = output / chain_cases.SCHEMA_FILE_NAME
        chain = chain_cases.build(schema_path)
        write_json(output / "event_chain.json", chain)

        geometry = geometry_cases.build(stl_dir, work_root / "step")
        write_json(output / "geometry_stl.json", geometry)

        provenance = provenance_cases.build()
        write_json(output / "domain_provenance.json", provenance)

        decimal = decimal_cases.build()
        write_json(output / "decimal_budget.json", decimal)

        canonical = canonical_json_cases.build()
        write_json(output / "canonical_json.json", canonical)

        cli = cli_cases.build(work_root / "cli")
        write_json(output / "cli_launch.json", cli)

        http = http_cases.build(work_root / "http")
        _apply_body_contracts(http["studio"]["cases"])  # type: ignore[index]
        write_json(
            output / "studio_http.json",
            {
                "corpus_schema_version": CORPUS_SCHEMA_VERSION,
                "kind": "http_studio",
                "capture": http["capture"],
                "service": http["studio"],
                "nondeterministic_fields": [
                    item
                    for item in http["nondeterministic_fields"]  # type: ignore[union-attr]
                    if item["service"] == "studio"
                ],
            },
        )
        write_json(
            output / "core_http.json",
            {
                "corpus_schema_version": CORPUS_SCHEMA_VERSION,
                "kind": "http_core",
                "capture": http["capture"],
                "service": http["core"],
                "normalizations": http["normalizations"],
                "nondeterministic_fields": [
                    item
                    for item in http["nondeterministic_fields"]  # type: ignore[union-attr]
                    if item["service"] == "core"
                ],
            },
        )

    inventory = _route_inventory()
    studio_cases = json.loads((output / "studio_http.json").read_text(encoding="utf-8"))[
        "service"
    ]["cases"]
    core_cases = json.loads((output / "core_http.json").read_text(encoding="utf-8"))[
        "service"
    ]["cases"]

    files: list[dict[str, object]] = []
    for path in sorted(item for item in output.rglob("*") if item.is_file()):
        if path.name == "manifest.json":
            continue
        relative = path.relative_to(output).as_posix()
        files.append(
            {
                "path": relative,
                "size_bytes": path.stat().st_size,
                "sha256": sha256_file(path),
                "sha256_role": (
                    "informational: SQLite file bytes are not asserted stable across "
                    "SQLite library versions"
                    if path.suffix == ".sqlite3"
                    else "exact"
                ),
            }
        )

    manifest: dict[str, object] = {
        "corpus_schema_version": CORPUS_SCHEMA_VERSION,        "kind": "python_compatibility_corpus",
        "status": (
            "compatibility-fixture preparation only; no Rust product code and no "
            "packaging cutover is authorized by this corpus"
        ),
        "fixture_root": FIXTURE_ROOT_RELATIVE,
        "corpus_sha256": corpus_digest(files),
        "corpus_sha256_scope": (
            "SHA-256 over the sorted (path, digest) list of the indexed fixture "
            "files; the SQLite fixture's file bytes are recorded as not-asserted "
            "because the SQLite library version changes them"
        ),
        "generator": {
            "path": GENERATOR_RELATIVE,
            "command": ".venv/bin/python backend/compat/generate_corpus.py",
            "check_command": ".venv/bin/python backend/compat/generate_corpus.py --check",
            "self_check_command": ".venv/bin/python backend/compat/self_check.py",
            "python_version": sys.version,
        },
        "oracle_environment": environment_facts(),
        "source_state": {
            "git": git_state(),
            "python_sources": source_tree_state(),
        },
        "determinism": {
            "fixed_clock_start": "2026-01-01T00:00:00+00:00",
            "json_fixtures_byte_identical_on_same_tree": True,
            "corpus_byte_identical_across_runs": True,
            "sqlite_file_bytes_asserted": False,
            "sqlite_logical_rows_asserted": True,
            "step_bytes_asserted": False,
            "stl_bytes_asserted": True,
            "normalized_fields": [
                "core GET /health data_dir",
                "CLI temporary data directory",
                "journal occurred_at via a fixture-only fixed clock",
                "studio journal occurred_at via a fixture-only fixed clock",
                "STEP writer timestamp and per-process translator counter",
                "design result STEP digest and the journal digests that bind it",
                "CAD provider probe result (availability, version and reason)",
            ],
            "normalisation": step_normalisation_facts(),
            "excluded_from_comparison": [
                {
                    "path": "files",
                    "reason": (
                        "the per-file list is covered by the per-file comparison and "
                        "by corpus_sha256"
                    ),
                },
                {
                    "path": "source_state.git",
                    "reason": (
                        "the working-tree path list names every unrelated file in "
                        "the repository, so it changes whenever any other worker "
                        "edits anything; the commit and the product source pin are "
                        "still compared"
                    ),
                },
                {
                    "path": "oracle_environment.cad_providers",
                    "reason": (
                        "ADR-0011 makes CAD provider availability a live probe of "
                        "the host, so the recorded result is a fact about this "
                        "host and not about the product; it is recorded for the "
                        "reader and never compared"
                    ),
                },
            ],
        },
        "coverage": {
            "studio_http": _coverage(inventory["studio"], studio_cases),
            "core_http": _coverage(inventory["core"], core_cases),
            "canonical_json_cases": len(canonical["cases"]),
            "chain_preimage_cases": len(chain["preimage_cases"]),
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
            "studio_body_comparisons": _body_comparison_counts(studio_cases),
        },
        "files": files,
        "known_gaps": [
            "No Rust implementation, Cargo workspace, packaging cutover, or benchmark is included.",
            "No Windows host, COM, NSIS, or Windows CI evidence exists; Windows milestone remains pending.",
            (
                "The live core route table exposes 17 routes including root /health "
                "(16 under /api/v1). The audit and task text say 18; the extra route "
                "could not be located in the current code. All 17 are covered."
            ),
            (
                "Journal timestamps are wall-clock in production. HTTP and CLI fixtures "
                "inject a fixed clock into kala_agent.persistence.journal.datetime so "
                "event digests are reproducible; this is a fixture override, not a "
                "change to product behavior."
            ),
            (
                "The Studio session journal is wall-clock in production too. The "
                "studio fixtures inject a fixed clock into "
                "kala_agent.studio.session_journal.datetime for the same reason; "
                "this is a fixture override, not a change to product behavior."
            ),
            (
                "The design result's STEP digest and the journal chain digests that "
                "bind it are masked with <VOLATILE>: the OCP STEP export embeds the "
                "wall clock of the write, so the product itself cannot reproduce "
                "them. The reproducible relations are frozen instead."
            ),
            (
                "The CAD provider probe results are evidence from the host. The "
                "recorded reasons change with the host's FreeCAD installation and "
                "are also recorded in oracle_environment.cad_providers."
            ),
            "HTTP is captured through Starlette TestClient, not a real uvicorn socket; transport-generated Date/Server headers are not frozen.",
            "The studio STEP 503-unavailable branch is not captured because OCP 8.0.1.1.0 is installed on this host.",
            "STEP is captured structurally for the default box only; cylinder and sphere STEP exports are not in the corpus.",
            "Core HTTP uses a fresh temporary database; it does not include a frozen core data-directory snapshot.",
            "The CLI serve command is resolved but not started; no live socket launch is recorded.",
            "Static-file traversal and symlink edge cases rely on existing Python security tests, not this corpus.",
            "The documented KALA_STUDIO_MESH_LIMIT knob is absent from the implementation and is not frozen.",
            "Timing and RSS are deliberately excluded; comparison is deferred until a release Rust build exists.",
            "Pydantic validation message text is frozen for the recorded package versions and may drift on upgrade.",
        ],
    }
    write_json(output / "manifest.json", manifest)
    try:
        output.chmod(0o755)
    except OSError:
        pass
    return manifest


def _drop_path(payload: object, path: str) -> None:
    """Remove a dotted path from a nested mapping, when it is present."""
    segments = [part for part in path.split(".") if part]
    cursor: object = payload
    for segment in segments[:-1]:
        if not isinstance(cursor, dict):
            return
        cursor = cursor.get(segment)
    if isinstance(cursor, dict):
        cursor.pop(segments[-1], None)


def _strip_volatile(payload: object) -> object:
    copied = copy.deepcopy(payload)
    if not isinstance(copied, dict):
        return copied
    for path in VOLATILE_JSON_PATHS:
        cursor: object = copied
        for key in path[:-1]:
            if not isinstance(cursor, dict):
                break
            cursor = cursor.get(key)
        if isinstance(cursor, dict):
            cursor.pop(path[-1], None)

    def walk(value: object) -> None:
        if isinstance(value, dict):
            if value.get("byte_equality_contract") is False:
                value.pop("sha256", None)
                value.pop("header_lines", None)
                value.pop("timestamp_line", None)
            if value.get("body_deterministic") is False:
                value.pop("body_sha256", None)
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)

    walk(copied)
    # The manifest hashes the JSON files whose volatile fields were just
    # removed. Content comparison above covers those files; hash equality is
    # checked separately by self_check against the checked-in manifest.
    for path in EXCLUDED_FROM_COMPARISON:
        _drop_path(copied, path)
    return copied


def compare(expected_dir: Path, actual_dir: Path) -> list[str]:
    problems: list[str] = []
    expected_files = {
        path.relative_to(expected_dir).as_posix()
        for path in expected_dir.rglob("*")
        if path.is_file()
    }
    actual_files = {
        path.relative_to(actual_dir).as_posix()
        for path in actual_dir.rglob("*")
        if path.is_file()
    }
    if expected_files != actual_files:
        problems.append(
            f"file set differs: only-expected={sorted(expected_files - actual_files)} "
            f"only-actual={sorted(actual_files - expected_files)}"
        )
    for relative in sorted(expected_files & actual_files):
        expected_path = expected_dir / relative
        actual_path = actual_dir / relative
        if relative.endswith(".sqlite3"):
            continue
        if relative.endswith(".json"):
            expected = _strip_volatile(json.loads(expected_path.read_text(encoding="utf-8")))
            actual = _strip_volatile(json.loads(actual_path.read_text(encoding="utf-8")))
            if expected != actual:
                problems.append(f"{relative}: JSON contents differ")
            continue
        if expected_path.read_bytes() != actual_path.read_bytes():
            problems.append(f"{relative}: bytes differ")
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Generate the Python compatibility corpus for the Rust backend."
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUTPUT,
        help=f"output directory (default: {FIXTURE_ROOT_RELATIVE})",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="generate into a temporary directory and compare with --output",
    )
    arguments = parser.parse_args(argv)
    output = arguments.output.expanduser().resolve()
    if not arguments.check:
        manifest = generate_all(output)
        print(f"generated corpus at {output}")
        print(f"files: {len(manifest['files'])}")
        print(json.dumps(manifest["coverage"], indent=2, sort_keys=True))
        silence_native_shutdown()
        return 0

    with tempfile.TemporaryDirectory(prefix="kala-corpus-check-") as temporary:
        regenerated = Path(temporary) / "corpus"
        generate_all(regenerated)
        problems = compare(output, regenerated)
    if problems:
        print("determinism check FAILED")
        for problem in problems:
            print(f"- {problem}")
        return 1
    print(f"determinism check passed: {output}")
    silence_native_shutdown()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
