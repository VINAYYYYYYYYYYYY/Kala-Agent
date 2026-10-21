// @vitest-environment happy-dom
// The composer's model picker.
//
// The states under test are the ones the operator can actually hit:
//   * no provider selected,
//   * a provider without a credential (no catalog to ask for),
//   * the list loading,
//   * the backend refusing the listing because the remote-egress consent
//     stamp is not ticked (403 `consent_required`),
//   * a failed listing (transport failure) and an envelope error,
//   * a provider that reported no models,
//   * a search that matches nothing,
//   * the list itself, with the context length, the price, and the free
//     badge.
//
// The rules that matter most:
//   * the consent flag is the operator's, never the UI's: the request
//     carries `allow_remote=false` until the stamp is ticked,
//   * the choice is remembered per provider, and ONLY the model id is
//     written to localStorage.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  getProviderModels: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return { ...actual, getProviderModels: mocks.getProviderModels };
});

import { StudioApiError, StudioAuthError, StudioNetworkError } from '../src/api/client';
import { ModelPicker } from '../src/components/chat/ModelPicker';
import { dispatch, resetStore } from '../src/state/store';
import { MODEL_STORAGE_PREFIX } from '../src/modelCatalog';

const connection = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'desktop' as const,
};

const CATALOG = {
  provider: 'openrouter',
  source: 'openrouter',
  fetched_at: '2026-10-20T12:00:00Z',
  models: [
    {
      id: 'anthropic/claude-3.5-sonnet',
      name: 'Claude 3.5 Sonnet',
      context_length: 200000,
      pricing: { prompt: 0.000003, completion: 0.000015 },
      free: false,
      supports_tools: true,
    },
    {
      id: 'minimax/minimax-m3.1-flash:free',
      name: 'MiniMax M3.1 Flash',
      context_length: 196608,
      pricing: { prompt: 0, completion: 0 },
      free: true,
      supports_tools: false,
    },
    {
      id: 'qwen/qwen3-coder',
      name: 'Qwen3 Coder',
      context_length: 262144,
      pricing: { prompt: 0.0000003, completion: 0.0000012 },
      free: false,
      supports_tools: true,
    },
  ],
};

function ready(): void {
  dispatch({ type: 'connection/ready', connection });
}

function renderPicker(props: Partial<Parameters<typeof ModelPicker>[0]> = {}) {
  const merged = {
    providerId: 'openrouter',
    providerName: 'OpenRouter',
    value: '',
    onChange: vi.fn(),
    configured: true,
    allowRemote: false,
    onGrantConsent: vi.fn(),
    optional: false,
    ...props,
  };
  const view = render(<ModelPicker {...merged} />);
  return { view, props: merged };
}

beforeEach(() => {
  resetStore();
  ready();
  mocks.getProviderModels.mockResolvedValue(CATALOG);
  try {
    localStorage.clear();
  } catch {
    // no localStorage in this environment
  }
});

afterEach(() => {
  cleanup();
  resetStore();
  mocks.getProviderModels.mockReset();
  try {
    localStorage.clear();
  } catch {
    // no localStorage in this environment
  }
});

