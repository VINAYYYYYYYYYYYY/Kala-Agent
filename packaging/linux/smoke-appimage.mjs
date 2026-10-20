#!/usr/bin/env node
// Headless smoke test for the built Linux AppImage.
//
// Preferred path (no FUSE, no X server needed):
//   * `--appimage-extract` the artifact into a temporary directory;
//   * run `resources/backend/kala-studio` directly with Python-free PATH and
//     the bundled `lib/` on LD_LIBRARY_PATH (the same fallback the Electron
//     main process uses when patchelf was unavailable);
//   * probe `/api/studio/health`, the packaged UI, an unauthenticated refusal,
//     and optionally the native OpenCascade L-bracket and FreeCAD provider.
//
// `xvfb-run` is not required and is not used: the native backend serves the
// UI over loopback, so the packaged runtime can be verified without a display.
// If a future host wants to exercise the Electron shell itself, run
// `xvfb-run -a <AppImage>` separately.
//
// Default behaviour: every check that does not need native geometry is
// mandatory; the L-bracket, drawing, section, STEP import and FreeCAD checks
// are reported as PENDING until native-design lands. Pass
// `--require-geometry` / `--require-freecad` to make them mandatory in the
// cutover re-run.
//
// Two backend phases are used:
//   1. `PATH=""` (no Python at all): health, UI, auth, OpenCascade L-bracket,
//      HLR drawing, XZ section and the STEP export/import round trip.
//   2. `PATH=<FreeCAD dir>:/usr/bin:/bin` (FreeCAD itself is installed on the
//      host, never bundled; no repository and no virtualenv anywhere on the
//      path): the FreeCAD provider build. `PYTHONPATH`, `PYTHONHOME` and
//      `KALA_FREECAD_SYS_PATH` are absent, and the working directory is a
//      fresh temporary directory.

