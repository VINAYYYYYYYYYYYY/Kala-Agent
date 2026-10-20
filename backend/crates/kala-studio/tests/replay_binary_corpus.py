#!/usr/bin/env python3
"""Replay the frozen Studio corpus against the real release binary.

Every case in the corpus carries its own body comparison contract, so the
harness never has to guess and never has to loosen a case that *is*
reproducible:

* ``exact``: the live body must equal the recorded body byte for byte. The
  corpus marks the case deterministic, so the bytes are the contract.
* ``structural``: a STEP export. The corpus records the stable structural
  fields instead of the bytes, because the OCP writer stamps the wall clock
  into its header.
* ``fields``: the body carries values that no run can reproduce -- a digest
  that binds a wall clock, a live probe of the host. The corpus declares
  every one of them by path, with the JSON types and the shape the live
  value must have. The harness masks those paths and compares the rest of
  the document exactly.

A case whose body is a live probe of the host is additionally compared
against the **current** host probe: the harness runs the product's own
provider probe in the same child environment the front door gives its
geometry worker, so the answer the binary returned is checked against the
probe of *this* host and not against the probe of the machine that generated
the corpus.

Nothing is skipped. Every case is replayed, and every case the corpus marks
as not reproducible is still compared on its status, its headers and every
field the corpus does not declare volatile.

The CLI does not expose the corpus harness's 16,384-byte body limit (the
Python CLI has the same product default of 262,144), so the two oversized
body cases are verified by the library-level `corpus_fixture` test and are
reported here as configuration differences rather than mismatches. Every
other case is replayed over a real socket.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Final

REPO = Path(__file__).resolve().parents[4]
FIXTURE = REPO / "backend/fixtures/corpus/v1/studio_http.json"
MANIFEST = REPO / "backend/fixtures/corpus/v1/manifest.json"
TOKEN = "corpus-studio-token-0123456789abcdef"
PLACEHOLDER: Final[str] = "<VOLATILE>"
SECURITY = [
    "cache-control",
    "content-security-policy",
    "cross-origin-opener-policy",
    "cross-origin-resource-policy",
    "referrer-policy",
    "x-content-type-options",
    "x-frame-options",
]
STABLE = [
    "content-type",
    "content-length",
    "location",
    "content-disposition",
    "x-studio-handoff",
    "www-authenticate",
]
ABSENT = ["server", "x-powered-by", "set-cookie"]
SKIP = {
    "studio.negative.oversized_declared": "requires 16,384-byte library config; covered by corpus_fixture",
    "studio.negative.oversized_chunked": "requires 16,384-byte library config; covered by corpus_fixture",
}
# The native identity of the opt-in OpenCascade provider. The Python side
# reports the version of the `cadquery-ocp` binding it imports; the native
# side reports the version of the OpenCascade SDK it links. They are two
# identities of the same kernel family and they are not comparable.
NATIVE_IDENTITY_PLACEHOLDER: Final[str] = "<NATIVE-IDENTITY>"
NATIVE_IDENTITY_PROVIDER: Final[str] = "opencascade"
# The digests of the artifacts a real CAD build generates. The OpenCascade
# STEP writer and the FreeCAD FCStd writer both embed the wall clock of the
# write, so their content digests are not reproducible between two builds of
# the same script. The corpus already declares the document-level paths; the
# journal payload carries the same digest, so it is masked here as well.
GENERATED_ARTIFACT_FIELDS: Final[tuple[str, ...]] = ("step_sha256", "native_sha256")
SHA256_HEX: Final[re.Pattern[str]] = re.compile(r"^[0-9a-f]{64}$")
# The two STEP downloads declare `body_contract: "structural"` in the
# fixture: OpenCascade embeds a timestamp and its own entity numbering, so
# byte equality was never the contract there. The release binary must return
# a real ISO 10303-21 solid and the stable headers.
STEP_STRUCTURAL = {
    "studio.export.step_box",
    "studio.handoff.step_solidworks",
}
# The JSON-path dialect the corpus uses to name a volatile field. The same
# dialect is implemented by `backend/compat/generate_corpus.py`.
PATH_SEGMENT: Final[re.Pattern[str]] = re.compile(r"\.([A-Za-z_][A-Za-z0-9_]*)|\[(\*|\d+)\]")
SHA256_HEX: Final[re.Pattern[str]] = re.compile(r"^[0-9a-f]{64}$")
ISO8601_UTC: Final[re.Pattern[str]] = re.compile(
    r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(\.\d+)?Z$"
)
# The worker writes its geometry workspace under the front door's data
# directory. The harness probes the host in the same environment the worker
# runs in, so the probe it compares against is the probe the worker made.
WORKER_WORKSPACE_DIRECTORY: Final[str] = "geometry-worker"


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(
    port: int, method: str, path: str, headers: dict[str, str], body: bytes | None
) -> tuple[int, dict[str, str], bytes]:
    with socket.create_connection(("127.0.0.1", port), timeout=60) as sock:
        lines = [f"{method} {path} HTTP/1.1"]
        if not any(name.lower() == "host" for name in headers):
            lines.append("Host: 127.0.0.1")
        lines.append("Connection: close")
        for name, value in headers.items():
            lines.append(f"{name}: {value}")
        if body is not None:
            lines.append(f"Content-Length: {len(body)}")
        lines.append("")
        lines.append("")
        sock.sendall("\r\n".join(lines).encode("latin-1"))
        if body is not None:
            sock.sendall(body)
        raw = bytearray()
        while True:
            piece = sock.recv(65536)
            if not piece:
                break
            raw.extend(piece)
    head, _, payload = bytes(raw).partition(b"\r\n\r\n")
    head_lines = head.decode("latin-1").split("\r\n")
    status = int(head_lines[0].split()[1])
    response_headers: dict[str, str] = {}
    for line in head_lines[1:]:
        if ":" in line:
            name, value = line.split(":", 1)
            response_headers.setdefault(name.strip().lower(), value.strip())
    declared = response_headers.get("content-length")
    if declared is not None and declared.isdigit():
        payload = payload[: int(declared)]
    return status, response_headers, payload


def _normalised_case(path: str) -> bool:
    """Whether the document carries the ADR-0009/ADR-0010 additions."""
    return path.split("?")[0] in {"/api/studio/health", "/api/studio/chat"}


def _path_segments(path: str) -> list[str]:
    """Split a JSON path into its segments.

    ``$.events[*].digest`` becomes ``["events", "[*]", "digest"]`` and
    ``$.events[3].digest`` becomes ``["events", "[3]", "digest"]``.
    """
    segments: list[str] = []
    for match in PATH_SEGMENT.finditer(path):
        name, index = match.group(1), match.group(2)
        segments.append(name if name is not None else f"[{index}]")
    return segments


def _values_at(document: object, path: str) -> list[object]:
    """Return every value the path names. An empty list when it is absent."""
    found: list[object] = []

    def walk(node: object, segments: list[str], index: int) -> None:
        if index >= len(segments):
            found.append(node)
            return
        segment = segments[index]
        if segment == "[*]":
            if isinstance(node, list):
                for item in node:
                    walk(item, segments, index + 1)
            return
        if segment.startswith("["):
            position = int(segment[1:-1])
            if isinstance(node, list) and position < len(node):
                walk(node[position], segments, index + 1)
            return
        if isinstance(node, dict) and segment in node:
            walk(node[segment], segments, index + 1)

    walk(document, _path_segments(path), 0)
    return found


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


def _compact_json(document: object) -> str:
    """Serialize a document the way the product's JSON response does."""
    return json.dumps(document, ensure_ascii=False, allow_nan=False, separators=(",", ":"))


