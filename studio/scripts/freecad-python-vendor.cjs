#!/usr/bin/env node
// Vendor the FreeCAD adapter's Python dependencies for FreeCAD's OWN interpreter.
//
// The FreeCAD provider runs the adapter inside a short-lived `FreeCADCmd`
// subprocess. That interpreter is FreeCAD's bundled Python, not the repository
// virtualenv, so the adapter's imports must exist inside the packaged app:
//
//   * `kala_agent` (contracts + tools.protocol), vendored once from
//     `src/kala_agent` because it is pure Python;
//   * `pydantic` and its runtime dependencies, downloaded for every CPython ABI
//     current FreeCAD releases ship (`cp311`, `cp312`, `cp313`, `cp314`) and
//     unpacked into a separate `site-packages-cpNNN` directory per ABI.
//
// The provider hands `adapters/freecad/src` and exactly one matching
// `adapters/freecad/site-packages-cpNNN` directory to the driver as `sys.path`
// entries. It selects the directory by probing `FreeCADCmd`'s own Python, so an
// AppImage built on this host works with a FreeCAD AppImage that ships a
// different Python. No repository checkout and no virtualenv are on the runtime
// path.
//
// Everything is recorded in `adapters/freecad/vendor.json` (schema 2): the
// layout, every supported ABI, every wheel file name and SHA-256, the vendored
// package roots, per-ABI file counts/digests, the detected FreeCAD/Python
// identity and the in-FreeCAD import verification for the build host's ABI.
//
// All child processes use argument arrays and `shell: false`.

'use strict';

const { spawnSync } = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const REPO_ROOT = path.resolve(__dirname, '..', '..');
const DEFAULT_PINS_FILE = path.join(__dirname, 'freecad-python-deps.json');
const VENDOR_MANIFEST_NAME = 'vendor.json';
const SITE_PACKAGES_PREFIX = 'site-packages-';
const DETECT_MARKER = 'KALA_FREECAD_PYTHON=';
const VERIFY_MARKER = 'KALA_FREECAD_VENDOR_OK=';
const SUPPORTED_ABI_PATTERN = /^cp3[0-9]{2}t?$/;
const SHA256_HEX_PATTERN = /^[0-9a-f]{64}$/;

// Top-level entries a vendored wheel may contribute. Anything else (scripts,
// `*.data` directories, a second interpreter) is refused instead of copied.
const WHEEL_ALLOWED_TOP_LEVEL = /^[A-Za-z_][A-Za-z0-9_]*$|^[A-Za-z_][A-Za-z0-9_.-]*\.(?:dist-info|py)$/;

const DETECT_SCRIPT = [
  'import json, sys, sysconfig',
  'info = {',
  '    "version": "%d.%d" % tuple(sys.version_info[:2]),',
  '    "fullVersion": sys.version.split()[0],',
  '    "implementation": sys.implementation.name,',
  '    "cacheTag": sys.implementation.cache_tag,',
  '    "soabi": sysconfig.get_config_var("SOABI"),',
  '    "platform": sysconfig.get_platform(),',
  '    "machine": sysconfig.get_config_var("HOST_GNU_TYPE") or "",',
  '}',
  `sys.stdout.write(${JSON.stringify(DETECT_MARKER)} + json.dumps(info) + "\\n")`,
  '',
].join('\n');

function abiToVersion(abi) {
  const match = /^cp(\d)(\d+)t?$/.exec(String(abi || ''));
  return match ? `${match[1]}.${match[2]}` : null;
}

function normalizeAbi(abi) {
  const value = String(abi || '').trim().toLowerCase();
  if (!SUPPORTED_ABI_PATTERN.test(value)) {
    throw new Error(`unusable CPython ABI tag ${JSON.stringify(abi)}; expected a tag such as cp311`);
  }
  return value;
}

function normalizePythonTargets(python = {}) {
  const fallbackAbi = normalizeAbi(python.defaultAbi || 'cp314');
  const fallbackVersion = String(python.defaultVersion || abiToVersion(fallbackAbi) || '3.14');
  const raw = Array.isArray(python.supported) && python.supported.length > 0
    ? python.supported
    : [{ abi: fallbackAbi, version: fallbackVersion }];
  const targets = [];
  const seen = new Set();
  for (const entry of raw) {
    const abi = normalizeAbi(typeof entry === 'string' ? entry : entry && entry.abi);
    if (seen.has(abi)) throw new Error(`the FreeCAD dependency pins list ${abi} twice`);
    seen.add(abi);
    const version = typeof entry === 'object' && entry.version
      ? String(entry.version)
      : (abiToVersion(abi) || fallbackVersion);
    if (!/^3\.[0-9]+$/.test(version)) {
      throw new Error(`unusable Python version ${JSON.stringify(version)} for ${abi}; expected major.minor`);
    }
    targets.push({ abi, version });
  }
  targets.sort((left, right) => (left.abi < right.abi ? -1 : left.abi > right.abi ? 1 : 0));
  return targets;
}

