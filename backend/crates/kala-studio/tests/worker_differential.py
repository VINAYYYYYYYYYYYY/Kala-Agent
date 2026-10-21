#!/usr/bin/env python3
"""Live Python-vs-native Studio differential through the Rust front door.

This is a test oracle, not product code. It launches the real native binary
with the real supervised Python geometry worker, builds the live Python
FastAPI application in this process with an empty environment view, and
replays the frozen Studio corpus plus the ADR-0009/ADR-0011 design routes
through both.

Three oracles are used:

* the **Python public server** for the routes it mounts, including the
  ADR-0011 CAD provider route;
* a **directly launched Python worker** for the worker-only routes the
  public server does not expose yet (`/import`, `journal/append`,
  `intent/*`);
* the **native front door**, which proxies the geometry, design, journal and
  import routes to its own worker.

The journal documents carry a timestamp and a chain digest, so they are
never byte-equal across two processes. The comparison replaces the volatile
fields (`occurred_at`, the payload digest, the chain digests and the journal
head) with a placeholder on both sides and compares everything else, so the
event sequence, the event types, the aggregates and the payloads are still
compared exactly.

One more normalisation exists for the chat cases that build a design: the
STEP and FCStd writers embed their build time, so two independent builds of
the same script have different artifact digests by construction
(reproducibly, at the STEP `FILE_NAME` timestamp). Those two generated
digests are compared as a placeholder when a case marks them volatile; the
design id, the provider, the version, every measured value, the checks, the
view and the script digest are still compared exactly.

The STEP downloads are compared structurally: OpenCascade's writer embeds a
timestamp and its own entity numbering, so the fixture contract for those
cases is a real ISO 10303-21 file with one manifold solid and the stable
headers, not byte equality.
"""

from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[4]
SRC = REPO / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

sys.path.insert(0, str(Path(__file__).resolve().parent))

from python_differential import (  # noqa: E402
    ABSENT,
    SECURITY,
    STABLE,
    TOKEN,
    build_cases,
    free_port,
    native_request,
    step_structure,
)

VENV_PYTHON = REPO / ".venv" / "bin" / "python"
WORKER_PREFIX = "/internal/v1"
WORKER_TOKEN = "differential-worker-token-0123456789abcdef"

# The ADR-0011 end-to-end sentence: a real FreeCAD build through the chat
# route must equal the live Python public server's answer.
STITCH_PROMPT = (
    "Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm wall "
    "thickness, with a 10 mm centered bore and 4x M5 corner mounting holes."
)

# The volatile fields of a journal document. They carry a timestamp and a
# chain digest, which are different in two processes by construction.
VOLATILE_JOURNAL_FIELDS = (
    "occurred_at",
    "digest",
    "event_digest",
    "previous_event_digest",
    "head_digest",
    "journal_head",
)
VOLATILE_PLACEHOLDER = "<volatile>"

# A real CAD build embeds its build time in the exported files: the FreeCAD
# and OpenCascade STEP writers put the second they ran into the ``FILE_NAME``
# header, and the FreeCAD FCStd is a zip whose entries carry the build time.
# Two independent builds therefore have different artifact digests by
# construction (reproducible: two consecutive builds of the identical script
# differ at the STEP ``FILE_NAME`` timestamp). A case can mark the two
# generated digests as volatile, and the chat cases that build a design do;
# the semantic result - design id, provider, version, measurements, checks,
# view and script digest - is still compared exactly.
GENERATED_ARTIFACT_FIELDS = ("step_sha256", "native_sha256")

# The native identity of the opt-in OpenCascade provider. The Python side
# reports the version of the ``cadquery-ocp`` binding it imports
# (``OCP.__version__``); the native side reports the version of the
# OpenCascade SDK it links (``OCC_VERSION_COMPLETE``). They are two
# identities of the same kernel family, they are not comparable, and no
# number is substituted for either of them. The field is therefore replaced
# with a placeholder **only** for the ``opencascade`` provider, so the
# FreeCAD and Solid Edge identities are still compared exactly.
NATIVE_IDENTITY_PLACEHOLDER = "<native-identity>"
NATIVE_IDENTITY_PROVIDER = "opencascade"

# The accepted worker contract (r1) predates the provider-aware public
# design request. The front door forwards the body unchanged, as the cell
# task requires, so these two routes differ until the worker contract is
# extended on the Python side.
WORKER_PROVIDER_GAP = (
    "the accepted worker's DesignIntent has no provider field, while the "
    "public design request requires one"
)

