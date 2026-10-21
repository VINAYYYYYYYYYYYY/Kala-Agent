// @vitest-environment happy-dom
// The composer's provider picker.
//
// The picker renders the backend's own provider registry and nothing more:
// the availability badge, the reason the backend gave, and where the
// credential comes from. The rules under test:
//   * a provider the backend probed and found unavailable cannot be picked
//     and carries its reason,
//   * a provider without a credential CAN be picked, so the operator can see
//     why it is unusable and go and set the key,
//   * the three subscription CLIs are marked as such and carry no key form,
//   * the popup is keyboard operable.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';

import { ProviderPicker } from '../src/components/chat/ProviderPicker';
import type { ProviderRecord } from '../src/types/studio';
import { openProviderPicker, pickProvider, providerOption, providerTrigger } from './providerFixtures';

const providers: ReadonlyArray<ProviderRecord> = [
  { id: 'local', name: 'Local Deterministic Helper', configured: true, notes: 'Offline helper.' },
  { id: 'openai', name: 'OpenAI (Chat Completions)', configured: true, notes: 'API-key route.' },
  {
    id: 'openrouter',
    name: 'OpenRouter (Chat Completions)',
    configured: false,
    notes: 'API-key route.',
  },
  {
    id: 'claude_code',
    name: 'Claude Code CLI',
    configured: true,
    available: true,
    source: 'subscription (CLI)',
    notes: 'Subscription (CLI).',
  },
  {
    id: 'codex_cli',
    name: 'Codex CLI',
    configured: false,
    available: false,
    reason: 'the codex CLI is not installed on PATH',
    notes: 'Subscription (CLI).',
  },
  {
    id: 'opencode_cli',
    name: 'OpenCode CLI',
    configured: true,
    available: true,
    notes: 'Subscription (CLI).',
  },
];

const credentials = [
  { provider: 'openai', env_var: 'OPENAI_API_KEY', configured: true, source: 'stored' as const },
  {
    provider: 'openrouter',
    env_var: 'OPENROUTER_API_KEY',
    configured: false,
    source: 'none' as const,
  },
  {
    provider: 'claude_code',
    env_var: 'CLAUDE_CODE_SESSION',
    configured: true,
    source: 'subscription' as const,
  },
];

function renderPicker(
  value = 'openai',
  onChange = vi.fn(),
  rows: ReadonlyArray<ProviderRecord> = providers,
) {
  const view = render(
    <ProviderPicker providers={rows} credentials={credentials} value={value} onChange={onChange} />,
  );
  return { view, onChange };
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  cleanup();
});

describe('the provider picker', () => {
  it('shows the selected provider and its availability badge', () => {
    const { view } = renderPicker('openai');

    const trigger = providerTrigger(view.container);
    expect(trigger.textContent).toContain('OpenAI (Chat Completions)');
    expect(trigger.querySelector('.provider-picker-badge')?.getAttribute('data-state')).toBe(
      'configured',
    );
    expect(trigger.getAttribute('aria-haspopup')).toBe('listbox');
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
  });

  it('lists every provider the backend registered, with its badge', () => {
    const { view } = renderPicker();

    openProviderPicker(view.container);

    const rows = view.container.querySelectorAll<HTMLElement>('.provider-picker-option');
    expect(rows).toHaveLength(6);
    expect(providerOption(view.container, 'local').textContent).toContain('Local Deterministic Helper');
    expect(providerOption(view.container, 'openrouter').getAttribute('data-provider-state')).toBe(
      'not-configured',
    );
    expect(providerOption(view.container, 'claude_code').textContent).toContain('Subscription (CLI)');
  });

  it('marks the subscription CLIs and where their credential comes from', () => {
    const { view } = renderPicker();

    openProviderPicker(view.container);

    const claude = providerOption(view.container, 'claude_code');
    expect(claude.textContent).toContain('Subscription (CLI)');
    expect(claude.textContent).toContain('subscription session already signed in on this machine');
  });

  it('refuses to pick a provider the backend reported as unavailable and shows why', () => {
    const onChange = vi.fn();
    const { view } = renderPicker('openai', onChange);

    openProviderPicker(view.container);

    const codex = providerOption(view.container, 'codex_cli');
    expect(codex.getAttribute('aria-disabled')).toBe('true');
    expect(codex.getAttribute('data-provider-state')).toBe('unavailable');
    expect(codex.textContent).toContain('the codex CLI is not installed on PATH');

    fireEvent.click(codex);
    expect(onChange).not.toHaveBeenCalled();
  });

  it('lets the operator pick a provider that has no credential yet', () => {
    const onChange = vi.fn();
    const { view } = renderPicker('openai', onChange);

    pickProvider(view.container, 'openrouter');

    expect(onChange).toHaveBeenCalledWith('openrouter');
  });

  it('closes the popup and returns the focus to the trigger after a pick', () => {
    const { view } = renderPicker();

    pickProvider(view.container, 'local');

    expect(view.container.querySelector('[role="listbox"]')).toBeNull();
    expect(document.activeElement).toBe(providerTrigger(view.container));
  });

  it('moves the active option with the arrow keys and picks it with Enter', () => {
    const onChange = vi.fn();
    const { view } = renderPicker('openai', onChange);

    openProviderPicker(view.container);
    const listbox = view.container.querySelector<HTMLElement>('[role="listbox"]')!;
    expect(document.activeElement).toBe(listbox);
    // The listbox owns the active row through `aria-activedescendant`.
    expect(listbox.getAttribute('aria-activedescendant')).toBeTruthy();

    fireEvent.keyDown(listbox, { key: 'ArrowDown' });
    fireEvent.keyDown(listbox, { key: 'Enter' });

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange.mock.calls[0]![0]).not.toBe('openai');
  });

  it('closes the popup on Escape without picking anything', () => {
    const onChange = vi.fn();
    const { view } = renderPicker('openai', onChange);

    openProviderPicker(view.container);
    const listbox = view.container.querySelector<HTMLElement>('[role="listbox"]')!;
    fireEvent.keyDown(listbox, { key: 'Escape' });

    expect(view.container.querySelector('[role="listbox"]')).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(providerTrigger(view.container));
  });

  it('opens the popup from the keyboard', () => {
    const { view } = renderPicker('openai');

    fireEvent.keyDown(providerTrigger(view.container), { key: 'ArrowDown' });

    expect(view.container.querySelector('[role="listbox"]')).toBeTruthy();
  });
});