/**
 * Validate the reviewed SHA-256 digest list for every pinned requirement.
 *
 * `pip download --require-hashes` is only as strong as this map: a missing
 * requirement or an empty list would turn the check into a no-op, so both are
 * refused here rather than at download time.
 */
function validateRequirementHashes(rawHashes, requirements, pinsFile) {
  if (!rawHashes || typeof rawHashes !== 'object' || Array.isArray(rawHashes)) {
    throw new Error(
      `FreeCAD dependency pins at ${pinsFile} must carry a "hashes" object keyed by exact requirement`,
    );
  }
  const hashes = {};
  for (const requirement of requirements) {
    const digests = rawHashes[requirement];
    if (!Array.isArray(digests) || digests.length === 0) {
      throw new Error(`FreeCAD dependency pins carry no SHA-256 hashes for ${requirement}`);
    }
    for (const digest of digests) {
      if (typeof digest !== 'string' || !SHA256_HEX_PATTERN.test(digest)) {
        throw new Error(
          `FreeCAD dependency pins carry an invalid SHA-256 for ${requirement}: ${JSON.stringify(digest)}`,
        );
      }
    }
    hashes[requirement] = [...digests];
  }
  const unknown = Object.keys(rawHashes).filter((key) => !requirements.includes(key));
  if (unknown.length > 0) {
    throw new Error(
      `FreeCAD dependency pins carry hashes for requirements that are not pinned: ${unknown.join(', ')}`,
    );
  }
  return hashes;
}

function readPins(pinsFile = DEFAULT_PINS_FILE) {
  const resolved = path.resolve(pinsFile);
  let payload;
  try {
    payload = JSON.parse(fs.readFileSync(resolved, 'utf8'));
  } catch (error) {
    throw new Error(`FreeCAD dependency pins are unreadable (${resolved}): ${error.message}`);
  }
  if (!payload || !Array.isArray(payload.requirements) || payload.requirements.length === 0) {
    throw new Error(`FreeCAD dependency pins at ${resolved} contain no requirements`);
  }
  for (const requirement of payload.requirements) {
    if (typeof requirement !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9._-]*==[^\s]+$/.test(requirement)) {
      throw new Error(
        `FreeCAD dependency pins must be exact "name==version" entries; got ${JSON.stringify(requirement)}`,
      );
    }
  }
  const targets = normalizePythonTargets(payload.python || {});
  const requirements = [...payload.requirements];
  return {
    file: resolved,
    requirements,
    hashes: validateRequirementHashes(payload.hashes, requirements, resolved),
    defaultVersion: payload.python?.defaultVersion || targets[targets.length - 1].version,
    defaultAbi: payload.python?.defaultAbi || targets[targets.length - 1].abi,
    targets,
    supportedAbis: targets.map((target) => target.abi),
    kalaAgentSource: payload.kalaAgent?.source || path.join('src', 'kala_agent'),
  };
}

function sitePackagesDirName(abi) {
  return `${SITE_PACKAGES_PREFIX}${abi}`;
}

function sha256File(filePath) {
  return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex');
}

function walkFiles(root) {
  const files = [];
  const stack = [''];
  while (stack.length > 0) {
    const relative = stack.pop();
    for (const entry of fs.readdirSync(path.join(root, relative), { withFileTypes: true })) {
      const next = relative ? path.join(relative, entry.name) : entry.name;
      if (entry.isSymbolicLink()) {
        throw new Error(`vendored dependency tree may not contain a symlink: ${path.join(root, next)}`);
      }
      if (entry.isDirectory()) stack.push(next);
      else if (entry.isFile()) files.push(next);
    }
  }
  return files.sort((left, right) => (left < right ? -1 : left > right ? 1 : 0));
}

function treeSha256(root) {
  const hash = crypto.createHash('sha256');
  hash.update('kala-freecad-python-vendor-tree\n');
  for (const relative of walkFiles(root)) {
    const absolute = path.join(root, relative);
    const key = relative.split(path.sep).join('/');
    hash.update(`${key}\0${fs.statSync(absolute).size}\0${sha256File(absolute)}\n`);
  }
  return hash.digest('hex');
}

function run(command, args, { cwd, env, timeout = 300_000 } = {}) {
  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: 'utf8',
    shell: false,
    windowsHide: true,
    timeout,
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error) {
    if (result.error.code === 'ENOENT') {
      throw new Error(`could not execute ${command}: not found`);
    }
    throw result.error;
  }
  return result;
}

/** A child environment without Python injection or credential-shaped keys. */
function cleanEnvironment(base = process.env, extra = {}) {
  const env = { ...base, ...extra };
  for (const key of ['PYTHONPATH', 'PYTHONHOME', 'PYTHONSTARTUP', 'PYTHONUSERBASE']) delete env[key];
  for (const key of Object.keys(env)) {
    const upper = key.toUpperCase();
    if (['TOKEN', 'API_KEY', 'SECRET', 'PASSWORD', 'CREDENTIAL'].some((marker) => upper.includes(marker))) {
      delete env[key];
    }
  }
  return env;
}