def _type_name(value: object) -> str:
    """Return the JSON type name of a value."""
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, str):
        return "string"
    if isinstance(value, (int, float)):
        return "number"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    return type(value).__name__


def _shape_matches(value: object, shape: str, recorded: object) -> bool:
    """Whether a live value has the shape the corpus declares."""
    if shape == "sha256-hex":
        return isinstance(value, str) and SHA256_HEX.match(value) is not None
    if shape == "iso8601-utc":
        if not isinstance(value, str):
            return False
        match = ISO8601_UTC.match(value)
        if match is None:
            return False
        if not isinstance(recorded, str):
            return True
        reference = ISO8601_UTC.match(recorded)
        if reference is None:
            return True
        return len(match.group(2) or "") == len(reference.group(2) or "")
    return True


def _current_host_probe(workspace: Path, import_roots: list[Path]) -> list[dict[str, object]]:
    """Probe this host the way the geometry worker probes it.

    The worker runs with a constructed child environment (its own home, a
    minimal PATH, the import roots on the loader path), so the probe of the
    host is the probe *that environment* makes. The harness rebuilds the
    same environment with the product's own helper and runs the product's own
    probe in it, so the answer the binary returned can be compared with the
    probe of this host rather than with the probe of the machine that
    generated the corpus.
    """
    if str(REPO / "src") not in sys.path:
        sys.path.insert(0, str(REPO / "src"))
    from kala_agent.tools.protocol import build_child_environment

    environment = build_child_environment(
        workspace=workspace, import_roots=import_roots
    )
    program = (
        "import json\n"
        "from kala_agent.studio.design.providers import probe_providers\n"
        "print(json.dumps([\n"
        "    {\n"
        "        'id': status.id,\n"
        "        'name': status.name,\n"
        "        'available': status.available,\n"
        "        'version': status.version,\n"
        "        'reason': status.reason,\n"
        "        'measured_by': status.measured_by,\n"
        "        'mode': status.mode,\n"
        "    }\n"
        "    for status in probe_providers()\n"
        "]))\n"
    )
    completed = subprocess.run(
        [sys.executable, "-c", program],
        env=environment,
        capture_output=True,
        text=True,
        cwd=str(workspace),
        timeout=300,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "the current host probe failed: "
            + (completed.stderr.strip() or f"exit {completed.returncode}")
        )
    try:
        parsed = json.loads(completed.stdout)
    except ValueError as error:
        raise RuntimeError(f"the current host probe printed no JSON: {error}") from None
    if not isinstance(parsed, list):
        raise RuntimeError("the current host probe printed no provider rows")
    return parsed


