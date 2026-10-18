# Task: Add Secure API Key Settings Store & Route to Kala Studio Backend

## 1. Goal & Scope
Add a secure, owner-only (`0o600`), secret-redacting API key settings store and routes (`GET /api/studio/settings` and `POST /api/studio/settings`) to the Kala Studio backend so operators can configure or clear their `openai` (`OPENAI_API_KEY`) and `openrouter` (`OPENROUTER_API_KEY`) API keys from the UI without restarting the server or editing shell profiles.

## 2. File Ownership (Strict Boundary)
You may ONLY create/edit:
- `src/kala_agent/studio/settings.py` (NEW)
- `src/kala_agent/studio/schemas.py`
- `src/kala_agent/studio/server.py`
- `tests/unit/test_studio_server.py`
- `tests/unit/test_studio_providers.py`
- `.mece/waves/studio-20261018/cells/backend/REPORT-SETTINGS-BACKEND.md` (NEW)

Do NOT touch `studio/**` or `docs/**` (handled by other cells).

## 3. Exact Design Requirements

### A. `src/kala_agent/studio/settings.py` (NEW)
Create a dedicated settings persistence helper for studio provider credentials:
- Constants:
  - `STUDIO_SETTINGS_FILE_NAME: Final[str] = "studio-settings.json"`
  - `SUPPORTED_CREDENTIAL_PROVIDERS: Final[tuple[str, ...]] = ("openai", "openrouter")`
  - `PROVIDER_ENV_VARS: Final[dict[str, str]] = {"openai": "OPENAI_API_KEY", "openrouter": "OPENROUTER_API_KEY"}`
  - `MAX_API_KEY_LENGTH: Final[int] = 256`
- Validation helper `validate_api_key(value: str | None) -> str`:
  - If `value is None` or `value.strip() == ""`, return `""` (empty string means clear the stored key).
  - Strip leading/trailing whitespace.
  - Reject control characters (`any(ord(ch) < 32 or ord(ch) == 127 for ch in stripped)`) with `ValueError("API key contains unsupported control characters")`.
  - Reject whitespace inside the key (`any(ch.isspace() for ch in stripped)`) with `ValueError("API key must not contain whitespace")`.
  - Reject keys longer than `MAX_API_KEY_LENGTH` (256) with `ValueError("API key exceeds maximum length")`.
  - Return `stripped`.
- Storage functions (using `_atomic_write_text` and `_tighten_file_mode` from `kala_agent.studio.boundary`):
  - `read_stored_keys(data_dir: Path) -> dict[str, str]`:
    - Reads `data_dir / STUDIO_SETTINGS_FILE_NAME`.
    - Returns a dict mapping `"openai"` / `"openrouter"` to their non-empty validated stored string keys.
    - If the file is missing, unreadable, malformed JSON, or not a dict, returns `{}` without raising.
  - `write_stored_key(data_dir: Path, provider: str, api_key: str | None) -> dict[str, str]`:
    - Validates `provider in SUPPORTED_CREDENTIAL_PROVIDERS` (raise `ValueError(f"unsupported provider for API key settings: {provider!r}")` if not).
    - Calls `cleaned = validate_api_key(api_key)`.
    - Reads current stored keys via `read_stored_keys(data_dir)`.
    - If `cleaned` is non-empty, sets `current[provider] = cleaned`; if `cleaned == ""`, removes `provider` from `current`.
    - Writes JSON `{"api_keys": current}` atomically via `_atomic_write_text(data_dir / STUDIO_SETTINGS_FILE_NAME, json.dumps({"api_keys": current}, indent=2, sort_keys=True) + "\n")` so file mode is `0o600`.
    - Returns `current`.
- Effective environment & status helpers:
  - `effective_environ(base_environ: Mapping[str, str], data_dir: Path | None) -> dict[str, str]`:
    - Starts with `merged = dict(base_environ)`.
    - If `data_dir is not None`, loads `stored = read_stored_keys(data_dir)`.
    - For `provider, env_name in PROVIDER_ENV_VARS.items()`: if `provider in stored and stored[provider]`, sets `merged[env_name] = stored[provider]` (stored UI setting overrides empty or prior env var for that provider).
    - Returns `merged`.
  - `describe_provider_credentials(base_environ: Mapping[str, str], data_dir: Path | None) -> tuple[ProviderCredentialStatus, ...]`:
    - For each `provider` in `("openai", "openrouter")`:
      - Check if stored in `data_dir` (`stored_key = stored.get(provider, "")`).
      - Check if set in `base_environ` (`env_key = base_environ.get(PROVIDER_ENV_VARS[provider], "").strip()`).
      - Determine `configured: bool` and `source: CredentialSource` (`"stored"` if `stored_key` is non-empty, `"environment"` if `env_key` is non-empty, else `"none"`).
      - **NEVER** include the key value or a prefix/suffix of the key in `ProviderCredentialStatus`!