function executableNames(platform = process.platform) {
  return platform === 'win32'
    ? ['FreeCADCmd.exe', 'freecadcmd.exe', 'FreeCADCmd', 'freecadcmd']
    : ['FreeCADCmd', 'freecadcmd'];
}

function isExecutableFile(candidate, platform = process.platform) {
  try {
    const stat = fs.statSync(candidate);
    if (!stat.isFile()) return false;
    if (platform === 'win32') return /\.exe$/i.test(candidate);
    fs.accessSync(candidate, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

/**
 * Locate `FreeCADCmd`. Explicit executable wins, then `PATH`, then the standard
 * Windows install directories (`%ProgramFiles%\FreeCAD*\bin`).
 */
function findFreeCadExecutable({ executable, env = process.env, platform = process.platform } = {}) {
  if (executable) {
    if (!isExecutableFile(executable, platform)) {
      throw new Error(`the FreeCAD executable does not exist or is not executable: ${executable}`);
    }
    return path.resolve(executable);
  }
  const pathValue = env.PATH || env.Path || '';
  for (const directory of pathValue.split(path.delimiter)) {
    if (!directory) continue;
    for (const name of executableNames(platform)) {
      const candidate = path.join(directory, name);
      if (isExecutableFile(candidate, platform)) return candidate;
    }
  }
  if (platform === 'win32') {
    const roots = [env.ProgramFiles, env['ProgramFiles(x86)'], 'C:\\Program Files', 'C:\\Program Files (x86)']
      .filter(Boolean);
    for (const root of roots) {
      let entries = [];
      try {
        entries = fs.readdirSync(root, { withFileTypes: true });
      } catch {
        continue;
      }
      for (const entry of entries) {
        if (!entry.isDirectory() || !/^FreeCAD/i.test(entry.name)) continue;
        for (const sub of ['bin', '']) {
          for (const name of executableNames('win32')) {
            const candidate = path.join(root, entry.name, ...(sub ? [sub] : []), name);
            if (isExecutableFile(candidate, 'win32')) return candidate;
          }
        }
      }
    }
  }
  return null;
}

/**
 * Ask FreeCAD's own interpreter for its exact version and ABI.
 *
 * `FreeCADCmd -c` is not reliable on every build (it can be parsed as a
 * document argument), so the probe is a temporary script file.
 */
function detectFreeCadPython({ executable, env = process.env, platform = process.platform, timeout = 60_000 } = {}) {
  const resolved = findFreeCadExecutable({ executable, env, platform });
  if (!resolved) return null;
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'kala-freecad-probe-'));
  const script = path.join(temporary, 'kala_freecad_python_probe.py');
  const probeEnvironment = cleanEnvironment(env, {
    HOME: temporary,
    XDG_CONFIG_HOME: path.join(temporary, 'config'),
    XDG_CACHE_HOME: path.join(temporary, 'cache'),
    XDG_DATA_HOME: path.join(temporary, 'data'),
  });
  fs.mkdirSync(path.join(temporary, 'config'), { recursive: true });
  fs.mkdirSync(path.join(temporary, 'cache'), { recursive: true });
  fs.mkdirSync(path.join(temporary, 'data'), { recursive: true });
  try {
    fs.writeFileSync(script, DETECT_SCRIPT, 'utf8');
    const result = run(resolved, [script], {
      cwd: temporary,
      env: probeEnvironment,
      timeout,
    });
    const output = `${result.stdout || ''}\n${result.stderr || ''}`;
    const line = output.split(/\r?\n/).find((candidate) => candidate.startsWith(DETECT_MARKER));
    if (!line) {
      throw new Error(
        `FreeCADCmd at ${resolved} did not report its Python identity (exit ${result.status}): ` +
        `${output.trim().slice(-1200)}`,
      );
    }
    const info = JSON.parse(line.slice(DETECT_MARKER.length));
    const versionResult = run(resolved, ['--version'], {
      cwd: temporary,
      env: probeEnvironment,
      timeout,
    });
    const freecadVersion = String(versionResult.stdout || versionResult.stderr || '')
      .split(/\r?\n/)
      .map((value) => value.trim())
      .find((value) => value.length > 0) || null;
    const cacheTag = String(info.cacheTag || '');
    const abiMatch = /^cpython-(\d+)(\d*)$/.exec(cacheTag);
    if (!abiMatch) {
      throw new Error(`FreeCADCmd reported an unusable Python cache tag: ${cacheTag}`);
    }
    return {
      executable: resolved,
      freecadVersion,
      version: info.version,
      fullVersion: info.fullVersion,
      implementation: info.implementation,
      cacheTag,
      abi: `cp${abiMatch[1]}${abiMatch[2] || ''}`,
      soabi: info.soabi || null,
      platform: info.platform || null,
      machine: info.machine || null,
      detected: true,
    };
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

/** The pip wheel platform tag for the interpreter's own platform. */
function wheelPlatform({ platform = process.platform, interpreterPlatform = null } = {}) {
  const value = String(interpreterPlatform || '');
  if (platform === 'win32') {
    if (/arm64/i.test(value)) return 'win_arm64';
    if (/amd64|x86_64|win64/i.test(value)) return 'win_amd64';
    // The installer is x64-only. Only an explicitly 32-bit interpreter tag
    // selects win32; an unknown tag must not silently resolve 32-bit wheels.
    if (/^(?:win32|i[3-6]86|x86)$/i.test(value.trim())) return 'win32';
    return 'win_amd64';
  }
  if (/aarch64|arm64/i.test(value)) return 'manylinux2014_aarch64';
  if (/i[3-6]86|x86$/i.test(value)) return 'manylinux2014_i686';
  if (/armv7/i.test(value)) return 'manylinux2014_armv7l';
  return 'manylinux2014_x86_64';
}

function defaultPipPython({ env = process.env, platform = process.platform } = {}) {
  if (env.KALA_PIP_PYTHON) return env.KALA_PIP_PYTHON;
  const candidates = platform === 'win32' ? ['python', 'py'] : ['python3', 'python'];
  for (const candidate of candidates) {
    const probe = run(candidate, ['-m', 'pip', '--version'], {
      env: cleanEnvironment(env),
      timeout: 60_000,
    });
    if (probe.status === 0) return candidate;
  }
  throw new Error(
    'no pip-capable Python was found to resolve the FreeCAD wheel set; set KALA_PIP_PYTHON',
  );
}

/**
 * Write the fully pinned, fully hashed requirements file pip is given.
 *
 * Hash-checking mode rejects an index that serves different bytes for a pinned
 * version, which closes the "`PIP_INDEX_URL` points at an attacker's mirror"
 * hole: the mirror may still answer, but only the reviewed wheels are accepted.
 */
function writeHashedRequirementsFile({ requirements, hashes, destination }) {
  if (!hashes || typeof hashes !== 'object') {
    throw new Error('downloading FreeCAD wheels requires the pinned SHA-256 hashes');
  }
  const lines = requirements.map((requirement) => {
    const digests = hashes[requirement];
    if (!Array.isArray(digests) || digests.length === 0) {
      throw new Error(`no reviewed SHA-256 hashes for ${requirement}`);
    }
    return `${requirement} ${digests.map((digest) => `--hash=sha256:${digest}`).join(' ')}`;
  });
  const requirementsFile = path.join(destination, '.kala-hashed-requirements.txt');
  fs.writeFileSync(requirementsFile, `${lines.join('\n')}\n`);
  return requirementsFile;
}

/**
 * Download the pinned wheels for one interpreter identity with
 * `pip download --only-binary=:all: --require-hashes` and return their file
 * names and digests.
 */
function downloadWheels({
  requirements,
  hashes,
  pythonVersion,
  abi,
  wheelTag,
  destination,
  pipPython,
  env = process.env,
  timeout = 600_000,
  runner = run,
}) {
  fs.mkdirSync(destination, { recursive: true });
  const requirementsFile = writeHashedRequirementsFile({ requirements, hashes, destination });
  const args = [
    '-m', 'pip',
    '--disable-pip-version-check',
    '--no-input',
    'download',
    '--only-binary=:all:',
    '--require-hashes',
    '--python-version', pythonVersion,
    '--implementation', 'cp',
    '--abi', abi,
    '--platform', wheelTag,
    '--progress-bar', 'off',
    '--dest', destination,
    '-r', requirementsFile,
  ];
  const result = runner(pipPython, args, { env: cleanEnvironment(env), timeout });
  if (result.status !== 0) {
    throw new Error(
      `pip download for Python ${pythonVersion}/${abi}/${wheelTag} failed (exit ${result.status}): ` +
      `${String(result.stderr || result.stdout || '').trim().slice(-2000)}`,
    );
  }
  const wheels = fs.readdirSync(destination)
    .filter((name) => name.toLowerCase().endsWith('.whl'))
    .sort();
  if (wheels.length === 0) {
    throw new Error(`pip download produced no wheels in ${destination}`);
  }
  for (const name of wheels) {
    if (!name.includes(`-${abi}-`) && !name.includes(`-py3-`) && !name.includes('-py2.py3-')) {
      throw new Error(
        `pip resolved ${name} for an unexpected ABI; expected ${abi} or a pure-Python wheel`,
      );
    }
  }
  return wheels.map((name) => {
    const absolute = path.join(destination, name);
    return { name, bytes: fs.statSync(absolute).size, sha256: sha256File(absolute), path: absolute };
  });
}

function unpackWheel({ wheel, pipPython, env = process.env, timeout = 120_000 }) {
  const target = fs.mkdtempSync(path.join(os.tmpdir(), 'kala-wheel-'));
  const result = run(pipPython, ['-m', 'zipfile', '-e', wheel.path, target], {
    env: cleanEnvironment(env),
    timeout,
  });
  if (result.status !== 0) {
    fs.rmSync(target, { recursive: true, force: true });
    throw new Error(
      `could not unpack wheel ${wheel.name}: ${String(result.stderr || '').trim() || `exit ${result.status}`}`,
    );
  }
  return target;
}

/**
 * Copy one unpacked wheel into the vendor root.
 *
 * `.dist-info` metadata is kept (importlib.metadata may be used by a future
 * adapter version); scripts and `*.data` trees are refused because the AppImage
 * must not gain executables.
 */
function installUnpackedWheel({ unpacked, wheel, destination }) {
  const roots = new Set();
  for (const entry of fs.readdirSync(unpacked, { withFileTypes: true })) {
    if (entry.name.endsWith('.data') || entry.name === 'bin' || entry.name === 'Scripts') {
      throw new Error(`wheel ${wheel.name} contains executable data (${entry.name}); refusing`);
    }
    if (!WHEEL_ALLOWED_TOP_LEVEL.test(entry.name)) {
      throw new Error(`wheel ${wheel.name} contains an unexpected top-level entry: ${entry.name}`);
    }
    roots.add(entry.name);
    copyTree(path.join(unpacked, entry.name), path.join(destination, entry.name));
  }
  return [...roots];
}

function copyTree(source, destination) {
  if (fs.existsSync(destination)) {
    throw new Error(`refusing to vendor over an existing path: ${destination}`);
  }
  const stat = fs.lstatSync(source);
  if (stat.isSymbolicLink()) {
    throw new Error(`refusing to vendor a symlink: ${source}`);
  }
  if (stat.isDirectory()) {
    fs.mkdirSync(destination, { recursive: true });
    for (const entry of fs.readdirSync(source, { withFileTypes: true })) {
      copyTree(path.join(source, entry.name), path.join(destination, entry.name));
    }
    return;
  }
  if (!stat.isFile()) {
    throw new Error(`refusing to vendor a non-file entry: ${source}`);
  }
  if (['.pyc', '.pyo'].includes(path.extname(source).toLowerCase())) return;
  fs.copyFileSync(source, destination);
}

/** Vendor `src/kala_agent` (Python sources only) as a top-level package. */
function vendorKalaAgent({ sourceRoot, destination }) {
  const resolved = path.resolve(sourceRoot);
  if (!fs.existsSync(path.join(resolved, '__init__.py'))) {
    throw new Error(`the kala_agent package is missing at ${resolved}`);
  }
  const target = path.join(destination, 'kala_agent');
  if (fs.existsSync(target)) {
    throw new Error(`refusing to vendor kala_agent over an existing path: ${target}`);
  }
  let fileCount = 0;
  const stack = [''];
  while (stack.length > 0) {
    const relative = stack.pop();
    for (const entry of fs.readdirSync(path.join(resolved, relative), { withFileTypes: true })) {
      const next = relative ? path.join(relative, entry.name) : entry.name;
      if (entry.isSymbolicLink()) {
        throw new Error(`the kala_agent package contains a symlink: ${path.join(resolved, next)}`);
      }
      if (entry.isDirectory()) {
        if (entry.name === '__pycache__') continue;
        stack.push(next);
        continue;
      }
      if (!entry.isFile()) continue;
      if (path.extname(entry.name).toLowerCase() !== '.py') continue;
      const destinationPath = path.join(target, next);
      fs.mkdirSync(path.dirname(destinationPath), { recursive: true });
      fs.copyFileSync(path.join(resolved, next), destinationPath);
      fileCount += 1;
    }
  }
  if (fileCount === 0) {
    throw new Error(`the kala_agent package at ${resolved} contained no Python modules`);
  }
  return {
    path: 'kala_agent',
    source: resolved,
    files: fileCount,
    treeSha256: treeSha256(target),
  };
}

/**
 * Import the adapter and its vendored dependencies inside FreeCAD's actual
 * interpreter. Proves the wheel set for that interpreter's ABI is importable
 * before it is packaged.
 */
function verifyFreeCadImports({
  executable,
  adapterSrc,
  sitePackages,
  env = process.env,
  timeout = 120_000,
}) {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'kala-freecad-verify-'));
  const script = path.join(temporary, 'kala_freecad_vendor_verify.py');
  try {
    const payload = [
      'import json, sys',
      `sys.path.insert(0, ${JSON.stringify(sitePackages)})`,
      `sys.path.insert(0, ${JSON.stringify(adapterSrc)})`,
      'import pydantic',
      'import kala_agent.contracts',
      'import kala_agent.tools.protocol as protocol',
      'import kala_agent_freecad.worker',
      `sys.stdout.write(${JSON.stringify(VERIFY_MARKER)} + json.dumps({`,
      '    "pydantic": pydantic.VERSION,',
      '    "protocol": protocol.PROTOCOL_VERSION,',
      '    "python": sys.version.split()[0],',
      '    "cacheTag": sys.implementation.cache_tag,',
      '}) + "\\n")',
      '',
    ].join('\n');
    fs.writeFileSync(script, payload, 'utf8');
    const verifyEnvironment = cleanEnvironment(env, {
      HOME: temporary,
      PYTHONDONTWRITEBYTECODE: '1',
      XDG_CONFIG_HOME: path.join(temporary, 'config'),
      XDG_CACHE_HOME: path.join(temporary, 'cache'),
      XDG_DATA_HOME: path.join(temporary, 'data'),
    });
    for (const directory of ['config', 'cache', 'data']) {
      fs.mkdirSync(path.join(temporary, directory), { recursive: true });
    }
    const result = run(executable, [script], {
      cwd: temporary,
      env: verifyEnvironment,
      timeout,
    });
    const output = `${result.stdout || ''}\n${result.stderr || ''}`;
    const line = output.split(/\r?\n/).find((candidate) => candidate.startsWith(VERIFY_MARKER));
    if (!line) {
      throw new Error(
        `the vendored FreeCAD dependencies could not be imported by ${executable} ` +
        `(exit ${result.status}): ${output.trim().slice(-2000)}`,
      );
    }
    return JSON.parse(line.slice(VERIFY_MARKER.length));
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

function pruneBytecode(root) {
  let removed = 0;
  const stack = [''];
  while (stack.length > 0) {
    const relative = stack.pop();
    for (const entry of fs.readdirSync(path.join(root, relative), { withFileTypes: true })) {
      const next = relative ? path.join(relative, entry.name) : entry.name;
      const absolute = path.join(root, next);
      if (entry.isDirectory()) {
        if (entry.name === '__pycache__') {
          fs.rmSync(absolute, { recursive: true, force: true });
          removed += 1;
          continue;
        }
        stack.push(next);
        continue;
      }
      if (/\.(?:pyc|pyo)$/i.test(entry.name)) {
        fs.rmSync(absolute, { force: true });
        removed += 1;
      }
    }
  }
  return removed;
}

/**
 * Resolve the interpreter targets to vendor.
 *
 * The default is every supported ABI from the pins file. An explicit
 * `pythonVersion`/`abi` restricts the set to one interpreter; this is a
 * development aid, not the packaged default.
 */
function resolvePythonTargets(pins, { pythonVersion = null, abi = null } = {}) {
  if (!pythonVersion && !abi) return pins.targets;
  const resolvedAbi = abi ? normalizeAbi(abi) : `cp${String(pythonVersion).replace('.', '')}`;
  normalizeAbi(resolvedAbi);
  const resolvedVersion = pythonVersion || abiToVersion(resolvedAbi);
  if (!resolvedVersion || !/^3\.[0-9]+$/.test(String(resolvedVersion))) {
    throw new Error(
      `cannot derive a Python version for ${resolvedAbi}; pass both --freecad-python-version and --freecad-abi`,
    );
  }
  return [{ abi: resolvedAbi, version: String(resolvedVersion) }];
}

/**
 * Vendor the adapter's Python dependencies for every supported ABI.
 *
 * Wheel packages are staged into `<adapter>/site-packages-<abi>`; the
 * pure-Python `kala_agent` package is staged once under `<adapter>/src`.
 *
 * Returns the manifest that is written to `<adapter>/vendor.json` and is
 * mirrored into `native-build.json`.
 */
function vendorFreeCadDependencies({
  adapterRoot,
  pinsFile = DEFAULT_PINS_FILE,
  freecadExecutable = null,
  platform = process.platform,
  env = process.env,
  pipPython = null,
  pythonVersion = null,
  abi = null,
  wheelTag = null,
  verify = true,
} = {}) {
  if (!adapterRoot) throw new Error('vendorFreeCadDependencies requires an adapterRoot');
  const pins = readPins(pinsFile);
  const targets = resolvePythonTargets(pins, { pythonVersion, abi });
  const srcRoot = path.join(adapterRoot, 'src');
  if (!fs.existsSync(path.join(srcRoot, 'kala_agent_freecad', '__init__.py'))) {
    throw new Error(`the FreeCAD adapter sources are missing under ${srcRoot}`);
  }

  let detected = null;
  try {
    detected = detectFreeCadPython({ executable: freecadExecutable, env, platform });
  } catch (error) {
    if (freecadExecutable) throw error;
    detected = null;
    console.warn(`[freecad-vendor] FreeCAD detection failed; vendoring every pinned ABI: ${error.message}`);
  }
  const resolvedWheelTag = wheelTag || wheelPlatform({
    platform,
    interpreterPlatform: detected?.platform,
  });
  const resolvedPip = pipPython || defaultPipPython({ env, platform });
  const supportedAbis = targets.map((target) => target.abi);
  const detectedUnsupported = Boolean(detected && !supportedAbis.includes(detected.abi));

  const unpackedDirs = [];
  try {
    const abis = {};
    let dependencyFileCount = 0;
    for (const target of targets) {
      const destination = path.join(adapterRoot, sitePackagesDirName(target.abi));
      if (fs.existsSync(destination)) {
        throw new Error(`refusing to vendor ${target.abi} over an existing path: ${destination}`);
      }
      const downloadDir = fs.mkdtempSync(
        path.join(os.tmpdir(), `kala-freecad-wheels-${target.abi}-`),
      );
      try {
        const wheels = downloadWheels({
          requirements: pins.requirements,
          hashes: pins.hashes,
          pythonVersion: target.version,
          abi: target.abi,
          wheelTag: resolvedWheelTag,
          destination: downloadDir,
          pipPython: resolvedPip,
          env,
        });

        const roots = new Set();
        for (const wheel of wheels) {
          const unpacked = unpackWheel({ wheel, pipPython: resolvedPip, env });
          unpackedDirs.push(unpacked);
          for (const root of installUnpackedWheel({ unpacked, wheel, destination })) {
            roots.add(root);
          }
        }
        const prunedBytecodeEntries = pruneBytecode(destination);
        const fileCount = walkFiles(destination).length;
        dependencyFileCount += fileCount;
        abis[target.abi] = {
          python: target.version,
          abi: target.abi,
          wheelPlatform: resolvedWheelTag,
          wheels: wheels.map((wheel) => ({
            name: wheel.name,
            bytes: wheel.bytes,
            sha256: wheel.sha256,
          })),
          packages: [...roots].sort(),
          prunedBytecodeEntries,
          fileCount,
          sourceDigest: treeSha256(destination),
        };
      } finally {
        fs.rmSync(downloadDir, { recursive: true, force: true });
      }
    }

    const kalaAgent = vendorKalaAgent({
      sourceRoot: path.join(REPO_ROOT, pins.kalaAgentSource),
      destination: srcRoot,
    });
    dependencyFileCount += kalaAgent.files;

    let verification = null;
    let verificationSkippedReason = null;
    if (detected && detectedUnsupported) {
      verificationSkippedReason =
        `FreeCADCmd at ${detected.executable} runs Python ${detected.fullVersion} ` +
        `(${detected.abi}), which is not one of the vendored ABIs (${supportedAbis.join(', ')}). ` +
        'The bundled adapter dependencies cannot load in that interpreter.';
      console.warn(`[freecad-vendor] ${verificationSkippedReason}`);
    } else if (verify && detected) {
      verification = verifyFreeCadImports({
        executable: detected.executable,
        adapterSrc: srcRoot,
        sitePackages: path.join(adapterRoot, sitePackagesDirName(detected.abi)),
        env,
      });
    }
    // FreeCAD's embedded interpreter can ignore PYTHONDONTWRITEBYTECODE while
    // importing the adapter and kala_agent. Bytecode never belongs in the
    // stage; prune it after the verification import.
    const prunedSourceBytecodeEntries = pruneBytecode(adapterRoot);
    const prunedBytecodeEntries = prunedSourceBytecodeEntries
      + Object.values(abis).reduce(
        (total, entry) => total + (entry.prunedBytecodeEntries || 0),
        0,
      );

    const manifest = {
      schemaVersion: 2,
      kind: 'staged-vendor',
      generatedBy: 'studio/scripts/freecad-python-vendor.cjs',
      generatedAt: new Date().toISOString(),
      layout: `${SITE_PACKAGES_PREFIX}{abi}`,
      platform: platform === 'win32' ? 'win_amd64' : resolvedWheelTag,
      wheelPlatform: resolvedWheelTag,
      supportedAbis,
      freecad: detected
        ? {
            executable: detected.executable,
            version: detected.freecadVersion,
            python: detected.fullVersion,
            abi: detected.abi,
            soabi: detected.soabi,
            cacheTag: detected.cacheTag,
            platform: detected.platform,
            detected: true,
            abiSupported: !detectedUnsupported,
          }
        : {
            executable: null,
            version: null,
            python: null,
            abi: null,
            soabi: null,
            cacheTag: null,
            platform: resolvedWheelTag,
            detected: false,
            abiSupported: null,
          },
      abis,
      requirements: pins.requirements,
      kalaAgent,
      verification,
      verificationSkippedReason,
      detectedUnsupported: detectedUnsupported ? freecadManifestIdentity(detected) : null,
      prunedBytecodeEntries,
      fileCount: dependencyFileCount,
      sourceDigest: treeSha256(adapterRoot),
    };
    fs.writeFileSync(
      path.join(adapterRoot, VENDOR_MANIFEST_NAME),
      `${JSON.stringify(manifest, null, 2)}\n`,
      'utf8',
    );
    return manifest;
  } finally {
    for (const directory of unpackedDirs) {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  }
}

function freecadManifestIdentity(detected) {
  return {
    executable: detected.executable,
    version: detected.freecadVersion,
    python: detected.fullVersion,
    abi: detected.abi,
  };
}

/**
 * Resolve the pinned wheels for every supported ABI on one or more platforms
 * without unpacking them.
 *
 * This writes the repository-level `adapters/freecad/vendor.json` matrix: the
 * exact wheel names, byte counts and SHA-256 values the Linux and Windows
 * staging runs resolve. Staged builds keep a stricter per-platform manifest of
 * the same shape under `<stage>/adapters/freecad/vendor.json`.
 */
function buildDependencyMatrix({
  pinsFile = DEFAULT_PINS_FILE,
  pipPython = null,
  platforms = ['linux', 'win32'],
  env = process.env,
  wheelTags = {},
} = {}) {
  const pins = readPins(pinsFile);
  const resolvedPip = pipPython || defaultPipPython({ env });
  const targets = {};
  for (const platform of platforms) {
    const wheelTag = wheelTags[platform] || wheelPlatform({ platform });
    const key = platform === 'win32' ? 'win_amd64' : 'linux-x86_64';
    const abis = {};
    for (const target of pins.targets) {
      const downloadDir = fs.mkdtempSync(
        path.join(os.tmpdir(), `kala-freecad-matrix-${target.abi}-`),
      );
      try {
        const wheels = downloadWheels({
          requirements: pins.requirements,
          hashes: pins.hashes,
          pythonVersion: target.version,
          abi: target.abi,
          wheelTag,
          destination: downloadDir,
          pipPython: resolvedPip,
          env,
        });
        abis[target.abi] = {
          python: target.version,
          abi: target.abi,
          wheelPlatform: wheelTag,
          wheels: wheels.map((wheel) => ({
            name: wheel.name,
            bytes: wheel.bytes,
            sha256: wheel.sha256,
          })),
        };
      } finally {
        fs.rmSync(downloadDir, { recursive: true, force: true });
      }
    }
    targets[key] = { platform: key, wheelPlatform: wheelTag, abis };
  }
  const manifest = {
    schemaVersion: 2,
    kind: 'dependency-matrix',
    generatedBy: 'studio/scripts/freecad-python-vendor.cjs',
    generatedAt: new Date().toISOString(),
    layout: `${SITE_PACKAGES_PREFIX}{abi}`,
    supportedAbis: pins.supportedAbis,
    requirements: pins.requirements,
    targets,
  };
  const destination = path.join(REPO_ROOT, 'adapters', 'freecad', VENDOR_MANIFEST_NAME);
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.writeFileSync(destination, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
  manifest.file = destination;
  return manifest;
}

function usage() {
  return [
    'Usage: node scripts/freecad-python-vendor.cjs --matrix',
    '',
    'Resolve the pinned FreeCAD adapter wheels for cp311-cp314 on both Linux',
    '(manylinux2014_x86_64) and Windows (win_amd64) and write the digest matrix',
    'to adapters/freecad/vendor.json. This does not unpack or stage anything;',
    '`scripts/native-backend.cjs` performs the per-platform staging.',
  ].join('\n');
}

function main(argv = process.argv.slice(2)) {
  if (argv.length === 1 && (argv[0] === '--help' || argv[0] === '-h')) {
    console.log(usage());
    return 0;
  }
  if (argv.length !== 1 || argv[0] !== '--matrix') {
    console.error(usage());
    return 2;
  }
  try {
    const matrix = buildDependencyMatrix();
    const wheelCount = Object.values(matrix.targets)
      .reduce((total, target) => total
        + Object.values(target.abis)
          .reduce((count, entry) => count + entry.wheels.length, 0), 0);
    console.log(
      `[freecad-vendor] wrote ${matrix.file} (${matrix.supportedAbis.join(', ')}, ` +
      `${Object.keys(matrix.targets).join(' + ')}, ${wheelCount} wheels)`,
    );
    return 0;
  } catch (error) {
    console.error(`[freecad-vendor] ${error.message}`);
    return 1;
  }
}

if (require.main === module) {
  process.exit(main());
}

module.exports = {
  DEFAULT_PINS_FILE,
  VENDOR_MANIFEST_NAME,
  SITE_PACKAGES_PREFIX,
  DETECT_SCRIPT,
  readPins,
  normalizePythonTargets,
  resolvePythonTargets,
  validateRequirementHashes,
  writeHashedRequirementsFile,
  sitePackagesDirName,
  cleanEnvironment,
  findFreeCadExecutable,
  detectFreeCadPython,
  wheelPlatform,
  defaultPipPython,
  downloadWheels,
  unpackWheel,
  installUnpackedWheel,
  vendorKalaAgent,
  verifyFreeCadImports,
  vendorFreeCadDependencies,
  buildDependencyMatrix,
  main,
  treeSha256,
  sha256File,
  walkFiles,
};
