// Tests for the design-turn client and the runtime shape validation.
//
// The contract is the backend cell's recorded payloads: the criterion-2
// L-bracket, its HLR drawing, its criterion-4 section cut, the first
// journal page, and both verification verdicts. A response that does not
// match the contract is REFUSED — a missing number is never turned into a
// zero, because a zero on the drafting sheet is a fabricated measurement.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  exportDesign,
  getCadProviders,
  getDesignDrawing,
  getDesignSection,
  getJournal,
  getJournalVerify,
  postChat,
  postDesign,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
  StudioTimeoutError,
  StudioValidationError,
} from '../src/api/client';
import {
  parseCadProviders,
  parseChatResponse,
  parseDesignResult,
  parseDrawingResponse,
  parseJournalPage,
  parseJournalVerify,
  parseSectionResponse,
} from '../src/api/validators';
import { getState, resetStore } from '../src/state/store';
import type { DesignRequest, StudioConnection } from '../src/types/studio';
import {
  BRACKET_DESIGN_ID,
  BRACKET_NATIVE_SHA,
  BRACKET_SCRIPT_SHA,
  BRACKET_STEP_SHA,
  bracketDesignResult,
  bracketDrawing,
  bracketJournalBroken,
  bracketJournalPage,
  bracketJournalVerified,
  bracketSection,
  cadProviders,
  notReportedDesignResult,
} from './designFixtures';

const connection: StudioConnection = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'desktop',
};

const bracketIntent: DesignRequest = {
  provider: 'freecad',
  template: {
    template: 'l_bracket',
    width: 60,
    depth: 45,
    height: 40,
    wall: 8,
    bore_d: 10,
    hole_d: 5.5,
    hole_count: 4,
  },
  material: 'aluminium',
  tolerances: ['ISO 2768-m (declared)'],
  label: 'L-bracket 60 x 45 x 40',
};

interface CapturedCall {
  url: string;
  init: RequestInit;
}

let captured: CapturedCall[] = [];
let responder: (url: string, init: RequestInit) => Response | Promise<Response> = () =>
  new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } });

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

