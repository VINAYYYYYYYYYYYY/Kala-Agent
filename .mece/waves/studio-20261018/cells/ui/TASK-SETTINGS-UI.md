# Task: Add API Key Settings UI to Kala Studio Frontend

## 1. Goal & Scope
Add a first-class **API Key Settings** experience in the Kala Studio UI so users can configure, update, or clear their `OpenAI` (`OPENAI_API_KEY`) and `OpenRouter` (`OPENROUTER_API_KEY`) API keys directly in the app—both from the **Providers / Settings** panel in the left sidebar and via a quick **"API Keys"** button in the `ChatPanel` header—without ever exposing stored secrets back to the browser.

## 2. File Ownership (Strict Boundary)
You may ONLY create/edit:
- `studio/src/types/studio.ts`
- `studio/src/api/client.ts`
- `studio/src/state/store.ts`
- `studio/src/components/panels/ProviderPanel.tsx`
- `studio/src/components/sidebar/Sidebar.tsx`
- `studio/src/components/chat/ChatPanel.tsx`
- `studio/src/styles/studio.css`
- `studio/test/apiClient.test.ts`
- `studio/test/AgentWorkspace.dom.test.tsx`
- `.mece/waves/studio-20261018/cells/ui/REPORT-SETTINGS-UI.md` (NEW)

Do NOT touch `src/kala_agent/**` or `docs/**`.

## 3. Exact Design Requirements

### A. Types (`studio/src/types/studio.ts`)
Add types matching the new backend `/api/studio/settings` schema:
```ts
export type CredentialProviderId = 'openai' | 'openrouter';
export type CredentialSource = 'none' | 'environment' | 'stored';

export interface ProviderCredentialStatus {
  provider: CredentialProviderId;
  env_var: string;
  configured: boolean;
  source: CredentialSource;
}

export interface StudioSettingsResponse {
  credentials: ReadonlyArray<ProviderCredentialStatus>;
  providers: ReadonlyArray<ProviderRecord>;
  offline: boolean;
}

export interface UpdateProviderKeyRequest {
  provider: CredentialProviderId;
  api_key: string | null;
}
```

### B. API Client (`studio/src/api/client.ts`)
Add two functions and export them:
```ts
export function getSettings(
  connection: StudioConnection,
  signal: AbortSignal | null = null,
): Promise<StudioSettingsResponse> {
  return sendRequest<StudioSettingsResponse>(connection, 'GET', '/api/studio/settings', null, signal);
}

export function postSettings(
  connection: StudioConnection,
  payload: UpdateProviderKeyRequest,
  signal: AbortSignal | null = null,
): Promise<StudioSettingsResponse> {
  return sendRequest<StudioSettingsResponse>(connection, 'POST', '/api/studio/settings', payload, signal);
}
```

### C. Store (`studio/src/state/store.ts`)
1. In `ServerState`, add `credentials: ReadonlyArray<ProviderCredentialStatus>;` (initialized to `[]` in `createInitialUiState()` and reset to `[]` on `server/reset`).
2. Add action `{ type: 'server/settings'; settings: StudioSettingsResponse }` to `Action`:
   - Updates `state.server.credentials = action.settings.credentials` and `state.server.providers = action.settings.providers`, plus updates `lastSyncAt: Date.now()`.

