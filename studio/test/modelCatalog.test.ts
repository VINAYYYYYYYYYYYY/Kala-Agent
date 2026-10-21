// The model catalog's pure parts: the response contract, the filter, the
// formatting, the remembered choice, and the provider policy.
//
// These run in the node environment (no DOM): everything here is a pure
// function of its arguments, except the storage helpers, which are written
// against a stubbed localStorage.

import { describe, expect, it, beforeEach, afterEach } from 'vitest';

import { parseProviderModelCatalog } from '../src/api/validators';
import { StudioValidationError } from '../src/api/validators';
import {
  catalogError,
  describeModel,
  describeModelDetail,
  filterModels,
  formatContextLength,
  formatPriceValue,
  formatPricing,
  modelMatchesQuery,
  MODEL_LIST_LIMIT,
  MODEL_STORAGE_PREFIX,
  readStoredModel,
  writeStoredModel,
  clearStoredModel,
} from '../src/modelCatalog';
import {
  describeProviderStatus,
  isLocalProvider,
  isSubscriptionProvider,
  isKnownProviderId,
  requiresModel,
  requiresRemoteConsent,
  subscriptionProviderId,
} from '../src/providerPolicy';
import type { ProviderModel } from '../src/types/studio';

function model(overrides: Partial<ProviderModel> = {}): ProviderModel {
  return {
    id: 'anthropic/claude-3.5-sonnet',
    name: 'Claude 3.5 Sonnet',
    context_length: 200000,
    pricing: { prompt: 0.000003, completion: 0.000015 },
    free: false,
    supports_tools: true,
    ...overrides,
  };
}

// A localStorage stub: the node test environment has no DOM storage.
class MemoryStorage {
  private readonly entries = new Map<string, string>();
  get length(): number {
    return this.entries.size;
  }
  key(index: number): string | null {
    return [...this.entries.keys()][index] ?? null;
  }
  getItem(key: string): string | null {
    return this.entries.get(key) ?? null;
  }
  setItem(key: string, value: string): void {
    this.entries.set(key, value);
  }
  removeItem(key: string): void {
    this.entries.delete(key);
  }
  clear(): void {
    this.entries.clear();
  }
}

let storage: MemoryStorage;

beforeEach(() => {
  storage = new MemoryStorage();
  (globalThis as unknown as { localStorage: MemoryStorage }).localStorage = storage;
  (globalThis as unknown as { window: unknown }).window = {};
});

afterEach(() => {
  delete (globalThis as unknown as { localStorage?: MemoryStorage }).localStorage;
  delete (globalThis as unknown as { window?: unknown }).window;
});

describe('the catalog response contract', () => {
  it('accepts the exact envelope the backend route renders', () => {
    const catalog = parseProviderModelCatalog({
      provider: 'openrouter',
      source: 'openrouter',
      fetched_at: '2026-10-20T12:00:00Z',
      models: [
        {
          id: 'minimax/minimax-m3.1-flash:free',
          name: 'MiniMax M3.1 Flash',
          context_length: 196608,
          pricing: { prompt: 0, completion: 0 },
          free: true,
          supports_tools: false,
        },
        {
          id: 'openai/gpt-4o-mini',
          name: 'GPT-4o mini',
        },
      ],
    });

    expect(catalog.provider).toBe('openrouter');
    expect(catalog.source).toBe('openrouter');
    expect(catalog.fetched_at).toBe('2026-10-20T12:00:00Z');
    expect(catalog.models).toHaveLength(2);
    expect(catalog.models[0]!.free).toBe(true);
    expect(catalog.models[0]!.pricing).toEqual({ prompt: 0, completion: 0 });
    // A model the backend reported without a context length carries "not
    // reported", never a number of the UI's own.
    expect(catalog.models[1]!.context_length).toBeNull();
    expect(catalog.models[1]!.pricing).toBeNull();
    expect(catalog.models[1]!.supports_tools).toBeNull();
  });

  it('accepts a static CLI list with no prices at all', () => {
    const catalog = parseProviderModelCatalog({
      provider: 'claude_code',
      source: 'static',
      fetched_at: '2026-10-20T12:00:00Z',
      models: [{ id: 'sonnet', name: 'Claude Sonnet (alias)' }],
    });

    expect(catalog.source).toBe('static');
    expect(catalog.models[0]!.id).toBe('sonnet');
  });

  it('carries the envelope error instead of hiding it', () => {
    const catalog = parseProviderModelCatalog({
      provider: 'openrouter',
      source: 'none',
      fetched_at: '2026-10-20T12:00:00Z',
      models: [],
      error: 'the stored OpenRouter credential was rejected by the provider',
    });

    expect(catalogError(catalog)).toBe('the stored OpenRouter credential was rejected by the provider');
  });

  it('refuses a malformed payload instead of rendering half a list', () => {
    expect(() =>
      parseProviderModelCatalog({
        provider: 'openrouter',
        source: 'openrouter',
        fetched_at: '2026-10-20T12:00:00Z',
        models: [{ id: 'x', name: 'X', pricing: { prompt: 'cheap', completion: 1 } }],
      }),
    ).toThrow(StudioValidationError);

    expect(() =>
      parseProviderModelCatalog({
        provider: 'openrouter',
        source: 'openrouter',
        fetched_at: '2026-10-20T12:00:00Z',
        models: [{ id: 'x', name: 'X', free: 'yes' }],
      }),
    ).toThrow(StudioValidationError);

    expect(() => parseProviderModelCatalog({ source: 'openrouter', models: [] })).toThrow(
      StudioValidationError,
    );
  });

  it('names the offending field', () => {
    try {
      parseProviderModelCatalog({
        provider: 'openrouter',
        source: 'openrouter',
        fetched_at: '2026-10-20T12:00:00Z',
        models: [{ id: 'x', name: 'X', context_length: 'big' }],
      });
      throw new Error('the payload should have been refused');
    } catch (cause) {
      expect(cause).toBeInstanceOf(StudioValidationError);
      expect((cause as StudioValidationError).path).toBe('providers/{id}/models.models[0].context_length');
    }
  });
});

