# model-ui: the provider and model pickers in Studio
Read `../../WAVE.md` and the briefs of cli-providers and model-catalog (their JSON contracts; poll their REPORT.md files and adapt when they land). You own `studio/src/**` and `studio/test/**`.
- Provider picker: show local, openai, openrouter, and the three subscription CLIs (Claude Code, Codex, OpenCode) with availability/configured badges and reasons (from /providers + /settings).
- Model picker: after a provider is configured (or a key is saved), fetch `/providers/{id}/models` (with the consent flag set by the existing remote-egress consent stamp; never auto-consent). Show a searchable dropdown (hundreds of OpenRouter models) with a "Free only" toggle, context length and price, and persist the last choice per provider for the session (localStorage allowed for the model id only, never keys). The chosen model id is sent as `model` in chat.
- Clear empty, error, and consent-required states. Keyboard accessible.
- Tests for each state. Gates: `npm run typecheck && npm test && npm run build`. REPORT.md.
