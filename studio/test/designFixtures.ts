// The payload samples the design-turn contract is checked against.
//
// The `DesignResult` sample is the B2 FreeCAD run recorded by the provider
// cell: the same criterion-2 L-bracket (60 x 45 x 40 mm, wall 8 mm, Ø10
// bore, 4 x Ø5.5 holes), built and measured by FreeCAD 1.1.4, volume
// 35 571.416 047 mm^3, with the display view imported from the provider's
// exact STEP bytes. The view arrays are shortened (a real view is hundreds
// of vertices) but every measured number is the recorded one.
//
// `BRACKET_SCRIPT_SHA` and `BRACKET_DESIGN_ID` are deliberately different:
// B2 keys the script by its own digest and folds the provider into the
// design id.

import type {
  CadProvidersResponse,
  DesignResult,
  DrawingResponse,
  JournalPage,
  JournalVerify,
  SectionResponse,
} from '../src/types/studio';

export const BRACKET_DESIGN_ID =
  'a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7';
export const BRACKET_SCRIPT_SHA =
  '88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae';
export const BRACKET_STEP_SHA =
  '24a5dca7bf55b1bcc2d4f33e796a739c210b2f85a89c964a7d59393f2bd58403';
export const BRACKET_NATIVE_SHA =
  'e0f9fb920cebad9eb39e79fb2124772f0ec99f93503aab93e158c9a4e44c710e';
export const BRACKET_JOURNAL_HEAD =
  '6924d115383725e16ff6f30ceea7c9496c1d444b9fd0e9d855aa07d65139214a';

