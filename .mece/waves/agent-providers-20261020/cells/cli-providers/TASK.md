# cli-providers: use the user's existing subscriptions via their official CLIs
Read `../../WAVE.md`, `kala-studio/src/providers/**` (the existing provider trait, consent, and limits; `codex` is currently a refusal stub), and ADR-0001.
Add three chat providers that run the user's locally installed, already-authenticated CLIs as subprocesses. Kala never sees or stores their credentials:
- `claude_code`: Claude Code CLI (`claude`, v2.1.x; non-interactive print mode with JSON output and a model flag; read `claude --help`);
- `codex_cli`: OpenAI Codex CLI (`codex`, v0.163; non-interactive `exec` mode with JSON output and a model flag; read `codex exec --help`). It replaces the old `codex` stub; keep the id `codex` as an alias if the DTOs require it;
- `opencode_cli`: OpenCode (`opencode`, v2.0; non-interactive `run` with a model `provider/model` and JSON output; read `opencode run --help`).
Requirements:
- Discovery: find the binary on PATH or at a configured path. Availability probe: version + a cheap auth-status command if the CLI has one (do NOT read their config or credential files); report `available` and `reason`.
- Execution: no shell, an argv array, the prompt via stdin or an argument as each CLI supports safely; NO tool use / file writes / agentic actions (use each CLI's flags to disable tools or permissions or run read-only, plus a temp working dir); a bounded wall time (default 180 s, configurable), bounded output; parse the final assistant text from the JSON output.
- Environment: pass only what the CLI needs to find its own auth (HOME, PATH, XDG_*, …) and nothing of Kala's secrets.
- Consent: these send prompts to remote services, so they REQUIRE `allow_remote`, exactly like openrouter/openai. Model: pass the request's `model` through; default to the CLI's default when absent.
- They plug into the same chat + design-turn flow (llm-design's schema prompt), so a design can be built through any of them.
- Settings and the providers list: show them with `configured` = CLI found + authenticated, the source "subscription (CLI)", and notes.
Tests: fake CLI scripts (bash) in temp dirs that emulate the JSON output, errors, timeouts, huge output, and an auth failure; argv assertions (no shell, tools disabled); env assertions (no Kala secrets); the consent refusal. Do NOT call the real CLIs in tests (Opus does the live test). Full Rust gates. REPORT.md with the exact argv used per CLI and why.