beforeEach(() => {
  captured = [];
  resetStore();
  vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input.toString();
    captured.push({ url, init: init ?? {} });
    return Promise.resolve(responder(url, init ?? {}));
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('design result validation', () => {
  it('accepts the recorded B2 design result', () => {
    const parsed = parseDesignResult(JSON.parse(JSON.stringify(bracketDesignResult)));
    expect(parsed.design_id).toBe(BRACKET_DESIGN_ID);
    expect(parsed.provider).toBe('freecad');
    expect(parsed.provider_version).toBe('1.1.4');
    expect(parsed.measured_by).toBe('FreeCAD');
    expect(parsed.measured.volume_mm3).toBeCloseTo(35571.416047, 6);
    expect(parsed.measured.centroid_mm).toBeNull();
    expect(parsed.view.bounds).toEqual({ min: [-30, 0, 0], max: [30, 45, 40] });
    expect(parsed.view.solids).toBe(1);
    expect(parsed.verified).toBe(true);
    expect(parsed.checks).toHaveLength(3);
    expect(parsed.checks.every((check) => check.status === 'passed')).toBe(true);
    expect(parsed.artifacts.step_sha256).toBe(BRACKET_STEP_SHA);
    expect(parsed.artifacts.native_sha256).toBe(BRACKET_NATIVE_SHA);
    expect(parsed.artifacts.native_kind).toBe('fcstd');
    expect(parsed.tolerances).toContain('ISO 2768-m (declared)');
    // The B2 design id folds in the provider; it is not the script digest.
    expect(parsed.artifacts.script_sha256).toBe(BRACKET_SCRIPT_SHA);
    expect(parsed.design_id).not.toBe(parsed.artifacts.script_sha256);
  });

  it('accepts nullable measurements and keeps them null, never zero', () => {
    const parsed = parseDesignResult(JSON.parse(JSON.stringify(notReportedDesignResult)));
    expect(parsed.measured.volume_mm3).toBeNull();
    expect(parsed.measured.surface_area_mm2).toBeNull();
    expect(parsed.measured.bounds).toBeNull();
    expect(parsed.measured.solid_count).toBeNull();
    expect(parsed.measured.mass_g).toBeNull();
    expect(parsed.verified).toBe(false);
    expect(parsed.checks[0]?.measured).toBeNull();
    expect(parsed.checks[0]?.status).toBe('not_reported');
  });

  it('rejects the removed B1 mesh field and requires view', () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    const view = broken['view'];
    delete broken['view'];
    broken['mesh'] = view;
    expect(() => parseDesignResult(broken)).toThrow(StudioValidationError);
    try {
      parseDesignResult(broken);
    } catch (cause) {
      expect((cause as StudioValidationError).path).toBe('design.view');
    }
  });

  it('requires the provider and measured_by of the B2 shape', () => {
    const noProvider = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    delete noProvider['provider'];
    expect(() => parseDesignResult(noProvider)).toThrow(/design.provider/);

    const noMeasurer = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    delete noMeasurer['measured_by'];
    expect(() => parseDesignResult(noMeasurer)).toThrow(/design.measured_by/);
  });

  it('refuses an unknown provider', () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    broken['provider'] = 'onshape';
    expect(() => parseDesignResult(broken)).toThrow(/expected one of opencascade, freecad, solidedge/);
  });

  it('refuses a missing measurement instead of coercing it to zero', () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    delete (broken['measured'] as Record<string, unknown>)['volume_mm3'];
    expect(() => parseDesignResult(broken)).toThrow(StudioValidationError);
    try {
      parseDesignResult(broken);
    } catch (cause) {
      expect(cause).toBeInstanceOf(StudioValidationError);
      expect((cause as StudioValidationError).path).toBe('design.measured.volume_mm3');
    }
  });

  it('refuses a non-finite measurement', () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    (broken['measured'] as Record<string, unknown>)['surface_area_mm2'] = '11889.1';
    expect(() => parseDesignResult(broken)).toThrow(StudioValidationError);
  });

  it('refuses a design id that is not a sha256 digest', () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    broken['design_id'] = 'not-a-digest';
    expect(() => parseDesignResult(broken)).toThrow(/64-character lowercase hex/);
  });

  it('refuses a view whose positions are not xyz triples', () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    (broken['view'] as Record<string, unknown>)['positions'] = [0, 0, 0, 1];
    expect(() => parseDesignResult(broken)).toThrow(/flat list of xyz triples/);
  });

  it('refuses a view without the imported bounds or solid count', () => {
    const noBounds = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    delete (noBounds['view'] as Record<string, unknown>)['bounds'];
    expect(() => parseDesignResult(noBounds)).toThrow(/design.view.bounds/);

    const noSolids = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    delete (noSolids['view'] as Record<string, unknown>)['solids'];
    expect(() => parseDesignResult(noSolids)).toThrow(/design.view.solids/);
  });

  it('refuses a check row whose expected value is not a number, boolean, or range', () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    (broken['checks'] as Array<Record<string, unknown>>)[0]!['expected'] = 'yes';
    expect(() => parseDesignResult(broken)).toThrow(StudioValidationError);
  });

  it('refuses a check row whose tolerance is missing', () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    delete (broken['checks'] as Array<Record<string, unknown>>)[0]!['tolerance'];
    expect(() => parseDesignResult(broken)).toThrow(StudioValidationError);
  });

  it('refuses a check row that contradicts its status', () => {
    const notReportedWithValue = JSON.parse(
      JSON.stringify(notReportedDesignResult),
    ) as Record<string, unknown>;
    (notReportedWithValue['checks'] as Array<Record<string, unknown>>)[0]!['measured'] = true;
    expect(() => parseDesignResult(notReportedWithValue)).toThrow(/not_reported/);

    const passedWithoutValue = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    (passedWithoutValue['checks'] as Array<Record<string, unknown>>)[0]!['measured'] = null;
    expect(() => parseDesignResult(passedWithoutValue)).toThrow(/must carry a measured value/);

    const passedFalse = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    (passedFalse['checks'] as Array<Record<string, unknown>>)[0]!['passed'] = false;
    expect(() => parseDesignResult(passedFalse)).toThrow(/must report passed/);
  });

  it('refuses a check row without a status', () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    delete (broken['checks'] as Array<Record<string, unknown>>)[0]!['status'];
    expect(() => parseDesignResult(broken)).toThrow(/checks\[0\]\.status/);
  });

  it('refuses a malformed native artefact digest', () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    (broken['artifacts'] as Record<string, unknown>)['native_sha256'] = 'not-a-digest';
    expect(() => parseDesignResult(broken)).toThrow(/native_sha256/);
  });
});