/** The recorded criterion-2 design result, built by FreeCAD. */
export const bracketDesignResult: DesignResult = {
  design_id: BRACKET_DESIGN_ID,
  label: 'L-bracket 60 x 45 x 40',
  provider: 'freecad',
  provider_version: '1.1.4',
  measured_by: 'FreeCAD',
  script: {
    summary: 'L-bracket 60 x 45 x 40 mm',
    components: [
      {
        name: 'base',
        steps: [
          {
            op: 'sketch',
            plane: 'XY',
            profile: 'rectangle',
            width: 60.0,
            height: 45.0,
            x: -30.0,
            y: 0.0,
            reversed: false,
          },
          { op: 'extrude', plane: 'XY', length: 8.0, x: 0.0, y: 0.0, reversed: false },
        ],
      },
      {
        name: 'upright',
        steps: [
          {
            op: 'sketch',
            plane: 'XZ',
            profile: 'rectangle',
            width: 60.0,
            height: 40.0,
            x: -30.0,
            y: 0.0,
            reversed: false,
          },
          { op: 'extrude', plane: 'XY', length: 8.0, x: 0.0, y: 0.0, reversed: false },
          { op: 'sketch', plane: 'XZ', profile: 'circle', diameter: 10.0, x: 0.0, y: 24.0 },
          { op: 'pocket', plane: 'XY', x: 0.0, y: 0.0, reversed: false },
        ],
      },
    ],
    assembly: [
      { op: 'place', component: 'upright', components: [], x: 0.0, y: 8.0, z: 0.0 },
      { op: 'fuse', component: null, components: ['base', 'upright'], x: 0.0, y: 0.0, z: 0.0 },
    ],
  },
  measured: {
    volume_mm3: 35571.416047113315,
    surface_area_mm2: 11889.101731097317,
    // FreeCAD 1.1.4 reports no centroid and no mass.
    centroid_mm: null,
    bounds: { min: [-30.0, 0.0, 0.0], max: [30.0, 45.0, 40.0] },
    size_mm: [60.0, 45.0, 40.0],
    valid: true,
    solid_count: 1,
    mass_g: null,
  },
  checks: [
    {
      id: 'brep_valid',
      label: 'B-Rep valid',
      expected: true,
      measured: true,
      tolerance: 0.0,
      unit: 'bool',
      passed: true,
      status: 'passed',
    },
    {
      id: 'single_solid',
      label: 'Single solid',
      expected: 1.0,
      measured: 1.0,
      tolerance: 0.0,
      unit: 'solid',
      passed: true,
      status: 'passed',
    },
    {
      id: 'envelope_x',
      label: 'Envelope X',
      expected: [-30.0, 30.0],
      measured: [-30.0, 30.0],
      tolerance: 0.01,
      unit: 'mm',
      passed: true,
      status: 'passed',
    },
  ],
  verified: true,
  view: {
    positions: [-30.0, 0.0, 0.0, -30.0, 0.0, 8.0, 30.0, 0.0, 0.0],
    normals: [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
    indices: [0, 1, 2],
    vertex_count: 3,
    triangle_count: 1,
    deflection_mm: 0.1,
    engine: 'ocp',
    bounds: { min: [-30.0, 0.0, 0.0], max: [30.0, 45.0, 40.0] },
    solids: 1,
    warnings: [],
  },
  artifacts: {
    script_sha256: BRACKET_SCRIPT_SHA,
    step_sha256: BRACKET_STEP_SHA,
    native_sha256: BRACKET_NATIVE_SHA,
    native_kind: 'fcstd',
  },
  journal_head: BRACKET_JOURNAL_HEAD,
  material: 'aluminium',
  tolerances: [
    'bore diameter 10 mm (declared)',
    '4 x hole diameter 5.5 mm (declared)',
    'ISO 2768-m (declared)',
  ],
  declared_envelope: { min: [-30.0, 0.0, 0.0], max: [30.0, 45.0, 40.0] },
  cached: false,
  warnings: [],
  component_count: 2,
  fused: true,
};

/**
 * A synthetic not-reported design: the provider returned a STEP (so the
 * view exists) but no measured numbers. Every measured field is null, every
 * check is `not_reported`, and no value is substituted. This is the state
 * ADR-0011 requires the UI to render as "—".
 */
export const notReportedDesignResult: DesignResult = {
  ...bracketDesignResult,
  design_id: 'c'.repeat(64),
  label: 'Not-reported probe part',
  measured: {
    volume_mm3: null,
    surface_area_mm2: null,
    centroid_mm: null,
    bounds: null,
    size_mm: null,
    valid: null,
    solid_count: null,
    mass_g: null,
  },
  checks: [
    {
      id: 'brep_valid',
      label: 'B-Rep valid',
      expected: true,
      measured: null,
      tolerance: 0.0,
      unit: 'bool',
      passed: false,
      status: 'not_reported',
    },
    {
      id: 'single_solid',
      label: 'Single solid',
      expected: 1.0,
      measured: null,
      tolerance: 0.0,
      unit: 'solid',
      passed: false,
      status: 'not_reported',
    },
  ],
  verified: false,
  artifacts: {
    script_sha256: BRACKET_SCRIPT_SHA,
    step_sha256: BRACKET_STEP_SHA,
    native_sha256: null,
    native_kind: null,
  },
  journal_head: null,
  material: null,
  tolerances: [],
  declared_envelope: null,
  cached: false,
  warnings: [],
  component_count: 0,
  fused: false,
};

/** The live probe rows `GET /api/studio/providers/cad` returns on this host. */
export const cadProviders: CadProvidersResponse = {
  providers: [
    {
      id: 'opencascade',
      name: 'OpenCascade (opt-in)',
      available: true,
      version: '8.0.1.1',
      reason: null,
      measured_by: 'OpenCascade (opt-in)',
      mode: 'opt-in',
    },
    {
      id: 'freecad',
      name: 'FreeCAD',
      available: true,
      version: '1.1.4',
      reason: null,
      measured_by: 'FreeCAD',
      mode: 'external',
    },
    {
      id: 'solidedge',
      name: 'Solid Edge',
      available: false,
      version: null,
      reason:
        'Solid Edge build requires the Windows bridge (not reachable); this host is linux, so the ADR-0008 bridge cannot run here',
      measured_by: 'Solid Edge',
      mode: 'bridge',
    },
  ],
};

/** The recorded front view of the same design. */
export const bracketDrawing: DrawingResponse = {
  design_id: BRACKET_DESIGN_ID,
  views: [
    {
      name: 'front',
      visible: [
        { points: [[-30.0, 0.0], [-30.0, 8.0]], closed: false, point_count: 2 },
        { points: [[-30.0, 8.0], [-30.0, 40.0]], closed: false, point_count: 2 },
      ],
      hidden: [{ points: [[30.0, 0.0], [30.0, 8.0]], closed: false, point_count: 2 }],
      truncated: false,
      point_count: 210,
      polyline_count: 31,
    },
  ],
  chord_tolerance_mm: 0.05,
  max_points: 20000,
  point_count: 210,
};

/** The recorded criterion-4 section cut (XZ plane at y = 4 mm). */
export const bracketSection: SectionResponse = {
  design_id: BRACKET_DESIGN_ID,
  plane: 'XZ',
  offset_mm: 4.0,
  loops: [
    {
      points: [
        [-30.0, 0.0],
        [-30.0, 8.0],
        [-30.0, 40.0],
        [30.0, 40.0],
        [30.0, 8.0],
        [30.0, 0.0],
      ],
      area_mm2: 2399.9999999999995,
      area_exact: true,
      outer: true,
      bounds: {
        min_u: -30.0,
        min_v: 0.0,
        max_u: 30.0,
        max_v: 40.0,
        width: 60.0,
        height: 40.0,
        center_u: 0.0,
        center_v: 20.0,
      },
      centroid: [-0.0, 20.0],
      point_count: 6,
    },
    {
      points: [
        [0.0, 19.0],
        [-0.6266661678215213, 19.039426493427612],
        [0.6266661678214969, 19.03942649342761],
      ],
      area_mm2: 78.53981633974483,
      area_exact: true,
      outer: false,
      bounds: {
        min_u: -4.990133642141358,
        min_v: 19.0,
        max_u: 4.990133642141359,
        max_v: 29.0,
        width: 9.980267284282718,
        height: 10.0,
        center_u: 4.440892098500626e-16,
        center_v: 24.0,
      },
      centroid: [2.0976145333101254e-16, 23.999999999999986],
      point_count: 50,
    },
  ],
  area_mm2: 2321.4601836602546,
  area_exact: true,
  chord_tolerance_mm: 0.01,
  truncated: false,
  outer_loop_count: 1,
  inner_loop_count: 1,
  point_count: 56,
};

/** The recorded first page of the session journal. */
export const bracketJournalPage: JournalPage = {
  events: [
    {
      sequence: 1,
      event_type: 'design.intent',
      aggregate_id: BRACKET_DESIGN_ID,
      payload: {
        cache_hit: false,
        design_id: BRACKET_DESIGN_ID,
        envelope: { max: [30.0, 45.0, 40.0], min: [-30.0, 0.0, 0.0] },
        label: 'L-bracket 60 x 45 x 40',
        material: 'aluminium',
        script_sha256: BRACKET_SCRIPT_SHA,
        template: 'l_bracket',
        tolerances: [
          'bore diameter 10 mm (declared)',
          '4 x hole diameter 5.5 mm (declared)',
          'ISO 2768-m (declared)',
        ],
      },
      digest: '7e6134981f8033d8b3de85ff8cc8d2c10d12210c99ffb901fa22a741e2ec0dbb',
      occurred_at: '2026-10-19T16:46:57.617024Z',
      previous_event_digest: '0000000000000000000000000000000000000000000000000000000000000000',
      event_digest: 'c2594c21375c576dbdf45a4efb32b5a3ff7cbb5c19fef28742075b9c8953d07b',
    },
    {
      sequence: 2,
      event_type: 'artifact.stored',
      aggregate_id: BRACKET_DESIGN_ID,
      payload: {
        design_id: BRACKET_DESIGN_ID,
        kind: 'script',
        sha256: BRACKET_SCRIPT_SHA,
        size_bytes: 1457,
      },
      digest: '09d2fc67cfcbef8cad4a4a11e395bb18fd0aecca10a3981f1ac452060dd53dbd',
      occurred_at: '2026-10-19T16:46:57.654634Z',
      previous_event_digest: 'c2594c21375c576dbdf45a4efb32b5a3ff7cbb5c19fef28742075b9c8953d07b',
      event_digest: '01509e146a3763fb7b7704e2acde3d44e391ede304fdb387a6e3f2a50859a20f',
    },
  ],
  next_after: 2,
  has_more: true,
  limit: 2,
};

/**
 * The recorded B2 journal page of one FreeCAD L-bracket turn (ten events).
 * Captured from the real Python backend with the FreeCAD provider:
 * design.intent → provider.build.requested → provider.build.completed →
 * artifact.stored ×3 → view.imported → design.measured → design.checked →
 * chat.turn. This is the page shape the poller must merge, not a hand-made
 * two-event stub.
 */
export const bracketJournalB2Page: JournalPage = {
  events: [
    {
      sequence: 1,
      event_type: "design.intent",
      aggregate_id: "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
      payload: {
            "cache_hit": false,
            "design_id": "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
            "envelope": {
                  "max": [
                        30.0,
                        45.0,
                        40.0
                  ],
                  "min": [
                        -30.0,
                        0.0,
                        0.0
                  ]
            },
            "label": "L-bracket 60 x 45 x 40",
            "material": "aluminium",
            "provider": "freecad",
            "script_sha256": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
            "template": "l_bracket",
            "tolerances": [
                  "bore diameter 10 mm (declared)",
                  "4 x hole diameter 5.5 mm (declared)",
                  "ISO 2768-m (declared)"
            ]
      },
      digest: "cb065a4d53a27e172335be6c19a457d301299bd2ff2582204371c7cbb757036c",
      occurred_at: "2026-10-19T20:18:59.588042Z",
      previous_event_digest: "0000000000000000000000000000000000000000000000000000000000000000",
      event_digest: "d0558e1f0306d65d4cf946f7ebd51bf86a860628092afc08208d56cb16158d4d",
    },
    {
      sequence: 2,
      event_type: "provider.build.requested",
      aggregate_id: "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
      payload: {
            "design_id": "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
            "provider": "freecad",
            "provider_version": "1.1.4",
            "script_sha256": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae"
      },
      digest: "44b879b9345dbd89bbed41d8f9179e9ffdcb217eac9ffab05a01d3028d6dc89e",
      occurred_at: "2026-10-19T20:18:59.608083Z",
      previous_event_digest: "d0558e1f0306d65d4cf946f7ebd51bf86a860628092afc08208d56cb16158d4d",
      event_digest: "17fc01e9acb9b1ce558d9ffbb239bcaca6c39e6ee610af44089ed246c8c07761",
    },
    {
      sequence: 3,
      event_type: "provider.build.completed",
      aggregate_id: "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
      payload: {
            "design_id": "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
            "measured": {
                  "bounds": {
                        "max": [
                              30.0,
                              45.0,
                              40.0
                        ],
                        "min": [
                              -30.0,
                              -5.329070518200751e-15,
                              0.0
                        ]
                  },
                  "centroid_mm": null,
                  "mass_g": null,
                  "surface_area_mm2": 11889.101731,
                  "volume_mm3": 35571.416047
            },
            "measured_by": "FreeCAD",
            "native_kind": "fcstd",
            "native_sha256": "53a3233a07819dec120daa55e0c4e49320930f5392152afb5b0381d3e80bcdb1",
            "provider": "freecad",
            "provider_version": "1.1.4",
            "script_sha256": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
            "solid_count": 1,
            "step_sha256": "01e54555e3627ef49d9e52a79aa74d4a45b633c84f8444e4a1f822e139be331f",
            "valid": true,
            "warnings": []
      },
      digest: "2be2e2c4a02dbfffd11bd21df0a122cdad95d4e929aa67567cb903a8fca80d49",
      occurred_at: "2026-10-19T20:19:00.140873Z",
      previous_event_digest: "17fc01e9acb9b1ce558d9ffbb239bcaca6c39e6ee610af44089ed246c8c07761",
      event_digest: "460fc66668640f62a6a171b88205298df8591876b3163eda1bc6796910b15702",
    },
    {
      sequence: 4,
      event_type: "artifact.stored",
      aggregate_id: "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
      payload: {
            "design_id": "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
            "kind": "script",
            "provider": "freecad",
            "sha256": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
            "size_bytes": 1457
      },
      digest: "57381bbfe00a7b525fe7b078a693dcef0cfa8e1ba60cf741e9e8dc6e9a2b25b0",
      occurred_at: "2026-10-19T20:19:00.141114Z",
      previous_event_digest: "460fc66668640f62a6a171b88205298df8591876b3163eda1bc6796910b15702",
      event_digest: "2da2666ba8923ea2e76ad8b502df7155c4d87f7f54d033cb0a1f620d17e2a8c3",
    },
    {
      sequence: 5,
      event_type: "artifact.stored",
      aggregate_id: "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
      payload: {
            "design_id": "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
            "kind": "step",
            "provider": "freecad",
            "sha256": "01e54555e3627ef49d9e52a79aa74d4a45b633c84f8444e4a1f822e139be331f",
            "size_bytes": 21648,
            "source": "provider:freecad"
      },
      digest: "022d3f88bb0e4c58dcfb410d453b6e9841565046b75259fe3e560dc285bdfe4b",
      occurred_at: "2026-10-19T20:19:00.141242Z",
      previous_event_digest: "2da2666ba8923ea2e76ad8b502df7155c4d87f7f54d033cb0a1f620d17e2a8c3",
      event_digest: "a7675155457ae2000566486fdd1a6c5979f5a09ea614f6696c72b00b1a2ba5ea",
    },
    {
      sequence: 6,
      event_type: "artifact.stored",
      aggregate_id: "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
      payload: {
            "design_id": "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
            "kind": "fcstd",
            "provider": "freecad",
            "sha256": "53a3233a07819dec120daa55e0c4e49320930f5392152afb5b0381d3e80bcdb1",
            "size_bytes": 40293,
            "source": "provider:freecad"
      },
      digest: "19fed0017a6fb363c37a9103a52e134e2143382e1265fddbf76ec883809338ed",
      occurred_at: "2026-10-19T20:19:00.141359Z",
      previous_event_digest: "a7675155457ae2000566486fdd1a6c5979f5a09ea614f6696c72b00b1a2ba5ea",
      event_digest: "b68b26637441b2dd8afff6868409b9374d8154d27421b9239ece7ed6b2d805a3",
    },
    {
      sequence: 7,
      event_type: "view.imported",
      aggregate_id: "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
      payload: {
            "bounds": {
                  "max": [
                        30.0,
                        45.0,
                        40.0
                  ],
                  "min": [
                        -30.0,
                        -8.881784197001e-16,
                        0.0
                  ]
            },
            "design_id": "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
            "provider": "freecad",
            "role": "view-only",
            "solids": 1,
            "step_sha256": "01e54555e3627ef49d9e52a79aa74d4a45b633c84f8444e4a1f822e139be331f",
            "triangles": 568,
            "vertices": 586,
            "viewer": "opencascade",
            "warnings": []
      },
      digest: "1164d2b5bfb9587608cc3f6fb9da156c091f28bf0239a6320dfb7e8ea5b2061e",
      occurred_at: "2026-10-19T20:19:00.141438Z",
      previous_event_digest: "b68b26637441b2dd8afff6868409b9374d8154d27421b9239ece7ed6b2d805a3",
      event_digest: "e6895c75a92b56349c262735c39b782b6ede1d1bdc2f3d205dc123156cf5e1b6",
    },
    {
      sequence: 8,
      event_type: "design.measured",
      aggregate_id: "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
      payload: {
            "bounds": {
                  "max": [
                        30.0,
                        45.0,
                        40.0
                  ],
                  "min": [
                        -30.0,
                        -5.329070518200751e-15,
                        0.0
                  ]
            },
            "centroid_mm": null,
            "design_id": "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
            "mass_g": null,
            "measured_by": "FreeCAD",
            "provider": "freecad",
            "solid_count": 1,
            "surface_area_mm2": 11889.101731,
            "valid": true,
            "volume_mm3": 35571.416047
      },
      digest: "22da0e877c35cf35968629d3b64da512a52d9c21717a803df48abee9672fcacb",
      occurred_at: "2026-10-19T20:19:00.141522Z",
      previous_event_digest: "e6895c75a92b56349c262735c39b782b6ede1d1bdc2f3d205dc123156cf5e1b6",
      event_digest: "124d7320d20a38f310ae4caf8c85f69f4573baff26c800db1936e816342ef1b8",
    },
    {
      sequence: 9,
      event_type: "design.checked",
      aggregate_id: "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
      payload: {
            "checks": [
                  {
                        "expected": true,
                        "id": "brep_valid",
                        "label": "B-Rep valid",
                        "measured": true,
                        "passed": true,
                        "status": "passed",
                        "tolerance": 0.0,
                        "unit": "bool"
                  },
                  {
                        "expected": 1.0,
                        "id": "single_solid",
                        "label": "Single solid",
                        "measured": 1.0,
                        "passed": true,
                        "status": "passed",
                        "tolerance": 0.0,
                        "unit": "solid"
                  },
                  {
                        "expected": [
                              -30.0,
                              30.0
                        ],
                        "id": "envelope_x",
                        "label": "Envelope X",
                        "measured": [
                              -30.0,
                              30.0
                        ],
                        "passed": true,
                        "status": "passed",
                        "tolerance": 0.01,
                        "unit": "mm"
                  },
                  {
                        "expected": [
                              0.0,
                              45.0
                        ],
                        "id": "envelope_y",
                        "label": "Envelope Y",
                        "measured": [
                              -5.329070518200751e-15,
                              45.0
                        ],
                        "passed": true,
                        "status": "passed",
                        "tolerance": 0.01,
                        "unit": "mm"
                  },
                  {
                        "expected": [
                              0.0,
                              40.0
                        ],
                        "id": "envelope_z",
                        "label": "Envelope Z",
                        "measured": [
                              0.0,
                              40.0
                        ],
                        "passed": true,
                        "status": "passed",
                        "tolerance": 0.01,
                        "unit": "mm"
                  }
            ],
            "design_id": "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
            "provider": "freecad",
            "verified": true
      },
      digest: "495a0eda334558caea587ee5783d34aa5e6981074155de8876b61c8e4390b96d",
      occurred_at: "2026-10-19T20:19:00.141633Z",
      previous_event_digest: "124d7320d20a38f310ae4caf8c85f69f4573baff26c800db1936e816342ef1b8",
      event_digest: "050c58143f2c4ece68a992214b06ad05b286d9ee1be4c7cb13659b868a9b8695",
    },
    {
      sequence: 10,
      event_type: "chat.turn",
      aggregate_id: "studio",
      payload: {
            "cad_provider": "freecad",
            "design_id": "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
            "design_requested": true,
            "missing": [],
            "model": "deterministic-paraphrase-v1",
            "offline": true,
            "prompt_chars": 145,
            "prompt_sha256": "cf59e312d147ea719f1a12c4dc3d6f91e9f5b367d6b1aa04a371e8f51107d13d",
            "provider": "local"
      },
      digest: "c4f61804eec1063fcd8ad756d779d610941dcb7f278de4ca2a8cb4f91dc3a71a",
      occurred_at: "2026-10-19T20:19:00.141968Z",
      previous_event_digest: "050c58143f2c4ece68a992214b06ad05b286d9ee1be4c7cb13659b868a9b8695",
      event_digest: "7dcc0b3ef7385f76c24b9e96c2dc6cd06aabe3042fa2df8588df66a74f19b48c",
    },
  ],
  next_after: 10,
  has_more: false,
  limit: 200,
};

/** The recorded verification of an untampered journal. */
export const bracketJournalVerified: JournalVerify = {
  ok: true,
  event_count: 5,
  head_digest: BRACKET_JOURNAL_HEAD,
  first_bad_sequence: null,
};

/** The recorded verification after one payload byte was tampered with. */
export const bracketJournalBroken: JournalVerify = {
  ok: false,
  event_count: 5,
  head_digest: 'ffb0b0fb354423ea20ce6133455579546b5114a21a88824f87ced9928cd76ba0',
  first_bad_sequence: 4,
};
