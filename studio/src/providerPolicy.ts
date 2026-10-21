// The provider policy of the Studio composer.
//
// One place decides what a provider id MEANS for the composer: whether a
// turn leaves the machine, whether a model id is mandatory, and whether the
// provider is one of the subscription CLIs that run the operator's own
// already-authenticated command-line tool.
//
// The rules come from the backend contracts, not from taste:
//
//   * `local` never leaves the loopback backend and ignores the model id.
//   * `openai` / `openrouter` are API-key routes: they leave the machine
//     (ADR-0001) and need a model id.
//   * `claude_code` / `codex_cli` / `opencode_cli` are the subscription
//     CLIs. They also leave the machine — the CLI talks to its vendor — so
//     they need the same remote-egress consent stamp, but their model id is
//     optional: the CLI's own default model is used when none is chosen.
//   * `codex` is the historical id of the Codex CLI route and behaves like
//     `codex_cli`.
//
// Two rules live here as well, because they are provider semantics and not
// presentation:
//   * the canonical id of an alias (`codex` → `codex_cli`), so the composer
//     never offers the same route twice,
//   * the honest label of a reply that used the CLI's OWN default model: the
//     CLIs do not report which model they picked, so the UI says exactly
//     that instead of inventing a model name.

import type { ProviderId, ProviderRecord, SubscriptionProviderId } from './types/studio';

const LOCAL_PROVIDERS: ReadonlySet<string> = new Set(['local', 'local-helper']);

const API_KEY_PROVIDERS: ReadonlySet<string> = new Set(['openai', 'openrouter']);

const SUBSCRIPTION_PROVIDERS: ReadonlySet<string> = new Set([
  'claude_code',
  'codex_cli',
  'opencode_cli',
  'codex',
]);

/** Whether the provider runs entirely on this machine. */
export function isLocalProvider(providerId: string): boolean {
  return LOCAL_PROVIDERS.has(providerId);
}

/** Whether the provider is one of the three subscription CLIs. */
export function isSubscriptionProvider(providerId: string): boolean {
  return SUBSCRIPTION_PROVIDERS.has(providerId);
}

/** The subscription-CLI id a provider id names, if it names one. */
export function subscriptionProviderId(providerId: string): SubscriptionProviderId | null {
  switch (providerId) {
    case 'claude_code':
      return 'claude_code';
    case 'codex_cli':
    case 'codex':
      return 'codex_cli';
    case 'opencode_cli':
      return 'opencode_cli';
    default:
      return null;
  }
}

/**
 * Whether a turn with this provider leaves the machine and therefore needs
 * the operator's remote-egress consent stamp.
 *
 * The local helper is the only provider that does not: it answers from the
 * loopback backend.
 */
export function requiresRemoteConsent(providerId: string): boolean {
  if (providerId.length === 0) return false;
  return !isLocalProvider(providerId);
}

/**
 * Whether the composer must refuse to send without a model id.
 *
 * The API-key routes need one; the local helper ignores it; the
 * subscription CLIs fall back to their own default model.
 */
export function requiresModel(providerId: string): boolean {
  return API_KEY_PROVIDERS.has(providerId);
}

/**
 * The human label of a provider id when the backend has not answered yet.
 * The backend's own name always wins; this is only the placeholder.
 */
export function providerFallbackName(providerId: string): string {
  switch (providerId) {
    case 'local':
      return 'Local Deterministic Helper';
    case 'openai':
      return 'OpenAI';
    case 'openrouter':
      return 'OpenRouter';
    case 'codex':
    case 'codex_cli':
      return 'Codex CLI';
    case 'claude_code':
      return 'Claude Code CLI';
    case 'opencode_cli':
      return 'OpenCode CLI';
    default:
      return providerId;
  }
}

/**
 * The historical ids that name a route the backend also lists under its
 * canonical name. The backend canonicalises them itself; the UI only needs
 * to know which row would be a duplicate.
 */
export const PROVIDER_ALIASES: Readonly<Record<string, string>> = {
  codex: 'codex_cli',
};

/**
 * The canonical id of a provider.
 *
 * `codex` is the historical id of the Codex CLI route; the backend maps it
 * to `codex_cli`. A stored selection that still names the alias therefore
 * keeps working and is shown as the canonical route.
 */
export function canonicalProviderId(providerId: string): string {
  return PROVIDER_ALIASES[providerId] ?? providerId;
}

