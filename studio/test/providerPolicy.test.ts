// The provider policy of the composer.
//
// Two rules are pinned here that the rest of the UI leans on:
//   * the canonical id of an alias, so the composer never offers the same
//     route twice,
//   * the honest label of a reply that used the CLI's OWN default model: the
//     CLIs do not report which model they picked, so the UI never names one.

import { describe, expect, it } from 'vitest';

import {
  canonicalProviderId,
  defaultModelLabel,
  isDefaultModelId,
  isSubscriptionProvider,
  optionalModelNote,
  PROVIDER_ALIASES,
  requiresModel,
  requiresRemoteConsent,
} from '../src/providerPolicy';

describe('the provider aliases', () => {
  it('maps the historical codex id onto the canonical one', () => {
    expect(canonicalProviderId('codex')).toBe('codex_cli');
    expect(canonicalProviderId('codex_cli')).toBe('codex_cli');
    expect(canonicalProviderId('openai')).toBe('openai');
  });

  it('names the alias it knows', () => {
    expect(PROVIDER_ALIASES['codex']).toBe('codex_cli');
  });

  it('treats the alias as the subscription route it is', () => {
    expect(isSubscriptionProvider('codex')).toBe(true);
    expect(isSubscriptionProvider('codex_cli')).toBe(true);
    expect(isSubscriptionProvider('openai')).toBe(false);
    // The alias still leaves the machine and still needs no model id.
    expect(requiresRemoteConsent('codex')).toBe(true);
    expect(requiresModel('codex')).toBe(false);
  });
});

describe('the CLI default model', () => {
  it('names the CLI and who chose the model', () => {
    expect(defaultModelLabel('codex_cli', 'default')).toBe(
      'Codex CLI default model (chosen by Codex)',
    );
    expect(defaultModelLabel('claude_code', 'default')).toBe(
      'Claude Code default model (chosen by Claude Code)',
    );
    expect(defaultModelLabel('opencode_cli', 'default')).toBe(
      'OpenCode CLI default model (chosen by OpenCode)',
    );
  });

  it('says the same when the backend reported no model at all', () => {
    expect(defaultModelLabel('codex_cli', null)).toBe(
      'Codex CLI default model (chosen by Codex)',
    );
    expect(defaultModelLabel('codex_cli', '')).toBe(
      'Codex CLI default model (chosen by Codex)',
    );
  });

  it('keeps the alias honest too', () => {
    expect(defaultModelLabel('codex', 'default')).toBe(
      'Codex CLI default model (chosen by Codex)',
    );
  });

  it('says nothing for a reply that named a real model', () => {
    expect(defaultModelLabel('codex_cli', 'gpt-5-codex')).toBeNull();
    expect(defaultModelLabel('openai', 'gpt-4o')).toBeNull();
  });

  it('says nothing for a route that is not a CLI', () => {
    expect(defaultModelLabel('openai', 'default')).toBeNull();
    expect(defaultModelLabel('local', 'default')).toBeNull();
  });

  it('recognises the literal default id', () => {
    expect(isDefaultModelId('default')).toBe(true);
    expect(isDefaultModelId(' default ')).toBe(true);
    expect(isDefaultModelId('gpt-4o')).toBe(false);
    expect(isDefaultModelId(null)).toBe(false);
  });
});

describe('the note about an empty model field', () => {
  it('is shown for a subscription CLI and says how to pin a model', () => {
    const note = optionalModelNote('codex_cli', 'Codex CLI');
    expect(note).toContain("Leave empty to use Codex CLI's own default model");
    expect(note).toContain('does not report which model it picked');
    expect(note).toContain('Pick a row here to pin one');
  });

  it('is not shown for a route that requires a model id', () => {
    expect(optionalModelNote('openai', 'OpenAI')).toBeNull();
    expect(optionalModelNote('local', 'Local helper')).toBeNull();
  });
});