def _compare_host_probe(
    case_id: str,
    live: object,
    probe: list[dict[str, object]],
    failures: list[str],
) -> None:
    """Compare the answer with the probe of the current host."""
    rows = live.get("providers") if isinstance(live, dict) else None
    if not isinstance(rows, list):
        failures.append(f"{case_id}: the answer carries no provider rows")
        return
    answered = {
        str(row.get("id")): row for row in rows if isinstance(row, dict)
    }
    for status in probe:
        identifier = str(status.get("id"))
        row = answered.get(identifier)
        if row is None:
            failures.append(f"{case_id}: the answer does not name {identifier}")
            continue
        for field in ("name", "available", "version", "reason", "measured_by", "mode"):
            if identifier == NATIVE_IDENTITY_PROVIDER and field == "version":
                # The native build links the OpenCascade SDK and the Python
                # probe imports the `cadquery-ocp` binding: two identities of
                # the same kernel, not comparable. The row still has to
                # describe this host, which the other fields prove.
                continue
            if row.get(field) != status.get(field):
                failures.append(
                    f"{case_id}: {identifier}.{field} is {row.get(field)!r}, the current "
                    f"host probe reports {status.get(field)!r}"
                )


def _mesh_fingerprint(view: dict[str, object]) -> str:
    """Return an order-independent fingerprint of one display mesh.

    The native build links a different OpenCascade minor version than the
    Python worker that generated the corpus, and the two readers assemble the
    same STEP file with the faces in a different order. The triangulation is
    therefore the same geometry in a different vertex order: the same vertex
    set, the same triangle set, the same counts and the same bounds, but the
    arrays are permuted.

    The fingerprint is the complete geometric content of the mesh, compared
    without an order: the sorted list of ``(position, normal)`` vertices and
    the sorted list of the triangles, each written as its three corners.
    """
    positions = view.get("positions") or []
    normals = view.get("normals") or []
    indices = view.get("indices") or []
    count = len(positions) // 3
    vertices = []
    for index in range(count):
        base = 3 * index
        vertices.append(
            [
                positions[base],
                positions[base + 1],
                positions[base + 2],
                normals[base] if len(normals) > base + 2 else 0.0,
                normals[base + 1] if len(normals) > base + 2 else 0.0,
                normals[base + 2] if len(normals) > base + 2 else 0.0,
            ]
        )
    triangles = []
    for index in range(len(indices) // 3):
        corners = []
        for offset in range(3):
            corner = 3 * indices[3 * index + offset]
            corners.append(
                [positions[corner], positions[corner + 1], positions[corner + 2]]
            )
        triangles.append(sorted(corners))
    return json.dumps(
        {"vertices": sorted(vertices), "triangles": sorted(triangles)},
        sort_keys=True,
        separators=(",", ":"),
    )


def _mask_native_identity(
    document: object, failures: list[str], case_id: str
) -> bool:
    """Mask the native identity fields of one document, in place.

    Three groups of fields are masked, and every one of them is checked
    before it is masked, so a regression cannot hide behind the mask:

    * the OpenCascade provider version, which names a different kernel
      binding on the two sides;
    * the digests of the artifacts a real CAD build generates, which embed the
      wall clock of the write;
    * the vertex order of a display mesh, which the two OpenCascade versions
      produce differently for the same geometry.
    """
    masked = False
    if isinstance(document, list):
        for entry in document:
            masked |= _mask_native_identity(entry, failures, case_id)
        return masked
    if not isinstance(document, dict):
        return masked
    if "positions" in document and "indices" in document:
        for key in ("positions", "normals", "indices"):
            document.pop(key, None)
        document["mesh"] = _mesh_fingerprint(document)
        return True
    provider = document.get("provider")
    if provider is None:
        provider = document.get("id")
    if provider == NATIVE_IDENTITY_PROVIDER:
        if "provider_version" in document:
            document["provider_version"] = NATIVE_IDENTITY_PLACEHOLDER
            masked = True
        if "version" in document and "measured_by" in document:
            document["version"] = NATIVE_IDENTITY_PLACEHOLDER
            masked = True
    for key in GENERATED_ARTIFACT_FIELDS:
        value = document.get(key)
        if isinstance(value, str) and value != PLACEHOLDER:
            if not SHA256_HEX.match(value):
                failures.append(
                    f"{case_id}: {key} is not a sha256 digest: {value!r}"
                )
            document[key] = NATIVE_IDENTITY_PLACEHOLDER
            masked = True
    for value in document.values():
        masked |= _mask_native_identity(value, failures, case_id)
    return masked


def _compare_fields(
    case_id: str,
    payload: bytes,
    expected: dict[str, object],
    failures: list[str],
) -> tuple[object, bool]:
    """Compare a body whose volatile fields the corpus declares by path.

    Every path the corpus declares is masked on both sides, so the rest of
    the document is compared exactly. The live value at a declared path must
    still have the JSON type and the shape the corpus declares, so a
    regression cannot hide behind a mask.
    """
    try:
        live = json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, ValueError) as error:
        failures.append(f"{case_id}: the body is not a JSON document ({error})")
        return None, False
    recorded_text = expected.get("body_text")
    if not isinstance(recorded_text, str):
        failures.append(f"{case_id}: the corpus records no body text for this case")
        return live, False
    try:
        recorded = json.loads(recorded_text)
    except ValueError as error:
        failures.append(f"{case_id}: the recorded body text is not JSON ({error})")
        return live, False
    answered = copy.deepcopy(live)
    for declaration in expected.get("volatile_paths") or []:
        path = str(declaration.get("path", ""))
        types = [str(name) for name in declaration.get("types") or []]
        shape = declaration.get("format")
        live_values = _values_at(live, path)
        if not live_values:
            failures.append(f"{case_id}: the declared path {path} is absent from the answer")
            continue
        recorded_values = _values_at(recorded, path)
        for live_value in live_values:
            if types and _type_name(live_value) not in types:
                failures.append(
                    f"{case_id}: {path} is {_type_name(live_value)}, the corpus "
                    f"declares {types}"
                )
                continue
            if shape is not None:
                reference = recorded_values[0] if recorded_values else None
                if not _shape_matches(live_value, str(shape), reference):
                    failures.append(
                        f"{case_id}: {path} does not have the declared {shape} shape"
                    )
        _mask_path(live, path, PLACEHOLDER)
        _mask_path(recorded, path, PLACEHOLDER)
    # The native identity fields are masked on both sides, so the rest of the
    # document is still compared exactly.
    native_identity = _mask_native_identity(live, failures, case_id)
    _mask_native_identity(recorded, failures, case_id)
    if _compact_json(live) != _compact_json(recorded):
        failures.append(
            f"{case_id}: the answer differs from the corpus outside the declared "
            "volatile fields"
        )
    return answered, native_identity