/** Whether the id is one the backend's `ProviderId` enum knows. */
export function isKnownProviderId(providerId: string): providerId is ProviderId {
  switch (providerId) {
    case 'local':
    case 'openai':
    case 'openrouter':
    case 'codex':
    case 'claude_code':
    case 'codex_cli':
    case 'opencode_cli':
      return true;
    default:
      return false;
  }
}

// --- The status of a provider row ---------------------------------------------

/** What the backend reported about one provider. */
export type ProviderState = 'configured' | 'not-configured' | 'unavailable';

export interface ProviderStatusView {
  state: ProviderState;
  /** The badge text: `Configured`, `Not configured`, or `Unavailable`. */
  badge: string;
  /**
   * Why the provider is unavailable, when the backend said so. `null` for a
   * provider that is simply without a credential — that is not a fault of
   * the provider.
   */
  reason: string | null;
  /**
   * Whether the operator may pick this provider. A provider the backend
   * probed and found unavailable cannot be picked; a provider without a
   * credential can, so the reason and the key form stay reachable.
   */
  selectable: boolean;
}

/**
 * Read one provider row.
 *
 * `available === false` is the backend's own probe result — the provider
 * cannot run on this host — and it wins over `configured`. Everything else
 * is the registry's `configured` flag, which for a subscription CLI means
 * "the binary was found and it is signed in".
 */
export function describeProviderStatus(provider: ProviderRecord): ProviderStatusView {
  const unavailable = provider.available === false;
  const state: ProviderState = unavailable
    ? 'unavailable'
    : provider.configured
      ? 'configured'
      : 'not-configured';
  const badge =
    state === 'configured'
      ? 'Configured'
      : state === 'unavailable'
        ? 'Unavailable'
        : 'Not configured';
  return {
    state,
    badge,
    reason: unavailable ? (provider.reason ?? provider.notes ?? null) : null,
    selectable: !unavailable,
  };
}

// --- The CLI's own default model ------------------------------------------------

/**
 * The model id the backend reports when the operator chose no model and the
 * CLI ran with its own default. It is a literal id, not a model name.
 */
export const DEFAULT_MODEL_ID = 'default';

/** How each subscription CLI is called, and who picks its default model. */
const CLI_NAMES: Readonly<Record<string, { label: string; vendor: string }>> = {
  claude_code: { label: 'Claude Code', vendor: 'Claude Code' },
  codex_cli: { label: 'Codex CLI', vendor: 'Codex' },
  opencode_cli: { label: 'OpenCode CLI', vendor: 'OpenCode' },
};

/** The names of the CLI that owns a provider id, alias included. */
function cliNames(providerId: string): { label: string; vendor: string } | null {
  return CLI_NAMES[canonicalProviderId(providerId)] ?? null;
}

/** Whether a model id is the literal "the CLI's own default" id. */
export function isDefaultModelId(model: string | null | undefined): boolean {
  if (model === null || model === undefined) return false;
  return model.trim() === DEFAULT_MODEL_ID;
}

/**
 * The honest label of a reply that ran on the CLI's OWN default model.
 *
 * The CLIs do not report which model they picked (checked live on
 * 2026-10-21), so the UI never names one. It says that the CLI's default is
 * in use and who chose it, and points at the model picker, where picking a
 * row pins the model and shows it.
 *
 * `null` when the reply named a real model, or when the provider is not one
 * of the subscription CLIs.
 */
export function defaultModelLabel(
  providerId: string,
  model: string | null | undefined,
): string | null {
  const cli = cliNames(providerId);
  if (!cli) return null;
  const trimmed = (model ?? '').trim();
  // Only the literal default id (or no model at all) means "the CLI decided".
  if (trimmed.length > 0 && trimmed !== DEFAULT_MODEL_ID) return null;
  return `${cli.label} default model (chosen by ${cli.vendor})`;
}

/**
 * The note under the model field of a provider whose model id is optional.
 *
 * It tells the operator what an empty field MEANS and how to pin a model.
 * `null` for a route that requires a model id: there, an empty field is a
 * mistake, not a choice.
 */
export function optionalModelNote(providerId: string, providerName: string): string | null {
  if (!isSubscriptionProvider(providerId)) return null;
  return (
    `Leave empty to use ${providerName}'s own default model — the CLI does not report ` +
    'which model it picked. Pick a row here to pin one; the reply then names it.'
  );
}