describe('CAD provider validation', () => {
  it('accepts the recorded probe response', () => {
    const parsed = parseCadProviders(JSON.parse(JSON.stringify(cadProviders)));
    expect(parsed.providers.map((record) => record.id)).toEqual([
      'opencascade',
      'freecad',
      'solidedge',
    ]);
    expect(parsed.providers[2]?.available).toBe(false);
    expect(parsed.providers[2]?.reason).toContain('Windows bridge');
  });

  it('refuses an unknown provider id', () => {
    const broken = JSON.parse(JSON.stringify(cadProviders)) as Record<string, unknown>;
    (broken['providers'] as Array<Record<string, unknown>>)[0]!['id'] = 'onshape';
    expect(() => parseCadProviders(broken)).toThrow(/expected one of opencascade, freecad, solidedge/);
  });

  it('refuses an unavailable row that hides its reason as undefined', () => {
    const broken = JSON.parse(JSON.stringify(cadProviders)) as Record<string, unknown>;
    delete (broken['providers'] as Array<Record<string, unknown>>)[2]!['reason'];
    expect(() => parseCadProviders(broken)).toThrow(/providers\[2\]\.reason/);
  });
});

describe('drawing and section validation', () => {
  it('accepts the recorded front view', () => {
    const parsed = parseDrawingResponse(JSON.parse(JSON.stringify(bracketDrawing)));
    expect(parsed.views[0]?.name).toBe('front');
    expect(parsed.views[0]?.visible).toHaveLength(2);
    expect(parsed.chord_tolerance_mm).toBe(0.05);
  });

  it('refuses an unknown view name', () => {
    const broken = JSON.parse(JSON.stringify(bracketDrawing)) as Record<string, unknown>;
    (broken['views'] as Array<Record<string, unknown>>)[0]!['name'] = 'bottom';
    expect(() => parseDrawingResponse(broken)).toThrow(/expected one of front, top, right, iso/);
  });

  it('refuses a polyline point that is not a pair of numbers', () => {
    const broken = JSON.parse(JSON.stringify(bracketDrawing)) as Record<string, unknown>;
    const view = (broken['views'] as Array<Record<string, unknown>>)[0]!;
    (view['visible'] as Array<Record<string, unknown>>)[0]!['points'] = [[0, 0, 0]];
    expect(() => parseDrawingResponse(broken)).toThrow(StudioValidationError);
  });

  it('accepts the recorded criterion-4 section cut', () => {
    const parsed = parseSectionResponse(JSON.parse(JSON.stringify(bracketSection)));
    expect(parsed.plane).toBe('XZ');
    expect(parsed.offset_mm).toBe(4);
    expect(parsed.outer_loop_count).toBe(1);
    expect(parsed.inner_loop_count).toBe(1);
    expect(parsed.area_mm2).toBeCloseTo(2321.4601836602546, 9);
    expect(parsed.area_exact).toBe(true);
  });

  it('refuses a section with an unknown plane', () => {
    const broken = JSON.parse(JSON.stringify(bracketSection)) as Record<string, unknown>;
    broken['plane'] = 'AB';
    expect(() => parseSectionResponse(broken)).toThrow(/expected one of XY, XZ, YZ/);
  });

  it('refuses a section whose measured area is missing', () => {
    const broken = JSON.parse(JSON.stringify(bracketSection)) as Record<string, unknown>;
    delete broken['area_mm2'];
    expect(() => parseSectionResponse(broken)).toThrow(StudioValidationError);
  });
});

describe('journal validation', () => {
  it('accepts the recorded first page', () => {
    const parsed = parseJournalPage(JSON.parse(JSON.stringify(bracketJournalPage)));
    expect(parsed.events).toHaveLength(2);
    expect(parsed.events[0]?.event_type).toBe('design.intent');
    expect(parsed.next_after).toBe(2);
    expect(parsed.has_more).toBe(true);
  });

  it('refuses an event with an unparseable timestamp', () => {
    const broken = JSON.parse(JSON.stringify(bracketJournalPage)) as Record<string, unknown>;
    (broken['events'] as Array<Record<string, unknown>>)[0]!['occurred_at'] = 'yesterday';
    expect(() => parseJournalPage(broken)).toThrow(/ISO-8601/);
  });

  it('refuses an event without a digest', () => {
    const broken = JSON.parse(JSON.stringify(bracketJournalPage)) as Record<string, unknown>;
    delete (broken['events'] as Array<Record<string, unknown>>)[0]!['digest'];
    expect(() => parseJournalPage(broken)).toThrow(StudioValidationError);
  });

  it('accepts both recorded verification verdicts', () => {
    expect(parseJournalVerify(bracketJournalVerified).ok).toBe(true);
    expect(parseJournalVerify(bracketJournalVerified).first_bad_sequence).toBeNull();
    const broken = parseJournalVerify(bracketJournalBroken);
    expect(broken.ok).toBe(false);
    expect(broken.first_bad_sequence).toBe(4);
  });

  it('refuses a verification without the ok flag', () => {
    const broken = { ...bracketJournalVerified } as Record<string, unknown>;
    delete broken['ok'];
    expect(() => parseJournalVerify(broken)).toThrow(StudioValidationError);
  });
});

