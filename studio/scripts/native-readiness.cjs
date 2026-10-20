#!/usr/bin/env node
// Current-feature native readiness gate.
//
// The native Rust `kala-studio` is the front door of the public `/api/studio`
// surface: it answers the non-geometry routes itself and proxies the geometry,
// design, journal, CAD-provider and import routes to the supervised Python
// geometry worker. The working Python reference is still the oracle. This
// helper compares the *actual* source surfaces (routes, route-methods and the
// DTO/capability fields the desktop consumes) instead of trusting a hardcoded
// "ready" flag. The report separates the Rust-native route-methods from the
// proxied ones, and computes readiness from what the sources actually expose.
//
// It is used in two places:
//   * `native-backend.cjs` snapshots the source digests/readiness into
//     `native-build.json`, so a stage can be tied to the source it was built
//     from.
//   * `desktop-pack.cjs` recomputes the report before packaging and refuses a
//     stale stage or a release artifact while parity is pending. Unpacked
//     `--dir` development review artifacts remain possible but are labelled.
//
// Exit codes: 0 ready (or `--report-only`), 1 parity pending, 2 usage/source
// parse failure. No network, provider, credential or publish call is made.

'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const STUDIO_ROOT = path.resolve(__dirname, '..');
const REPO_ROOT = path.resolve(STUDIO_ROOT, '..');
const PYTHON_STUDIO_ROOT = path.join(REPO_ROOT, 'src', 'kala_agent', 'studio');
const NATIVE_CRATE_ROOT = path.join(REPO_ROOT, 'backend', 'crates', 'kala-studio');
const SCHEMA_VERSION = 1;
const KIND = 'kala-studio-native-current-feature-readiness';

// Labels are explanatory only. `ready` and the missing lists are computed from
// the parsed sources; unknown additions still fail the route-surface diff.
const FEATURE_LABELS = [
  {
    id: 'design.compile',
    match: (route) => route === 'POST /api/studio/design',
    label: 'POST /design: compile/measure/journal an ADR-0009 design turn',
  },
  {
    id: 'design.drawing',
    match: (route) => route === 'GET /api/studio/design/{}/drawing',
    label: 'GET /design/{id}/drawing: hidden-line views',
  },
  {
    id: 'design.section',
    match: (route) => route === 'GET /api/studio/design/{}/section',
    label: 'GET /design/{id}/section: section loops',
  },
  {
    id: 'journal.page',
    match: (route) => route === 'GET /api/studio/journal',
    label: 'GET /journal: bounded session journal page',
  },
  {
    id: 'journal.verify',
    match: (route) => route === 'GET /api/studio/journal/verify',
    label: 'GET /journal/verify: hash-chain verification',
  },
  {
    id: 'health.design_available',
    match: (capability) => capability === 'health.design_available',
    label: 'health.design_available capability flag',
  },
  {
    id: 'chat.design_result',
    match: (capability) => capability === 'chat.design_result',
    label: 'chat response optional design result',
  },
  {
    id: 'export.design_id',
    match: (capability) => capability === 'export.design_id',
    label: 'export request stored design_id',
  },
];