### B. `src/kala_agent/studio/schemas.py`
Add strict Pydantic models:
- `class CredentialProviderId(StrEnum)`:
  - `OPENAI = "openai"`
  - `OPENROUTER = "openrouter"`
- `class CredentialSource(StrEnum)`:
  - `NONE = "none"`
  - `ENVIRONMENT = "environment"`
  - `STORED = "stored"`
- `class ProviderCredentialStatus(_StrictModel)`:
  - `provider: CredentialProviderId`
  - `env_var: Annotated[str, Field(min_length=1, max_length=64)]`
  - `configured: bool`
  - `source: CredentialSource`
- `class StudioSettingsResponse(_StrictModel)`:
  - `credentials: tuple[ProviderCredentialStatus, ...]`
  - `providers: tuple[ProviderRecord, ...]`
  - `offline: bool`
- `class UpdateProviderKeyRequest(_RequestModel)`:
  - `provider: CredentialProviderId`
  - `api_key: Annotated[str | None, Field(default=None, max_length=256)] = None`
- Export the new types in `__all__`.

### C. `src/kala_agent/studio/server.py`
Wire `data_dir` and `base_environ` into the FastAPI app state so routes use `effective_environ` and can read/write settings:
- In `create_studio_app`:
  - Store `app.state.studio_data_dir = data_dir` and `app.state.studio_environ = source`.
- Update `health()`, `providers()`, and `chat_route()` (using `fastapi.Request` injection `request: Request`) so they resolve the effective environment via `effective_environ(getattr(request.app.state, "studio_environ", os.environ), getattr(request.app.state, "studio_data_dir", None))`:
  - `health(request: Request) -> HealthResponse`: checks `_any_remote_provider_configured(env)` and `list_provider_records(env)`.
  - `providers(request: Request) -> ProvidersResponse`: returns `ProvidersResponse(providers=list_provider_records(env))`.
  - `chat_route(payload: ChatRequest, request: Request) -> ChatResponse | Response`: passes `environ=env` to `chat(...)`.
  - Keep `health()` and `providers()` callable without arguments in unit tests if needed, or handle `request: Request` cleanly (FastAPI injects `Request` automatically when annotated `request: Request`).
- Add two new routes on `router`:
  1. `GET /api/studio/settings` (`response_model=StudioSettingsResponse`):
     - Reads `base_env` and `data_dir` from `request.app.state`.
     - Computes `env = effective_environ(base_env, data_dir)`.
     - Returns `StudioSettingsResponse(credentials=describe_provider_credentials(base_env, data_dir), providers=list_provider_records(env), offline=not _any_remote_provider_configured(env))`.
  2. `POST /api/studio/settings` (`response_model=StudioSettingsResponse`):
     - Requires the Bearer token (enforced automatically by `BearerTokenMiddleware` on all `POST` routes).
     - Calls `write_stored_key(data_dir, payload.provider.value, payload.api_key)`. If `ValueError` is raised, return `_invalid_request_response(error)` (HTTP 422 flat error envelope) WITHOUT echoing the submitted key! Make sure any `ValueError` message never contains the submitted key text.
     - Recomputes `env = effective_environ(base_env, data_dir)` and returns the updated `StudioSettingsResponse`.

### D. Unit Tests (`tests/unit/test_studio_server.py`)
Add comprehensive tests verifying:
1. `GET /api/studio/settings` returns `credentials` for `openai` and `openrouter` with `configured=False, source="none"` by default.
2. `POST /api/studio/settings` with `{"provider": "openai", "api_key": "sk-test-openai-key-123456"}`:
   - Returns HTTP 200.
   - Marks `openai` as `configured=True, source="stored"` in `credentials` and `configured=True` in `providers`, and sets `offline=False`.
   - **Never** echoes `"sk-test-openai-key-123456"` anywhere in `POST /api/studio/settings`, `GET /api/studio/settings`, `GET /api/studio/providers`, or `GET /api/studio/health`.
   - Persists `studio-settings.json` inside `data_dir`.
3. Clearing a key via `POST /api/studio/settings` with `{"provider": "openai", "api_key": ""}` (or `null`) resets `configured=False, source="none"` and `offline=True`.
4. Rejecting invalid keys (e.g., key with spaces `"sk-bad key"` or control characters) returns HTTP 422 `invalid_request` and does not leak the key in the error response.
5. `POST /api/studio/settings` without a Bearer token is rejected with HTTP 401 by `BearerTokenMiddleware`.

## 4. Verification Commands
Run:
- `.venv/bin/pytest tests/unit/test_studio_server.py tests/unit/test_studio_providers.py tests/security/test_studio_security.py`
- `.venv/bin/ruff check src/kala_agent/studio tests/unit/test_studio_server.py`
- `.venv/bin/ruff format --check src/kala_agent/studio tests/unit/test_studio_server.py`
- `.venv/bin/mypy src/kala_agent/studio`

Write your summary report to `.mece/waves/studio-20261018/cells/backend/REPORT-SETTINGS-BACKEND.md`.