# The health document the Rust front door extends with the ADR-0010 worker
# state. The Python public server has no worker, so the field is compared
# separately.
HEALTH_RUST_ONLY_FIELDS = ("geometry_worker",)


def native_identity_fields(entry: dict[str, Any]) -> tuple[str, ...]:
    """Return the fields of one document that carry a native identity.

    A design document and a journal payload name their provider in the
    sibling ``provider`` field; a CAD provider row names it in ``id``.
    """
    provider = entry.get("provider")
    if provider is None:
        provider = entry.get("id")
    if provider != NATIVE_IDENTITY_PROVIDER:
        return ()
    if "provider_version" in entry:
        return ("provider_version",)
    if "version" in entry and "measured_by" in entry:
        return ("version",)
    return ()


def mesh_fingerprint(view: dict[str, Any]) -> str:
    """Return an order-independent fingerprint of one display mesh.

    The native build links a different OpenCascade minor version than the
    Python oracle, and the two readers assemble the same STEP file with the
    faces in a different order. The triangulation is therefore the same
    geometry in a different vertex order: the same vertex set, the same
    triangle set, the same counts and the same bounds, but the arrays are
    permuted.

    The fingerprint is the complete geometric content of the mesh, compared
    without an order: the sorted list of ``(position, normal)`` vertices and
    the sorted list of the triangles, each written as its three corners. Two
    meshes with the same fingerprint draw exactly the same surface.
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


def normalise(value: Any, *, generated_artifacts: bool = False) -> Any:
    """Replace the volatile journal fields (and generated digests) with a placeholder."""
    if isinstance(value, dict):
        if "positions" in value and "indices" in value:
            # A display mesh: compare its geometry without an order.
            rest = {
                key: normalise(entry, generated_artifacts=generated_artifacts)
                for key, entry in value.items()
                if key not in ("positions", "normals", "indices")
            }
            rest["mesh"] = mesh_fingerprint(value)
            return rest
        identity = native_identity_fields(value)
        return {
            key: (
                VOLATILE_PLACEHOLDER
                if key in VOLATILE_JOURNAL_FIELDS
                or (
                    generated_artifacts
                    and key in GENERATED_ARTIFACT_FIELDS
                    and isinstance(entry, str)
                )
                else (
                    NATIVE_IDENTITY_PLACEHOLDER
                    if key in identity
                    else normalise(entry, generated_artifacts=generated_artifacts)
                )
            )
            for key, entry in value.items()
        }
    if isinstance(value, list):
        return [normalise(entry, generated_artifacts=generated_artifacts) for entry in value]
    return value


def canonical(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def start_worker(data_dir: Path) -> tuple[subprocess.Popen[bytes], int]:
    """Start one Python geometry worker and return it with its port."""
    data_dir.mkdir(parents=True, exist_ok=True)
    environment = dict(os.environ)
    environment["KALA_WORKER_TOKEN"] = WORKER_TOKEN
    environment["KALA_WORKER_STDIN_LIFELINE"] = "1"
    environment["PYTHONPATH"] = str(SRC)
    process = subprocess.Popen(
        [
            str(VENV_PYTHON),
            "-m",
            "kala_agent.studio",
            "--worker",
            "--data-dir",
            str(data_dir),
        ],
        env=environment,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        cwd=str(REPO),
    )
    deadline = time.time() + 60
    while time.time() < deadline:
        line = process.stdout.readline()
        if not line:
            break
        text = line.decode("utf-8", "replace").strip()
        if text.startswith("KALA_WORKER_READY"):
            payload = json.loads(text.split(" ", 1)[1])
            return process, int(payload["port"])
    process.kill()
    raise SystemExit("the python geometry worker did not announce readiness")


def worker_environment(work: Path) -> dict[str, str]:
    """Return the environment view the supervised worker process runs under.

    The CAD provider probe reads ``PATH`` and needs a writable ``HOME``:
    ``FreeCADCmd`` creates its user directory on the first run, and without
    one it reports "Could not create directories" instead of a version. The
    public application must therefore see the same environment as the
    supervised worker, or the two availability reports would differ for a
    reason that has nothing to do with the code under test.

    The values mirror the ``kala-process`` child environment the front door
    builds for the worker, so both sides probe the same host the same way.
    """
    home = work / "public-home"
    home.mkdir(parents=True, exist_ok=True)
    return {
        "HOME": str(home),
        "LANG": "C.UTF-8",
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "PYTHONDONTWRITEBYTECODE": "1",
        "PYTHONHASHSEED": "0",
        "PYTHONIOENCODING": "utf-8",
        "PYTHONNOUSERSITE": "1",
        "PYTHONPATH": str(SRC),
        "PYTHONSAFEPATH": "1",
        "TMPDIR": str(home),
    }


def worker_request(
    port: int, method: str, path: str, body: bytes | None = None
) -> tuple[int, dict[str, str], bytes]:
    """Call one worker route with the worker's own bearer token."""
    lines = [f"{method} {path} HTTP/1.1", "Host: 127.0.0.1:{port}".format(port=port)]
    lines.append("Authorization: Bearer " + WORKER_TOKEN)
    lines.append("Connection: close")
    if body is not None:
        lines.append("content-type: application/json")
        lines.append(f"Content-Length: {len(body)}")
    lines.append("")
    raw = b"".join(line.encode("latin-1") + b"\r\n" for line in lines)
    if body is not None:
        raw += body
    with socket.create_connection(("127.0.0.1", port), timeout=120) as sock:
        sock.sendall(raw)
        collected = bytearray()
        while True:
            piece = sock.recv(65536)
            if not piece:
                break
            collected.extend(piece)
    head, _, payload = bytes(collected).partition(b"\r\n\r\n")
    head_lines = head.decode("latin-1").split("\r\n")
    status = int(head_lines[0].split()[1])
    headers: dict[str, str] = {}
    for line in head_lines[1:]:
        if ":" in line:
            name, value = line.split(":", 1)
            headers.setdefault(name.strip().lower(), value.strip())
    declared = headers.get("content-length")
    if declared is not None and declared.isdigit():
        payload = payload[: int(declared)]
    return status, headers, payload