describe('the model filter', () => {
  const models: ReadonlyArray<ProviderModel> = [
    model({ id: 'anthropic/claude-3.5-sonnet', name: 'Claude 3.5 Sonnet' }),
    model({ id: 'minimax/minimax-m3.1-flash:free', name: 'MiniMax M3.1 Flash', free: true }),
    model({ id: 'qwen/qwen3-coder', name: 'Qwen3 Coder' }),
  ];

  it('matches the id and the name, case-insensitively', () => {
    expect(modelMatchesQuery(models[0]!, 'claude')).toBe(true);
    expect(modelMatchesQuery(models[0]!, 'SONNET')).toBe(true);
    expect(modelMatchesQuery(models[0]!, 'anthropic')).toBe(true);
    expect(modelMatchesQuery(models[0]!, 'gpt')).toBe(false);
  });

  it('requires every term of a multi-word search to match', () => {
    expect(modelMatchesQuery(models[0]!, 'claude sonnet')).toBe(true);
    expect(modelMatchesQuery(models[0]!, 'claude coder')).toBe(false);
  });

  it('keeps only the free models when the free-only filter is on', () => {
    const filtered = filterModels(models, '', true);
    expect(filtered).toHaveLength(1);
    expect(filtered[0]!.id).toBe('minimax/minimax-m3.1-flash:free');
  });

  it('bounds the number of rendered rows', () => {
    const many: ProviderModel[] = [];
    for (let index = 0; index < MODEL_LIST_LIMIT + 25; index += 1) {
      many.push(model({ id: `model-${index}`, name: `Model ${index}` }));
    }
    expect(filterModels(many, '', false)).toHaveLength(MODEL_LIST_LIMIT);
  });
});

describe('the model formatting', () => {
  it('prints a context window in thousands', () => {
    expect(formatContextLength(200000)).toBe('200k ctx');
    expect(formatContextLength(8192)).toBe('8.2k ctx');
    expect(formatContextLength(512)).toBe('512 ctx');
    expect(formatContextLength(null)).toBe('');
  });

  it('prints the prices exactly as the provider reported them', () => {
    expect(formatPriceValue(0)).toBe('0');
    expect(formatPriceValue(0.000003)).toBe('0.000003');
    expect(formatPriceValue(1.5)).toBe('1.5');
    expect(formatPricing({ prompt: 0.000003, completion: 0.000015 })).toBe(
      'in 0.000003 · out 0.000015',
    );
    expect(formatPricing(null)).toBe('');
  });

  it('summarises a model row and keeps the exact numbers in the detail', () => {
    const row = model();
    expect(describeModel(row)).toContain('200k ctx');
    expect(describeModel(row)).toContain('tools');
    expect(describeModelDetail(row)).toContain('context: 200000 tokens');
    expect(describeModelDetail(row)).toContain('free: no');
  });
});