describe('chat response validation', () => {
  it('accepts a reply without a design', () => {
    const parsed = parseChatResponse({ text: 'hi', provider: 'local', model: null, offline: true });
    expect(parsed.design).toBeNull();
  });

  it('accepts a reply that carries a design', () => {
    const parsed = parseChatResponse({
      text: 'compiled',
      provider: 'local',
      model: 'deterministic-paraphrase-v1',
      offline: true,
      design: bracketDesignResult,
    });
    expect(parsed.design?.design_id).toBe(BRACKET_DESIGN_ID);
  });

  it('refuses a reply whose design does not match the contract', () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    delete (broken['measured'] as Record<string, unknown>)['volume_mm3'];
    expect(() =>
      parseChatResponse({
        text: 'compiled',
        provider: 'local',
        model: null,
        offline: true,
        design: broken,
      }),
    ).toThrow(StudioValidationError);
  });

  it('refuses an unknown provider', () => {
    expect(() =>
      parseChatResponse({ text: 'hi', provider: 'skynet', model: null, offline: true }),
    ).toThrow(/expected one of local, openai, openrouter, codex/);
  });
});

describe('design client methods', () => {
  it('posts a design request with its required provider to /api/studio/design', async () => {
    responder = () => jsonResponse(bracketDesignResult);
    const design = await postDesign(connection, bracketIntent);
    expect(design.verified).toBe(true);
    const call = captured[0]!;
    expect(call.url).toBe('http://127.0.0.1:8765/api/studio/design');
    expect(call.init.method).toBe('POST');
    const headers = call.init.headers as Record<string, string>;
    expect(headers['Authorization']).toBe(`Bearer ${connection.token}`);
    expect(JSON.parse(call.init.body as string)).toEqual(bracketIntent);
    expect(JSON.parse(call.init.body as string).provider).toBe('freecad');
  });

  it('refuses a design request with no provider before it reaches the network', async () => {
    await expect(
      postDesign(connection, { ...bracketIntent, provider: undefined } as unknown as DesignRequest),
    ).rejects.toBeInstanceOf(StudioValidationError);
    expect(captured).toHaveLength(0);
  });

  it('refuses a design request with an unknown provider before it reaches the network', async () => {
    await expect(
      postDesign(connection, { ...bracketIntent, provider: 'onshape' } as unknown as DesignRequest),
    ).rejects.toBeInstanceOf(StudioValidationError);
    expect(captured).toHaveLength(0);
  });

  it('reads the CAD provider probe with the bearer and validates it', async () => {
    responder = () => jsonResponse(cadProviders);
    const probe = await getCadProviders(connection);
    const call = captured[0]!;
    expect(call.url).toBe('http://127.0.0.1:8765/api/studio/providers/cad');
    expect(call.init.method).toBe('GET');
    const headers = call.init.headers as Record<string, string>;
    expect(headers['Authorization']).toBe(`Bearer ${connection.token}`);
    expect(probe.providers).toHaveLength(3);
    expect(probe.providers[1]?.version).toBe('1.1.4');
  });

  it('refuses a malformed CAD provider payload', async () => {
    responder = () =>
      jsonResponse({
        providers: [
          {
            id: 'freecad',
            name: 'FreeCAD',
            available: 'yes',
            version: '1.1.4',
            reason: null,
            measured_by: 'FreeCAD',
            mode: 'external',
          },
        ],
      });
    await expect(getCadProviders(connection)).rejects.toBeInstanceOf(StudioValidationError);
  });

  it('records a compiled design in the store and activates it', async () => {
    responder = () => jsonResponse(bracketDesignResult);
    await postDesign(connection, bracketIntent);
    expect(getState().designs.activeDesignId).toBe(BRACKET_DESIGN_ID);
    expect(getState().designs.designs.size).toBe(1);
  });

  it('leaves the store alone when the design response is refused', async () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    delete (broken['measured'] as Record<string, unknown>)['volume_mm3'];
    responder = () => jsonResponse(broken);
    await expect(postDesign(connection, bracketIntent)).rejects.toBeInstanceOf(StudioValidationError);
    expect(getState().designs.designs.size).toBe(0);
  });

  it('refuses a design response that does not match the contract', async () => {
    const broken = JSON.parse(JSON.stringify(bracketDesignResult)) as Record<string, unknown>;
    delete (broken['measured'] as Record<string, unknown>)['volume_mm3'];
    responder = () => jsonResponse(broken);
    await expect(postDesign(connection, bracketIntent)).rejects.toBeInstanceOf(StudioValidationError);
  });

  it('reads a drawing with the bearer and the views query', async () => {
    responder = () => jsonResponse(bracketDrawing);
    const drawing = await getDesignDrawing(connection, BRACKET_DESIGN_ID, ['front', 'iso']);
    const call = captured[0]!;
    expect(call.url).toBe(
      `http://127.0.0.1:8765/api/studio/design/${BRACKET_DESIGN_ID}/drawing?views=front%2Ciso`,
    );
    expect(call.init.method).toBe('GET');
    const headers = call.init.headers as Record<string, string>;
    expect(headers['Authorization']).toBe(`Bearer ${connection.token}`);
    expect(drawing.views[0]?.name).toBe('front');
  });

  it('asks for every view when no view is named', async () => {
    responder = () => jsonResponse(bracketDrawing);
    await getDesignDrawing(connection, BRACKET_DESIGN_ID);
    expect(captured[0]!.url).toBe(
      `http://127.0.0.1:8765/api/studio/design/${BRACKET_DESIGN_ID}/drawing`,
    );
  });

  it('refuses a design id that is not a digest, before it can become a path segment', async () => {
    await expect(getDesignDrawing(connection, '../../secret')).rejects.toBeInstanceOf(
      StudioValidationError,
    );
    expect(captured).toHaveLength(0);
  });

  it('reads a section with the plane and offset', async () => {
    responder = () => jsonResponse(bracketSection);
    const section = await getDesignSection(connection, BRACKET_DESIGN_ID, 'XZ', 4);
    expect(captured[0]!.url).toBe(
      `http://127.0.0.1:8765/api/studio/design/${BRACKET_DESIGN_ID}/section?plane=XZ&offset=4`,
    );
    expect(section.outer_loop_count).toBe(1);
  });

  it('refuses a section offset outside the backend bound', async () => {
    await expect(getDesignSection(connection, BRACKET_DESIGN_ID, 'XZ', 20_000)).rejects.toBeInstanceOf(
      StudioValidationError,
    );
    expect(captured).toHaveLength(0);
  });

  it('refuses a section plane that is not allow-listed', async () => {
    await expect(
      getDesignSection(connection, BRACKET_DESIGN_ID, 'AB' as unknown as 'XZ'),
    ).rejects.toBeInstanceOf(StudioValidationError);
    expect(captured).toHaveLength(0);
  });

  it('reads a bounded journal page with the after and limit', async () => {
    responder = () => jsonResponse(bracketJournalPage);
    const page = await getJournal(connection, 0, 2);
    expect(captured[0]!.url).toBe('http://127.0.0.1:8765/api/studio/journal?after=0&limit=2');
    expect(page.events).toHaveLength(2);
  });

  it('clamps an over-large journal limit to the backend bound', async () => {
    responder = () => jsonResponse(bracketJournalPage);
    await getJournal(connection, 3, 10_000);
    expect(captured[0]!.url).toContain('limit=500');
  });

  it('reads the chain verification', async () => {
    responder = () => jsonResponse(bracketJournalVerified);
    const verify = await getJournalVerify(connection);
    expect(captured[0]!.url).toBe('http://127.0.0.1:8765/api/studio/journal/verify');
    expect(verify.ok).toBe(true);
  });

  it('exports a design and reports the digest the backend sent', async () => {
    responder = () =>
      new Response(new Uint8Array([1, 2, 3, 4]), {
        status: 200,
        headers: {
          'Content-Type': 'application/step',
          'content-disposition': `attachment; filename="kala-studio-${BRACKET_DESIGN_ID.slice(0, 8)}.step"`,
          'x-studio-design-id': BRACKET_DESIGN_ID,
          'x-studio-digest': 'a'.repeat(64),
          'x-studio-artifact-kind': 'step',
        },
      });
    const artifact = await exportDesign(connection, BRACKET_DESIGN_ID, 'step');
    expect(captured[0]!.url).toBe('http://127.0.0.1:8765/api/studio/export');
    expect(JSON.parse(captured[0]!.init.body as string)).toEqual({
      design_id: BRACKET_DESIGN_ID,
      format: 'step',
    });
    expect(artifact.bytes).toBe(4);
    expect(artifact.digest).toBe('a'.repeat(64));
    expect(artifact.designId).toBe(BRACKET_DESIGN_ID);
    expect(artifact.filename).toBe(`kala-studio-${BRACKET_DESIGN_ID.slice(0, 8)}.step`);
  });

  it('surfaces a server refusal with its code', async () => {
    responder = () =>
      jsonResponse({ code: 'design_rejected', message: 'helix is not supported', details: [] }, 422);
    await expect(postDesign(connection, bracketIntent)).rejects.toBeInstanceOf(StudioApiError);
  });

  it('surfaces a missing bearer as an auth error', async () => {
    responder = () => jsonResponse({ code: 'unauthorized', message: 'token bad', details: [] }, 401);
    await expect(getJournalVerify(connection)).rejects.toBeInstanceOf(StudioAuthError);
  });

  it('surfaces a network failure as a network error', async () => {
    vi.unstubAllGlobals();
    vi.stubGlobal('fetch', () => Promise.reject(new TypeError('failed to fetch')));
    await expect(getJournal(connection)).rejects.toBeInstanceOf(StudioNetworkError);
  });

  it('gives up on a request that outruns its budget', async () => {
    vi.useFakeTimers();
    vi.unstubAllGlobals();
    vi.stubGlobal('fetch', (_input: RequestInfo | URL, init?: RequestInit) => {
      return new Promise((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => {
          reject(new DOMException('aborted', 'AbortError'));
        });
      });
    });
    const pending = getJournalVerify(connection);
    const assertion = expect(pending).rejects.toBeInstanceOf(StudioTimeoutError);
    await vi.advanceTimersByTimeAsync(11_000);
    await assertion;
  });
});