describe('the model picker states', () => {
  it('asks for a provider when none is selected and asks for nothing', () => {
    const { view } = renderPicker({ providerId: '', providerName: 'No provider' });

    expect(view.container.textContent).toContain('Choose a provider to list its models.');
    expect(mocks.getProviderModels).not.toHaveBeenCalled();
  });

  it('does not ask a provider without a credential for its models', () => {
    const { view } = renderPicker({ configured: false });

    expect(view.container.textContent).toContain('is not configured server-side');
    expect(mocks.getProviderModels).not.toHaveBeenCalled();
  });

  it('shows the loading state and then the list', async () => {
    let release: (value: typeof CATALOG) => void = () => {};
    mocks.getProviderModels.mockImplementation(
      () => new Promise((resolve) => {
        release = resolve;
      }),
    );
    const { view } = renderPicker();

    expect(view.container.textContent).toContain('Loading the model list for OpenRouter');
    release(CATALOG);
    await waitFor(() => expect(view.container.textContent).toContain('3 models'));
  });

  it('shows the consent-required state when the backend refuses the listing', async () => {
    mocks.getProviderModels.mockRejectedValue(
      new StudioAuthError(403, {
        code: 'consent_required',
        message: 'the remote model listing requires explicit consent',
        details: ["pass allow_remote=true to list models for provider 'openrouter'"],
      }),
    );
    const onGrantConsent = vi.fn();
    const { view } = renderPicker({ onGrantConsent });

    await waitFor(() =>
      expect(view.container.querySelector('.model-picker-consent')?.textContent).toContain(
        'Remote egress consent required',
      ),
    );
    expect(view.container.textContent).toContain('consent_required');
    expect(view.container.textContent).toContain('allow_remote=true');

    fireEvent.click(view.container.querySelector<HTMLButtonElement>('.model-picker-action')!);
    expect(onGrantConsent).toHaveBeenCalledTimes(1);
  });

  it('passes the operator consent stamp and never sets it by itself', async () => {
    renderPicker({ allowRemote: false });
    await waitFor(() => expect(mocks.getProviderModels).toHaveBeenCalled());
    expect(mocks.getProviderModels.mock.calls[0]![2]).toBe(false);

    cleanup();
    mocks.getProviderModels.mockClear();
    renderPicker({ allowRemote: true });
    await waitFor(() => expect(mocks.getProviderModels).toHaveBeenCalled());
    expect(mocks.getProviderModels.mock.calls[0]![2]).toBe(true);
  });

  it('reports a failed listing and retries it', async () => {
    mocks.getProviderModels.mockRejectedValueOnce(
      new StudioNetworkError('network request to /api/studio/providers/openrouter/models failed', new Error('down')),
    );
    const { view } = renderPicker();

    await waitFor(() =>
      expect(view.container.querySelector('.model-picker-error')?.textContent).toContain(
        'network request',
      ),
    );

    fireEvent.click(view.container.querySelector<HTMLButtonElement>('.model-picker-action')!);
    await waitFor(() => expect(mocks.getProviderModels).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(view.container.textContent).toContain('3 models'));
  });

  it('reports an error carried inside the catalog envelope', async () => {
    mocks.getProviderModels.mockResolvedValue({
      provider: 'openrouter',
      source: 'openrouter',
      fetched_at: '2026-10-20T12:00:00Z',
      models: [],
      error: 'the stored OpenRouter credential was rejected by the provider',
    });
    const { view } = renderPicker();

    await waitFor(() =>
      expect(view.container.querySelector('.model-picker-error')?.textContent).toContain(
        'rejected by the provider',
      ),
    );
  });

  it('reports a provider that listed no models', async () => {
    mocks.getProviderModels.mockResolvedValue({
      provider: 'claude_code',
      source: 'static',
      fetched_at: '2026-10-20T12:00:00Z',
      models: [],
    });
    const { view } = renderPicker({ providerId: 'claude_code', providerName: 'Claude Code CLI' });

    await waitFor(() =>
      expect(view.container.textContent).toContain('Claude Code CLI reported no models.'),
    );
  });

  it('says so when the search matches nothing', async () => {
    const { view } = renderPicker();
    await waitFor(() => expect(view.container.textContent).toContain('3 models'));

    fireEvent.change(view.container.querySelector<HTMLInputElement>('#chat-model-input')!, {
      target: { value: 'no-such-model' },
    });

    await waitFor(() => expect(view.container.textContent).toContain('No model matches'));
  });

  it('never shows one provider’s models under another provider', async () => {
    const { view } = renderPicker();
    await waitFor(() => expect(view.container.textContent).toContain('3 models'));

    view.rerender(
      <ModelPicker
        providerId="claude_code"
        providerName="Claude Code CLI"
        value=""
        onChange={vi.fn()}
        configured
        allowRemote={false}
        onGrantConsent={vi.fn()}
        optional
      />,
    );

    await waitFor(() =>
      expect(mocks.getProviderModels).toHaveBeenCalledWith(
        expect.anything(),
        'claude_code',
        false,
        expect.any(AbortSignal),
      ),
    );
    // The previous provider's rows are gone.
    expect(
      view.container.querySelector('[data-model-id="anthropic/claude-3.5-sonnet"]'),
    ).toBeNull();
  });

  it('asks for the list as soon as the provider becomes configured', async () => {
    const { view } = renderPicker({ configured: false });
    expect(mocks.getProviderModels).not.toHaveBeenCalled();

    view.rerender(
      <ModelPicker
        providerId="openrouter"
        providerName="OpenRouter"
        value=""
        onChange={vi.fn()}
        configured
        allowRemote={false}
        onGrantConsent={vi.fn()}
        optional={false}
      />,
    );

    await waitFor(() => expect(mocks.getProviderModels).toHaveBeenCalledTimes(1));
    expect(mocks.getProviderModels.mock.calls[0]![1]).toBe('openrouter');
  });
});