import { spawn, spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..', '..');
const DEFAULT_RELEASE_DIR = path.join(REPO_ROOT, 'studio', 'release');
const L_BRACKET_VOLUME_MM3 = 35_571.416_047;
const VOLUME_REL_TOLERANCE = 1e-6;
// XZ section at y = 4 of the smoke L-bracket: 60 x (0..8) base rectangle minus
// the Ø10 upright bore -> 2400 - 25*pi, exactly (the kernel reports
// area_exact=true).
const SECTION_EXPECTED_AREA_MM2 = 2400 - 25 * Math.PI;
const SECTION_AREA_ABS_TOLERANCE = 1e-9;
const BOUNDS_ABS_TOLERANCE_MM = 0.001;
const REQUIRED_ROUTE = '/api/studio/health';

function usage() {
  return [
    'Usage: node packaging/linux/smoke-appimage.mjs [options]',
    '',
    'Options:',
    '  --appimage <path>    AppImage to test (default: newest studio/release/*.AppImage)',
    '  --timeout <seconds>  backend readiness timeout (default 60)',
    '  --require-geometry   fail unless the native OpenCascade L-bracket, drawing, section and STEP import pass',
    '  --require-freecad    fail unless the FreeCAD provider builds the L-bracket from the extracted app',
    '  --freecad-path <dir> directory containing FreeCADCmd (default: the host PATH)',
    '  --keep               keep the extracted temporary tree',
    '  -h, --help           show this help',
  ].join('\n');
}

function parseArguments(argv) {
  const options = {
    appimage: null,
    timeoutSeconds: 60,
    requireGeometry: false,
    requireFreecad: false,
    freecadDirectory: null,
    keep: false,
    help: false,
  };
  const remaining = [...argv];
  while (remaining.length > 0) {
    const argument = remaining.shift();
    switch (argument) {
      case '--appimage':
        options.appimage = path.resolve(takeValue(remaining, argument));
        break;
      case '--freecad-path':
        options.freecadDirectory = path.resolve(takeValue(remaining, argument));
        break;
      case '--timeout': {
        const value = Number(takeValue(remaining, argument));
        if (!Number.isFinite(value) || value <= 0) throw new Error('--timeout must be positive');
        options.timeoutSeconds = value;
        break;
      }
      case '--require-geometry':
        options.requireGeometry = true;
        break;
      case '--require-freecad':
        options.requireFreecad = true;
        break;
      case '--keep':
        options.keep = true;
        break;
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

function takeValue(remaining, argument) {
  const value = remaining.shift();
  if (!value || value.startsWith('--')) throw new Error(`${argument} requires a value`);
  return value;
}

function discoverAppImage(explicit) {
  if (explicit) {
    if (!fs.existsSync(explicit) || !fs.statSync(explicit).isFile()) {
      throw new Error(`AppImage not found: ${explicit}`);
    }
    return explicit;
  }
  if (!fs.existsSync(DEFAULT_RELEASE_DIR)) {
    throw new Error(`release directory not found: ${DEFAULT_RELEASE_DIR}; run packaging/linux/build-appimage.sh first`);
  }
  const candidates = fs.readdirSync(DEFAULT_RELEASE_DIR)
    .filter((name) => name.endsWith('.AppImage'))
    .map((name) => path.join(DEFAULT_RELEASE_DIR, name))
    .sort((left, right) => fs.statSync(right).mtimeMs - fs.statSync(left).mtimeMs);
  if (candidates.length === 0) {
    throw new Error(`no AppImage found under ${DEFAULT_RELEASE_DIR}`);
  }
  return candidates[0];
}

function sha256(filePath) {
  const hash = crypto.createHash('sha256');
  const descriptor = fs.openSync(filePath, 'r');
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  try {
    let read = 0;
    while ((read = fs.readSync(descriptor, buffer, 0, buffer.length, null)) > 0) {
      hash.update(buffer.subarray(0, read));
    }
  } finally {
    fs.closeSync(descriptor);
  }
  return hash.digest('hex');
}

function walkFiles(root) {
  const files = [];
  const stack = [''];
  while (stack.length > 0) {
    const relative = stack.pop();
    for (const entry of fs.readdirSync(path.join(root, relative), { withFileTypes: true })) {
      const next = relative ? path.join(relative, entry.name) : entry.name;
      if (entry.isSymbolicLink()) {
        try {
          const stat = fs.statSync(path.join(root, next));
          if (stat.isFile()) files.push(next);
        } catch {
          // A broken symlink carries no payload to inspect.
        }
        continue;
      }
      if (entry.isDirectory()) stack.push(next);
      else if (entry.isFile()) files.push(next);
    }
  }
  return files;
}

function isFreeCadAdapterPath(relative) {
  const normalized = relative.split(path.sep).join('/');
  return normalized.startsWith('adapters/freecad/');
}

function assertNoPythonRuntime(backend) {
  const offenders = [];
  for (const relative of walkFiles(backend)) {
    const base = path.basename(relative).toLowerCase();
    const normalized = relative.split(path.sep).join('/');
    const parts = normalized.split('/');
    if (/^libpython/.test(base)) {
      offenders.push(relative);
      continue;
    }
    if (/^python(?:[0-9.]*)?(?:\.exe)?$/.test(base)) {
      offenders.push(relative);
      continue;
    }
    if (isFreeCadAdapterPath(normalized)) {
      // The adapter and its vendored FreeCAD-interpreter dependencies are the
      // only Python payload. Interpreter binaries were rejected above;
      // bytecode caches and a site-packages tree are still refused.
      if (parts.includes('__pycache__')) offenders.push(relative);
      else if (/\.(?:pyc|pyo)$/.test(base)) offenders.push(relative);
      else if (parts.includes('site-packages')) offenders.push(relative);
      continue;
    }
    if (base.endsWith('.py') || base.endsWith('.pyc') || base.endsWith('.pyd')) offenders.push(relative);
    else if (parts.includes('__pycache__')) offenders.push(relative);
    else if (parts.includes('site-packages')) offenders.push(relative);
  }
  if (offenders.length > 0) {
    throw new Error(
      'the packaged backend contains a Python runtime/bytecode outside the FreeCAD adapter:\n  ' +
      offenders.join('\n  '),
    );
  }
}

function verifyBundledLibraries(backend, metadata) {
  const libraries = Array.isArray(metadata.sharedLibraries) ? metadata.sharedLibraries : [];
  const missing = Array.isArray(metadata.sharedLibraryMissing) ? metadata.sharedLibraryMissing : [];
  if (missing.length > 0) {
    throw new Error(
      'the staged OCCT dependency closure is incomplete: ' +
      missing.map((item) => item.name).join(', '),
    );
  }
  const libDir = path.join(backend, 'lib');
  for (const library of libraries) {
    const target = path.join(libDir, library.name);
    if (!fs.existsSync(target)) throw new Error(`bundled OCCT library is missing: ${library.name}`);
    const digest = sha256(target);
    if (digest !== library.sha256) {
      throw new Error(`bundled OCCT library digest mismatch: ${library.name}`);
    }
  }
  if (libraries.length > 0 && metadata.rpath !== '$ORIGIN/lib' && !metadata.patchelf) {
    // RPATH is optional when the launcher supplies LD_LIBRARY_PATH; the smoke
    // run below supplies it exactly like Electron does.
    console.log('[appimage-smoke] NOTE: no $ORIGIN/lib RPATH; using LD_LIBRARY_PATH fallback.');
  }
  if (libraries.length > 0 && metadata.rpath === '$ORIGIN/lib' && process.platform === 'linux') {
    // With an empty environment (no LD_LIBRARY_PATH), every bundled OCCT
    // library must resolve from resources/backend/lib through the RPATH.
    const ldd = spawnSync('ldd', [path.join(backend, metadata.binary)], {
      encoding: 'utf8',
      env: {},
      shell: false,
    });
    if (ldd.error) throw new Error(`ldd could not inspect the packaged runtime: ${ldd.error.message}`);
    const unresolved = String(ldd.stdout || '').split(/\r?\n/).filter((line) => /not found/.test(line));
    if (unresolved.length > 0) {
      throw new Error(`the packaged runtime has unresolved libraries:\n  ${unresolved.join('\n  ')}`);
    }
    const bundled = String(ldd.stdout || '').split(/\r?\n/).filter((line) => /libTK[A-Za-z0-9_]*\.so/.test(line));
    const outside = bundled.filter((line) => !line.includes(libDir));
    if (outside.length > 0) {
      throw new Error(
        'the packaged runtime resolves OCCT libraries outside resources/backend/lib:\n  ' +
        outside.join('\n  '),
      );
    }
    console.log(
      `[appimage-smoke] RPATH closure ok: ${bundled.length} libTK resolutions from ${libDir} ` +
      '(empty environment, no LD_LIBRARY_PATH)',
    );
  }
  return libraries.length;
}

function treeSha256(root) {
  const hash = crypto.createHash('sha256');
  hash.update('kala-studio-native-tree\n');
  const relativePaths = walkFiles(root)
    .map((relative) => relative.split(path.sep).join('/'))
    .sort();
  for (const relative of relativePaths) {
    const absolute = path.join(root, ...relative.split('/'));
    hash.update(relative);
    hash.update('\0');
    hash.update(String(fs.statSync(absolute).size));
    hash.update('\0');
    hash.update(sha256(absolute));
    hash.update('\n');
  }
  return hash.digest('hex');
}

function verifyAdapter(backend, metadata) {
  const adapterRoot = path.join(backend, 'adapters', 'freecad');
  const required = [
    path.join(adapterRoot, 'src', 'kala_agent_freecad', '__init__.py'),
    path.join(adapterRoot, 'mod', 'Kala', 'InitGui.py'),
    path.join(adapterRoot, 'vendor.json'),
    path.join(adapterRoot, 'src', 'kala_agent', '__init__.py'),
  ];
  for (const candidate of required) {
    if (!fs.existsSync(candidate)) throw new Error(`bundled FreeCAD adapter is incomplete: ${candidate}`);
  }
  const recorded = metadata.freeCadAdapter;
  if (recorded && typeof recorded.treeSha256 === 'string') {
    const actual = treeSha256(adapterRoot);
    if (actual !== recorded.treeSha256) {
      throw new Error('the bundled FreeCAD adapter digest does not match native-build.json');
    }
  }
  const manifestPath = path.join(adapterRoot, 'vendor.json');
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  if (manifest.schemaVersion !== 2) throw new Error('the FreeCAD vendor manifest schema is unsupported');
  if (!recorded || recorded.vendor?.state !== 'vendored') {
    throw new Error('native-build.json does not record vendored FreeCAD Python dependencies');
  }
  const supportedAbis = [...(manifest.supportedAbis || [])].sort();
  if (supportedAbis.join('\n') !== ['cp311', 'cp312', 'cp313', 'cp314'].join('\n')) {
    throw new Error(`vendor.json does not bundle the four supported ABIs: ${supportedAbis.join(', ')}`);
  }
  const recordedAbis = [...(recorded.vendor.supportedAbis || [])].sort();
  if (recordedAbis.join('\n') !== supportedAbis.join('\n')) {
    throw new Error('native-build.json and vendor.json disagree on the vendored ABI list');
  }
  let wheelCount = 0;
  for (const abi of supportedAbis) {
    const entry = manifest.abis?.[abi];
    if (!entry || entry.abi !== abi) {
      throw new Error(`vendor.json has no dependency set for ${abi}`);
    }
    wheelCount += entry.wheels.length;
    const directory = path.join(adapterRoot, `site-packages-${abi}`);
    for (const candidate of [
      path.join(directory, 'pydantic', '__init__.py'),
      path.join(directory, 'pydantic_core', '__init__.py'),
    ]) {
      if (!fs.existsSync(candidate)) {
        throw new Error(`bundled FreeCAD dependencies are incomplete: ${candidate}`);
      }
    }
    const core = walkFiles(path.join(directory, 'pydantic_core'))
      .filter((relative) => /_pydantic_core.*\.(?:so|pyd)$/i.test(relative));
    if (core.length === 0) {
      throw new Error(`the vendored pydantic-core C extension is missing for ${abi}`);
    }
    const recordedWheels = (recorded.vendor.abis?.[abi]?.wheels || [])
      .map((wheel) => `${wheel.name}:${wheel.sha256}`).sort();
    const manifestWheels = entry.wheels
      .map((wheel) => `${wheel.name}:${wheel.sha256}`).sort();
    if (recordedWheels.join('\n') !== manifestWheels.join('\n')) {
      throw new Error(`native-build.json and vendor.json disagree on the ${abi} wheels`);
    }
  }
  const interpreter = walkFiles(adapterRoot)
    .map((relative) => path.basename(relative).toLowerCase())
    .filter((name) => /^python(?:[0-9.]*)?(?:\.exe)?$/.test(name) || /^libpython/.test(name));
  if (interpreter.length > 0) {
    throw new Error(`the FreeCAD adapter bundle contains a Python interpreter: ${interpreter.join(', ')}`);
  }
  const detected = manifest.freecad?.detected
    ? `${manifest.freecad.version} / Python ${manifest.freecad.python} (${manifest.freecad.abi})`
    : 'no build-host FreeCAD';
  console.log(
    `[appimage-smoke] FreeCAD Python deps ok: ABIs ${supportedAbis.join(', ')} ` +
    `(${wheelCount} wheels, ${manifest.fileCount} files; ${detected})`,
  );
  return manifest;
}

function pickFreePort() {
  return new Promise((resolve, reject) => {
    const server = http.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      server.close(() => resolve(address.port));
    });
  });
}

function request(port, method, requestPath, {
  token, payload, bodyBytes, contentType = 'application/json', timeout = 15_000,
} = {}) {
  return new Promise((resolve, reject) => {
    const body = bodyBytes !== undefined
      ? Buffer.from(bodyBytes)
      : payload === undefined ? null : Buffer.from(JSON.stringify(payload));
    const headers = { Accept: 'application/json' };
    if (body) {
      headers['Content-Type'] = contentType;
      headers['Content-Length'] = String(body.length);
    }
    if (token) headers.Authorization = `Bearer ${token}`;
    const req = http.request(
      { host: '127.0.0.1', port, method, path: requestPath, headers, timeout },
      (response) => {
        const chunks = [];
        response.on('data', (chunk) => chunks.push(chunk));
        response.on('end', () => {
          const raw = Buffer.concat(chunks);
          let json = null;
          try { json = JSON.parse(raw.toString('utf8')); } catch { /* non-JSON */ }
          resolve({ status: response.statusCode, headers: response.headers, body: raw, json });
        });
      },
    );
    req.on('timeout', () => req.destroy(new Error(`request timed out: ${method} ${requestPath}`)));
    req.on('error', reject);
    if (body) req.write(body);
    req.end();
  });
}

async function waitForHealth(port, processHandle, timeoutSeconds, captured) {
  const deadline = Date.now() + timeoutSeconds * 1000;
  while (Date.now() < deadline) {
    if (processHandle.exitCode !== null) {
      throw new Error(
        `backend exited before health (code ${processHandle.exitCode}):\n${captured.join('\n')}`,
      );
    }
    try {
      const response = await request(port, 'GET', REQUIRED_ROUTE, { timeout: 2_000 });
      if (response.status === 200 && response.json && response.json.status === 'ok') {
        return response.json;
      }
    } catch {
      // retry
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`backend did not become healthy within ${timeoutSeconds}s:\n${captured.join('\n')}`);
}

function lBracketScript() {
  const baseSteps = [
    { op: 'sketch', profile: 'rectangle', plane: 'XY', width: 60.0, height: 45.0, x: -30.0, y: 0.0 },
    { op: 'extrude', length: 8.0 },
  ];
  for (const y of [16.25, 36.75]) {
    for (const x of [-21.75, 21.75]) {
      baseSteps.push(
        { op: 'sketch', profile: 'circle', plane: 'XY', diameter: 5.5, x, y },
        { op: 'pocket' },
      );
    }
  }
  const uprightSteps = [
    { op: 'sketch', profile: 'rectangle', plane: 'XZ', width: 60.0, height: 40.0, x: -30.0, y: 0.0 },
    { op: 'extrude', length: 8.0 },
    { op: 'sketch', profile: 'circle', plane: 'XZ', diameter: 10.0, x: 0.0, y: 24.0 },
    { op: 'pocket' },
  ];
  return {
    summary: 'packaging smoke L-bracket',
    components: [
      { name: 'base', steps: baseSteps },
      { name: 'upright', steps: uprightSteps },
    ],
    assembly: [
      { op: 'place', component: 'upright', y: 8.0 },
      { op: 'fuse', components: ['base', 'upright'] },
    ],
  };
}

function volumeMatches(volume) {
  if (typeof volume !== 'number' || !Number.isFinite(volume)) return false;
  return Math.abs(volume - L_BRACKET_VOLUME_MM3) / L_BRACKET_VOLUME_MM3 <= VOLUME_REL_TOLERANCE;
}

function findFreeCadDirectory() {
  for (const directory of String(process.env.PATH || '').split(path.delimiter)) {
    if (!directory) continue;
    for (const name of ['FreeCADCmd', 'freecadcmd']) {
      const candidate = path.join(directory, name);
      try {
        if (fs.statSync(candidate).isFile()) return directory;
      } catch {
        // keep searching
      }
    }
  }
  return null;
}

function assertBounds(actual, expected, label) {
  if (!actual || !expected) throw new Error(`${label}: no bounds were reported`);
  for (const key of ['min', 'max']) {
    const left = actual[key];
    const right = expected[key];
    if (!Array.isArray(left) || !Array.isArray(right) || left.length !== 3 || right.length !== 3) {
      throw new Error(`${label}: malformed bounds`);
    }
    for (let index = 0; index < 3; index += 1) {
      if (Math.abs(left[index] - right[index]) > BOUNDS_ABS_TOLERANCE_MM) {
        throw new Error(
          `${label}: bounds differ at ${key}[${index}]: ${left[index]} vs ${right[index]}`,
        );
      }
    }
  }
}

async function checkDrawing(port, token, designId, requireIt) {
  const response = await request(
    port, 'GET', `/api/studio/design/${designId}/drawing?views=front,top,right`,
    { token, timeout: 120_000 },
  );
  if (response.status !== 200) {
    const detail = JSON.stringify(response.json || response.body.toString('utf8')).slice(0, 400);
    if (requireIt) throw new Error(`drawing returned HTTP ${response.status}: ${detail}`);
    console.log(`[appimage-smoke] PENDING: drawing returned HTTP ${response.status}: ${detail}`);
    return false;
  }
  const views = Array.isArray(response.json?.views) ? response.json.views : [];
  const names = views.map((view) => view.name).sort();
  if (names.join(',') !== 'front,right,top') {
    throw new Error(`drawing returned views ${names.join(',')}`);
  }
  for (const view of views) {
    if (!Array.isArray(view.visible) || view.visible.length === 0) {
      throw new Error(`drawing view ${view.name} has no visible edges`);
    }
    for (const segment of view.visible) {
      if (!Array.isArray(segment.points) || segment.points.length < 2) {
        throw new Error(`drawing view ${view.name} contains a malformed segment`);
      }
    }
  }
  console.log(
    `[appimage-smoke] drawing ok: ${views.map((view) => `${view.name}(${view.visible.length})`).join(' ')}`,
  );
  return true;
}

async function checkSection(port, token, designId, requireIt) {
  const response = await request(
    port, 'GET', `/api/studio/design/${designId}/section?plane=XZ&offset=4`,
    { token, timeout: 120_000 },
  );
  if (response.status !== 200) {
    const detail = JSON.stringify(response.json || response.body.toString('utf8')).slice(0, 400);
    if (requireIt) throw new Error(`section returned HTTP ${response.status}: ${detail}`);
    console.log(`[appimage-smoke] PENDING: section returned HTTP ${response.status}: ${detail}`);
    return false;
  }
  const payload = response.json || {};
  if (payload.area_exact !== true) throw new Error('section is not an exact-area section');
  if (typeof payload.area_mm2 !== 'number'
    || Math.abs(payload.area_mm2 - SECTION_EXPECTED_AREA_MM2) > SECTION_AREA_ABS_TOLERANCE) {
    throw new Error(
      `section area ${payload.area_mm2} != ${SECTION_EXPECTED_AREA_MM2} (2400 - 25*pi)`,
    );
  }
  if (payload.outer_loop_count !== 1 || payload.inner_loop_count !== 1) {
    throw new Error(
      `section loops outer=${payload.outer_loop_count} inner=${payload.inner_loop_count}, expected 1/1`,
    );
  }
  console.log(
    `[appimage-smoke] section ok: XZ@4 area ${payload.area_mm2} mm2 (exact), ` +
    `outer ${payload.outer_loop_count}, inner ${payload.inner_loop_count}`,
  );
  return true;
}

async function checkStepRoundTrip(port, token, design, requireIt) {
  const exported = await request(port, 'POST', '/api/studio/export', {
    token,
    payload: { design_id: design.design_id, format: 'step' },
    timeout: 180_000,
  });
  if (exported.status !== 200 || !exported.body.toString('utf8').startsWith('ISO-10303-21;')) {
    const detail = JSON.stringify(exported.json || exported.body.toString('utf8')).slice(0, 400);
    if (requireIt) throw new Error(`STEP export returned HTTP ${exported.status}: ${detail}`);
    console.log(`[appimage-smoke] PENDING: STEP export returned HTTP ${exported.status}: ${detail}`);
    return false;
  }
  const expectedDigest = exported.headers['x-studio-digest'];
  const imported = await request(port, 'POST', '/api/studio/import', {
    token,
    bodyBytes: exported.body,
    contentType: 'application/step',
    timeout: 180_000,
  });
  if (imported.status !== 200) {
    const detail = JSON.stringify(imported.json || '').slice(0, 400);
    throw new Error(`STEP import returned HTTP ${imported.status}: ${detail}`);
  }
  if (expectedDigest && imported.json?.step_sha256 !== expectedDigest) {
    throw new Error('STEP import digest does not match the export digest');
  }
  if (imported.json?.solids !== design.measured?.solid_count) {
    throw new Error(
      `STEP import solids ${imported.json?.solids} != design solids ${design.measured?.solid_count}`,
    );
  }
  assertBounds(imported.json?.view?.bounds, design.measured?.bounds, 'STEP import');
  console.log(
    `[appimage-smoke] STEP import ok: ${exported.body.length} bytes, ` +
    `${imported.json.solids} solid(s), bounds ${JSON.stringify(imported.json.view.bounds)}`,
  );
  return true;
}

async function startBackend({
  binary,
  backend,
  temporaryRoot,
  label,
  pathValue,
  useLdLibraryPath,
}) {
  const dataDir = path.join(temporaryRoot, `${label}-data`);
  const homeDir = path.join(temporaryRoot, `${label}-home`);
  const xdgDir = path.join(temporaryRoot, `${label}-xdg`);
  const cwd = path.join(temporaryRoot, `${label}-cwd`);
  for (const directory of [dataDir, homeDir, xdgDir, cwd]) fs.mkdirSync(directory, { recursive: true });
  const port = await pickFreePort();
  const token = `kala-appimage-smoke-${crypto.randomBytes(16).toString('hex')}`;
  const captured = [];
  const libDir = path.join(backend, 'lib');
  const env = {
    PATH: pathValue,
    HOME: homeDir,
    XDG_DATA_HOME: xdgDir,
    KALA_STUDIO_DATA_DIR: dataDir,
    KALA_AGENT_DATA_DIR: dataDir,
    KALA_STUDIO_TOKEN: token,
    TEMP: temporaryRoot,
    TMP: temporaryRoot,
    TMPDIR: temporaryRoot,
    LC_ALL: 'C.UTF-8',
    ...(useLdLibraryPath && fs.existsSync(libDir) ? { LD_LIBRARY_PATH: libDir } : {}),
  };
  // The installed app must not see the development checkout or its virtualenv
  // through any child variable or through its working directory.
  const repositoryVirtualenv = path.join(REPO_ROOT, '.venv');
  const offenders = Object.entries(env).filter(([, value]) => (
    String(value).includes(REPO_ROOT) || String(value).includes(repositoryVirtualenv)
  ));
  if (offenders.length > 0) {
    throw new Error(
      `the ${label} backend environment references the repository: ` +
      offenders.map(([key]) => key).join(', '),
    );
  }
  if (cwd === REPO_ROOT || cwd.startsWith(`${REPO_ROOT}${path.sep}`)) {
    throw new Error(`the ${label} backend working directory is inside the repository: ${cwd}`);
  }
  if ('PYTHONPATH' in env || 'PYTHONHOME' in env || 'KALA_FREECAD_SYS_PATH' in env) {
    throw new Error(`the ${label} backend environment injects a Python path`);
  }
  const child = spawn(binary, ['--host', '127.0.0.1', '--port', String(port)], {
    cwd,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  child.stdout.on('data', (chunk) => captured.push(chunk.toString('utf8').trimEnd()));
  child.stderr.on('data', (chunk) => captured.push(chunk.toString('utf8').trimEnd()));
  console.log(
    `[appimage-smoke] ${label} backend env: PATH="${pathValue}" cwd=${cwd} ` +
    `(no repository/virtualenv path)`,
  );
  return { child, captured, port, token, cwd };
}

async function checkDesign(port, token, provider, requireIt, label, expectedMeasuredBy) {
  let response;
  try {
    response = await request(port, 'POST', '/api/studio/design', {
      token,
      payload: { provider, script: lBracketScript() },
      timeout: 180_000,
    });
  } catch (error) {
    if (requireIt) throw error;
    console.log(`[appimage-smoke] PENDING native-design: ${label} request failed: ${error.message}`);
    return null;
  }
  if (response.status !== 200) {
    const detail = response.json ? JSON.stringify(response.json).slice(0, 400) : response.body.toString('utf8').slice(0, 400);
    if (requireIt) throw new Error(`${label} returned HTTP ${response.status}: ${detail}`);
    console.log(`[appimage-smoke] PENDING native-design: ${label} returned HTTP ${response.status}: ${detail}`);
    return null;
  }
  const volume = response.json?.measured?.volume_mm3;
  if (!volumeMatches(volume)) {
    const detail = `volume ${String(volume)} (expected ${L_BRACKET_VOLUME_MM3})`;
    if (requireIt) throw new Error(`${label} measured ${detail}`);
    console.log(`[appimage-smoke] PENDING native-design: ${label} measured ${detail}`);
    return null;
  }
  if (expectedMeasuredBy && response.json?.measured_by !== expectedMeasuredBy) {
    throw new Error(`${label} was measured by ${response.json?.measured_by}, expected ${expectedMeasuredBy}`);
  }
  console.log(
    `[appimage-smoke] ${label} ok: volume ${volume} mm3, measured_by ${response.json.measured_by}, ` +
    `design_id ${String(response.json.design_id).slice(0, 12)}…`,
  );
  return response.json;
}

function stopBackend(child) {
  if (child.exitCode !== null) return Promise.resolve();
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      try { child.kill('SIGKILL'); } catch { /* ignore */ }
      resolve();
    }, 5_000);
    child.once('exit', () => {
      clearTimeout(timer);
      resolve();
    });
    try { child.kill('SIGTERM'); } catch { /* ignore */ }
  });
}

async function main(argv = process.argv.slice(2)) {
  let options;
  try {
    options = parseArguments(argv);
  } catch (error) {
    console.error(`[appimage-smoke] ${error.message}`);
    console.error(usage());
    return 2;
  }
  if (options.help) {
    console.log(usage());
    return 0;
  }

  let temporaryRoot = '';
  let child = null;
  const freecadDirectory = options.freecadDirectory || findFreeCadDirectory();
  try {
    const appimage = discoverAppImage(options.appimage);
    const size = fs.statSync(appimage).size;
    const digest = sha256(appimage);
    console.log(`[appimage-smoke] AppImage: ${appimage}`);
    console.log(`[appimage-smoke] size: ${size} bytes (${(size / (1024 * 1024)).toFixed(1)} MiB)`);
    console.log(`[appimage-smoke] sha256: ${digest}`);

    temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'kala-appimage-smoke-'));
    const extraction = spawnSync(appimage, ['--appimage-extract'], {
      cwd: temporaryRoot,
      env: { ...process.env, HOME: path.join(temporaryRoot, 'home') },
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      maxBuffer: 16 * 1024 * 1024,
      timeout: 300_000,
    });
    if (extraction.status !== 0) {
      const detail = `${extraction.stderr || ''}\n${extraction.stdout || ''}`.trim().slice(-4000);
      throw new Error(
        `AppImage extraction failed (status ${extraction.status}): ${detail}`,
      );
    }
    const backend = path.join(temporaryRoot, 'squashfs-root', 'resources', 'backend');
    const metadataPath = path.join(backend, 'native-build.json');
    if (!fs.existsSync(metadataPath)) throw new Error(`packaged metadata is missing: ${metadataPath}`);
    const metadata = JSON.parse(fs.readFileSync(metadataPath, 'utf8'));
    const binary = path.join(backend, process.platform === 'win32' ? 'kala-studio.exe' : 'kala-studio');
    if (!fs.existsSync(binary)) throw new Error(`packaged backend binary is missing: ${binary}`);
    if (typeof metadata.binarySha256 !== 'string' || sha256(binary) !== metadata.binarySha256) {
      throw new Error('the packaged kala-studio binary digest does not match native-build.json');
    }
    const libraryCount = verifyBundledLibraries(backend, metadata);
    const vendorManifest = verifyAdapter(backend, metadata);
    assertNoPythonRuntime(backend);
    console.log(
      `[appimage-smoke] payload ok: metadata schema ${metadata.schemaVersion}, ` +
      `${libraryCount} OCCT libs, FreeCAD adapter + ${vendorManifest.supportedAbis.length} ABI sets, ` +
      'no Python runtime',
    );

    // -------- phase 1: no Python on PATH at all -----------------------------
    const usingRpath = metadata.rpath === '$ORIGIN/lib';
    if (!usingRpath) {
      console.log('[appimage-smoke] NOTE: no $ORIGIN/lib RPATH; using LD_LIBRARY_PATH fallback.');
    }
    let phase = await startBackend({
      binary,
      backend,
      temporaryRoot,
      label: 'python-free',
      pathValue: '',
      useLdLibraryPath: !usingRpath,
    });
    child = phase.child;
    const health = await waitForHealth(phase.port, child, options.timeoutSeconds, phase.captured);
    console.log(`[appimage-smoke] health ok: ${JSON.stringify(health)}`);
    if (health.design_available !== true) {
      throw new Error('health reports design_available=false; the native kernel is not linked');
    }

    const ui = await request(phase.port, 'GET', '/');
    if (ui.status !== 200 || !ui.body.toString('utf8').toLowerCase().includes('<html')) {
      throw new Error(`GET / returned ${ui.status}; the packaged UI was not served`);
    }
    console.log('[appimage-smoke] packaged UI root ok');

    const refused = await request(phase.port, 'POST', '/api/studio/preview', {
      payload: { kind: 'box', parameters: { size_x: 20, size_y: 30, size_z: 40 } },
    });
    if (refused.status !== 401) {
      throw new Error(`unauthenticated preview returned ${refused.status}, expected 401`);
    }
    console.log('[appimage-smoke] unauthenticated refusal ok');

    const geometry = await checkDesign(
      phase.port, phase.token, 'opencascade', options.requireGeometry, 'OpenCascade L-bracket',
    );
    if (geometry) {
      await checkDrawing(phase.port, phase.token, geometry.design_id, true);
      await checkSection(phase.port, phase.token, geometry.design_id, true);
      await checkStepRoundTrip(phase.port, phase.token, geometry, true);
    } else {
      console.log('[appimage-smoke] geometry smoke is PENDING native-design (re-run with --require-geometry)');
    }
    await stopBackend(child);
    child = null;

    // -------- phase 2: installed FreeCAD, no repo/venv on any path ----------
    let freecadBuild = null;
    if (freecadDirectory) {
      const freecadPath = [...new Set([
        freecadDirectory,
        '/usr/bin',
        '/bin',
        '/usr/local/bin',
      ])].join(':');
      phase = await startBackend({
        binary,
        backend,
        temporaryRoot,
        label: 'freecad',
        pathValue: freecadPath,
        useLdLibraryPath: !usingRpath,
      });
      child = phase.child;
      const freecadHealth = await waitForHealth(
        phase.port, child, options.timeoutSeconds, phase.captured,
      );
      console.log(`[appimage-smoke] FreeCAD phase health ok (PATH=${freecadPath})`);
      const providers = await request(phase.port, 'GET', '/api/studio/providers/cad', {
        token: phase.token,
      });
      const freecadProvider = Array.isArray(providers.json?.providers)
        ? providers.json.providers.find((provider) => provider.id === 'freecad')
        : null;
      if (!freecadProvider || freecadProvider.available !== true) {
        throw new Error(
          `FreeCAD provider is not available in the installed app: ${JSON.stringify(freecadProvider)}`,
        );
      }
      console.log(
        `[appimage-smoke] FreeCAD provider available: version ${freecadProvider.version || 'unknown'}`,
      );
      freecadBuild = await checkDesign(
        phase.port, phase.token, 'freecad', options.requireFreecad, 'FreeCAD L-bracket', 'FreeCAD',
      );
      if (freecadBuild) {
        await checkDrawing(phase.port, phase.token, freecadBuild.design_id, true);
        await checkSection(phase.port, phase.token, freecadBuild.design_id, true);
        await checkStepRoundTrip(phase.port, phase.token, freecadBuild, true);
      }
      await stopBackend(child);
      child = null;
    } else if (options.requireFreecad) {
      throw new Error(
        'the FreeCAD smoke was required but no FreeCADCmd is installed on this host ' +
        '(FreeCAD itself is not bundled)',
      );
    } else {
      console.log('[appimage-smoke] FreeCAD is not installed on this host; FreeCAD smoke skipped');
    }

    console.log(
      `[appimage-smoke] PASS (${appimage})` +
      `${freecadBuild ? ' with FreeCAD provider' : ''}`,
    );
    return 0;
  } catch (error) {
    console.error(`[appimage-smoke] FAIL: ${error instanceof Error ? error.message : error}`);
    return 1;
  } finally {
    if (child) await stopBackend(child);
    if (temporaryRoot && !options.keep) {
      fs.rmSync(temporaryRoot, { recursive: true, force: true });
    } else if (temporaryRoot) {
      console.log(`[appimage-smoke] kept extraction: ${temporaryRoot}`);
    }
  }
}

main().then((code) => process.exit(code)).catch((error) => {
  console.error('[appimage-smoke] unexpected failure:', error);
  process.exit(1);
});
