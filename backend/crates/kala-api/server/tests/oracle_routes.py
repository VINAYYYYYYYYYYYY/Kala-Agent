"""The live Python oracle of the local Kala HTTP API.

The script drives the **live** FastAPI application of `kala_agent.api` — the
real middleware stack, the real routers and the real error handlers — over a
loopback socket, and writes a transcript of the answers.

The oracle is a test oracle only: the crate never invokes it, and the
library has no Python dependency. The script reads a request script, replays
it against the live application and writes the transcript as JSON.

The script is run as::

    <repo>/.venv/bin/python -I tests/oracle_routes.py <script> <transcript> <data-dir>

The repository `src` is inserted by the script itself, because `-I` ignores
`PYTHONPATH`.
"""

from __future__ import annotations

import base64
import http.client
import json
import os
import socket
import sys
import threading
import time
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPOSITORY / "src"))

from kala_agent.api.app import create_app  # noqa: E402
from kala_agent.api.boundary import issue_api_token  # noqa: E402
from kala_agent.application import KalaApplication  # noqa: E402
from kala_agent.config import Settings  # noqa: E402

# The headers the transcript carries. The framing headers the servers add on
# their own belong to the servers and not to the application.
RECORDED_HEADERS = ("content-type", "content-length", "location")


def free_port() -> int:
    """Return a port nothing listens on."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.bind(("127.0.0.1", 0))
        return int(probe.getsockname()[1])


def substitute(value: str, token: str, request_id: str) -> str:
    """Replace the placeholders of one request."""
    return value.replace("${TOKEN}", token).replace("{request_id}", request_id)


def run_case(
    port: int,
    case: dict,
    token: str,
    request_id: str,
) -> dict:
    """Send one case and read its answer."""
    method = case.get("method", "GET")
    path = substitute(case.get("path", "/"), token, request_id)
    headers = [
        (name, substitute(value, token, request_id))
        for name, value in case.get("headers", [])
    ]
    if not any(name.lower() == "host" for name, _ in headers):
        headers.append(("Host", f"127.0.0.1:{port}"))
    body = base64.b64decode(case.get("body", "") or "")
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
    try:
        connection.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
        for name, value in headers:
            connection.putheader(name, value)
        connection.putheader("Content-Length", str(len(body)))
        connection.endheaders(body)
        response = connection.getresponse()
        payload = response.read()
        recorded = {
            name.lower(): value
            for name, value in response.getheaders()
            if name.lower() in RECORDED_HEADERS
        }
    finally:
        connection.close()
    return {
        "name": case.get("name", ""),
        "status": response.status,
        "headers": recorded,
        "body": base64.b64encode(payload).decode("ascii"),
    }


def main(argv: list[str]) -> int:
    """Replay the script against the live application."""
    if len(argv) != 4:
        print("usage: oracle_routes.py <script> <transcript> <data-dir>", file=sys.stderr)
        return 2
    script_path, transcript_path, data_dir = argv[1:4]
    script = json.loads(Path(script_path).read_text(encoding="utf-8"))

    os.environ["KALA_AGENT_DATA_DIR"] = data_dir
    settings = Settings.from_env()
    application = KalaApplication(settings)
    token = issue_api_token(Path(settings.data_dir))
    application_object = create_app(application, api_token=token)

    import uvicorn  # noqa: PLC0415

    port = free_port()
    config = uvicorn.Config(
        application_object,
        host="127.0.0.1",
        port=port,
        log_level="warning",
        access_log=False,
    )
    server = uvicorn.Server(config)
    thread = threading.Thread(target=server.run, daemon=True)
    thread.start()
    deadline = time.monotonic() + 30
    while not server.started and time.monotonic() < deadline:
        if not thread.is_alive():
            print("the live application did not start", file=sys.stderr)
            return 1
        time.sleep(0.01)
    if not server.started:
        print("the live application did not start", file=sys.stderr)
        return 1

    answers: list[dict] = []
    request_id = ""
    try:
        for case in script.get("cases", []):
            answer = run_case(port, case, token, request_id)
            answers.append(answer)
            try:
                document = json.loads(base64.b64decode(answer["body"]))
            except (ValueError, UnicodeDecodeError):
                document = None
            if isinstance(document, dict) and isinstance(document.get("request_id"), str):
                request_id = document["request_id"]
    finally:
        server.should_exit = True
        thread.join(timeout=30)
        application.close()

    Path(transcript_path).write_text(
        json.dumps({"cases": answers, "port": port}, indent=2, sort_keys=True),
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
