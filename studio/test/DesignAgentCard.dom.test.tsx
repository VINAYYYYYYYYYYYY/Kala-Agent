// @vitest-environment happy-dom
// The agent card of a design turn.
//
// The card is the honest record of a compiled part: the measured table, the
// backend's checks, the artefact digests, the verification stamp, and the
// jump-to-section pill. Every assertion below checks that a value is the
// one the geometry kernel reported, and that a stamp or a chip is ABSENT
// when no data backs it.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

import { ChatPanel } from '../src/components/chat/ChatPanel';
import { DesignAgentCard } from '../src/components/chat/DesignAgentCard';
import { dispatch, getState, resetStore } from '../src/state/store';
import type { DesignResult } from '../src/types/studio';
import {
  BRACKET_DESIGN_ID,
  bracketDesignResult,
  notReportedDesignResult,
} from './designFixtures';

const OTHER_DESIGN_ID = 'a'.repeat(64);

function otherDesign(): DesignResult {
  return {
    ...bracketDesignResult,
    design_id: OTHER_DESIGN_ID,
    label: 'L-bracket 60 x 45 x 40 (copy)',
  };
}

beforeEach(() => {
  resetStore();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function row(container: HTMLElement, key: string): HTMLElement | null {
  return container.querySelector<HTMLElement>(`[data-measure="${key}"]`);
}

function rowText(container: HTMLElement, key: string): string {
  return row(container, key)?.textContent ?? '';
}

function stamp(container: HTMLElement): HTMLElement | null {
  return container.querySelector<HTMLElement>('.chat-panel-design-stamp');
}

describe('the measured table', () => {
  it('prints the volume, area, and envelope the provider measured', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    expect(rowText(container, 'volume')).toContain('35571.416047 mm³');
    expect(rowText(container, 'area')).toContain('11889.101731 mm²');
    expect(rowText(container, 'envelope_x')).toContain('60.00 mm');
    expect(rowText(container, 'envelope_y')).toContain('45.00 mm');
    expect(rowText(container, 'envelope_z')).toContain('40.00 mm');
  });

  it('names the provider that measured the part', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    const provider = container.querySelector<HTMLElement>('.chat-panel-design-provider')!;
    expect(provider.textContent).toBe('measured by FreeCAD 1.1.4');
    expect(provider.getAttribute('data-provider')).toBe('freecad');
    expect(container.querySelector('.chat-panel-design-caption')?.textContent).toBe(
      'measured by FreeCAD 1.1.4',
    );
  });

  it('prints an em dash for every value the provider did not report', () => {
    dispatch({ type: 'design/received', design: notReportedDesignResult });
    const { container } = render(
      <DesignAgentCard designId={notReportedDesignResult.design_id} />,
    );

    for (const key of ['volume', 'area', 'envelope_x', 'envelope_y', 'envelope_z']) {
      expect(rowText(container, key)).toContain('—');
    }
    expect(rowText(container, 'volume')).not.toContain('0.00');
    expect(container.textContent).not.toContain('B-Rep Verified');
  });

  it('omits the mass because the backend declares no density and reports none', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    expect(row(container, 'mass')).toBeNull();
    expect(container.textContent).not.toContain('mass');
  });

  it('prints the provider-reported mass when it reports one', () => {
    const withMass: DesignResult = {
      ...bracketDesignResult,
      measured: { ...bracketDesignResult.measured, mass_g: 96.04 },
    };
    dispatch({ type: 'design/received', design: withMass });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    expect(rowText(container, 'mass')).toContain('96.04 g');
  });

  it('prints the mass when the backend declared a density', () => {
    const withDensity = {
      ...bracketDesignResult,
      declared_density_g_cm3: 2.7,
    } as DesignResult;
    dispatch({ type: 'design/received', design: withDensity });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    // 35571.416 mm^3 = 35.571 cm^3; 35.571 cm^3 x 2.7 g/cm^3 = 96.04 g.
    expect(rowText(container, 'mass')).toContain('96.04 g');
  });
});