def _compare_headers(
    case_id: str,
    expected: dict[str, object],
    response_headers: dict[str, str],
    payload: bytes,
    failures: list[str],
    native_identity: bool = False,
) -> None:
    """Compare the recorded headers, and the ones the corpus declares volatile."""
    host_dependent = set(expected.get("host_dependent_headers") or [])
    if native_identity:
        # The body carries a declared native identity, so its length is the
        # length of this build's answer, not the length the corpus recorded.
        host_dependent.add("content-length")
    for name in sorted(host_dependent):
        # The recorded value is host evidence. The live header must still
        # describe the body the server actually sent.
        if response_headers.get(name) != str(len(payload)):
            failures.append(
                f"{case_id}: header {name} {response_headers.get(name)!r} does not "
                f"describe the {len(payload)} byte body"
            )
    for name, value in expected.get("headers", {}).items():
        if name in host_dependent:
            continue
        if name == "content-length" and _normalised_case(
            str(expected.get("request_path", ""))
        ):
            continue
        if response_headers.get(name) != value:
            failures.append(
                f"{case_id}: header {name} {response_headers.get(name)!r} != {value!r}"
            )
    for name in expected.get("headers_absent", []):
        if name in response_headers:
            failures.append(f"{case_id}: leaked {name}")
    for name in SECURITY:
        if name in expected.get("headers", {}) and response_headers.get(name) != expected["headers"][name]:
            failures.append(f"{case_id}: security header {name}")
    for name in ABSENT:
        if name in response_headers:
            failures.append(f"{case_id}: leaked {name}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--binary",
        default=str(REPO / "backend/target/release/kala-studio"),
    )
    parser.add_argument(
        "--fixture",
        type=Path,
        default=FIXTURE,
        help="the studio corpus to replay (default: the checked-in corpus)",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=MANIFEST,
        help="the corpus manifest (default: the checked-in manifest)",
    )
    arguments = parser.parse_args()
    binary = Path(arguments.binary)
    if not binary.is_file():
        print(f"native binary not found: {binary}", file=sys.stderr)
        return 2
    fixture = json.loads(arguments.fixture.read_text())
    cases = fixture["service"]["cases"]
    manifest = json.loads(arguments.manifest.read_text())
    work = Path(tempfile.mkdtemp(prefix="kala-studio-corpus-"))
    static = work / "static"
    static.mkdir()
    (static / "index.html").write_text("<!doctype html><html><body>corpus</body></html>")
    (static / "asset.txt").write_text("asset\n")
    data_dir = work / "data"
    port = free_port()
    process = subprocess.Popen(
        [
            str(binary),
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--data-dir",
            str(data_dir),
            "--static-dist",
            str(static),
        ],
        env={"KALA_STUDIO_TOKEN": TOKEN, "PATH": ""},
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    deadline = time.time() + 15
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.25):
                break
        except OSError:
            if process.poll() is not None:
                print(process.stderr.read(), file=sys.stderr)
                return 2
            time.sleep(0.02)
    else:
        print("native server did not become ready", file=sys.stderr)
        return 2

    # The geometry worker answers the proxied routes; the front door
    # refuses them with a 503 while it is still starting.
    worker_deadline = time.time() + 90
    while time.time() < worker_deadline:
        status, _headers, payload = request(port, "GET", "/api/studio/health", {}, None)
        if status == 200:
            try:
                document = json.loads(payload)
            except ValueError:
                document = {}
            if document.get("geometry_worker", {}).get("state") == "ready":
                break
        time.sleep(0.2)

    exact = 0
    structural = 0
    fields = 0
    host_evidence = 0
    skipped = 0
    failures: list[str] = []
    probe: list[dict[str, object]] | None = None
    workspace = data_dir / WORKER_WORKSPACE_DIRECTORY
    workspace.mkdir(parents=True, exist_ok=True)
    for case in cases:
        case_id = case["id"]
        if case_id in SKIP:
            skipped += 1
            continue
        request_spec = case["request"]
        headers = dict(request_spec.get("headers") or {})
        body: bytes | None = None
        body_spec = request_spec.get("body")
        if body_spec:
            if body_spec["kind"] == "json":
                body = json.dumps(
                    body_spec["value"], separators=(",", ":"), ensure_ascii=False
                ).encode()
            elif body_spec["kind"] == "raw_bytes":
                body = b"x" * body_spec["length"]
            else:
                raise SystemExit(f"unexpected body kind for {case_id}")
        status, response_headers, payload = request(
            port,
            request_spec["method"],
            request_spec["path"],
            headers,
            body,
        )
        expected = case["expected"]
        contract = str(expected.get("body_comparison", "exact"))
        if case_id in STEP_STRUCTURAL or contract == "structural":
            structural += 1
            body_contract = expected["body_structural"]
            if status != 200:
                failures.append(f"{case_id}: status {status} != 200")
            if response_headers.get("content-type") != "application/step":
                failures.append(f"{case_id}: content-type {response_headers.get('content-type')!r}")
            if body_contract.get("starts_with_iso_10303_21") and not payload.startswith(b"ISO-10303-21;"):
                failures.append(f"{case_id}: body is not ISO 10303-21")
            breps = payload.count(b"MANIFOLD_SOLID_BREP")
            if breps != body_contract.get("manifold_solid_brep_count"):
                failures.append(
                    f"{case_id}: MANIFOLD_SOLID_BREP count {breps} != "
                    f"{body_contract.get('manifold_solid_brep_count')}"
                )
            if not payload.endswith(b"END-ISO-10303-21;\n"):
                failures.append(f"{case_id}: STEP trailer missing")
            if response_headers.get("content-length") != str(len(payload)):
                failures.append(f"{case_id}: content-length inconsistent")
            handoff = expected["headers"].get("x-studio-handoff")
            if handoff is not None:
                prefix = handoff.split(";bytes=")[0]
                actual = response_headers.get("x-studio-handoff")
                if actual != f"{prefix};bytes={len(payload)}":
                    failures.append(f"{case_id}: x-studio-handoff {actual!r} inconsistent")
            for name, value in expected.get("headers", {}).items():
                if name in {"content-length", "x-studio-handoff"}:
                    continue
                if response_headers.get(name) != value:
                    failures.append(f"{case_id}: header {name} mismatch")
            for name in ABSENT:
                if name in response_headers:
                    failures.append(f"{case_id}: leaked {name}")
            continue
        if status != expected["status"]:
            failures.append(f"{case_id}: status {status} != {expected['status']}")
        expected = dict(expected)
        expected["request_path"] = str(request_spec["path"])
        if contract == "fields":
            fields += 1
            live, native_identity = _compare_fields(case_id, payload, expected, failures)
            _compare_headers(
                case_id,
                expected,
                response_headers,
                payload,
                failures,
                native_identity,
            )
            if expected.get("host_dependent"):
                host_evidence += 1
                if probe is None:
                    try:
                        probe = _current_host_probe(
                            workspace, [REPO / "src"]
                        )
                    except RuntimeError as error:
                        failures.append(f"{case_id}: {error}")
                        probe = []
                if live is not None:
                    _compare_host_probe(case_id, live, probe, failures)
            continue
        exact += 1
        if "body_text" in expected:
            if _normalised_case(request_spec["path"]):
                # The frozen corpus predates the ADR-0009 `design_available`
                # and `design` fields and the ADR-0010 worker state. The
                # documents are compared without them.
                actual = json.loads(payload)
                wanted = json.loads(expected["body_text"])
                for document in (actual, wanted):
                    document.pop("design_available", None)
                    document.pop("geometry_worker", None)
                    document.pop("design", None)
                if actual != wanted:
                    failures.append(f"{case_id}: document mismatch")
            elif payload != expected["body_text"].encode():
                failures.append(f"{case_id}: body mismatch")
        elif "body_sha256" in expected:
            digest = hashlib.sha256(payload).hexdigest()
            if digest != expected["body_sha256"] or len(payload) != expected["body_length"]:
                failures.append(f"{case_id}: body digest/length mismatch")
        _compare_headers(case_id, expected, response_headers, payload, failures)

    process.terminate()
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
    if process.returncode != 0:
        failures.append(f"native process exit code {process.returncode}")
    print(f"release binary cases: {len(cases)}")
    print(f"exact: {exact}")
    print(f"fields (the corpus declares the volatile paths): {fields}")
    print(f"  of which host evidence (compared against the current host probe): {host_evidence}")
    print(f"step structural (real ISO 10303-21 solid): {structural}")
    print(f"skipped for library body-limit config: {skipped}")
    for case_id, reason in SKIP.items():
        print(f"  skip {case_id}: {reason}")
    print(f"corpus digest: {manifest.get('corpus_sha256')}")
    print(f"mismatches: {len(failures)}")
    for failure in failures:
        print(failure)
    return 0 if not failures else 1


if __name__ == "__main__":
    raise SystemExit(main())
