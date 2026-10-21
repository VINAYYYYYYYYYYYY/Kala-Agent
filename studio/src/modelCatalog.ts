// The model catalog's pure helpers: filtering, formatting, and the
// session-persisted choice.
//
// The persisted value is the MODEL ID and nothing else. No API key, no
// credential, no prompt, and no provider secret ever reaches localStorage:
// the key is `kala-studio.model.<provider id>` and the value is the model
// id the operator picked.

import type { ProviderModel, ProviderModelCatalog, ProviderModelPricing } from './types/studio';

/** The localStorage key prefix for the per-provider model choice. */
export const MODEL_STORAGE_PREFIX = 'kala-studio.model.';

/**
 * The maximum number of catalog rows the picker renders at once.
 *
 * It matches the backend's own bound (`MAX_MODELS` in the catalog route), so
 * the count the picker prints is the number of models the provider reported
 * and never a silently truncated list.
 */
export const MODEL_LIST_LIMIT = 500;

/** The model id the operator picked for a provider, or null. */
export function readStoredModel(providerId: string): string | null {
  if (!providerId) return null;
  try {
    if (typeof window === 'undefined' || typeof localStorage === 'undefined') return null;
    const stored = localStorage.getItem(MODEL_STORAGE_PREFIX + providerId);
    return stored && stored.trim().length > 0 ? stored : null;
  } catch {
    // A locked-down browser profile can refuse storage; the in-memory choice
    // still applies for this session.
    return null;
  }
}

/**
 * Remember the model id of one provider. Only the id is written: the
 * key never carries a credential.
 */
export function writeStoredModel(providerId: string, modelId: string): void {
  if (!providerId) return;
  const trimmed = modelId.trim();
  try {
    if (typeof window === 'undefined' || typeof localStorage === 'undefined') return;
    if (trimmed.length === 0) {
      localStorage.removeItem(MODEL_STORAGE_PREFIX + providerId);
      return;
    }
    localStorage.setItem(MODEL_STORAGE_PREFIX + providerId, trimmed);
  } catch {
    // Storage may be unavailable or full; the session keeps the choice in
    // memory and nothing is lost that the operator cannot retype.
  }
}

/** Forget the remembered model id of one provider. */
export function clearStoredModel(providerId: string): void {
  try {
    if (typeof window === 'undefined' || typeof localStorage === 'undefined') return;
    localStorage.removeItem(MODEL_STORAGE_PREFIX + providerId);
  } catch {
    // Nothing to do: a profile that refuses storage never stored anything.
  }
}

/**
 * Whether a model matches the operator's search.
 *
 * The search looks at the id and the name, case-insensitively. Every
 * whitespace-separated term must match somewhere, so `claude sonnet`
 * narrows a long list instead of widening it.
 */
export function modelMatchesQuery(model: ProviderModel, query: string): boolean {
  const terms = query.trim().toLowerCase().split(/\s+/).filter((term) => term.length > 0);
  if (terms.length === 0) return true;
  const haystack = `${model.id} ${model.name}`.toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

/**
 * The rows the picker shows: the search filter, then the "Free only"
 * toggle, then a bound so a provider with thousands of models cannot lock
 * the browser up.
 */
export function filterModels(
  models: ReadonlyArray<ProviderModel>,
  query: string,
  freeOnly: boolean,
): ReadonlyArray<ProviderModel> {
  const matched = models.filter(
    (model) => (!freeOnly || model.free) && modelMatchesQuery(model, query),
  );
  return matched.length > MODEL_LIST_LIMIT ? matched.slice(0, MODEL_LIST_LIMIT) : matched;
}

/** A context window, e.g. `128k ctx`. The exact token count is the title. */
export function formatContextLength(contextLength: number | null | undefined): string {
  if (contextLength === null || contextLength === undefined) return '';
  if (!Number.isFinite(contextLength) || contextLength <= 0) return '';
  if (contextLength >= 1000) {
    const thousands = contextLength / 1000;
    const rounded = Math.round(thousands * 10) / 10;
    return `${rounded}k ctx`;
  }
  return `${contextLength} ctx`;
}

/** A price number, printed exactly as the provider reported it. */
export function formatPriceValue(value: number): string {
  if (!Number.isFinite(value)) return '—';
  if (value === 0) return '0';
  const absolute = Math.abs(value);
  if (absolute >= 0.001) {
    return value
      .toFixed(6)
      .replace(/0+$/, '')
      .replace(/\.$/, '');
  }
  return String(value);
}

/**
 * The price pair, printed without a unit claim.
 *
 * The backend reports the numbers it read from the provider; it does not
 * say whether they are per token or per million tokens, so the UI does not
 * invent a unit. The exact values stay in the tooltip.
 */
export function formatPricing(pricing: ProviderModelPricing | null | undefined): string {
  if (!pricing) return '';
  return `in ${formatPriceValue(pricing.prompt)} · out ${formatPriceValue(pricing.completion)}`;
}

/** The one-line summary of a model row. */
export function describeModel(model: ProviderModel): string {
  const parts: string[] = [];
  const context = formatContextLength(model.context_length);
  if (context) parts.push(context);
  const price = formatPricing(model.pricing);
  if (price) parts.push(price);
  if (model.supports_tools === true) parts.push('tools');
  return parts.join(' · ');
}

/**
 * The exact numbers of a model, for the tooltip of a row.
 *
 * The `default` row of a subscription CLI carries one extra fact: it is the
 * CLI's own default model and the CLI does not report which one that is.
 */
export function describeModelDetail(model: ProviderModel): string {
  const parts: string[] = [`id: ${model.id}`];
  if (model.id === 'default') {
    parts.push('the CLI\'s own default model — the CLI does not report which model it picked');
  }
  if (model.context_length !== null && model.context_length !== undefined) {
    parts.push(`context: ${model.context_length} tokens`);
  }
  if (model.pricing) {
    parts.push(`prompt price: ${model.pricing.prompt}`);
    parts.push(`completion price: ${model.pricing.completion}`);
  }
  parts.push(`free: ${model.free ? 'yes' : 'no'}`);
  if (model.supports_tools !== null && model.supports_tools !== undefined) {
    parts.push(`tools: ${model.supports_tools ? 'yes' : 'no'}`);
  }
  return parts.join(' · ');
}

/**
 * Whether the catalog carries an error instead of a usable list.
 *
 * The backend answers with the envelope either way; an `error` means the
 * list could not be fetched, and the UI says so instead of showing an
 * empty dropdown as if the provider had no models.
 */
export function catalogError(catalog: ProviderModelCatalog | null): string | null {
  if (!catalog) return null;
  const error = catalog.error;
  if (error && error.trim().length > 0) return error;
  return null;
}