describe('the checks', () => {
  it('marks every check and prints the expected value beside the measured one', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    const brep = container.querySelector<HTMLElement>('[data-check="brep_valid"]')!;
    expect(brep.getAttribute('data-passed')).toBe('yes');
    expect(brep.getAttribute('data-status')).toBe('passed');
    expect(brep.textContent).toContain('✓');
    expect(brep.textContent).toContain('expected true');
    expect(brep.textContent).toContain('measured true');

    const envelope = container.querySelector<HTMLElement>('[data-check="envelope_x"]')!;
    expect(envelope.textContent).toContain('expected [-30, 30]');
    expect(envelope.textContent).toContain('measured [-30, 30] mm');
  });

  it('renders a solid_count check generically, without a special case', () => {
    const assembly: DesignResult = {
      ...bracketDesignResult,
      checks: [
        {
          id: 'solid_count',
          label: 'Solid count',
          expected: 2,
          measured: 2,
          tolerance: 0,
          unit: 'solid',
          passed: true,
          status: 'passed',
        },
      ],
      verified: true,
    };
    dispatch({ type: 'design/received', design: assembly });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    const solid = container.querySelector<HTMLElement>('[data-check="solid_count"]')!;
    expect(solid.getAttribute('data-status')).toBe('passed');
    expect(solid.textContent).toContain('Solid count');
    expect(solid.textContent).toContain('expected 2');
    expect(solid.textContent).toContain('measured 2 solid');
    expect(solid.textContent).toContain('✓');
  });

  it('marks a failed check with a cross', () => {
    const failed: DesignResult = {
      ...bracketDesignResult,
      verified: false,
      checks: [
        {
          id: 'brep_valid',
          label: 'B-Rep valid',
          expected: true,
          measured: false,
          tolerance: 0,
          unit: 'bool',
          passed: false,
          status: 'failed',
        },
      ],
    };
    dispatch({ type: 'design/received', design: failed });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    const brep = container.querySelector<HTMLElement>('[data-check="brep_valid"]')!;
    expect(brep.getAttribute('data-passed')).toBe('no');
    expect(brep.getAttribute('data-status')).toBe('failed');
    expect(brep.textContent).toContain('✗');
  });

  it('shows a not-reported check as an em dash, neither pass nor fail', () => {
    dispatch({ type: 'design/received', design: notReportedDesignResult });
    const { container } = render(
      <DesignAgentCard designId={notReportedDesignResult.design_id} />,
    );

    const brep = container.querySelector<HTMLElement>('[data-check="brep_valid"]')!;
    expect(brep.getAttribute('data-status')).toBe('not_reported');
    expect(brep.getAttribute('data-passed')).toBe('no');
    expect(brep.textContent).toContain('—');
    expect(brep.textContent).toContain('measured —');
    expect(brep.textContent).not.toContain('✗');
    expect(brep.textContent).not.toContain('✓');
    expect(brep.querySelector('[role="img"]')?.getAttribute('aria-label')).toBe('not reported');
  });
});

describe('the verification stamp', () => {
  it('stamps B-Rep Verified when the design is verified', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    expect(stamp(container)?.textContent).toBe('B-Rep Verified');
  });

  it('never stamps B-Rep Verified when the design is not verified', () => {
    const unverified: DesignResult = { ...bracketDesignResult, verified: false };
    dispatch({ type: 'design/received', design: unverified });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    expect(container.textContent).not.toContain('B-Rep Verified');
    // The honest count replaces the stamp.
    expect(stamp(container)?.textContent).toContain('checks pass');
  });

  it('does not present a not-reported design as verified or failed', () => {
    dispatch({ type: 'design/received', design: notReportedDesignResult });
    const { container } = render(
      <DesignAgentCard designId={notReportedDesignResult.design_id} />,
    );

    expect(container.textContent).not.toContain('B-Rep Verified');
    expect(stamp(container)?.textContent).toContain('not reported');
    expect(stamp(container)?.getAttribute('data-stamp')).toBe('unverified');
  });
});

describe('the digest chips', () => {
  it('shows the script, STEP, and native artefact digests', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    const script = container.querySelector<HTMLElement>('[data-digest-kind="script"]')!;
    expect(script.textContent).toContain('88a7ec13');
    const step = container.querySelector<HTMLElement>('[data-digest-kind="step"]')!;
    expect(step.textContent).toContain('24a5dca7');
    const native = container.querySelector<HTMLElement>('[data-digest-kind="native"]')!;
    expect(native.textContent).toContain('FCStd');
    expect(native.textContent).toContain('e0f9fb92');
  });

  it('shows no STEP or native chip when the provider delivered no such artefact', () => {
    const noNative: DesignResult = {
      ...bracketDesignResult,
      artifacts: {
        script_sha256: bracketDesignResult.artifacts.script_sha256,
        step_sha256: null,
        native_sha256: null,
        native_kind: null,
      },
    };
    dispatch({ type: 'design/received', design: noNative });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    expect(container.querySelector('[data-digest-kind="script"]')).toBeTruthy();
    expect(container.querySelector('[data-digest-kind="step"]')).toBeNull();
    expect(container.querySelector('[data-digest-kind="native"]')).toBeNull();
  });

  it('copies the full digest, not the shortened one', async () => {
    const writeText = vi.fn(async (_text: string) => {});
    Object.defineProperty(window.navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    });

    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);

    const copy = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Copy script sha256"]',
    )!;
    fireEvent.click(copy);

    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(writeText.mock.calls[0]![0]).toBe(bracketDesignResult.artifacts.script_sha256);
    await waitFor(() =>
      expect(copy.getAttribute('data-copied')).toBe('yes'),
    );
  });
});