describe('the remembered choice', () => {
  it('stores the model id under the provider key and nothing else', () => {
    writeStoredModel('openrouter', 'minimax/minimax-m3.1-flash:free');

    expect(storage.getItem(`${MODEL_STORAGE_PREFIX}openrouter`)).toBe(
      'minimax/minimax-m3.1-flash:free',
    );
    expect(storage.length).toBe(1);
    expect(readStoredModel('openrouter')).toBe('minimax/minimax-m3.1-flash:free');
    expect(readStoredModel('openai')).toBeNull();
  });

  it('forgets a provider when the choice is cleared', () => {
    writeStoredModel('openrouter', 'gpt-4o-mini');
    clearStoredModel('openrouter');
    expect(readStoredModel('openrouter')).toBeNull();
  });

  it('never stores a key', () => {
    writeStoredModel('openrouter', 'minimax/minimax-m3.1-flash:free');
    for (let index = 0; index < storage.length; index += 1) {
      const key = storage.key(index)!;
      expect(key.startsWith(MODEL_STORAGE_PREFIX)).toBe(true);
      expect(storage.getItem(key)).not.toContain('sk-or');
    }
    expect(storage.getItem(`${MODEL_STORAGE_PREFIX}openrouter`)).toBe(
      'minimax/minimax-m3.1-flash:free',
    );
  });
});

describe('the provider policy', () => {
  it('keeps the local helper on the machine and every other route off it', () => {
    expect(isLocalProvider('local')).toBe(true);
    expect(requiresRemoteConsent('local')).toBe(false);
    for (const id of ['openai', 'openrouter', 'claude_code', 'codex_cli', 'opencode_cli', 'codex']) {
      expect(requiresRemoteConsent(id)).toBe(true);
    }
  });

  it('needs a model id only for the API-key routes', () => {
    expect(requiresModel('openai')).toBe(true);
    expect(requiresModel('openrouter')).toBe(true);
    // The local helper ignores it and the CLIs fall back to their own
    // default model.
    expect(requiresModel('local')).toBe(false);
    expect(requiresModel('claude_code')).toBe(false);
    expect(requiresModel('codex_cli')).toBe(false);
    expect(requiresModel('opencode_cli')).toBe(false);
  });

  it('knows the three subscription CLIs and the historical codex alias', () => {
    expect(isSubscriptionProvider('claude_code')).toBe(true);
    expect(isSubscriptionProvider('codex_cli')).toBe(true);
    expect(isSubscriptionProvider('opencode_cli')).toBe(true);
    expect(isSubscriptionProvider('codex')).toBe(true);
    expect(isSubscriptionProvider('openai')).toBe(false);
    expect(subscriptionProviderId('codex')).toBe('codex_cli');
    expect(subscriptionProviderId('openai')).toBeNull();
  });

  it('knows the closed provider id set', () => {
    expect(isKnownProviderId('opencode_cli')).toBe(true);
    expect(isKnownProviderId('llama.cpp')).toBe(false);
  });
});

describe('the provider status', () => {
  it('prefers the backend availability probe over the configured flag', () => {
    const status = describeProviderStatus({
      id: 'codex_cli',
      name: 'Codex CLI',
      configured: true,
      available: false,
      reason: 'the codex CLI is not installed on PATH',
      notes: 'Subscription (CLI).',
    });

    expect(status.state).toBe('unavailable');
    expect(status.badge).toBe('Unavailable');
    expect(status.reason).toBe('the codex CLI is not installed on PATH');
    expect(status.selectable).toBe(false);
  });

  it('keeps a provider without a credential selectable so the reason stays reachable', () => {
    const status = describeProviderStatus({
      id: 'openrouter',
      name: 'OpenRouter',
      configured: false,
      notes: 'API-key route.',
    });

    expect(status.state).toBe('not-configured');
    expect(status.badge).toBe('Not configured');
    expect(status.reason).toBeNull();
    expect(status.selectable).toBe(true);
  });

  it('reports a configured provider', () => {
    const status = describeProviderStatus({
      id: 'openai',
      name: 'OpenAI',
      configured: true,
      notes: '',
    });

    expect(status.state).toBe('configured');
    expect(status.badge).toBe('Configured');
  });
});