const CAPABILITY_SPECS = [
  {
    id: 'health.design_available',
    // Python: the HealthResponse model field.
    python: /class\s+HealthResponse\b[\s\S]*?\bdesign_available\s*:/,
    // Native: the health JSON body / response field.
    native: /\bdesign_available\b/,
  },
  {
    id: 'chat.design_result',
    // Python: ChatResponse.design: DesignResult | None.
    python: /class\s+ChatResponse\b[\s\S]*?\bdesign\s*:\s*DesignResult\b/,
    // Native: a chat response design member/JSON key.
    native: /(?:["']design["']\s*[,:]|\bdesign\s*:\s*(?:Option|Json|Design|&?str))/,
  },
  {
    id: 'export.design_id',
    // Python: ExportRequest.design_id.
    python: /class\s+ExportRequest\b[\s\S]*?\bdesign_id\s*:/,
    // Native: a stored-design export identifier.
    native: /\bdesign_id\b/,
  },
];

function normalizeRoutePath(routePath) {
  const withoutQuery = routePath.split('?', 1)[0].replace(/\/+$/, '');
  const normalized = withoutQuery === '' ? '/' : withoutQuery;
  return normalized.replace(/\{[^}/]+\}/g, '{}');
}

function routeKey(method, routePath) {
  const normalized = normalizeRoutePath(routePath);
  const absolute = normalized.startsWith('/api/') ? normalized : `/api/studio${normalized}`;
  return `${method.toUpperCase()} ${absolute}`;
}

// Split a joined source back into its `// FILE <relative>` blocks. A raw
// snippet without markers is treated as one public block, which keeps
// synthetic test sources behaving exactly as before.
function pythonSourceBlocks(source) {
  const marker = /^\/\/ FILE ([^\n]+)\n/gm;
  const blocks = [];
  let match;
  let start = 0;
  let name = '';
  while ((match = marker.exec(source)) !== null) {
    if (match.index > start || blocks.length > 0) {
      blocks.push({ name, text: source.slice(start, match.index) });
    }
    name = match[1];
    start = marker.lastIndex;
  }
  blocks.push({ name, text: source.slice(start) });
  return blocks;
}

function pythonRouterPrefix(block) {
  const match = /APIRouter\(\s*prefix\s*=\s*([^,)]+)/.exec(block);
  if (!match) return '';
  const raw = match[1].trim();
  const literal = /^["']([^"']*)["']$/.exec(raw);
  if (literal) return literal[1];
  // A module constant is the common form (`APIRouter(prefix=INTERNAL_PREFIX)`);
  // resolve it from the same source block rather than guessing.
  const escaped = raw.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const constant = new RegExp(
    `(?:^|\\n)\\s*${escaped}\\s*(?::[^=\\n]+)?=\\s*["']([^"']+)["']`,
  ).exec(block);
  return constant ? constant[1] : '';
}

function resolvePythonRoute(prefix, routePath) {
  if (routePath.startsWith('/api/') || routePath.startsWith('/internal/')) return routePath;
  const relative = routePath.startsWith('/') ? routePath : `/${routePath}`;
  if (prefix) return `${prefix.replace(/\/+$/, '')}${relative}`;
  // The public Studio router has no prefix; middleware mounts it under
  // `/api/studio`. A relative route with no declared prefix is public.
  return `/api/studio${relative}`;
}

/**
 * The Python route surface, split by audience. Only `/api/studio/*`
 * route-methods are public and count toward native parity; the worker's
 * `/internal/*` routes are reported separately so they can never be
 * mistaken for browser-facing API surface.
 */
function parsePythonRouteSurface(source) {
  const publicRoutes = new Set();
  const internalRoutes = new Set();
  const pattern = /@router\.(get|post|put|patch|delete)\(\s*["']([^"']+)["']/gi;
  for (const block of pythonSourceBlocks(source)) {
    const prefix = pythonRouterPrefix(block.text);
    let match;
    while ((match = pattern.exec(block.text)) !== null) {
      const resolved = resolvePythonRoute(prefix, match[2]);
      if (resolved.startsWith('/api/')) {
        publicRoutes.add(`${match[1].toUpperCase()} ${normalizeRoutePath(resolved)}`);
      } else {
        internalRoutes.add(`${match[1].toUpperCase()} ${normalizeRoutePath(resolved)}`);
      }
    }
    pattern.lastIndex = 0;
  }
  return {
    publicRoutes: [...publicRoutes].sort(),
    internalRoutes: [...internalRoutes].sort(),
  };
}

function parsePythonRoutes(source) {
  return parsePythonRouteSurface(source).publicRoutes;
}

/**
 * Return the balanced `{ ... }` block starting at `openIndex` (which must
 * point at `{`). Rust strings, char literals and comments are skipped so a
 * brace inside one of them cannot unbalance the scan.
 */
function rustBraceBlock(source, openIndex) {
  if (source[openIndex] !== '{') {
    throw new Error('readiness parser error: expected a `{` block in the native source');
  }
  let depth = 0;
  let index = openIndex;
  let mode = 'code';
  while (index < source.length) {
    const char = source[index];
    const next = source[index + 1];
    if (mode === 'code') {
      if (char === '/' && next === '/') {
        mode = 'line';
        index += 2;
      } else if (char === '/' && next === '*') {
        mode = 'block';
        index += 2;
      } else if (char === '"') {
        mode = 'string';
        index += 1;
      } else if (char === "'") {
        // A Rust lifetime (`'a`) is not a character literal.
        if (next === '\\' || source[index + 2] === "'") {
          mode = 'char';
        }
        index += 1;
      } else if (char === '{') {
        depth += 1;
        index += 1;
      } else if (char === '}') {
        depth -= 1;
        if (depth === 0) {
          return { body: source.slice(openIndex + 1, index), end: index + 1 };
        }
        index += 1;
      } else {
        index += 1;
      }
      continue;
    }
    if (mode === 'line') {
      if (char === '\n') mode = 'code';
      index += 1;
    } else if (mode === 'block') {
      if (char === '*' && next === '/') {
        mode = 'code';
        index += 2;
      } else {
        index += 1;
      }
    } else if (mode === 'string') {
      if (char === '\\') index += 2;
      else {
        if (char === '"') mode = 'code';
        index += 1;
      }
    } else {
      if (char === '\\') index += 2;
      else {
        if (char === "'") mode = 'code';
        index += 1;
      }
    }
  }
  throw new Error('unbalanced braces in the native Studio source');
}

/**
 * The body of a free function declaration. A trait declaration (`fn x();`)
 * has no body and is refused rather than parsed as the route table.
 */
function rustFunctionBody(source, name) {
  const pattern = new RegExp(`\\bfn\\s+${name}\\s*\\(`);
  const match = pattern.exec(source);
  if (!match) return null;
  const open = source.indexOf('{', match.index);
  if (open === -1 || source.slice(match.index, open).includes(';')) return null;
  return rustBraceBlock(source, open).body;
}

/**
 * The current r2 route table is two functions: `classify(path)` maps a path
 * (or a `strip_prefix`/`strip_suffix` family) to a `RouteKind`, and
 * `allowed_methods(kind)` maps each kind to its methods. The `route` dispatch
 * then decides which kinds call `proxy_route` (the Python geometry worker).
 *
 * The parser requires both tables, requires every classified kind to have a
 * method arm and vice versa, and refuses any classifier arm it does not
 * understand. It never falls back to a guessed surface.
 */
function parseRustRouteSurface(source) {
  const classifyBody = rustFunctionBody(source, 'classify');
  if (classifyBody === null) {
    throw new Error(
      'could not find `fn classify` in the native Studio source; ' +
      'the readiness checker refuses to guess the native route surface',
    );
  }
  const classified = new Map();
  let classifyArmCount = 0;
  const addClassified = (routePath, variant) => {
    const normalized = normalizeRoutePath(routePath);
    const existing = classified.get(normalized);
    if (existing !== undefined && existing !== variant) {
      throw new Error(
        `the native Studio classifier maps ${normalized} to both ` +
        `${existing} and ${variant}; the readiness checker refuses the ambiguity`,
      );
    }
    classified.set(normalized, variant);
    classifyArmCount += 1;
  };

  // `"/api/studio/health" => RouteKind::Health`
  const literalArm = /"([^"]+)"\s*=>\s*RouteKind::(\w+)/g;
  let arm;
  while ((arm = literalArm.exec(classifyBody)) !== null) {
    addClassified(arm[1], arm[2]);
  }

  // `strip_prefix("/api/studio/design/")` + `strip_suffix("/drawing")`
  const prefixArm = /strip_prefix\(\s*"([^"]+)"\s*\)\s*\{/g;
  while ((arm = prefixArm.exec(classifyBody)) !== null) {
    const prefixBlock = rustBraceBlock(classifyBody, prefixArm.lastIndex - 1);
    const suffixArm = /strip_suffix\(\s*"([^"]+)"\s*\)\s*\{/g;
    let suffix;
    while ((suffix = suffixArm.exec(prefixBlock.body)) !== null) {
      const suffixBlock = rustBraceBlock(prefixBlock.body, suffixArm.lastIndex - 1);
      const kind = /RouteKind::(\w+)/.exec(suffixBlock.body);
      if (!kind) {
        throw new Error(
          'the native Studio classifier has a `strip_suffix` arm with no RouteKind; ' +
          'the readiness checker refuses to guess that route',
        );
      }
      addClassified(`${arm[1]}{}${suffix[1]}`, kind[1]);
      suffixArm.lastIndex = suffixBlock.end;
    }
    prefixArm.lastIndex = prefixBlock.end;
  }

  const kindOccurrences = (classifyBody.match(/RouteKind::\w+/g) || []).length;
  if (classifyArmCount === 0 || classifyArmCount !== kindOccurrences) {
    throw new Error(
      'the native Studio classifier contains a route arm shape the readiness ' +
      'checker does not understand; it refuses to guess the route surface',
    );
  }

  const methodsBody = rustFunctionBody(source, 'allowed_methods');
  if (methodsBody === null) {
    throw new Error(
      'could not find `fn allowed_methods` in the native Studio source; ' +
      'the readiness checker refuses to guess the native route surface',
    );
  }
  const methodTable = new Map();
  const methodArm = /((?:RouteKind::\w+\s*\|\s*)*RouteKind::\w+)\s*=>\s*&?\s*\[([^\]]*)\]/g;
  while ((arm = methodArm.exec(methodsBody)) !== null) {
    const variants = [...arm[1].matchAll(/RouteKind::(\w+)/g)].map((item) => item[1]);
    const rawMethods = arm[2];
    const methods = [...rawMethods.matchAll(/Method::(GET|POST|PUT|PATCH|DELETE)/g)]
      .map((item) => item[1].toUpperCase());
    const residue = rawMethods
      .replace(/Method::(?:GET|POST|PUT|PATCH|DELETE)/g, '')
      .replace(/[\s,]/g, '');
    if (methods.length === 0 || residue !== '') {
      throw new Error(
        'the native Studio method table contains an arm shape the readiness ' +
        'checker does not understand; it refuses to guess the route methods',
      );
    }
    for (const variant of variants) {
      if (methodTable.has(variant)) {
        throw new Error(`the native Studio method table defines ${variant} twice`);
      }
      methodTable.set(variant, methods);
    }
  }
  if (methodTable.size === 0) {
    throw new Error(
      'the native Studio method table was parsed but contained no route kinds; ' +
      'the readiness checker refuses to report readiness from an empty surface',
    );
  }

  const classifiedVariants = new Set(classified.values());
  for (const variant of classifiedVariants) {
    if (!methodTable.has(variant)) {
      throw new Error(
        `the native Studio classifier returns ${variant} but the method table ` +
        'does not define it; the readiness checker refuses to guess its methods',
      );
    }
  }
  for (const variant of methodTable.keys()) {
    if (!classifiedVariants.has(variant)) {
      throw new Error(
        `the native Studio method table defines ${variant} but the classifier ` +
        'never returns it; the readiness checker refuses the inconsistent table',
      );
    }
  }

  // Which kinds are forwarded to the geometry worker? Read the real dispatch,
  // never a label. A source that contains `proxy_route` must expose a
  // parseable `fn route` with at least one proxy arm.
  const proxiedVariants = new Set();
  if (/\bfn\s+proxy_route\s*\(/.test(source)) {
    const routeBody = rustFunctionBody(source, 'route');
    if (routeBody === null) {
      throw new Error(
        'the native Studio source has a `proxy_route` but no `fn route` dispatch; ' +
        'the readiness checker refuses to guess which routes are proxied',
      );
    }
    const dispatchArm = /((?:RouteKind::\w+\s*\|\s*)*RouteKind::\w+)\s*=>\s*/g;
    while ((arm = dispatchArm.exec(routeBody)) !== null) {
      const variants = [...arm[1].matchAll(/RouteKind::(\w+)/g)].map((item) => item[1]);
      let armBody;
      if (routeBody[dispatchArm.lastIndex] === '{') {
        const block = rustBraceBlock(routeBody, dispatchArm.lastIndex);
        armBody = block.body;
        dispatchArm.lastIndex = block.end;
      } else {
        armBody = routeBody.slice(dispatchArm.lastIndex).split('\n', 1)[0];
      }
      if (/\bproxy_route\s*\(/.test(armBody)) {
        for (const variant of variants) proxiedVariants.add(variant);
      }
    }
    if (proxiedVariants.size === 0) {
      throw new Error(
        'the native Studio route dispatch contains no `proxy_route` arm; ' +
        'the readiness checker refuses to guess the native/proxied split',
      );
    }
    for (const variant of proxiedVariants) {
      if (!classifiedVariants.has(variant)) {
        throw new Error(
          `the native Studio dispatch proxies ${variant} but the classifier ` +
          'never returns it; the readiness checker refuses the inconsistent table',
        );
      }
    }
  }

  const nativeRoutes = new Set();
  const proxiedRoutes = new Set();
  for (const [routePath, variant] of classified) {
    for (const method of methodTable.get(variant)) {
      const key = routeKey(method, routePath);
      if (proxiedVariants.has(variant)) proxiedRoutes.add(key);
      else nativeRoutes.add(key);
    }
  }
  const routes = new Set([...nativeRoutes, ...proxiedRoutes]);
  if (routes.size === 0) {
    throw new Error(
      'the native Studio route table was parsed but contained no route-methods; ' +
      'the readiness checker refuses to report readiness from an empty surface',
    );
  }
  return {
    routeMethods: [...routes].sort(),
    rustNativeRouteMethods: [...nativeRoutes].sort(),
    proxiedRouteMethods: [...proxiedRoutes].sort(),
  };
}

function parseRustRoutes(source) {
  return parseRustRouteSurface(source).routeMethods;
}

function capabilityReport(pythonSource, nativeSource) {
  return CAPABILITY_SPECS.map((spec) => {
    const requiredByPython = spec.python.test(pythonSource);
    const presentInNative = spec.native.test(nativeSource);
    return {
      id: spec.id,
      requiredByPython,
      presentInNative,
    };
  });
}

function sourceFeatureLabels(missingRouteMethods, missingCapabilities) {
  const features = [];
  for (const spec of FEATURE_LABELS) {
    if (missingRouteMethods.some((route) => spec.match(route))) {
      features.push({ id: spec.id, label: spec.label });
    } else if (missingCapabilities.some((capability) => spec.match(capability))) {
      features.push({ id: spec.id, label: spec.label });
    }
  }
  return features;
}

/**
 * Compare already-loaded source text. Exposed separately from filesystem
 * discovery so tests can prove the comparison is not hardcoded to false.
 */
function analyzeStudioSources({ pythonSource, nativeSource }) {
  if (typeof pythonSource !== 'string' || typeof nativeSource !== 'string') {
    throw new Error('readiness analysis requires pythonSource and nativeSource text');
  }
  const pythonRoutes = parsePythonRouteSurface(pythonSource);
  const pythonRouteMethods = pythonRoutes.publicRoutes;
  const internalRouteMethods = pythonRoutes.internalRoutes;
  const rustSurface = parseRustRouteSurface(nativeSource);
  const nativeRouteMethods = rustSurface.routeMethods;
  if (pythonRouteMethods.length === 0) {
    throw new Error('the Python Studio source exposed no @router routes');
  }
  const nativeSet = new Set(nativeRouteMethods);
  const pythonSet = new Set(pythonRouteMethods);
  const missingRouteMethods = pythonRouteMethods.filter((route) => !nativeSet.has(route));
  const extraNativeRouteMethods = nativeRouteMethods.filter((route) => !pythonSet.has(route));
  const capabilities = capabilityReport(pythonSource, nativeSource);
  const missingCapabilities = capabilities
    .filter((row) => row.requiredByPython && !row.presentInNative)
    .map((row) => row.id);
  const ready = missingRouteMethods.length === 0 && missingCapabilities.length === 0;
  return {
    ready,
    pythonRouteMethods,
    internalRouteMethods,
    internalRouteMethodCount: internalRouteMethods.length,
    nativeRouteMethods,
    rustNativeRouteMethods: rustSurface.rustNativeRouteMethods,
    rustNativeRouteMethodCount: rustSurface.rustNativeRouteMethods.length,
    proxiedRouteMethods: rustSurface.proxiedRouteMethods,
    proxiedRouteMethodCount: rustSurface.proxiedRouteMethods.length,
    missingRouteMethods,
    extraNativeRouteMethods,
    capabilities,
    missingCapabilities,
    pendingFeatures: sourceFeatureLabels(missingRouteMethods, missingCapabilities),
  };
}

function listFiles(root, extensions) {
  const files = [];
  const stack = [''];
  while (stack.length > 0) {
    const relative = stack.pop();
    const directory = path.join(root, relative);
    const entries = fs.readdirSync(directory, { withFileTypes: true })
      .sort((left, right) => (left.name < right.name ? -1 : left.name > right.name ? 1 : 0));
    for (const entry of entries) {
      const next = relative ? path.join(relative, entry.name) : entry.name;
      const absolute = path.join(root, next);
      const stat = fs.lstatSync(absolute);
      if (stat.isSymbolicLink()) {
        throw new Error(`refusing to hash a symlinked source file: ${absolute}`);
      }
      if (stat.isDirectory()) {
        if (entry.name === '__pycache__' || entry.name === 'target') continue;
        stack.push(next);
      } else if (stat.isFile() && extensions.includes(path.extname(entry.name).toLowerCase())) {
        files.push(next);
      }
    }
  }
  return files.sort();
}

function readSources(root, extensions) {
  const files = listFiles(root, extensions);
  if (files.length === 0) {
    throw new Error(`no ${extensions.join('/')} sources found under ${root}`);
  }
  return files.map((relative) => ({
    relative: relative.split(path.sep).join('/'),
    text: fs.readFileSync(path.join(root, relative), 'utf8'),
  }));
}

function joinedSource(entries) {
  return entries.map((entry) => `\n// FILE ${entry.relative}\n${entry.text}`).join('');
}

function sourceTreeSha256(entries) {
  const hash = crypto.createHash('sha256');
  hash.update('kala-studio-current-source\n');
  for (const entry of entries) {
    hash.update(entry.relative);
    hash.update('\0');
    hash.update(entry.text);
    hash.update('\n');
  }
  return hash.digest('hex');
}

function checkStudioReadiness(options = {}) {
  const pythonRoot = path.resolve(options.pythonStudioRoot || PYTHON_STUDIO_ROOT);
  const nativeRoot = path.resolve(options.nativeCrateRoot || NATIVE_CRATE_ROOT);
  const allPythonEntries = readSources(pythonRoot, ['.py']);
  // Digest only the source files that actually define the route table or the
  // DTO capability fields being gated. Other ADR-0009 implementation modules
  // may churn without changing the desktop contract; including them would
  // make a stage spuriously stale while the concurrent design wave works.
  const pythonEntries = allPythonEntries.filter((entry) =>
    /@router\.|class\s+(?:HealthResponse|ChatResponse|ExportRequest)\b/.test(entry.text),
  );
  const effectivePythonEntries = pythonEntries.length > 0 ? pythonEntries : allPythonEntries;
  const nativeEntries = readSources(nativeRoot, ['.rs']);
  const analysis = analyzeStudioSources({
    pythonSource: joinedSource(effectivePythonEntries),
    nativeSource: joinedSource(nativeEntries),
  });
  const nativeSet = new Set(analysis.nativeRouteMethods);
  const coveredPythonRoutes = analysis.pythonRouteMethods
    .filter((route) => nativeSet.has(route)).length;
  return {
    schemaVersion: SCHEMA_VERSION,
    kind: KIND,
    generatedAt: options.generatedAt || new Date().toISOString(),
    ready: analysis.ready,
    python: {
      root: pythonRoot,
      sourceSha256: sourceTreeSha256(effectivePythonEntries),
      fileCount: effectivePythonEntries.length,
      routeMethodCount: analysis.pythonRouteMethods.length,
      routeMethods: analysis.pythonRouteMethods,
      internalRouteMethodCount: analysis.internalRouteMethodCount,
      internalRouteMethods: analysis.internalRouteMethods,
    },
    native: {
      root: nativeRoot,
      sourceSha256: sourceTreeSha256(nativeEntries),
      fileCount: nativeEntries.length,
      routeMethodCount: analysis.nativeRouteMethods.length,
      routeMethods: analysis.nativeRouteMethods,
      rustNativeRouteMethodCount: analysis.rustNativeRouteMethodCount,
      rustNativeRouteMethods: analysis.rustNativeRouteMethods,
      proxiedRouteMethodCount: analysis.proxiedRouteMethodCount,
      proxiedRouteMethods: analysis.proxiedRouteMethods,
    },
    missingRouteMethods: analysis.missingRouteMethods,
    extraNativeRouteMethods: analysis.extraNativeRouteMethods,
    capabilities: analysis.capabilities,
    missingCapabilities: analysis.missingCapabilities,
    pendingFeatures: analysis.pendingFeatures,
    summary: analysis.ready
      ? `native Studio front door covers all ${analysis.pythonRouteMethods.length} current Python route-methods and DTO capabilities ` +
        `(${analysis.rustNativeRouteMethodCount} answered directly, ${analysis.proxiedRouteMethodCount} through the geometry-backend seam, native by default)`
      : `native Studio front door covers ${coveredPythonRoutes} of ${analysis.pythonRouteMethods.length} current Python route-methods ` +
        `(${analysis.rustNativeRouteMethodCount} answered directly, ${analysis.proxiedRouteMethodCount} through the geometry-backend seam); ` +
        `${analysis.missingRouteMethods.length} route-method(s) and ${analysis.missingCapabilities.length} capability field(s) remain`,
  };
}

function readinessSnapshot(report) {
  return {
    ready: report.ready,
    pythonSourceSha256: report.python.sourceSha256,
    nativeSourceSha256: report.native.sourceSha256,
    pythonRouteMethodCount: report.python.routeMethodCount,
    nativeRouteMethodCount: report.native.routeMethodCount,
    missingRouteMethods: report.missingRouteMethods,
    missingCapabilities: report.missingCapabilities,
    pendingFeatures: report.pendingFeatures,
    summary: report.summary,
  };
}

function formatReadinessReport(report) {
  const lines = [
    `[native-readiness] ${report.summary}`,
    `[native-readiness] Python source ${report.python.sourceSha256} (${report.python.fileCount} files)`,
    `[native-readiness] native source ${report.native.sourceSha256} (${report.native.fileCount} files)`,
    `[native-readiness] internal worker route-methods (not public parity): ${report.python.internalRouteMethodCount}`,
    `[native-readiness] Rust-native route-methods (in-process): ${report.native.rustNativeRouteMethodCount}`,
    `[native-readiness] proxied route-methods (Python geometry worker): ${report.native.proxiedRouteMethodCount}`,
  ];
  if (report.python.internalRouteMethods.length > 0) {
    lines.push('[native-readiness] worker-only route-methods:');
    for (const route of report.python.internalRouteMethods) lines.push(`  - ${route}`);
  }
  if (report.native.rustNativeRouteMethods.length > 0) {
    lines.push('[native-readiness] Rust-native route-methods:');
    for (const route of report.native.rustNativeRouteMethods) lines.push(`  - ${route}`);
  }
  if (report.native.proxiedRouteMethods.length > 0) {
    lines.push('[native-readiness] proxied route-methods:');
    for (const route of report.native.proxiedRouteMethods) lines.push(`  - ${route}`);
  }
  if (report.missingRouteMethods.length > 0) {
    lines.push('[native-readiness] missing route-methods:');
    for (const route of report.missingRouteMethods) lines.push(`  - ${route}`);
  }
  if (report.missingCapabilities.length > 0) {
    lines.push('[native-readiness] missing DTO/capabilities:');
    for (const capability of report.missingCapabilities) lines.push(`  - ${capability}`);
  }
  if (report.extraNativeRouteMethods.length > 0) {
    lines.push('[native-readiness] native-only route-methods (allowed, not parity):');
    for (const route of report.extraNativeRouteMethods) lines.push(`  - ${route}`);
  }
  lines.push(
    report.ready
      ? '[native-readiness] READY for current-feature packaging'
      : '[native-readiness] PENDING: release packaging/NSIS must remain blocked; ' +
        'use the Python reference by default or an unpacked --dir development artifact',
  );
  return lines.join('\n');
}

function writeReport(report, outputPath) {
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, 'utf8');
  return outputPath;
}