describe('the section pill', () => {
  it('activates the design and switches the auxiliary view to Section', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    // A second design takes the active slot; the pill must bring back the
    // design the operator is reading.
    dispatch({ type: 'design/received', design: otherDesign() });
    expect(getState().designs.activeDesignId).toBe(OTHER_DESIGN_ID);

    const { container } = render(<DesignAgentCard designId={BRACKET_DESIGN_ID} />);
    const pill = container.querySelector<HTMLButtonElement>('.chat-panel-design-section-pill')!;
    expect(pill.textContent).toContain('Section A–A');

    fireEvent.click(pill);

    expect(getState().designs.activeDesignId).toBe(BRACKET_DESIGN_ID);
    expect(getState().auxView).toBe('section');
  });
});

describe('the chat turn', () => {
  it('renders the card of the design the turn produced', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    dispatch({
      type: 'chat/response',
      response: {
        text: 'L-bracket 60 x 45 x 40: B-Rep verified.',
        provider: 'local',
        model: null,
        offline: true,
        durationMs: 12,
        designId: BRACKET_DESIGN_ID,
      },
    });

    const { container } = render(<ChatPanel />);
    expect(container.querySelector('.chat-panel-design-card')).toBeTruthy();
    expect(container.textContent).toContain('35571.42 mm³');
    expect(container.textContent).toContain('B-Rep Verified');
  });

  it('renders no card for a turn that produced no design', () => {
    dispatch({
      type: 'chat/response',
      response: {
        text: 'I need the width, depth, height, and wall thickness.',
        provider: 'local',
        model: null,
        offline: true,
        durationMs: 12,
      },
    });

    const { container } = render(<ChatPanel />);
    expect(container.querySelector('.chat-panel-design-card')).toBeNull();
  });

  it('quotes the active design in the geometry context, not the primitive', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<ChatPanel />);

    const context = container.querySelector<HTMLElement>('.chat-panel-context')!;
    expect(context.textContent).toContain('design');
    expect(context.textContent).toContain('60.00 × 45.00 × 40.00 mm');
    expect(context.textContent).toContain('35571.42 mm³');
    expect(context.textContent).toContain('measured by FreeCAD 1.1.4');
    // The primitive's closed-form estimate is not the active geometry any
    // more, so it is not quoted beside the measured one.
    expect(context.textContent).not.toContain('40.00 mm × 20.00 mm × 30.00 mm');
  });
});

describe('a backend without design support', () => {
  function seedServer(designAvailable: boolean | null): void {
    dispatch({
      type: 'server/providers',
      providers: [{ id: 'local', name: 'Local helper', configured: true, notes: '' }],
      health: 'ok',
      healthMessage: 'ok',
      version: '0.1.0',
      stepAvailable: false,
    });
    dispatch({ type: 'server/capabilities', designAvailable });
  }

  it('says plainly that design turns need the geometry backend', () => {
    seedServer(false);
    const { container } = render(<ChatPanel />);

    const note = container.querySelector<HTMLElement>('.chat-panel-agent-card-note')!;
    expect(note.textContent).toBe(
      'Design turns need the Python geometry backend; this backend reports design_available: false.',
    );
  });

  it('says the backend did not report the flag when it is absent', () => {
    seedServer(null);
    const { container } = render(<ChatPanel />);

    const note = container.querySelector<HTMLElement>('.chat-panel-agent-card-note')!;
    expect(note.textContent).toBe(
      'Design turns need the Python geometry backend; this backend did not report design_available.',
    );
  });

  it('shows no note when the backend does support design turns', () => {
    seedServer(true);
    const { container } = render(<ChatPanel />);

    expect(container.querySelector('.chat-panel-agent-card-note')).toBeNull();
  });

  it('keeps every primitive feature working', () => {
    seedServer(false);
    const { container } = render(<ChatPanel />);

    // The composer, the starters, and the active-geometry context stay.
    expect(container.querySelector('textarea.chat-panel-prompt')).toBeTruthy();
    expect(container.querySelectorAll('.chat-panel-starter').length).toBeGreaterThan(0);
    expect(container.querySelector('.chat-panel-context-table')).toBeTruthy();
  });
});