describe('the Codex alias', () => {
  const codexRows: ReadonlyArray<ProviderRecord> = [
    { id: 'codex', name: 'Codex CLI (ChatGPT subscription)', configured: true, available: true, notes: 'Historical alias.' },
    { id: 'codex_cli', name: 'Codex CLI (ChatGPT subscription)', configured: true, available: true, notes: '' },
  ];
  const withAlias: ReadonlyArray<ProviderRecord> = [
    ...providers.filter((entry) => entry.id !== 'codex_cli'),
    ...codexRows,
  ];

  it('shows ONE Codex row when the backend lists the alias and the canonical id', () => {
    const { view } = renderPicker('openai', vi.fn(), withAlias);

    openProviderPicker(view.container);

    const codexRows = Array.from(
      view.container.querySelectorAll<HTMLElement>('[data-provider-id^="codex"]'),
    );
    expect(codexRows).toHaveLength(1);
    expect(codexRows[0]!.getAttribute('data-provider-id')).toBe('codex_cli');
  });

  it('keeps a stored selection of the alias and shows it as the canonical route', () => {
    const { view } = renderPicker('codex', vi.fn(), withAlias);

    const trigger = providerTrigger(view.container);
    expect(trigger.textContent).toContain('Codex CLI (ChatGPT subscription)');
    expect(trigger.querySelector('.provider-picker-badge')?.getAttribute('data-state')).toBe(
      'configured',
    );

    openProviderPicker(view.container);
    const canonical = view.container.querySelector<HTMLElement>('[data-provider-id="codex_cli"]')!;
    expect(canonical.getAttribute('aria-selected')).toBe('true');
  });

  it('picks the canonical id when the operator chooses the Codex row', () => {
    const onChange = vi.fn();
    const { view } = renderPicker('openai', onChange, withAlias);

    pickProvider(view.container, 'codex_cli');

    expect(onChange).toHaveBeenCalledWith('codex_cli');
  });

  it('still shows the alias row when the backend lists no canonical id', () => {
    const onlyAlias: ReadonlyArray<ProviderRecord> = [
      ...providers.filter((entry) => entry.id !== 'codex_cli'),
      codexRows[0]!,
    ];
    const { view } = renderPicker('openai', vi.fn(), onlyAlias);

    openProviderPicker(view.container);

    expect(
      view.container.querySelector('[data-provider-id="codex"]'),
    ).toBeTruthy();
  });
});

describe('the subscription rows', () => {
  it('says that the model of a CLI is optional and defaults to the CLI own model', () => {
    const { view } = renderPicker();

    openProviderPicker(view.container);

    const claude = providerOption(view.container, 'claude_code');
    expect(claude.textContent).toContain('Model optional');
    expect(claude.textContent).toContain('its own default model');
    expect(claude.textContent).toContain('pin one in the model picker');
  });

  it('says nothing about an optional model for an API route', () => {
    const { view } = renderPicker();

    openProviderPicker(view.container);

    expect(providerOption(view.container, 'openai').textContent).not.toContain('Model optional');
  });
});