describe('the chat turn hands its design to the store', () => {
  it('sends the selected CAD target as cad_provider', async () => {
    responder = () =>
      jsonResponse({
        text: 'Choose a CAD provider before I build this design.',
        provider: 'local',
        model: null,
        offline: true,
        design: null,
      });
    await postChat(connection, {
      provider: 'local',
      model: null,
      prompt: 'make me a bracket',
      allow_remote: false,
      cad_provider: 'freecad',
    });
    const body = JSON.parse(captured[0]!.init.body as string) as Record<string, unknown>;
    expect(body['cad_provider']).toBe('freecad');
  });

  it('sends cad_provider: null when no target was chosen', async () => {
    responder = () =>
      jsonResponse({
        text: 'Choose a CAD provider before I build this design.',
        provider: 'local',
        model: null,
        offline: true,
        design: null,
      });
    await postChat(connection, {
      provider: 'local',
      model: null,
      prompt: 'make me a bracket',
      allow_remote: false,
      cad_provider: null,
    });
    const body = JSON.parse(captured[0]!.init.body as string) as Record<string, unknown>;
    expect(body['cad_provider']).toBeNull();
  });

  it('records the design of a chat reply and activates it', async () => {
    responder = () =>
      jsonResponse({
        text: 'L-bracket compiled and measured.',
        provider: 'local',
        model: 'deterministic-paraphrase-v1',
        offline: true,
        design: bracketDesignResult,
      });
    const response = await postChat(connection, {
      provider: 'local',
      model: null,
      prompt: 'make me a bracket',
      allow_remote: false,
    });
    expect(response.design?.design_id).toBe(BRACKET_DESIGN_ID);
    const state = getState();
    expect(state.designs.activeDesignId).toBe(BRACKET_DESIGN_ID);
    expect(state.designs.designs.size).toBe(1);
    expect(state.chat.thread).toHaveLength(0);
  });

  it('leaves the store alone when the reply carries no design', async () => {
    responder = () =>
      jsonResponse({
        text: 'I need the width, depth, height, and wall thickness.',
        provider: 'local',
        model: null,
        offline: true,
        design: null,
      });
    await postChat(connection, {
      provider: 'local',
      model: null,
      prompt: 'make me a bracket',
      allow_remote: false,
    });
    expect(getState().designs.designs.size).toBe(0);
    expect(getState().designs.activeDesignId).toBeNull();
  });
});
