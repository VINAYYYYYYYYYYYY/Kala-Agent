# api-core-r1: kala-api (config, boundary gates, error envelope, middleware)
Follow `../CONVENTIONS.md`. Port into NEW standalone `backend/crates/kala-api`:
- `src/kala_agent/config.py`
- `src/kala_agent/api/boundary.py`, `errors.py`, `envelope.py`, `middleware.py`, and the parts of `api/__init__.py` they rely on.
These are the core API's connection layer: Host/Origin/bearer gates, body limits, the error envelope, and request middleware. NOTE: `api/__init__.py`, `envelope.py`, `middleware.py`, and `schemas.py` contain the user's UNCOMMITTED working-tree changes. Port the CURRENT working tree exactly, and never edit these Python files.
Use the SAME HTTP stack and server conventions as the accepted `backend/crates/kala-studio` (read its Cargo.toml and REVIEW-4.md). Do not add a second HTTP framework. Routes (`app.py`, `dependencies.py`, `schemas.py`) and application composition are LATER cells. Expose the gates and middleware as a reusable layer plus a minimal test router.
Differential: drive the live Python FastAPI app (`src/kala_agent/api`, via its test client or a real uvicorn on loopback, test-only) and the Rust layer with the same corpus: good/bad/missing bearer, Host/Origin variants (IDNA, ports, IPv6, case), oversized/chunked bodies, malformed JSON, and error-envelope bytes and status codes. Zero unexplained mismatches. Config: the same env/file precedence and validation messages.
Full Rust gates. REPORT.md as usual.