def design_cases() -> list[dict[str, Any]]:
    """The ADR-0009/ADR-0011 design routes through the public API."""
    bracket = {
        "template": {
            "template": "l_bracket",
            "width": 60.0,
            "depth": 45.0,
            "height": 40.0,
            "wall": 6.0,
        }
    }
    cases = [
        {
            "name": "design.with_provider",
            "method": "POST",
            "path": "/api/studio/design",
            "json_body": {**bracket, "provider": "opencascade"},
            "volatile": True,
            "generated_artifacts_volatile": True,
        },
        {
            "name": "design.without_provider",
            "method": "POST",
            "path": "/api/studio/design",
            "json_body": bracket,
            "volatile": True,
        },
        {
            "name": "design.empty_body",
            "method": "POST",
            "path": "/api/studio/design",
            "json_body": {},
        },
        {
            "name": "design.bad_template",
            "method": "POST",
            "path": "/api/studio/design",
            "json_body": {"template": {"template": "torus"}},
        },
        {
            "name": "design.both_sources",
            "method": "POST",
            "path": "/api/studio/design",
            "json_body": {"template": bracket["template"], "script": {}},
        },
        {
            "name": "design.journal_verify",
            "method": "GET",
            "path": "/api/studio/journal/verify",
            "volatile": True,
        },
        {
            "name": "design.journal_page",
            "method": "GET",
            "path": "/api/studio/journal?after=0&limit=5",
            "volatile": True,
        },
        {
            "name": "design.journal_page_limit",
            "method": "GET",
            "path": "/api/studio/journal?after=0&limit=900",
        },
        {
            "name": "design.journal_verify_no_token",
            "method": "GET",
            "path": "/api/studio/journal/verify",
            "auth": False,
        },
        {
            "name": "design.journal_no_token",
            "method": "GET",
            "path": "/api/studio/journal",
            "auth": False,
        },
        {
            "name": "design.drawing_bad_id",
            "method": "GET",
            "path": "/api/studio/design/not-a-digest/drawing",
        },
        {
            "name": "design.section_bad_id",
            "method": "GET",
            "path": "/api/studio/design/not-a-digest/section",
        },
        {
            "name": "design.unknown_design",
            "method": "GET",
            "path": "/api/studio/design/" + ("a" * 64) + "/drawing",
        },
        {
            "name": "design.providers_cad",
            "method": "GET",
            "path": "/api/studio/providers/cad",
            "volatile": True,
        },
        {
            "name": "design.chat_with_provider",
            "method": "POST",
            "path": "/api/studio/chat",
            "json_body": {
                "provider": "local",
                "prompt": "make me an L-bracket 60 x 45 x 40 mm with a 6 mm wall",
                "cad_provider": "opencascade",
            },
            "volatile": True,
            "generated_artifacts_volatile": True,
        },
        # ADR-0011 through the chat route: every CAD provider is passed to the
        # worker unchanged. The FreeCAD case is a REAL FreeCADCmd build and
        # must equal the live Python server's answer, including the reply
        # text; Solid Edge must relay the worker's honest refusal.
        {
            "name": "design.chat_with_freecad",
            "method": "POST",
            "path": "/api/studio/chat",
            "json_body": {
                "provider": "local",
                "prompt": STITCH_PROMPT,
                "cad_provider": "freecad",
            },
            "volatile": True,
            "generated_artifacts_volatile": True,
        },
        {
            "name": "design.chat_with_opencascade",
            "method": "POST",
            "path": "/api/studio/chat",
            "json_body": {
                "provider": "local",
                "prompt": STITCH_PROMPT,
                "cad_provider": "opencascade",
            },
            "volatile": True,
            "generated_artifacts_volatile": True,
        },
        {
            "name": "design.chat_with_solidedge",
            "method": "POST",
            "path": "/api/studio/chat",
            "json_body": {
                "provider": "local",
                "prompt": STITCH_PROMPT,
                "cad_provider": "solidedge",
            },
            "volatile": True,
        },
        {
            "name": "design.chat_bad_cad_provider",
            "method": "POST",
            "path": "/api/studio/chat",
            "json_body": {
                "provider": "local",
                "prompt": "x",
                "cad_provider": "nope",
            },
        },
        {
            "name": "design.chat_cad_provider_type",
            "method": "POST",
            "path": "/api/studio/chat",
            "json_body": {
                "provider": "local",
                "prompt": "x",
                "cad_provider": 1,
            },
        },
        {
            "name": "design.method_not_allowed",
            "method": "POST",
            "path": "/api/studio/journal/verify",
            "json_body": {},
        },
    ]
    return cases