describe('the CLI default model', () => {
  const CLI_CATALOG = {
    provider: 'codex_cli',
    source: 'static',
    fetched_at: '2026-10-21T12:00:00Z',
    models: [
      { id: 'default', name: 'Codex CLI default', free: false },
      { id: 'gpt-5-codex', name: 'GPT-5 Codex', free: false },
    ],
  };

  it('names the default row honestly and never invents a model', async () => {
    mocks.getProviderModels.mockResolvedValue(CLI_CATALOG);
    const { view } = renderPicker({ providerId: 'codex_cli', providerName: 'Codex CLI' });

    await waitFor(() => expect(view.container.textContent).toContain('2 models'));
    fireEvent.focus(view.container.querySelector<HTMLInputElement>('#chat-model-input')!);

    const defaultRow = view.container.querySelector<HTMLElement>('[data-model-id="default"]')!;
    expect(defaultRow.textContent).toContain('Codex CLI default model (chosen by Codex)');
    // The row never claims a concrete model name.
    expect(defaultRow.textContent).not.toMatch(/gpt-/i);
    // The literal id stays visible: it is what the request carries.
    expect(defaultRow.querySelector('.model-picker-option-id')?.textContent).toBe('default');
  });

  it('says what an empty model field means and how to pin one', () => {
    const { view } = renderPicker({ providerId: 'codex_cli', providerName: 'Codex CLI' });

    const note = view.container.querySelector('.model-picker-note-default');
    expect(note?.textContent).toContain(
      "Leave empty to use Codex CLI's own default model",
    );
    expect(note?.textContent).toContain('does not report which model it picked');
    expect(note?.textContent).toContain('Pick a row here to pin one');
  });

  it('says no such thing for a route that requires a model id', () => {
    const { view } = renderPicker({ providerId: 'openrouter', providerName: 'OpenRouter' });
    expect(view.container.querySelector('.model-picker-note-default')).toBeNull();
  });
});

describe('a CLI without a model list', () => {
  it('gives the real reason and offers no API-key button', () => {
    const { view } = renderPicker({
      providerId: 'claude_code',
      providerName: 'Claude Code CLI',
      configured: false,
      providerAvailable: false,
      providerReason:
        'the Claude Code CLI was not found on PATH or at KALA_STUDIO_CLAUDE_CLI_PATH',
    });

    const note = view.container.querySelector('.model-picker-note-unavailable');
    expect(note?.textContent).toContain('the Claude Code CLI was not found on PATH');
    // A CLI has no key: the operator must not be sent to the API-key form.
    expect(view.container.querySelector('.model-picker-action')).toBeNull();
    expect(view.container.textContent).not.toContain('is not configured server-side');
    expect(mocks.getProviderModels).not.toHaveBeenCalled();
  });

  it('falls back to the generic CLI reason when the backend gives none', () => {
    const { view } = renderPicker({
      providerId: 'codex_cli',
      providerName: 'Codex CLI',
      configured: false,
      providerAvailable: false,
      providerReason: null,
    });

    const note = view.container.querySelector('.model-picker-note-unavailable');
    expect(note?.textContent).toContain('Codex CLI was not found on this machine or is unavailable');
    expect(view.container.querySelector('.model-picker-action')).toBeNull();
  });

  it('still offers the API-key form for an API route without a credential', () => {
    const onOpenKeySettings = vi.fn();
    const { view } = renderPicker({
      providerId: 'openrouter',
      providerName: 'OpenRouter',
      configured: false,
      onOpenKeySettings,
    });

    expect(view.container.textContent).toContain('is not configured server-side');
    const action = view.container.querySelector<HTMLButtonElement>('.model-picker-action');
    expect(action).toBeTruthy();
    fireEvent.click(action!);
    expect(onOpenKeySettings).toHaveBeenCalledTimes(1);
  });
});