### D. Interactive API Key Settings in `ProviderPanel.tsx`
Enhance `studio/src/components/panels/ProviderPanel.tsx` so that alongside listing all providers, it provides an interactive **API Key Settings** form for configurable providers (`openai` and `openrouter`):
- Read `connection = useUiState((s) => s.connection)`, `providers = useUiState((s) => s.server.providers)`, and `credentials = useUiState((s) => s.server.credentials)`.
- On mount when `connection.kind === 'ready'`, fetch `getSettings(connection.connection)` (ignoring errors gracefully so older/mocked tests that don't mock `/api/studio/settings` still work without crashing!) and dispatch `server/settings`.
- For each provider in `providers`:
  - Render the existing `<li key={provider.id} className="panel-list-item">` with `.panel-list-name`, `.panel-list-status`, and `.panel-list-notes`.
  - If `provider.id === 'openai' || provider.id === 'openrouter'`, render an inline **API Key Setup** form (`.provider-key-form`) inside the list item:
    - Show the credential source badge if known from `credentials` (e.g. `Stored in Studio settings`, `From environment (${envVar})`, or `No key set`).
    - A password input (`type="password"`, `autoComplete="off"`, `spellCheck={false}`, `aria-label={`${provider.name} API key`}`, placeholder `provider.id === 'openai' ? 'sk-...' : 'sk-or-...'`).
    - A **Save key** button (`type="submit"`, disabled when input is empty/whitespace or a save is in flight).
    - A **Clear key** button (`type="button"`, enabled when `provider.configured` is true and no request is in flight) that sends `postSettings(connection, { provider: provider.id, api_key: null })`.
    - When **Save key** or **Clear key** succeeds:
      - Clear the password input state immediately (`''`) so the secret does not linger in the DOM.
      - Dispatch `{ type: 'server/settings', settings: response }` so the provider immediately flips to `Configured server-side` across the entire app (including `ChatPanel`'s provider dropdown!).
      - Show a brief accessible status message (`role="status"`, `.provider-key-status`, e.g., `"API key saved on backend."` or `"API key cleared."`).
    - When `postSettings` fails, show an accessible error message (`role="alert"`, `.provider-key-error`).

### E. Quick "API Keys & Providers" Access in `ChatPanel.tsx` & `Sidebar.tsx`
1. In `studio/src/components/chat/ChatPanel.tsx`:
   - Add a button in `.chat-panel-header` (next to `Clear`) labeled `API Keys` (`.chat-panel-settings-toggle`, `aria-expanded={showKeySettings}`) that toggles an inline `<section className="chat-panel-settings" aria-label="API key settings">` directly inside `ChatPanel` rendering `<ProviderPanel compact />`.
   - Also, if the user sees `(not configured)` or wants to set up OpenAI / OpenRouter right from the chat window, clicking `API Keys` lets them paste their `OPENAI_API_KEY` or `OPENROUTER_API_KEY`, click **Save key**, and immediately select and use that provider in the chat dropdown!
   - When `server.providers` updates and the currently selected `providerId` becomes configured, `ChatPanel` automatically reflects `configured = true`.
2. In `studio/src/components/sidebar/Sidebar.tsx`:
   - Update the `Providers` workspace description to `"API keys & chat providers"` and update the `Providers workspace summary` copy to explain that API keys for OpenAI and OpenRouter can be saved directly to the local backend settings store (or provided via environment variables) and are never echoed back to the browser.

### F. Styling (`studio/src/styles/studio.css`)
- Add clean, compact styling for `.provider-key-form`, `.provider-key-input`, `.provider-key-actions`, `.provider-key-status`, `.provider-key-error`, `.chat-panel-header-actions`, `.chat-panel-settings-toggle`, and `.chat-panel-settings`.
- CRITICAL: Preserve all existing CSS layout invariants tested by `studio/test/layoutCss.test.ts` (do not put any `.chat-panel` selector rule before the base `.chat-panel {` block!).

### G. Tests (`studio/test/apiClient.test.ts` & `studio/test/AgentWorkspace.dom.test.tsx`)
1. In `studio/test/apiClient.test.ts`:
   - Add tests for `getSettings` (sends `GET /api/studio/settings` without bearer token) and `postSettings` (sends `POST /api/studio/settings` with bearer token and `{ provider, api_key }`).
2. In `studio/test/AgentWorkspace.dom.test.tsx`:
   - Add DOM tests verifying:
     - Clicking the `API Keys` button in the `ChatPanel` header opens the inline API key settings panel (`aria-label="API key settings"`).
     - Entering an OpenAI API key into the password input (`OpenAI (Chat Completions) API key`) and clicking `Save key` calls `POST /api/studio/settings`, clears the password input, and updates the OpenAI provider in the chat dropdown from `(not configured)` to enabled/configured!
     - Clicking `Clear key` sends `api_key: null` and updates the provider state.

## 4. Verification Commands
Run inside `studio/`:
- `npm test`
- `npm run typecheck`
- `npm run build`

Write your completion report to `.mece/waves/studio-20261018/cells/ui/REPORT-SETTINGS-UI.md`.