def build_request(item: dict[str, Any]) -> tuple[list[tuple[str, str]], bytes | None]:
    headers: list[tuple[str, str]] = []
    body: bytes | None = None
    if item.get("auth", True):
        headers.append(("Authorization", f"Bearer {TOKEN}"))
    if item.get("json_body") is not None:
        body = json.dumps(item["json_body"], separators=(",", ":")).encode()
        headers.append(("content-type", "application/json"))
    elif item.get("body") is not None:
        body = item["body"]
        headers.append(("content-type", item.get("content_type", "application/json")))
    return headers, body


def health_documents(python_body: bytes, native_body: bytes) -> bool:
    """Whether both health answers are JSON documents to compare."""
    for body in (python_body, native_body):
        try:
            document = json.loads(body)
        except ValueError:
            return False
        if not isinstance(document, dict):
            return False
    return True


def difference(left: Any, right: Any, path: str = "") -> list[str]:
    """Return the paths at which two normalised documents differ."""
    if isinstance(left, dict) and isinstance(right, dict):
        if list(left.keys()) != list(right.keys()):
            return [f"{path}: keys {list(left.keys())} != {list(right.keys())}"]
        found: list[str] = []
        for key in left:
            found.extend(difference(left[key], right[key], f"{path}.{key}"))
            if len(found) > 8:
                break
        return found
    if isinstance(left, list) and isinstance(right, list):
        if len(left) != len(right):
            return [f"{path}: length {len(left)} != {len(right)}"]
        found = []
        for index, (one, other) in enumerate(zip(left, right)):
            found.extend(difference(one, other, f"{path}[{index}]"))
            if len(found) > 8:
                break
        return found
    if left != right:
        return [f"{path}: {left!r} != {right!r}"]
    return []