describe('the model list', () => {  it('shows the context length, the price, and the free badge', async () => {
    const { view } = renderPicker();
    await waitFor(() => expect(view.container.textContent).toContain('3 models'));

    fireEvent.focus(view.container.querySelector<HTMLInputElement>('#chat-model-input')!);

    const rows = view.container.querySelectorAll<HTMLElement>('.model-picker-option');
    expect(rows).toHaveLength(3);
    expect(rows[0]!.textContent).toContain('Claude 3.5 Sonnet');
    expect(rows[0]!.textContent).toContain('anthropic/claude-3.5-sonnet');
    expect(rows[0]!.textContent).toContain('200k ctx');
    expect(rows[0]!.textContent).toContain('in 0.000003');
    expect(rows[0]!.textContent).toContain('out 0.000015');
    expect(rows[0]!.getAttribute('data-free')).toBe('no');
    expect(rows[1]!.querySelector('.model-picker-badge')?.textContent).toBe('Free');
    expect(rows[1]!.getAttribute('data-free')).toBe('yes');
  });

  it('filters the list by the search text', async () => {
    const { view } = renderPicker();
    await waitFor(() => expect(view.container.textContent).toContain('3 models'));

    fireEvent.change(view.container.querySelector<HTMLInputElement>('#chat-model-input')!, {
      target: { value: 'qwen' },
    });

    await waitFor(() => {
      const rows = view.container.querySelectorAll<HTMLElement>('.model-picker-option');
      expect(rows).toHaveLength(1);
      expect(rows[0]!.getAttribute('data-model-id')).toBe('qwen/qwen3-coder');
    });
    expect(view.container.textContent).toContain('1 of 3 models');
  });

  it('filters the list with the free-only toggle', async () => {
    const { view } = renderPicker();
    await waitFor(() => expect(view.container.textContent).toContain('3 models'));

    fireEvent.focus(view.container.querySelector<HTMLInputElement>('#chat-model-input')!);
    fireEvent.click(view.container.querySelector<HTMLInputElement>('.model-picker-free input')!);

    await waitFor(() => {
      const rows = view.container.querySelectorAll<HTMLElement>('.model-picker-option');
      expect(rows).toHaveLength(1);
      expect(rows[0]!.getAttribute('data-model-id')).toBe('minimax/minimax-m3.1-flash:free');
    });
  });

  it('picks a model with the keyboard and reports it to the composer', async () => {
    const onChange = vi.fn();
    const { view } = renderPicker({ onChange });
    await waitFor(() => expect(view.container.textContent).toContain('3 models'));

    const input = view.container.querySelector<HTMLInputElement>('#chat-model-input')!;
    fireEvent.focus(input);
    // The first row is active when the list opens; ArrowDown moves to the
    // next one, Enter takes it.
    await waitFor(() => expect(input.getAttribute('aria-activedescendant')).toBeTruthy());
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'Enter' });

    expect(onChange).toHaveBeenCalledWith('minimax/minimax-m3.1-flash:free');
  });

  it('closes the list when the operator clicks outside it', async () => {
    const { view } = renderPicker();
    await waitFor(() => expect(view.container.textContent).toContain('3 models'));

    fireEvent.focus(view.container.querySelector<HTMLInputElement>('#chat-model-input')!);
    expect(view.container.querySelector('[role="listbox"]')).toBeTruthy();

    fireEvent.mouseDown(document.body);
    expect(view.container.querySelector('[role="listbox"]')).toBeNull();
  });

  it('closes the list on Escape without changing the choice', async () => {
    const onChange = vi.fn();
    const { view } = renderPicker({ onChange, value: 'qwen/qwen3-coder' });
    await waitFor(() => expect(view.container.textContent).toContain('3 models'));

    const input = view.container.querySelector<HTMLInputElement>('#chat-model-input')!;
    fireEvent.focus(input);
    fireEvent.keyDown(input, { key: 'Escape' });

    expect(view.container.querySelector('[role="listbox"]')).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
    expect(input.value).toBe('qwen/qwen3-coder');
  });
});

describe('the remembered choice', () => {
  it('stores the model id of the provider and nothing else', async () => {
    const { view } = renderPicker();
    await waitFor(() => expect(view.container.textContent).toContain('3 models'));

    fireEvent.focus(view.container.querySelector<HTMLInputElement>('#chat-model-input')!);
    fireEvent.click(
      view.container.querySelector<HTMLElement>('[data-model-id="minimax/minimax-m3.1-flash:free"]')!,
    );

    await waitFor(() => {
      expect(localStorage.getItem(`${MODEL_STORAGE_PREFIX}openrouter`)).toBe(
        'minimax/minimax-m3.1-flash:free',
      );
    });
    // The key is not in the storage: the value is the model id alone.
    for (let index = 0; index < localStorage.length; index += 1) {
      const key = localStorage.key(index)!;
      expect(key.startsWith(MODEL_STORAGE_PREFIX)).toBe(true);
      expect(localStorage.getItem(key)).not.toContain('sk-or');
    }
  });
});

describe('the catalog contract', () => {
  it('refuses a malformed catalog instead of rendering half a list', async () => {
    mocks.getProviderModels.mockRejectedValue(
      new StudioApiError(500, {
        code: 'internal_error',
        message: 'the catalog payload was not valid',
        details: [],
      }),
    );
    const { view } = renderPicker();

    await waitFor(() =>
      expect(view.container.querySelector('.model-picker-error')?.textContent).toContain(
        'internal_error',
      ),
    );
  });
});