function parseArguments(argv) {
  const options = { reportOnly: false, json: false, output: null, help: false };
  const remaining = [...argv];
  while (remaining.length > 0) {
    const argument = remaining.shift();
    switch (argument) {
      case '--report-only':
        options.reportOnly = true;
        break;
      case '--json':
        options.json = true;
        break;
      case '--output': {
        const value = remaining.shift();
        if (!value || value.startsWith('--')) {
          throw new Error('--output requires a file path');
        }
        options.output = path.resolve(value);
        break;
      }
      case '--help':
      case '-h':
        options.help = true;
        break;
      default:
        throw new Error(`unknown argument: ${argument}`);
    }
  }
  return options;
}

function usage() {
  return [
    'Usage: node scripts/native-readiness.cjs [--report-only] [--json] [--output <file>]',
    '',
    'Compare the current Python Studio route/DTO surface with the native Rust',
    'slice. Exit 1 when release parity is pending, 0 when ready or --report-only.',
  ].join('\n');
}

function main(argv = process.argv.slice(2)) {
  let options;
  try {
    options = parseArguments(argv);
  } catch (error) {
    console.error(`[native-readiness] ${error.message}`);
    console.error(usage());
    return 2;
  }
  if (options.help) {
    console.log(usage());
    return 0;
  }
  try {
    const report = checkStudioReadiness();
    if (options.output) writeReport(report, options.output);
    if (options.json) console.log(JSON.stringify(report, null, 2));
    else console.log(formatReadinessReport(report));
    return report.ready || options.reportOnly ? 0 : 1;
  } catch (error) {
    console.error(`[native-readiness] ${error.message}`);
    return 2;
  }
}

if (require.main === module) {
  process.exit(main());
}

module.exports = {
  STUDIO_ROOT,
  REPO_ROOT,
  PYTHON_STUDIO_ROOT,
  NATIVE_CRATE_ROOT,
  SCHEMA_VERSION,
  KIND,
  normalizeRoutePath,
  routeKey,
  parsePythonRoutes,
  parsePythonRouteSurface,
  parseRustRoutes,
  parseRustRouteSurface,
  analyzeStudioSources,
  checkStudioReadiness,
  readinessSnapshot,
  formatReadinessReport,
  writeReport,
  parseArguments,
  main,
};