def compare(
    name: str,
    python_status: int,
    python_headers: dict[str, str],
    python_body: bytes,
    native_status: int,
    native_headers: dict[str, str],
    native_body: bytes,
    volatile: bool,
    structural: bool,
    mismatches: list[str],
    generated_artifacts: bool = False,
) -> None:
    """Compare one answer, honouring the documented structural contracts."""
    if structural:
        python_shape = step_structure(python_body)
        native_shape = step_structure(native_body)
        if (
            python_status != native_status
            or python_status != 200
            or not native_shape["iso"]
            or not native_shape["trailer"]
            or native_shape["breps"] != 1
            or python_shape["breps"] != 1
            or native_headers.get("content-type") != "application/step"
            or python_headers.get("content-type") != "application/step"
        ):
            mismatches.append(
                f"{name}: python={python_status} {python_shape} "
                f"native={native_status} {native_shape}"
            )
        return
    same_body = python_body == native_body
    detail = ""
    if volatile:
        python_document = normalise(
            json.loads(python_body), generated_artifacts=generated_artifacts
        )
        native_document = normalise(
            json.loads(native_body), generated_artifacts=generated_artifacts
        )
        same_body = canonical(python_document) == canonical(native_document)
        if not same_body:
            detail = " " + "; ".join(difference(python_document, native_document))
    # The content length follows the body, and a volatile body may differ in
    # length (the native OpenCascade version string is one character longer
    # than the Python binding's). The length of the *normalised* body is the
    # one that is compared for those answers.
    stable_headers = [
        entry for entry in STABLE if not (volatile and entry == "content-length")
    ]
    if (
        python_status != native_status
        or not same_body
        or any(python_headers.get(entry) != native_headers.get(entry) for entry in stable_headers)
        or any(python_headers.get(entry) != native_headers.get(entry) for entry in SECURITY)
        or any(native_headers.get(entry) is not None for entry in ABSENT)
    ):
        mismatches.append(
            f"{name}: python={python_status} {python_body[:220]!r} "
            f"native={native_status} {native_body[:220]!r}{detail}"
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--binary",
        default=str(REPO / "backend/target/release/kala-studio"),
    )
    parser.add_argument(
        "--backend",
        choices=("native", "worker"),
        default="native",
        help=(
            "the geometry backend the native front door uses: the native one "
            "(the default, no Python process) or the supervised Python worker "
            "(the development fallback)"
        ),
    )
    arguments = parser.parse_args()
    binary = Path(arguments.binary)
    if not binary.is_file():
        print(f"native binary not found: {binary}", file=sys.stderr)
        return 2
    if not VENV_PYTHON.is_file():
        print(f"the oracle interpreter is missing: {VENV_PYTHON}", file=sys.stderr)
        return 2

    from fastapi.testclient import TestClient
    from kala_agent.studio.server import create_studio_app

    work = Path(tempfile.mkdtemp(prefix="kala-studio-worker-diff-"))
    native_static = work / "native-static"
    python_static = work / "python-static"
    for root in (native_static, python_static):
        root.mkdir()
        (root / "index.html").write_text("<!doctype html><html><body>diff</body></html>")
        (root / "asset.txt").write_text("asset\n")
    outside = work / "outside.txt"
    outside.write_text("outside")
    os.symlink(outside, native_static / "escape")
    os.symlink(outside, python_static / "escape")

    port = free_port()
    environment = {
        "KALA_STUDIO_TOKEN": TOKEN,
        "PATH": "",
        "PYTHONPATH": str(SRC),
    }
    if arguments.backend == "worker":
        # The development fallback: the front door supervises the Python
        # geometry worker and proxies to it. The default is the native
        # backend, which starts no Python process at all.
        environment["KALA_STUDIO_GEOMETRY_WORKER"] = json.dumps(
            [str(VENV_PYTHON), "-m", "kala_agent.studio", "--worker"]
        )
    process = subprocess.Popen(
        [
            str(binary),
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--data-dir",
            str(work / "native-data"),
            "--static-dist",
            str(native_static),
        ],
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    deadline = time.time() + 30
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

    # The geometry worker has to be ready before the differential runs: the
    # front door answers 503 on the proxied routes while it is starting.
    ready_deadline = time.time() + 90
    worker_ready = False
    while time.time() < ready_deadline:
        status, _headers, body = native_request(
            port, "GET", "/api/studio/health", [], None, None
        )
        if status == 200:
            try:
                document = json.loads(body)
            except ValueError:
                document = {}
            if document.get("geometry_worker", {}).get("state") == "ready":
                worker_ready = True
                break
        time.sleep(0.2)
    if not worker_ready:
        print("the geometry worker did not become ready", file=sys.stderr)
        process.terminate()
        return 2

    oracle_worker, oracle_port = start_worker(work / "oracle-worker")

    python_app = create_studio_app(
        data_dir=work / "python-data",
        api_token=TOKEN,
        max_body_bytes=262_144,
        static_root=python_static,
        environ=worker_environment(work),
    )
    cases = build_cases()
    mismatches: list[str] = []
    known_gaps: list[str] = []
    checked = 0
    with TestClient(
        python_app,
        base_url="http://127.0.0.1",
        raise_server_exceptions=False,
        follow_redirects=False,
    ) as python_client:
        for item in cases:
            request_headers = item["headers"]
            kwargs: dict[str, Any] = {"headers": request_headers}
            if item["chunked"] is not None:
                kwargs["content"] = item["chunked"]
            elif item["body"] is not None:
                kwargs["content"] = item["body"]
            python_response = python_client.request(
                item["method"], item["path"], **kwargs
            )
            native_status, native_headers, native_body = native_request(
                port,
                item["method"],
                item["path"],
                request_headers,
                item["body"],
                item["chunked"],
            )
            python_headers = {
                key.lower(): value for key, value in python_response.headers.items()
            }
            python_status = python_response.status_code
            python_body = python_response.content
            checked += 1
            if item["name"] in STEP_STRUCTURAL:
                compare(
                    item["name"],
                    python_status,
                    python_headers,
                    python_body,
                    native_status,
                    native_headers,
                    native_body,
                    False,
                    True,
                    mismatches,
                )
                continue
            if item["path"].split("?")[0] == "/api/studio/health" and health_documents(
                python_body, native_body
            ):
                # The Rust front door publishes the ADR-0010 worker state.
                python_document = json.loads(python_body)
                native_document = json.loads(native_body)
                native_document.pop("geometry_worker", None)
                if python_document != native_document:
                    mismatches.append(
                        f"{item['name']}: health documents differ\n"
                        f"python={canonical(python_document)}\n"
                        f"native={canonical(native_document)}"
                    )
                continue
            compare(
                item["name"],
                python_status,
                python_headers,
                python_body,
                native_status,
                native_headers,
                native_body,
                False,
                False,
                mismatches,
            )

        # The ADR-0009/ADR-0011 design routes.
        for item in design_cases():
            request_headers, body = build_request(item)
            kwargs = {"headers": request_headers}
            if body is not None:
                kwargs["content"] = body
            python_response = python_client.request(item["method"], item["path"], **kwargs)
            native_status, native_headers, native_body = native_request(
                port, item["method"], item["path"], request_headers, body, None
            )
            python_headers = {
                key.lower(): value for key, value in python_response.headers.items()
            }
            checked += 1
            bucket = known_gaps if item.get("known_gap") else mismatches
            compare(
                item["name"],
                python_response.status_code,
                python_headers,
                python_response.content,
                native_status,
                native_headers,
                native_body,
                bool(item.get("volatile")),
                False,
                bucket,
                bool(item.get("generated_artifacts_volatile")),
            )
            if bucket is known_gaps:
                known_gaps.append(f"{item['name']}: {item['known_gap']}")

        # The stored-artifact download route (`GET /design/{id}/artifact/{kind}`):
        # the same design id, the same digest-verified script bytes, and the same
        # envelope for a kind the design did not store. The STEP bytes carry a
        # timestamp and a per-process entity numbering, so their byte contract is
        # covered by the in-tree tests (the download serves exactly the stored
        # digest), not by a cross-process equality here.
        artifact_request = {
            "template": {
                "template": "l_bracket",
                "width": 60.0,
                "depth": 45.0,
                "height": 40.0,
                "wall": 6.0,
            },
            "provider": "opencascade",
        }
        artifact_payload = json.dumps(artifact_request, separators=(",", ":")).encode()
        python_design_response = python_client.post(
            "/api/studio/design",
            headers={
                "Authorization": f"Bearer {TOKEN}",
                "content-type": "application/json",
            },
            content=artifact_payload,
        )
        native_design_status, _headers, native_design_body = native_request(
            port,
            "POST",
            "/api/studio/design",
            [("Authorization", f"Bearer {TOKEN}"), ("content-type", "application/json")],
            artifact_payload,
            None,
        )
        if python_design_response.status_code == 200 and native_design_status == 200:
            python_design_id = python_design_response.json()["design_id"]
            native_design_id = json.loads(native_design_body)["design_id"]
            if python_design_id != native_design_id:
                mismatches.append(
                    f"design.artifact: design ids differ python={python_design_id} "
                    f"native={native_design_id}"
                )
            for kind in ("script", "fcstd", "bogus"):
                path = f"/api/studio/design/{python_design_id}/artifact/{kind}"
                python_response = python_client.get(
                    path, headers={"Authorization": f"Bearer {TOKEN}"}
                )
                native_status, native_headers, native_body = native_request(
                    port,
                    "GET",
                    path,
                    [("Authorization", f"Bearer {TOKEN}")],
                    None,
                    None,
                )
                checked += 1
                compare(
                    f"design.artifact.{kind}",
                    python_response.status_code,
                    {key.lower(): value for key, value in python_response.headers.items()},
                    python_response.content,
                    native_status,
                    native_headers,
                    native_body,
                    False,
                    False,
                    mismatches,
                )
        else:
            mismatches.append(
                "design.artifact: the design turn did not build on both sides "
                f"(python={python_design_response.status_code}, "
                f"native={native_design_status}, "
                f"python body={python_design_response.content[:300]!r})"
            )

    # The worker-only routes: the public Python server does not mount them
    # yet, so the directly launched worker is the oracle.
    worker_only = [
        ("worker.import_empty", "POST", "/internal/v1/import", b""),
        ("worker.import_garbage", "POST", "/internal/v1/import", b"not a step file"),
    ]
    for name, method, path, body in worker_only:
        oracle_status, _oracle_headers, oracle_body = worker_request(
            oracle_port, method, path, body
        )
        native_status, _native_headers, native_body = native_request(
            port,
            method,
            "/api/studio/import",
            [("Authorization", f"Bearer {TOKEN}")],
            body,
            None,
        )
        checked += 1
        if native_status != oracle_status or native_body != oracle_body:
            mismatches.append(
                f"{name}: native={native_status} {native_body[:200]!r} "
                f"worker={oracle_status} {oracle_body[:200]!r}"
            )

    # A real STEP file exported by the worker is imported by the front door
    # and by the worker: the same bytes must give the same view, the same
    # digest and the same solid count.
    export_status, _headers, step_bytes = worker_request(
        oracle_port,
        "POST",
        "/internal/v1/export",
        json.dumps({"kind": "box", "format": "step"}).encode(),
    )
    if step_bytes.startswith(b"ISO-10303-21;"):
        oracle_status, _headers, oracle_body = worker_request(
            oracle_port, "POST", "/internal/v1/import", step_bytes
        )
        native_status, _headers, native_body = native_request(
            port,
            "POST",
            "/api/studio/import",
            [("Authorization", f"Bearer {TOKEN}")],
            step_bytes,
            None,
        )
        checked += 1
        oracle_document = normalise(json.loads(oracle_body))
        native_document = normalise(json.loads(native_body))
        if native_status != oracle_status or canonical(oracle_document) != canonical(
            native_document
        ):
            mismatches.append(
                f"worker.import_real_step: native={native_status} "
                f"{native_body[:200]!r} worker={oracle_status} {oracle_body[:200]!r}"
            )

    python_settings = work / "python-data" / "studio-settings.json"
    native_settings = work / "native-data" / "studio-settings.json"
    if python_settings.exists() and native_settings.exists():
        if python_settings.read_bytes() != native_settings.read_bytes():
            mismatches.append("settings file bytes differ from the Python store")

    oracle_worker.terminate()
    try:
        oracle_worker.wait(timeout=15)
    except subprocess.TimeoutExpired:
        oracle_worker.kill()
    process.terminate()
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
    if process.returncode != 0:
        mismatches.append(f"native process exit code {process.returncode}")
    print(f"differential cases: {checked}")
    print(f"mismatches: {len(mismatches)}")
    for mismatch in mismatches:
        print(mismatch)
    print(f"known gaps: {len(known_gaps)}")
    for gap in known_gaps:
        print(gap)
    return 0 if not mismatches else 1


STEP_STRUCTURAL = {
    "export.step_box",
    "export.step_cylinder",
    "export.step_sphere",
    "handoff.step_solidworks",
    "handoff.step_solid_edge",
}


if __name__ == "__main__":
    raise SystemExit(main())
