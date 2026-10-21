#!/usr/bin/env node
// Build and stage the native Kala Studio backend for desktop packaging.
//
// This helper builds exactly one package (`kala-studio`) from the root Cargo
// workspace and copies the reviewed release binary plus the Vite UI into a
// native-only staging directory:
//
// Then stages the exact OCCT shared libraries the binary links (Linux: `ldd`,
// relocated with `patchelf` to `$ORIGIN/lib`) and the FreeCAD adapter sources
// plus workbench addon under `adapters/freecad/`. The adapter is discovered
// from the executable's ancestors, so a packaged app needs no repository
// checkout. FreeCAD itself is not bundled.
//
//   studio/backend-native/
//     kala-studio[.exe]      native runtime
//     studio_dist/           static UI, resolved beside the executable
//     lib/                  bundled OCCT libraries (Linux, when linked)
//     adapters/freecad/     adapter modules + FreeCAD workbench addon
//     native-build.json      digests, ownership and toolchain metadata
//
// The existing Python/PyInstaller staging directory (`studio/backend/`) is
// intentionally not touched. The native stage never contains a Python
// interpreter, site-packages, OCP, or any vendor Python tree.
//
// Staging safety:
//   * The destination is either the dedicated `studio/backend-native/`
//     directory, a new directory, an empty custom directory, or a previously
//     helper-owned stage whose metadata is bound to that exact path.
//   * A non-empty unmanaged/custom/project/control directory is refused
//     without deleting anything.
//   * The replacement is built and validated in a sibling temporary
//     directory and swapped in only after hashing/validation succeeds, so a
//     bad replacement, missing argument or I/O failure preserves the prior
//     good stage.
//   * The destination is snapshotted (device/inode + recursive entry
//     digest) before staging. Immediately before the swap it must still be
//     the exact same state. Anything created, replaced or modified while the
//     candidate was being hashed is left untouched and the stage is refused.
//     The directory moved to the backup path is re-verified after the move;
//     the backup is only recursively removed when it still matches the
//     authorized snapshot.
//   * Symlinked UI roots/components/assets and symlinked stage paths or
//     ancestors are refused before traversal/copy.
//
// All child processes are spawned with argument arrays and `shell: false` so
// Windows paths and spaces never pass through a shell or an npm `.cmd` shim.

'use strict';

const { spawn, spawnSync } = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const freecadVendor = require('./freecad-python-vendor.cjs');
const readiness = require('./native-readiness.cjs');

const STUDIO_ROOT = path.resolve(__dirname, '..');
const REPO_ROOT = path.resolve(STUDIO_ROOT, '..');
const BACKEND_ROOT = path.join(REPO_ROOT, 'backend');
const DEFAULT_STAGE_DIR = path.join(STUDIO_ROOT, 'backend-native');
const DEFAULT_DIST_DIR = path.join(STUDIO_ROOT, 'dist');
const PACKAGE_NAME = 'kala-studio';
const BINARY_STEM = 'kala-studio';
const METADATA_NAME = 'native-build.json';
const STATIC_DIR_NAME = 'studio_dist';
const SHARED_LIB_DIR_NAME = 'lib';
const ADAPTER_DIR_NAME = 'adapters';
const FREECAD_SRC_RELATIVE = path.join('freecad', 'src', 'kala_agent_freecad');
const FREECAD_MOD_RELATIVE = path.join('freecad', 'mod', 'Kala');
const FREECAD_VENDOR_MANIFEST = freecadVendor.VENDOR_MANIFEST_NAME;
// The marker the staged add-on prints when it imported itself inside
// FreeCAD's own interpreter.
const FREECAD_ADDON_VERIFY_MARKER = 'KALA_FREECAD_ADDON_OK ';
// Vendored runtime dependencies of the adapter, resolved for every supported
// FreeCAD Python by `freecad-python-vendor.cjs`. Each ABI owns one
// `adapters/freecad/site-packages-cpNNN` directory; the provider selects the
// single directory matching FreeCADCmd's own Python and hands it to the driver.
const FREECAD_SITE_PACKAGES_PREFIX = freecadVendor.SITE_PACKAGES_PREFIX;
// The CPython ABIs current FreeCAD releases ship. The vendoring helper writes
// one directory per entry; staging refuses any other ABI directory.
const FREECAD_PYTHON_ABIS = ['cp311', 'cp312', 'cp313', 'cp314'];
const STAGE_OWNER = 'kala-studio-native-stage';
const HELPER_ID = 'studio/scripts/native-backend.cjs';
const TREE_HASH_PREFIX = 'kala-studio-native-tree\n';
const ORIGIN_LIB_RPATH = '$ORIGIN/lib';
const ORIGIN_RPATH = '$ORIGIN';
const ADAPTER_EXCLUDED_DIRECTORIES = new Set(['__pycache__', '.mypy_cache', '.pytest_cache']);
const ADAPTER_EXCLUDED_EXTENSIONS = new Set(['.pyc', '.pyo', '.pyd']);
const MAX_BUNDLED_LIBRARIES = 256;
const MAX_BUNDLED_LIBRARY_BYTES = 512 * 1024 * 1024;
const SYSTEM_LIBRARY_PREFIXES = [
  '/lib/',
  '/lib64/',
  '/usr/lib/',
  '/usr/lib64/',
];

const FORBIDDEN_EXTENSIONS = new Set(['.py', '.pyc', '.pyo', '.pyd']);
const FORBIDDEN_DIRECTORY_NAMES = new Set([
  '__pycache__',
  'site-packages',
  'dist-info',
  'egg-info',
  'pip',
  'setuptools',
  'pyinstaller',
  'pydantic',
  'fastapi',
  'uvicorn',
  'cadquery',
  'ocp',
  'freecad',
  'numpy',
  'python',
]);
const FORBIDDEN_FILE_PATTERNS = [
  /^python(?:[0-9.]*)?(?:\.exe)?$/i,
  /^python[0-9.]*\.(?:dll|so|dylib|pyd)$/i,
  /^_?MEIPASS/i,
  /^libpython/i,
  /^ocp[0-9._-]*\.(?:dll|so|dylib)$/i,
];

function nativeBinaryName({ platform = process.platform, target } = {}) {
  const windows = target ? /windows/i.test(target) : platform === 'win32';
  return windows ? `${BINARY_STEM}.exe` : BINARY_STEM;
}

function cargoTargetDirectory({ backendRoot = BACKEND_ROOT, targetDir } = {}) {
  return path.resolve(targetDir || path.join(backendRoot, 'target'));
}

function cargoBinaryPath({ backendRoot = BACKEND_ROOT, targetDir, target, profile = 'release' } = {}) {
  const root = cargoTargetDirectory({ backendRoot, targetDir });
  const targetPart = target ? [target] : [];
  return path.join(root, ...targetPart, profile, nativeBinaryName({ target }));
}

function buildCargoArguments({
  backendRoot = BACKEND_ROOT,
  targetDir,
  target,
  noDefaultFeatures = false,
  features,
} = {}) {
  const args = [
    'build',
    '--manifest-path', path.join(backendRoot, 'Cargo.toml'),
    '--package', PACKAGE_NAME,
    '--release',
    '--locked',
    '--target-dir', cargoTargetDirectory({ backendRoot, targetDir }),
  ];
  if (target) args.push('--target', target);
  if (noDefaultFeatures) args.push('--no-default-features');
  if (features) args.push('--features', features);
  return args;
}

function nativePaths({ studioRoot = STUDIO_ROOT, stageDir = DEFAULT_STAGE_DIR, platform, target } = {}) {
  const resolvedStage = path.resolve(stageDir);
  return {
    studioRoot: path.resolve(studioRoot),
    stageDir: resolvedStage,
    binaryName: nativeBinaryName({ platform, target }),
    binaryPath: path.join(resolvedStage, nativeBinaryName({ platform, target })),
    staticDir: path.join(resolvedStage, STATIC_DIR_NAME),
    indexFile: path.join(resolvedStage, STATIC_DIR_NAME, 'index.html'),
    metadataPath: path.join(resolvedStage, METADATA_NAME),
    libDir: path.join(resolvedStage, SHARED_LIB_DIR_NAME),
    adapterDir: path.join(resolvedStage, ADAPTER_DIR_NAME),
  };
}

function lstatOrNull(candidate) {
  try {
    return fs.lstatSync(candidate);
  } catch (error) {
    if (error && error.code === 'ENOENT') return null;
    throw error;
  }
}

function samePath(left, right) {
  const first = path.resolve(left);
  const second = path.resolve(right);
  return process.platform === 'win32'
    ? first.toLowerCase() === second.toLowerCase()
    : first === second;
}

function isInside(parent, candidate) {
  const relative = path.relative(path.resolve(parent), path.resolve(candidate));
  return relative === '' || (!relative.startsWith('..') && !path.isAbsolute(relative));
}

function lstatBigIntOrNull(candidate) {
  try {
    return fs.lstatSync(candidate, { bigint: true });
  } catch (error) {
    if (error && error.code === 'ENOENT') return null;
    throw error;
  }
}

/**
 * Digest every entry in a stage directory without following symlinks.
 *
 * The digest binds the directory identity the helper validated to the exact
 * state it later swaps away. It includes relative paths, entry types, sizes,
 * nanosecond mtimes and the content of small files (the ownership metadata
 * and UI entry points). Large binary/UI content is additionally bound by
 * `verifyStageIntegrity` before an existing stage is moved.
 */
function directoryStateDigest(root) {
  const hash = crypto.createHash('sha256');
  hash.update('kala-studio-native-stage-state\n');
  const stack = [''];
  try {
    while (stack.length > 0) {
      const relative = stack.pop();
      const directory = path.join(root, relative);
      const entries = fs.readdirSync(directory, { withFileTypes: true })
        .sort((left, right) => (left.name < right.name ? -1 : left.name > right.name ? 1 : 0));
      for (const entry of entries) {
        const next = relative ? path.join(relative, entry.name) : entry.name;
        const absolute = path.join(root, next);
        const stat = fs.lstatSync(absolute, { bigint: true });
        if (stat.isSymbolicLink()) {
          throw new Error(`native distribution may not contain a symlink: ${absolute}`);
        }
        const key = next.split(path.sep).join('/');
        if (stat.isDirectory()) {
          hash.update(`D\0${key}\0${stat.mtimeNs}\n`);
          stack.push(next);
        } else if (stat.isFile()) {
          hash.update(`F\0${key}\0${stat.size}\0${stat.mtimeNs}\0`);
          if (stat.size <= 1024n * 1024n) {
            hash.update(crypto.createHash('sha256').update(fs.readFileSync(absolute)).digest('hex'));
          }
          hash.update('\n');
        } else {
          hash.update(`O\0${key}\0${stat.mtimeNs}\n`);
        }
      }
    }
  } catch (error) {
    if (error && error.code === 'ENOENT') {
      throw new Error(`stage directory changed while it was being inspected: ${root}`);
    }
    throw error;
  }
  return hash.digest('hex');
}

function captureStageState(stageDir) {
  const resolved = path.resolve(stageDir);
  const stat = lstatBigIntOrNull(resolved);
  if (!stat) {
    return { exists: false, identity: null, stateSha256: null, nonempty: false };
  }
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    throw new Error(`refusing to stage: destination is not a directory: ${resolved}`);
  }
  const entries = fs.readdirSync(resolved);
  return {
    exists: true,
    identity: `${stat.dev}:${stat.ino}`,
    stateSha256: directoryStateDigest(resolved),
    nonempty: entries.length > 0,
  };
}

function sameStageState(left, right) {
  if (left.exists !== right.exists) return false;
  if (!left.exists) return true;
  return left.identity === right.identity && left.stateSha256 === right.stateSha256;
}

function describeStageState(original, current) {
  if (original.exists !== current.exists) {
    return original.exists ? 'disappeared or was replaced' : 'appeared';
  }
  if (original.identity !== current.identity) return 'was replaced';
  return 'was modified';
}

function assertStageStateUnchanged(original, current, destination) {
  if (sameStageState(original, current)) return;
  throw new Error(
    `refusing to stage: ${destination} ${describeStageState(original, current)} while the ` +
    'replacement was being built. Any concurrent directory and its files were left untouched.',
  );
}

function assertNoSymlinkAncestors(target, label) {
  let current = path.resolve(target);
  while (true) {
    const stat = lstatOrNull(current);
    if (stat && stat.isSymbolicLink()) {
      throw new Error(
        `refusing ${label}: ${current} is a symlink; symlinked source/staging paths are not supported`,
      );
    }
    const parent = path.dirname(current);
    if (parent === current) return;
    current = parent;
  }
}

function assertSafeStageDir(stageDir, { studioRoot = STUDIO_ROOT, repoRoot = REPO_ROOT } = {}) {
  const resolved = path.resolve(stageDir);
  const studio = path.resolve(studioRoot);
  const repo = path.resolve(repoRoot);
  const defaultStage = path.join(studio, 'backend-native');

  if (samePath(resolved, path.parse(resolved).root)) {
    throw new Error('refusing to stage into a filesystem root');
  }
  if (!samePath(resolved, defaultStage) && isInside(repo, resolved)) {
    throw new Error(
      `refusing to stage into protected path ${resolved}; the repository is only allowed to ` +
      `use its dedicated ${defaultStage} stage, and custom stages must be new/empty directories outside the project`,
    );
  }
  if (!samePath(resolved, repo) && isInside(resolved, repo)) {
    throw new Error(
      `refusing to stage into ${resolved}: it contains the project root ${repo}`,
    );
  }
  assertNoSymlinkAncestors(resolved, 'staging destination');
  return resolved;
}

function assertRegularSourceFile(filePath, label) {
  const resolved = path.resolve(filePath);
  assertNoSymlinkAncestors(resolved, label);
  const stat = lstatOrNull(resolved);
  if (!stat || !stat.isFile() || stat.isSymbolicLink()) {
    throw new Error(`${label} is missing or not a regular file: ${resolved}`);
  }
  return resolved;
}

function walkFiles(root) {
  const result = [];
  const stack = [''];
  while (stack.length > 0) {
    const relative = stack.pop();
    const directory = path.join(root, relative);
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const next = relative ? path.join(relative, entry.name) : entry.name;
      if (entry.isSymbolicLink()) {
        throw new Error(`native distribution may not contain a symlink: ${path.join(root, next)}`);
      }
      if (entry.isDirectory()) {
        stack.push(next);
      } else if (entry.isFile()) {
        result.push(next);
      } else {
        throw new Error(`native distribution contains an unsupported entry: ${path.join(root, next)}`);
      }
    }
  }
  return result.sort((left, right) => (left < right ? -1 : left > right ? 1 : 0));
}

function assertSourceTreeNoSymlinks(root, label) {
  const resolved = path.resolve(root);
  assertNoSymlinkAncestors(resolved, label);
  const stat = lstatOrNull(resolved);
  if (!stat || !stat.isDirectory() || stat.isSymbolicLink()) {
    throw new Error(`${label} is missing or not a directory: ${resolved}`);
  }
  const offenders = [];
  for (const relative of walkFiles(resolved)) {
    const match = forbiddenName(relative);
    if (match) offenders.push(`${relative} (${match})`);
  }
  if (offenders.length > 0) {
    throw new Error(
      `${label} contained Python/OCP/vendor artifacts:\n  ${offenders.join('\n  ')}`,
    );
  }
  return resolved;
}

function copyDirectoryTree(source, destination) {
  fs.mkdirSync(destination, { recursive: true });
  const entries = fs.readdirSync(source, { withFileTypes: true })
    .sort((left, right) => (left.name < right.name ? -1 : left.name > right.name ? 1 : 0));
  for (const entry of entries) {
    const sourcePath = path.join(source, entry.name);
    const destinationPath = path.join(destination, entry.name);
    const stat = fs.lstatSync(sourcePath);
    if (stat.isSymbolicLink()) {
      throw new Error(`refusing to copy symlinked input: ${sourcePath}`);
    }
    if (stat.isDirectory()) {
      copyDirectoryTree(sourcePath, destinationPath);
    } else if (stat.isFile()) {
      fs.copyFileSync(sourcePath, destinationPath);
    } else {
      throw new Error(`refusing to copy unsupported input entry: ${sourcePath}`);
    }
  }
}

/**
 * Copy the FreeCAD adapter sources and workbench addon next to the native
 * runtime. The driver discovers `adapters/freecad/src` by walking executable
 * ancestors, so `resources/backend/adapters/freecad/src` is found without any
 * repository checkout. Bytecode caches, `.egg-info` metadata and compiled
 * Python modules are never copied.
 */
function copyFilteredTree(source, destination, options = {}) {
  const excludedDirectories = options.excludedDirectories || ADAPTER_EXCLUDED_DIRECTORIES;
  const excludedExtensions = options.excludedExtensions || ADAPTER_EXCLUDED_EXTENSIONS;
  fs.mkdirSync(destination, { recursive: true });
  const entries = fs.readdirSync(source, { withFileTypes: true })
    .sort((left, right) => (left.name < right.name ? -1 : left.name > right.name ? 1 : 0));
  for (const entry of entries) {
    const lowered = entry.name.toLowerCase();
    const sourcePath = path.join(source, entry.name);
    const destinationPath = path.join(destination, entry.name);
    const stat = fs.lstatSync(sourcePath);
    if (stat.isSymbolicLink()) {
      throw new Error(`refusing to copy symlinked adapter input: ${sourcePath}`);
    }
    if (stat.isDirectory()) {
      if (excludedDirectories.has(lowered) || lowered.endsWith('.egg-info') || lowered.endsWith('.dist-info')) {
        continue;
      }
      copyFilteredTree(sourcePath, destinationPath, options);
      continue;
    }
    if (!stat.isFile()) {
      throw new Error(`refusing to copy unsupported adapter input: ${sourcePath}`);
    }
    if (excludedExtensions.has(path.extname(entry.name).toLowerCase())) continue;
    fs.copyFileSync(sourcePath, destinationPath);
  }
}

function stageFreeCadAdapter({ repoRoot = REPO_ROOT, destination, vendor } = {}) {
  const adapterRoot = path.join(path.resolve(repoRoot), 'adapters', 'freecad');
  const sourceRoot = path.join(adapterRoot, 'src', 'kala_agent_freecad');
  const modRoot = path.join(adapterRoot, 'mod', 'Kala');
  for (const [label, candidate] of [
    ['FreeCAD adapter sources', sourceRoot],
    ['FreeCAD workbench addon', modRoot],
  ]) {
    assertNoSymlinkAncestors(candidate, label);
    const stat = lstatOrNull(candidate);
    if (!stat || !stat.isDirectory() || stat.isSymbolicLink()) {
      throw new Error(
        `${label} are missing at ${candidate}; the packaged desktop app bundles the ` +
        'FreeCAD adapter so the FreeCAD provider works without a repository checkout',
      );
    }
  }

  const stagingRoot = path.join(path.resolve(destination), ADAPTER_DIR_NAME, 'freecad');
  const stagedSourceRoot = path.join(stagingRoot, 'src', 'kala_agent_freecad');
  const stagedModRoot = path.join(stagingRoot, 'mod', 'Kala');
  if (lstatOrNull(stagingRoot)) {
    throw new Error(`refusing to stage the FreeCAD adapter over an existing path: ${stagingRoot}`);
  }
  copyFilteredTree(sourceRoot, stagedSourceRoot);
  copyFilteredTree(modRoot, stagedModRoot);

  let vendorManifest = null;
  const vendorMode = vendor?.mode || 'skip';
  if (vendorMode !== 'skip') {
    try {
      vendorManifest = freecadVendor.vendorFreeCadDependencies({
        adapterRoot: stagingRoot,
        pinsFile: vendor?.pinsFile,
        freecadExecutable: vendor?.executable || null,
        platform: vendor?.platform,
        env: vendor?.env,
        pipPython: vendor?.pipPython,
        pythonVersion: vendor?.pythonVersion || null,
        abi: vendor?.abi || null,
        wheelTag: vendor?.wheelTag || null,
        verify: vendor?.verify !== false,
      });
    } catch (error) {
      if (vendorMode === 'require') throw error;
      console.warn(
        `[native-backend] the FreeCAD Python dependencies could not be vendored: ${error.message}. ` +
        'The adapter will only work when FreeCAD\'s interpreter already provides them.',
      );
    }
    if (vendorMode === 'require' && vendorManifest) {
      if (!vendorManifest.freecad?.detected) {
        throw new Error(
          'FreeCAD was not detected; the packaged adapter dependencies could not be verified ' +
          'inside FreeCADCmd. Install FreeCAD or pass --freecad-python.',
        );
      }
      if (vendorManifest.detectedUnsupported) {
        throw new Error(vendorManifest.verificationSkippedReason);
      }
      if (!vendorManifest.verification) {
        throw new Error('the vendored FreeCAD dependencies were not verified inside FreeCADCmd');
      }
    }
  }

  const files = walkFiles(stagingRoot);
  const pythonFiles = files.filter((relative) => path.extname(relative).toLowerCase() === '.py');
  for (const required of [
    path.join('src', 'kala_agent_freecad', '__init__.py'),
    path.join('mod', 'Kala', 'Init.py'),
    path.join('mod', 'Kala', 'InitGui.py'),
    path.join('mod', 'Kala', 'kala_paths.py'),
  ]) {
    if (!files.includes(required)) {
      throw new Error(`the staged FreeCAD adapter is incomplete: ${required} is missing`);
    }
  }
  if (vendorMode !== 'skip') {
    for (const required of [
      FREECAD_VENDOR_MANIFEST,
      path.join('src', 'kala_agent', '__init__.py'),
    ]) {
      if (!files.includes(required)) {
        throw new Error(`the vendored FreeCAD dependencies are incomplete: ${required} is missing`);
      }
    }
    if (vendorManifest) validateFreeCadVendorManifest(stagingRoot);
  }
  if (pythonFiles.length === 0) {
    throw new Error('the staged FreeCAD adapter contained no Python modules');
  }
  // The add-on has to find the adapter and the ABI-matched dependencies on
  // its own: the check below puts only the add-on folder on `sys.path`, the
  // way FreeCAD does when it loads the add-on from the user's Mod directory.
  const addonVerification = verifyStagedFreeCadAddOn({
    modRoot: stagedModRoot,
    vendor: vendorManifest,
    vendorMode,
    env: vendor?.env,
  });
  // The check imports the adapter inside FreeCADCmd, which writes bytecode
  // caches into the stage. They are removed again: a stage carries sources.
  const prunedBytecode = pruneStagedBytecode(stagingRoot);
  console.log(
    `[native-backend] the staged FreeCAD add-on was verified inside FreeCADCmd ` +
    `(${prunedBytecode} bytecode caches removed)`,
  );
  return {
    root: stagingRoot,
    sourceRoot: stagedSourceRoot,
    modRoot: stagedModRoot,
    fileCount: files.length,
    pythonFileCount: pythonFiles.length,
    vendor: vendorManifest,
    addonVerification,
  };
}

/**
 * Remove every bytecode cache from a staged tree.
 *
 * FreeCAD's embedded interpreter ignores `PYTHONDONTWRITEBYTECODE` while it
 * imports the adapter and its dependencies, so the add-on check below leaves
 * `__pycache__` directories behind. Bytecode never belongs in a stage: it is
 * interpreter specific, it is not reproducible from the sources and it makes
 * the stage digest depend on the build host's Python.
 */
function pruneStagedBytecode(root) {
  let removed = 0;
  const stack = [root];
  while (stack.length > 0) {
    const directory = stack.pop();
    let entries;
    try {
      entries = fs.readdirSync(directory, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const candidate = path.join(directory, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) {
        if (entry.name === '__pycache__') {
          fs.rmSync(candidate, { recursive: true, force: true });
          removed += 1;
          continue;
        }
        stack.push(candidate);
        continue;
      }
      if (entry.isFile() && (entry.name.endsWith('.pyc') || entry.name.endsWith('.pyo'))) {
        fs.rmSync(candidate, { force: true });
        removed += 1;
      }
    }
  }
  return removed;
}

/**
 * Import the staged add-on inside FreeCAD's own interpreter.
 *
 * Only the add-on folder is put on `sys.path`; the add-on resolves the
 * adapter sources, the shared contracts package and the vendored
 * dependencies for the running interpreter's ABI by itself. That is exactly
 * what happens when FreeCAD loads the add-on from the user's Mod directory,
 * so a stage that passes this check installs and works.
 */
function verifyStagedFreeCadAddOn({
  modRoot,
  vendor = null,
  vendorMode = 'skip',
  env = process.env,
  timeout = 120_000,
} = {}) {
  const detected = vendor && vendor.freecad ? vendor.freecad : null;
  if (!detected || !detected.detected || !detected.executable) {
    const reason = 'FreeCAD was not detected on the build host';
    if (vendorMode === 'require') {
      throw new Error(
        `${reason}; the staged FreeCAD add-on could not be verified inside FreeCADCmd`,
      );
    }
    console.warn(
      `[native-backend] ${reason}; the staged FreeCAD add-on was not verified inside FreeCADCmd`,
    );
    return { verified: false, reason };
  }
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'kala-freecad-addon-verify-'));
  const script = path.join(temporary, 'kala_freecad_addon_verify.py');
  try {
    const payload = [
      'import json, sys',
      `sys.path.insert(0, ${JSON.stringify(modRoot)})`,
      'import kala_paths',
      'state = kala_paths.prepare()',
      'import kala_agent_freecad.cad.models as models',
      `sys.stdout.write(${JSON.stringify(FREECAD_ADDON_VERIFY_MARKER)} + json.dumps({`,
      '    "abi": state.get("abi"),',
      '    "adapter": state.get("adapter"),',
      '    "sitePackages": state.get("sitePackages"),',
      '    "problems": kala_paths.problems(),',
      '    "models": models.__file__,',
      '}) + "\\n")',
      '',
    ].join('\n');
    fs.writeFileSync(script, payload, 'utf8');
    const verifyEnvironment = freecadVendor.cleanEnvironment(env, {
      HOME: temporary,
      PYTHONDONTWRITEBYTECODE: '1',
      XDG_CONFIG_HOME: path.join(temporary, 'config'),
      XDG_CACHE_HOME: path.join(temporary, 'cache'),
      XDG_DATA_HOME: path.join(temporary, 'data'),
    });
    for (const directory of ['config', 'cache', 'data']) {
      fs.mkdirSync(path.join(temporary, directory), { recursive: true });
    }
    const result = spawnSync(detected.executable, [script], {
      cwd: temporary,
      env: verifyEnvironment,
      encoding: 'utf8',
      shell: false,
      windowsHide: true,
      timeout,
      maxBuffer: 8 * 1024 * 1024,
    });
    if (result.error) throw result.error;
    const output = `${result.stdout || ''}\n${result.stderr || ''}`;
    const line = output
      .split(/\r?\n/)
      .find((candidate) => candidate.startsWith(FREECAD_ADDON_VERIFY_MARKER));
    if (!line) {
      throw new Error(
        `the staged FreeCAD add-on could not be imported by ${detected.executable} ` +
        `(exit ${result.status}): ${output.trim().slice(-2000)}`,
      );
    }
    const report = JSON.parse(line.slice(FREECAD_ADDON_VERIFY_MARKER.length));
    if (Array.isArray(report.problems) && report.problems.length > 0) {
      throw new Error(
        `the staged FreeCAD add-on did not find its payload inside FreeCADCmd: ` +
        `${report.problems.join('; ')}`,
      );
    }
    return { verified: true, ...report };
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

function validateBundledFreeCadAdapter(paths) {
  const root = path.join(paths.stageDir, ADAPTER_DIR_NAME, 'freecad');
  const sourceInit = path.join(root, 'src', 'kala_agent_freecad', '__init__.py');
  const addonInit = path.join(root, 'mod', 'Kala', 'InitGui.py');
  for (const [label, candidate] of [
    ['FreeCAD adapter module', sourceInit],
    ['FreeCAD workbench addon', addonInit],
  ]) {
    const stat = lstatOrNull(candidate);
    if (!stat || !stat.isFile() || stat.isSymbolicLink()) {
      throw new Error(
        `the native stage has no bundled ${label} (${candidate}); the packaged app cannot ` +
        'offer the FreeCAD provider. Restage with `npm run build:native`.',
      );
    }
  }
  const files = walkFiles(root);
  const pythonFiles = files.filter((relative) => path.extname(relative).toLowerCase() === '.py');
  if (pythonFiles.length === 0) {
    throw new Error(`the bundled FreeCAD adapter at ${root} contains no Python modules`);
  }
  const offenders = [];
  for (const relative of files) {
    const match = forbiddenName(path.join(ADAPTER_DIR_NAME, 'freecad', relative), {
      allowAdapterPython: true,
    });
    if (match) offenders.push(`${relative} (${match})`);
  }
  if (offenders.length > 0) {
    throw new Error(
      'the bundled FreeCAD adapter contained bytecode/vendor artifacts:\n  ' +
      offenders.join('\n  '),
    );
  }
  const vendor = validateFreeCadVendorManifest(root);
  return { root, fileCount: files.length, pythonFileCount: pythonFiles.length, vendor };
}

/**
 * Read and validate `adapters/freecad/vendor.json`, the record of the Python
 * dependencies vendored for FreeCAD's own interpreter. A stage without the
 * manifest is accepted here (development stages may skip vendoring); the
 * release gate in `desktop-pack.cjs` requires it.
 */
function validateFreeCadVendorManifest(adapterRoot) {
  const manifestPath = path.join(adapterRoot, FREECAD_VENDOR_MANIFEST);
  const stat = lstatOrNull(manifestPath);
  if (!stat) return null;
  if (!stat.isFile() || stat.isSymbolicLink()) {
    throw new Error(`the FreeCAD vendor manifest is not a regular file: ${manifestPath}`);
  }
  let manifest;
  try {
    manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  } catch (error) {
    throw new Error(`the FreeCAD vendor manifest is not valid JSON: ${error.message}`);
  }
  if (!manifest || manifest.schemaVersion !== 2) {
    throw new Error('the FreeCAD vendor manifest has an unsupported schemaVersion');
  }
  if (!Array.isArray(manifest.supportedAbis) || manifest.supportedAbis.length === 0) {
    throw new Error('the FreeCAD vendor manifest records no supported ABIs');
  }
  if (!manifest.abis || typeof manifest.abis !== 'object') {
    throw new Error('the FreeCAD vendor manifest records no per-ABI dependency sets');
  }
  const abis = Object.keys(manifest.abis).sort();
  const supported = [...manifest.supportedAbis].sort();
  if (abis.join('\n') !== supported.join('\n')) {
    throw new Error('the FreeCAD vendor manifest ABI list and dependency sets disagree');
  }
  for (const abi of supported) {
    if (!FREECAD_PYTHON_ABIS.includes(abi)) {
      throw new Error(`the FreeCAD vendor manifest records an unsupported ABI: ${abi}`);
    }
    const entry = manifest.abis[abi];
    if (!entry || entry.abi !== abi) {
      throw new Error(`the FreeCAD vendor manifest has no usable dependency set for ${abi}`);
    }
    if (!Array.isArray(entry.wheels) || entry.wheels.length === 0) {
      throw new Error(`the FreeCAD vendor manifest records no wheels for ${abi}`);
    }
    for (const wheel of entry.wheels) {
      if (!wheel || typeof wheel.name !== 'string' || typeof wheel.sha256 !== 'string'
        || !/^[a-f0-9]{64}$/.test(wheel.sha256)) {
        throw new Error(`the FreeCAD vendor manifest contains an invalid ${abi} wheel entry`);
      }
    }
    const packages = new Set(Array.isArray(entry.packages) ? entry.packages : []);
    for (const required of ['pydantic', 'pydantic_core']) {
      if (!packages.has(required)) {
        throw new Error(`the FreeCAD vendor manifest does not list ${required} for ${abi}`);
      }
    }
    const directory = path.join(adapterRoot, `${FREECAD_SITE_PACKAGES_PREFIX}${abi}`);
    for (const relative of [
      path.join(directory, 'pydantic', '__init__.py'),
      path.join(directory, 'pydantic_core', '__init__.py'),
    ]) {
      if (!lstatOrNull(relative)) {
        throw new Error(`the bundled FreeCAD dependencies are incomplete: ${relative} is missing`);
      }
    }
    const extension = walkFiles(path.join(directory, 'pydantic_core'))
      .find((relative) => /_pydantic_core.*\.(?:so|pyd)$/i.test(relative));
    if (!extension) {
      throw new Error(`the vendored pydantic-core C extension is missing for ${abi}`);
    }
  }
  if (!lstatOrNull(path.join(adapterRoot, 'src', 'kala_agent', '__init__.py'))) {
    throw new Error(
      'the bundled FreeCAD dependencies are incomplete: src/kala_agent/__init__.py is missing',
    );
  }
  return manifest;
}

function assertFreeCadAdapterBundled(paths) {
  return validateBundledFreeCadAdapter(paths);
}

function fileSha256(filePath) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256');
    const stream = fs.createReadStream(filePath);
    stream.on('error', reject);
    stream.on('data', (chunk) => hash.update(chunk));
    stream.on('end', () => resolve(hash.digest('hex')));
  });
}

async function treeSha256(root) {
  if (!fs.existsSync(root) || !fs.statSync(root).isDirectory()) {
    throw new Error(`static UI directory is missing: ${root}`);
  }
  const hash = crypto.createHash('sha256');
  hash.update(TREE_HASH_PREFIX);
  const relativePaths = walkFiles(root)
    .map((relative) => relative.split(path.sep).join('/'))
    .sort((left, right) => (left < right ? -1 : left > right ? 1 : 0));
  for (const relative of relativePaths) {
    const absolute = path.join(root, ...relative.split('/'));
    hash.update(relative);
    hash.update('\0');
    hash.update(String(fs.statSync(absolute).size));
    hash.update('\0');
    hash.update(await fileSha256(absolute));
    hash.update('\n');
  }
  return hash.digest('hex');
}

function isOcctSharedLibraryName(name) {
  return /^libTK[A-Za-z0-9_]*\.so(?:\..*)?$/i.test(name);
}

function isSystemLibraryPath(resolved) {
  const normalized = path.resolve(resolved).split(path.sep).join('/');
  return SYSTEM_LIBRARY_PREFIXES.some((prefix) => normalized.startsWith(prefix))
    || /^\/usr\/lib\/[^/]+\//.test(normalized);
}

function parseLddOutput(output) {
  const entries = [];
  const missing = [];
  for (const raw of String(output || '').split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const notFound = /^(\S+)\s+=>\s+not found$/.exec(line);
    if (notFound) {
      missing.push(notFound[1]);
      continue;
    }
    const mapped = /^(\S+)\s+=>\s+(\S+)(?:\s+\(0x[0-9a-fA-F]+\))?$/.exec(line);
    if (mapped) {
      entries.push({ name: mapped[1], path: mapped[2] });
      continue;
    }
    const direct = /^(\/\S+)\s+\(0x[0-9a-fA-F]+\)$/.exec(line);
    if (direct) entries.push({ name: path.basename(direct[1]), path: direct[1] });
  }
  return { entries, missing };
}

function runLdd(filePath) {
  const result = spawnSync('ldd', [filePath], {
    encoding: 'utf8',
    shell: false,
    windowsHide: true,
    env: { ...process.env, LC_ALL: 'C' },
  });
  if (result.error) {
    if (result.error.code === 'ENOENT') {
      throw new Error('ldd is unavailable; install the Linux binutils or pass --occt-lib-dir');
    }
    throw result.error;
  }
  const parsed = parseLddOutput(result.stdout);
  return {
    ...parsed,
    status: result.status,
    stderr: String(result.stderr || '').trim(),
  };
}

/**
 * Resolve the exact OCCT shared libraries the native executable links, plus
 * their non-system dependencies. The loader name (`libTKernel.so.7`) is what
 * the runtime looks up, so the staged copy uses that name even when the
 * resolved file is `libTKernel.so.7.9.3`.
 */
async function collectLddSharedLibraries(binaryPath) {
  const root = runLdd(binaryPath);
  const selected = new Map();
  const missing = new Map();
  const enqueue = (name, sourcePath, requiredBy, { force = false } = {}) => {
    if (!name || name.startsWith('linux-vdso') || name.startsWith('ld-linux')) return;
    if (selected.has(name)) return;
    if (!force && isSystemLibraryPath(sourcePath)) return;
    selected.set(name, { name, sourcePath, requiredBy });
  };
  for (const entry of root.entries) {
    if (isOcctSharedLibraryName(entry.name)) enqueue(entry.name, entry.path, 'kala-studio', { force: true });
  }
  for (const name of root.missing) {
    if (isOcctSharedLibraryName(name)) missing.set(name, { name, requiredBy: 'kala-studio' });
  }
  if (selected.size === 0) {
    return {
      mode: 'ldd',
      libraries: [],
      missing: [...missing.values()],
      lddStatus: root.status,
      note: root.status === 0
        ? 'the native executable links no OCCT shared libraries'
        : `ldd reported: ${root.stderr || `exit ${root.status}`}`,
    };
  }

  const queue = [...selected.values()];
  while (queue.length > 0) {
    const current = queue.shift();
    let deps;
    try {
      deps = runLdd(current.sourcePath);
    } catch (error) {
      missing.set(current.name, { name: current.name, requiredBy: 'ldd', reason: error.message });
      continue;
    }
    for (const name of deps.missing) {
      if (!missing.has(name)) {
        missing.set(name, { name, requiredBy: current.name, reason: 'ldd did not resolve it' });
      }
    }
    for (const dependency of deps.entries) {
      const force = isOcctSharedLibraryName(dependency.name);
      if (selected.has(dependency.name)) continue;
      if (!force && isSystemLibraryPath(dependency.path)) continue;
      if (dependency.name.startsWith('linux-vdso') || dependency.name.startsWith('ld-linux')) continue;
      enqueue(dependency.name, dependency.path, current.name, { force });
      const item = selected.get(dependency.name);
      if (item && item.sourcePath === dependency.path) queue.push(item);
    }
  }

  return {
    mode: 'ldd',
    libraries: [...selected.values()],
    missing: [...missing.values()],
    lddStatus: root.status,
    note: null,
  };
}

function listExplicitLibraries(directory, platform) {
  const resolved = path.resolve(directory);
  const stat = lstatOrNull(resolved);
  if (!stat || !stat.isDirectory() || stat.isSymbolicLink()) {
    throw new Error(`--occt-lib-dir is not a directory: ${resolved}`);
  }
  const libraries = [];
  for (const entry of fs.readdirSync(resolved, { withFileTypes: true }).sort(
    (left, right) => (left.name < right.name ? -1 : left.name > right.name ? 1 : 0),
  )) {
    const candidate = path.join(resolved, entry.name);
    let name = entry.name;
    if (platform === 'linux' && !isOcctSharedLibraryName(name)) continue;
    if (platform === 'win32' && !name.toLowerCase().endsWith('.dll')) continue;
    let sourcePath = candidate;
    const candidateStat = fs.lstatSync(candidate);
    if (candidateStat.isSymbolicLink()) {
      sourcePath = fs.realpathSync(candidate);
    } else if (!candidateStat.isFile()) {
      continue;
    }
    if (!fs.statSync(sourcePath).isFile()) continue;
    libraries.push({ name, sourcePath, requiredBy: 'explicit --occt-lib-dir' });
  }
  return { mode: 'explicit', libraries, missing: [], lddStatus: null, note: null };
}

async function stageSharedLibraries({
  binaryPath,
  destination,
  platform = process.platform,
  occtLibDir,
}) {
  if (platform !== 'linux' && platform !== 'win32') {
    return { mode: 'none', libraries: [], missing: [], targetDir: null, note: 'not a packaged platform' };
  }
  let resolved;
  if (occtLibDir) {
    resolved = listExplicitLibraries(occtLibDir, platform);
  } else if (platform === 'linux') {
    resolved = await collectLddSharedLibraries(binaryPath);
  } else {
    return {
      mode: 'none',
      libraries: [],
      missing: [],
      targetDir: null,
      note: 'Windows DLL staging needs --occt-lib-dir pointing at the vcpkg bin directory',
    };
  }
  if (resolved.libraries.length > MAX_BUNDLED_LIBRARIES) {
    throw new Error(
      `refusing to bundle ${resolved.libraries.length} shared libraries (limit ${MAX_BUNDLED_LIBRARIES})`,
    );
  }
  const targetDir = platform === 'win32'
    ? path.resolve(destination)
    : path.join(path.resolve(destination), SHARED_LIB_DIR_NAME);
  const libraries = [];
  if (resolved.libraries.length > 0) {
    fs.mkdirSync(targetDir, { recursive: true });
    for (const library of resolved.libraries) {
      const bytes = fs.statSync(library.sourcePath).size;
      if (bytes > MAX_BUNDLED_LIBRARY_BYTES) {
        throw new Error(`refusing to bundle oversized shared library ${library.name} (${bytes} bytes)`);
      }
      const target = path.join(targetDir, library.name);
      if (!isInside(targetDir, target)) {
        throw new Error(`refusing to bundle a shared library outside the stage: ${library.name}`);
      }
      fs.copyFileSync(library.sourcePath, target);
      if (platform !== 'win32') fs.chmodSync(target, 0o755);
      libraries.push({
        name: library.name,
        source: library.sourcePath,
        requiredBy: library.requiredBy,
        bytes,
        sha256: await fileSha256(target),
      });
    }
  }
  return { ...resolved, libraries, targetDir };
}

function runSyncOrThrow(command, args) {
  const result = spawnSync(command, args, {
    encoding: 'utf8',
    shell: false,
    windowsHide: true,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(' ')} exited with code ${result.status}: ` +
      `${String(result.stderr || result.stdout || '').trim()}`,
    );
  }
  return result;
}

/**
 * Relocate the staged ELF runtime to `$ORIGIN/lib` so the AppImage does not
 * depend on the build host's absolute OCCT rpath. `patchelf` is used when
 * present; the Electron main process also prepends the bundled `lib` directory
 * to `LD_LIBRARY_PATH`, which is the documented fallback.
 */
function patchElfRpaths({ binaryPath, libDir, libraries, platform = process.platform }) {
  if (platform !== 'linux') {
    return { patchelf: null, binaryRpath: null, libraryRpath: null, note: 'not an ELF platform' };
  }
  if (!libraries || libraries.length === 0) {
    return { patchelf: null, binaryRpath: null, libraryRpath: null, note: 'no OCCT libraries to relocate' };
  }
  const version = commandVersion('patchelf', ['--version']);
  if (!version) {
    console.warn(
      '[native-backend] patchelf is unavailable; the staged binary keeps its build-time rpath. ' +
      'The Electron launcher prepends backend/lib to LD_LIBRARY_PATH.',
    );
    return {
      patchelf: null,
      binaryRpath: null,
      libraryRpath: null,
      note: 'patchelf unavailable; LD_LIBRARY_PATH fallback',
    };
  }
  runSyncOrThrow('patchelf', ['--set-rpath', ORIGIN_LIB_RPATH, binaryPath]);
  for (const library of libraries) {
    const target = path.join(libDir, library.name);
    runSyncOrThrow('patchelf', ['--set-rpath', ORIGIN_RPATH, target]);
  }
  return {
    patchelf: version,
    binaryRpath: ORIGIN_LIB_RPATH,
    libraryRpath: ORIGIN_RPATH,
    note: 'patchelf relocated the staged runtime to $ORIGIN/lib',
  };
}

function forbiddenName(relative, { allowAdapterPython = false } = {}) {
  const normalized = relative.split(/[\\/]+/).join('/');
  if (allowAdapterPython && isBundledAdapterPath(normalized)) {
    return forbiddenBundledAdapterName(normalized);
  }
  const parts = normalized.split('/');
  for (const part of parts) {
    if (FORBIDDEN_DIRECTORY_NAMES.has(part.toLowerCase())) return part;
  }
  const base = parts[parts.length - 1] || '';
  if (FORBIDDEN_EXTENSIONS.has(path.extname(base).toLowerCase())) return base;
  for (const pattern of FORBIDDEN_FILE_PATTERNS) {
    if (pattern.test(base)) return base;
  }
  return null;
}

/**
 * The bundled `kala_agent_freecad` sources are Python by design: FreeCAD's own
 * interpreter imports them inside `FreeCADCmd`. They are the only Python files
 * a native stage may contain. The Python interpreter, OCP wheels, bytecode
 * caches and vendor site-packages remain forbidden everywhere, including in
 * this subtree.
 */
function isBundledAdapterPath(relative) {
  const normalized = relative.split(/[\\/]+/).join('/');
  const srcRoot = `adapters/${FREECAD_SRC_RELATIVE.split(path.sep).join('/')}`;
  const modRoot = `adapters/${FREECAD_MOD_RELATIVE.split(path.sep).join('/')}`;
  return normalized === srcRoot
    || normalized.startsWith(`${srcRoot}/`)
    || normalized === modRoot
    || normalized.startsWith(`${modRoot}/`)
    || normalized === `adapters/freecad/${FREECAD_VENDOR_MANIFEST}`
    || isFreeCadVendorPath(normalized);
}

/**
 * Whether a path is a vendored FreeCAD-interpreter dependency: either the
 * shared pure-Python `kala_agent` package under `src/`, or a per-ABI
 * `adapters/freecad/site-packages-cpNNN/` directory.
 */
function isFreeCadVendorPath(relative) {
  const normalized = relative.split(/[\\/]+/).join('/');
  const parts = normalized.split('/');
  if (parts.length >= 4 && parts[0] === 'adapters' && parts[1] === 'freecad'
    && parts[2] === 'src' && parts[3] === 'kala_agent') {
    return true;
  }
  return parts.length >= 3
    && parts[0] === 'adapters'
    && parts[1] === 'freecad'
    && parts[2].startsWith(FREECAD_SITE_PACKAGES_PREFIX)
    && FREECAD_PYTHON_ABIS.includes(parts[2].slice(FREECAD_SITE_PACKAGES_PREFIX.length));
}

function forbiddenBundledAdapterName(relative) {
  const normalized = relative.split(/[\\/]+/).join('/');
  if (normalized === `adapters/freecad/${FREECAD_VENDOR_MANIFEST}`) return null;
  if (isFreeCadVendorPath(normalized)) {
    return forbiddenFreeCadVendorName(normalized.split('/').slice(2));
  }
  const parts = normalized.split('/');
  // `adapters/freecad` is the bundle root; only inspect what is below it so the
  // adapter's own name is not mistaken for a vendor Python tree.
  for (const part of parts.slice(2)) {
    const lowered = part.toLowerCase();
    if (lowered === 'freecad') continue;
    if (ADAPTER_EXCLUDED_DIRECTORIES.has(lowered)) return part;
    if (FORBIDDEN_DIRECTORY_NAMES.has(lowered)) return part;
    if (lowered.endsWith('.egg-info') || lowered.endsWith('.dist-info')) return part;
  }
  const base = parts[parts.length - 1] || '';
  if (ADAPTER_EXCLUDED_EXTENSIONS.has(path.extname(base).toLowerCase())) return base;
  for (const pattern of FORBIDDEN_FILE_PATTERNS) {
    if (pattern.test(base)) return base;
  }
  return null;
}

/**
 * The vendored FreeCAD dependency tree is the one place a `.pyd`/`.so` C
 * extension (pydantic-core) is legitimate. A Python interpreter, `libpython`,
 * bytecode caches, OCP and `site-packages` remain forbidden.
 */
function forbiddenFreeCadVendorName(parts) {
  for (const part of parts) {
    const lowered = part.toLowerCase();
    if (ADAPTER_EXCLUDED_DIRECTORIES.has(lowered)) return part;
    if (lowered === 'site-packages' || lowered === 'pip' || lowered === 'setuptools') return part;
    if (lowered === 'python' || lowered === 'python3' || lowered === 'python.exe') return part;
    if (lowered.endsWith('.egg-info')) return part;
  }
  const base = parts[parts.length - 1] || '';
  const extension = path.extname(base).toLowerCase();
  if (extension === '.pyc' || extension === '.pyo') return base;
  for (const pattern of FORBIDDEN_FILE_PATTERNS) {
    if (pattern.test(base)) return base;
  }
  return null;
}

function binaryContainsNativeStep(filePath) {
  const marker = Buffer.from('ISO-10303-21;', 'utf8');
  const bytes = fs.readFileSync(filePath);
  return bytes.includes(marker);
}

function readStageMetadata(paths) {
  const stat = lstatOrNull(paths.metadataPath);
  if (!stat || !stat.isFile() || stat.isSymbolicLink()) {
    throw new Error(`native build metadata is missing: ${paths.metadataPath}`);
  }
  let metadata;
  try {
    metadata = JSON.parse(fs.readFileSync(paths.metadataPath, 'utf8'));
  } catch (error) {
    throw new Error(`native build metadata is not valid JSON: ${error.message}`);
  }
  if (!metadata || typeof metadata !== 'object') {
    throw new Error('native build metadata was not a JSON object');
  }
  return metadata;
}

/**
 * A helper-owned stage has exactly the binary, `studio_dist/`, and
 * `native-build.json` at its root, plus the optional bundled `lib/` and
 * `adapters/` directories and Windows `.dll` files next to the executable.
 * Anything else was not written by this helper and must not be swept away
 * with the stage. Extra entries inside `studio_dist/` already fail the static
 * tree digest.
 */
function validateStageLayout(paths) {
  const required = new Set([paths.binaryName, METADATA_NAME, STATIC_DIR_NAME]);
  const allowed = new Set(required);
  const entries = fs.readdirSync(paths.stageDir);
  const windows = paths.binaryName.toLowerCase().endsWith('.exe');
  for (const name of entries) {
    if (name === SHARED_LIB_DIR_NAME || name === ADAPTER_DIR_NAME) {
      allowed.add(name);
      continue;
    }
    if (windows && name.toLowerCase().endsWith('.dll')) {
      allowed.add(name);
    }
  }
  const unexpected = entries.filter((name) => !allowed.has(name)).sort();
  if (unexpected.length > 0) {
    throw new Error(
      `native stage contains unexpected top-level entries: ${unexpected.join(', ')}; ` +
      'refusing to replace a stage containing files this helper did not create. ' +
      'Inspect and remove them manually if they belong to the stage.',
    );
  }
  for (const name of required) {
    if (!entries.includes(name)) {
      throw new Error(`native stage layout is missing ${name}: ${paths.stageDir}`);
    }
  }
}

function validateNativeTree(paths, { requireBundles = false } = {}) {
  if (!fs.existsSync(paths.binaryPath) || !fs.statSync(paths.binaryPath).isFile()) {
    throw new Error(`native backend binary is missing: ${paths.binaryPath}`);
  }
  if (!fs.existsSync(paths.indexFile) || !fs.statSync(paths.indexFile).isFile()) {
    throw new Error(`static UI index is missing: ${paths.indexFile}`);
  }
  if (fs.existsSync(paths.metadataPath)) {
    const metadata = readStageMetadata(paths);
    if (metadata && metadata.nativeStep === true && !binaryContainsNativeStep(paths.binaryPath)) {
      throw new Error(
        'the staged binary was built without the native primitives but metadata says ' +
        'nativeStep=true; rebuild with `npm run build:native -- --features native-primitives`',
      );
    }
  }
  const offenders = [];
  for (const relative of walkFiles(paths.stageDir)) {
    const match = forbiddenName(relative, { allowAdapterPython: true });
    if (match) offenders.push(`${relative} (${match})`);
  }
  if (offenders.length > 0) {
    throw new Error(
      'native distribution contained Python/OCP/vendor artifacts:\n  ' +
      offenders.join('\n  '),
    );
  }
  const assets = fs.existsSync(path.join(paths.staticDir, 'assets'))
    ? fs.readdirSync(path.join(paths.staticDir, 'assets'))
    : [];
  if (!assets.some((name) => /\.(?:js|css)$/i.test(name))) {
    throw new Error(`static UI has no JS/CSS assets under ${path.join(paths.staticDir, 'assets')}`);
  }
  const hasAdapter = fs.existsSync(path.join(paths.stageDir, ADAPTER_DIR_NAME));
  if (hasAdapter || requireBundles) {
    validateBundledFreeCadAdapter(paths);
  }
  validateStageLayout(paths);
  return true;
}

async function verifyBundledLibraries(metadata, paths) {
  const libraries = Array.isArray(metadata.sharedLibraries) ? metadata.sharedLibraries : [];
  const windows = paths.binaryName.toLowerCase().endsWith('.exe');
  const targetDir = windows ? paths.stageDir : paths.libDir;
  if (!windows && fs.existsSync(paths.libDir) && libraries.length === 0) {
    throw new Error(
      `the native stage contains an unexpected ${SHARED_LIB_DIR_NAME}/ directory with no ` +
      'recorded shared libraries; restage with `npm run build:native`',
    );
  }
  const expected = new Set();
  for (const library of libraries) {
    if (!library || typeof library.name !== 'string' || typeof library.sha256 !== 'string') {
      throw new Error('native build metadata contains an invalid shared-library entry');
    }
    expected.add(library.name);
    const target = path.join(targetDir, library.name);
    const stat = lstatOrNull(target);
    if (!stat || !stat.isFile() || stat.isSymbolicLink()) {
      throw new Error(`bundled shared library is missing: ${target}`);
    }
    if (stat.size !== library.bytes) {
      throw new Error(`bundled shared library size mismatch: ${library.name}`);
    }
    const digest = await fileSha256(target);
    if (digest !== library.sha256) {
      throw new Error(`bundled shared library digest mismatch: ${library.name}`);
    }
  }
  if (libraries.length > 0) {
    const actual = fs.readdirSync(targetDir)
      .filter((name) => (windows ? name.toLowerCase().endsWith('.dll') : true));
    const unexpected = actual.filter((name) => !expected.has(name)).sort();
    if (unexpected.length > 0) {
      throw new Error(
        `the native stage contains shared-library files not recorded in native-build.json: ` +
        `${unexpected.join(', ')}; restage`,
      );
    }
  }
  return libraries.length;
}

async function verifyBundledAdapter(metadata, paths) {
  if (!fs.existsSync(path.join(paths.stageDir, ADAPTER_DIR_NAME))) return null;
  const adapter = validateBundledFreeCadAdapter(paths);
  const recorded = metadata.freeCadAdapter;
  if (recorded && typeof recorded.treeSha256 === 'string') {
    const digest = await treeSha256(adapter.root);
    if (digest !== recorded.treeSha256) {
      throw new Error(
        'the bundled FreeCAD adapter digest does not match native-build.json; restage',
      );
    }
  }
  const recordedVendor = recorded && recorded.vendor;
  if (adapter.vendor) {
    if (!recordedVendor || recordedVendor.state !== 'vendored') {
      throw new Error(
        'the bundled FreeCAD adapter has vendored Python dependencies that native-build.json ' +
        'does not record; restage with `npm run build:native`',
      );
    }
    const expectedAbis = [...(recordedVendor.supportedAbis || [])].sort();
    const actualAbis = [...adapter.vendor.supportedAbis].sort();
    if (expectedAbis.join('\n') !== actualAbis.join('\n')) {
      throw new Error('the FreeCAD vendor metadata ABI list does not match vendor.json');
    }
    for (const abi of actualAbis) {
      const expected = (recordedVendor.abis?.[abi]?.wheels || [])
        .map((wheel) => `${wheel.name}:${wheel.sha256}`).sort();
      const actual = adapter.vendor.abis[abi].wheels
        .map((wheel) => `${wheel.name}:${wheel.sha256}`).sort();
      if (expected.join('\n') !== actual.join('\n')) {
        throw new Error(`the FreeCAD vendor metadata wheels for ${abi} do not match vendor.json`);
      }
    }
  } else if (recordedVendor && recordedVendor.state === 'vendored') {
    throw new Error('native-build.json records vendored FreeCAD dependencies but vendor.json is missing');
  }
  return adapter;
}

/**
 * Recompute the binary and static-tree digests and compare them with the
 * metadata that is packaged. A corrupted, partially copied or stale stage is
 * refused before electron-builder sees it.
 */
async function verifyStageIntegrity(paths, { requireBundles = false } = {}) {
  const metadata = readStageMetadata(paths);
  if (metadata.package !== PACKAGE_NAME) {
    throw new Error(`native build metadata package mismatch: ${String(metadata.package)}`);
  }
  if (metadata.binary !== paths.binaryName) {
    throw new Error(
      `native build metadata binary mismatch: ${String(metadata.binary)} != ${paths.binaryName}`,
    );
  }
  if (!fs.existsSync(paths.binaryPath) || !fs.statSync(paths.binaryPath).isFile()) {
    throw new Error(`native backend binary is missing: ${paths.binaryPath}`);
  }
  const binaryBytes = fs.statSync(paths.binaryPath).size;
  if (metadata.binaryBytes !== binaryBytes) {
    throw new Error(
      `native binary size mismatch: metadata=${String(metadata.binaryBytes)} actual=${binaryBytes}; restage`,
    );
  }
  const binarySha256 = await fileSha256(paths.binaryPath);
  if (metadata.binarySha256 !== binarySha256) {
    throw new Error(
      'native binary digest mismatch; the stage is corrupted or stale. Run `npm run build:native` again.',
    );
  }
  let staticDistSha256;
  try {
    staticDistSha256 = await treeSha256(paths.staticDir);
  } catch (error) {
    throw new Error(`native static tree integrity check failed: ${error.message}`);
  }
  if (metadata.staticDistSha256 !== staticDistSha256) {
    throw new Error(
      'native static UI digest mismatch; the stage is corrupted or stale. Run `npm run build` and `npm run build:native` again.',
    );
  }
  if (metadata.nativeStep === true && !binaryContainsNativeStep(paths.binaryPath)) {
    throw new Error(
      'the staged binary was built without the native primitives but metadata says ' +
      'nativeStep=true; rebuild with `npm run build:native -- --features native-primitives`',
    );
  }
  await verifyBundledLibraries(metadata, paths);
  await verifyBundledAdapter(metadata, paths);
  validateNativeTree(paths, { requireBundles });
  return metadata;
}

function assertStageSourceParity(metadata, currentReport) {
  const parity = metadata && metadata.sourceParity;
  if (!parity || typeof parity !== 'object') {
    throw new Error(
      'native build metadata has no current-source parity snapshot; restage with `npm run build:native`',
    );
  }
  if (
    parity.pythonSourceSha256 !== currentReport.python.sourceSha256 ||
    parity.nativeSourceSha256 !== currentReport.native.sourceSha256
  ) {
    throw new Error(
      'native stage is stale relative to the current Studio Python/Rust sources; ' +
      'run `npm run build` and `npm run build:native` again before packaging',
    );
  }
  return parity;
}

async function assertStageUiMatches(metadata, { distDir = DEFAULT_DIST_DIR } = {}) {
  let currentSha256;
  try {
    currentSha256 = await treeSha256(distDir);
  } catch (error) {
    throw new Error(`current static UI cannot be verified: ${error.message}`);
  }
  if (!metadata || metadata.staticDistSha256 !== currentSha256) {
    throw new Error(
      'native stage static UI is stale relative to studio/dist; ' +
      'run `npm run build` and `npm run build:native` again before packaging',
    );
  }
  return currentSha256;
}

function validateStageOwnership(stageDir, { studioRoot = STUDIO_ROOT } = {}) {
  const resolved = path.resolve(stageDir);
  const defaultStage = path.join(path.resolve(studioRoot), 'backend-native');
  const unmanaged = () => new Error(
    `refusing to replace nonempty unmanaged directory ${resolved}; ` +
    'native staging only replaces the dedicated studio/backend-native stage or a stage ' +
    'previously created by this helper (validated ownership metadata bound to this path)',
  );
  let metadata;
  try {
    metadata = readStageMetadata(nativePaths({ studioRoot, stageDir: resolved }));
  } catch {
    throw unmanaged();
  }
  const modern =
    metadata.stageOwner === STAGE_OWNER &&
    metadata.managedBy === HELPER_ID &&
    metadata.package === PACKAGE_NAME &&
    typeof metadata.stageDir === 'string' &&
    samePath(metadata.stageDir, resolved);
  if (modern) return metadata;
  const legacyDefaultStage =
    samePath(resolved, defaultStage) &&
    metadata.createdBy === HELPER_ID &&
    metadata.package === PACKAGE_NAME &&
    Number(metadata.schemaVersion) === 1;
  if (legacyDefaultStage) return metadata;
  throw unmanaged();
}

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      shell: false,
      stdio: options.stdio || 'inherit',
      cwd: options.cwd || REPO_ROOT,
      env: options.env || process.env,
      windowsHide: true,
    });
    child.on('error', (error) => {
      if (error && error.code === 'ENOENT') {
        reject(new Error(
          `could not execute ${command}; install the Rust toolchain ` +
          '(https://rustup.rs) and make cargo available on PATH',
        ));
        return;
      }
      reject(error);
    });
    child.on('exit', (code, signal) => {
      if (signal) {
        reject(new Error(`${command} exited via ${signal}`));
        return;
      }
      if (code !== 0) {
        reject(new Error(`${command} exited with code ${code}`));
        return;
      }
      resolve();
    });
  });
}

function commandVersion(command, args) {
  try {
    const result = spawnSync(command, args, {
      encoding: 'utf8',
      shell: false,
      windowsHide: true,
    });
    if (result.status !== 0) return null;
    return (result.stdout || '').trim().split('\n')[0] || null;
  } catch {
    return null;
  }
}

function packageVersion(cargoManifest = path.join(BACKEND_ROOT, 'crates', PACKAGE_NAME, 'Cargo.toml')) {
  try {
    const text = fs.readFileSync(cargoManifest, 'utf8');
    const match = text.match(/^version\s*=\s*"([^"]+)"/m);
    return match ? match[1] : '0.0.0';
  } catch {
    return '0.0.0';
  }
}

function gitCommit() {
  try {
    const result = spawnSync('git', ['-C', REPO_ROOT, 'rev-parse', 'HEAD'], {
      encoding: 'utf8',
      shell: false,
      windowsHide: true,
    });
    return result.status === 0 ? (result.stdout || '').trim() || null : null;
  } catch {
    return null;
  }
}

function currentSourceParity() {
  try {
    return readiness.readinessSnapshot(readiness.checkStudioReadiness());
  } catch (error) {
    return {
      ready: false,
      error: error.message,
      pythonSourceSha256: null,
      nativeSourceSha256: null,
      pythonRouteMethodCount: 0,
      nativeRouteMethodCount: 0,
      missingRouteMethods: [],
      missingCapabilities: [],
      pendingFeatures: [],
      summary: `source readiness could not be evaluated: ${error.message}`,
    };
  }
}

function discardCandidate(candidate, stageDir) {
  const expectedPrefix = `${stageDir}.staging-`;
  if (candidate.startsWith(expectedPrefix)) {
    fs.rmSync(candidate, { recursive: true, force: true });
  }
}

async function stageNativeBackend({
  stageDir = DEFAULT_STAGE_DIR,
  distDir = DEFAULT_DIST_DIR,
  binaryPath,
  target,
  platform = process.platform,
  noDefaultFeatures = false,
  features,
  studioRoot = STUDIO_ROOT,
  backendRoot = BACKEND_ROOT,
  repoRoot = REPO_ROOT,
  targetDir,
  occtLibDir,
  freecadVendor = { mode: 'skip' },
  onBeforeInstall,
  onBeforeBackupRemoval,
} = {}) {
  const paths = nativePaths({ studioRoot, stageDir, platform, target });
  const destination = assertSafeStageDir(paths.stageDir, { studioRoot, repoRoot });
  const parent = path.dirname(destination);
  const parentStat = lstatOrNull(parent);
  if (!parentStat || !parentStat.isDirectory() || parentStat.isSymbolicLink()) {
    throw new Error(`native stage parent directory is missing or not a directory: ${parent}`);
  }

  let sourceBinary;
  try {
    sourceBinary = assertRegularSourceFile(
      binaryPath || cargoBinaryPath({ backendRoot, targetDir, target }),
      'native backend source binary',
    );
  } catch (error) {
    throw new Error(`${error.message}; run \`npm run build:native\` in studio/ first`);
  }
  let sourceDist;
  try {
    sourceDist = assertSourceTreeNoSymlinks(distDir, 'static UI source');
  } catch (error) {
    throw new Error(`${error.message}; run \`npm run build\` in studio/ first`);
  }
  if (!fs.existsSync(path.join(sourceDist, 'index.html'))) {
    throw new Error(
      `static UI is missing at ${path.join(sourceDist, 'index.html')}; ` +
      'run `npm run build` in studio/ first',
    );
  }
  if (
    isInside(sourceDist, destination) ||
    isInside(destination, sourceDist) ||
    isInside(destination, sourceBinary)
  ) {
    throw new Error(
      'refusing to stage: the staging destination and the input binary/UI must be separate paths',
    );
  }

  // Capture the exact destination identity/state before any temporary work.
  // A nonempty directory must additionally carry validated ownership
  // metadata bound to this path. The snapshot is rechecked immediately before
  // the swap so a directory created/replaced/modified while the candidate was
  // being hashed is never moved or deleted.
  const originalState = captureStageState(destination);
  if (originalState.nonempty) {
    validateStageOwnership(destination, { studioRoot });
  }

  const token = `${process.pid}-${Date.now()}-${crypto.randomBytes(6).toString('hex')}`;
  const candidate = `${destination}.staging-${token}`;
  const backup = `${destination}.backup-${token}`;
  let candidateCreated = false;

  try {
    if (lstatOrNull(candidate)) {
      throw new Error(`refusing to stage: temporary candidate path already exists: ${candidate}`);
    }
    if (lstatOrNull(backup)) {
      throw new Error(`refusing to stage: temporary backup path already exists: ${backup}`);
    }
    fs.mkdirSync(candidate, { recursive: false });
    candidateCreated = true;
    const candidatePaths = nativePaths({ studioRoot, stageDir: candidate, platform, target });
    fs.copyFileSync(sourceBinary, candidatePaths.binaryPath);
    if (platform !== 'win32') fs.chmodSync(candidatePaths.binaryPath, 0o755);
    copyDirectoryTree(sourceDist, candidatePaths.staticDir);

    const sourceBinarySha = await fileSha256(sourceBinary);
    const copiedBinarySha = await fileSha256(candidatePaths.binaryPath);
    if (sourceBinarySha !== copiedBinarySha) {
      throw new Error('native binary changed while it was being copied; retry the stage');
    }
    const copiedStaticSha = await treeSha256(candidatePaths.staticDir);
    const expectedStaticSha = await treeSha256(sourceDist);
    if (copiedStaticSha !== expectedStaticSha) {
      throw new Error('static UI changed while it was being copied; retry the stage');
    }

    // Stage the exact OCCT shared libraries (and their non-system
    // dependencies), then relocate the candidate ELF runtime to $ORIGIN/lib.
    // The source binary is never modified.
    const sharedLibraries = await stageSharedLibraries({
      binaryPath: candidatePaths.binaryPath,
      destination: candidate,
      platform,
      occtLibDir,
    });
    const rpath = patchElfRpaths({
      binaryPath: candidatePaths.binaryPath,
      libDir: candidatePaths.libDir,
      libraries: sharedLibraries.libraries,
      platform,
    });
    // patchelf rewrites each staged library's dynamic section, so the
    // metadata digests must describe the relocated bytes, not the copies
    // before relocation.
    const bundledLibraries = [];
    for (const library of sharedLibraries.libraries) {
      const target = platform === 'win32'
        ? path.join(candidate, library.name)
        : path.join(candidatePaths.libDir, library.name);
      bundledLibraries.push({
        ...library,
        bytes: fs.statSync(target).size,
        sha256: await fileSha256(target),
      });
    }
    if (sharedLibraries.missing.length > 0) {
      console.warn(
        '[native-backend] the OCCT dependency closure has unresolved libraries: ' +
        sharedLibraries.missing.map((item) => item.name).join(', '),
      );
    }

    const freeCadAdapter = stageFreeCadAdapter({
      repoRoot,
      destination: candidate,
      vendor: freecadVendor,
    });
    const adapterTreeSha256 = await treeSha256(freeCadAdapter.root);
    const finalBinarySha = await fileSha256(candidatePaths.binaryPath);

    const featureList = features
      ? features.split(',').map((feature) => feature.trim()).filter(Boolean)
      : [];
    const nativeStep = featureList.includes('native-step')
      || featureList.includes('native-primitives');
    const metadata = {
      schemaVersion: 3,
      stageOwner: STAGE_OWNER,
      stageDir: destination,
      managedBy: HELPER_ID,
      package: PACKAGE_NAME,
      packageVersion: packageVersion(),
      profile: 'release',
      features: features || (noDefaultFeatures ? null : 'default'),
      nativeStep,
      target: target || null,
      platform,
      binary: candidatePaths.binaryName,
      binaryBytes: fs.statSync(candidatePaths.binaryPath).size,
      binarySha256: finalBinarySha,
      sourceBinarySha256: sourceBinarySha,
      staticDistSha256: copiedStaticSha,
      sharedLibraryMode: sharedLibraries.mode,
      sharedLibraries: bundledLibraries,
      sharedLibraryMissing: sharedLibraries.missing,
      rpath: rpath.binaryRpath,
      patchelf: rpath.patchelf,
      freeCadAdapter: {
        path: path.join(ADAPTER_DIR_NAME, 'freecad'),
        srcPath: path.join(ADAPTER_DIR_NAME, 'freecad', 'src', 'kala_agent_freecad'),
        modPath: path.join(ADAPTER_DIR_NAME, 'freecad', 'mod', 'Kala'),
        fileCount: freeCadAdapter.fileCount,
        pythonFileCount: freeCadAdapter.pythonFileCount,
        treeSha256: adapterTreeSha256,
        vendor: freeCadAdapter.vendor
          ? {
              state: 'vendored',
              manifest: path.join(ADAPTER_DIR_NAME, 'freecad', FREECAD_VENDOR_MANIFEST),
              layout: freeCadAdapter.vendor.layout,
              supportedAbis: [...freeCadAdapter.vendor.supportedAbis],
              wheelPlatform: freeCadAdapter.vendor.wheelPlatform,
              freecad: freeCadAdapter.vendor.freecad,
              requirements: freeCadAdapter.vendor.requirements,
              abis: Object.fromEntries(
                freeCadAdapter.vendor.supportedAbis.map((abi) => {
                  const entry = freeCadAdapter.vendor.abis[abi];
                  return [abi, {
                    python: entry.python,
                    wheels: entry.wheels.map((wheel) => ({
                      name: wheel.name,
                      bytes: wheel.bytes,
                      sha256: wheel.sha256,
                    })),
                    packages: [...entry.packages],
                    fileCount: entry.fileCount,
                    sourceDigest: entry.sourceDigest,
                  }];
                }),
              ),
              fileCount: freeCadAdapter.vendor.fileCount,
              sourceDigest: freeCadAdapter.vendor.sourceDigest,
              verification: freeCadAdapter.vendor.verification,
              verificationSkippedReason: freeCadAdapter.vendor.verificationSkippedReason || null,
            }
          : { state: 'absent' },
      },
      addonVerification: freeCadAdapter.addonVerification || null,
      sourceParity: currentSourceParity(),
      cargo: commandVersion('cargo', ['--version']),
      rustc: commandVersion('rustc', ['--version']),
      gitCommit: gitCommit(),
      createdBy: HELPER_ID,
    };
    fs.writeFileSync(candidatePaths.metadataPath, `${JSON.stringify(metadata, null, 2)}\n`, 'utf8');
    await verifyStageIntegrity(candidatePaths, { requireBundles: true });

    if (typeof onBeforeInstall === 'function') {
      await onBeforeInstall({
        destination,
        candidate,
        stagePaths: candidatePaths,
        originalState,
      });
    }

    // Revalidate the exact authorized destination immediately before moving
    // anything. This is the deterministic guard for a destination created or
    // modified while the candidate was being hashed above.
    assertStageStateUnchanged(originalState, captureStageState(destination), destination);

    if (originalState.exists && originalState.nonempty) {
      // The state digest binds names/sizes/mtimes; hashing the stage bytes as
      // well refuses an in-place rewrite that preserved them. Recheck the
      // directory state after the async verification to narrow the window.
      await verifyStageIntegrity(nativePaths({
        studioRoot,
        stageDir: destination,
        platform,
        target,
      }));
      assertStageStateUnchanged(originalState, captureStageState(destination), destination);
    }

    let movedExisting = false;
    if (originalState.exists) {
      if (lstatOrNull(backup)) {
        throw new Error(`refusing to stage: temporary backup path appeared during staging: ${backup}`);
      }
      fs.renameSync(destination, backup);
      movedExisting = true;

      // The rename moves whatever is at the path at that instant. Verify it
      // is still the authorized snapshot before deleting anything; otherwise
      // restore the concurrent directory untouched and refuse.
      let movedMatches = false;
      try {
        movedMatches = sameStageState(originalState, captureStageState(backup));
      } catch {
        movedMatches = false;
      }
      if (!movedMatches) {
        try {
          fs.renameSync(backup, destination);
        } catch (restoreError) {
          throw new Error(
            `refusing to stage: ${destination} changed concurrently and the concurrent ` +
            `directory is preserved at ${backup}, but it could not be restored to ` +
            `${destination}: ${restoreError.message}`,
          );
        }
        throw new Error(
          `refusing to stage: ${destination} changed concurrently while the replacement ` +
          'was being built; the concurrent directory and its files were left untouched.',
        );
      }
    }
    try {
      fs.renameSync(candidate, destination);
    } catch (error) {
      if (movedExisting) {
        try {
          fs.renameSync(backup, destination);
        } catch (restoreError) {
          throw new Error(
            `failed to install the new native stage (${error.message}) and failed to restore ` +
            `the previous stage from ${backup}: ${restoreError.message}`,
          );
        }
      }
      throw error;
    }
    if (movedExisting) {
      if (typeof onBeforeBackupRemoval === 'function') {
        await onBeforeBackupRemoval({
          destination,
          backup,
          stagePaths: nativePaths({ studioRoot, stageDir: destination, platform, target }),
        });
      }
      // Destructive cleanup only after the backup is re-verified against the
      // authorized snapshot. If anything changed, keep it and report the path.
      let backupMatches = false;
      try {
        backupMatches = sameStageState(originalState, captureStageState(backup));
      } catch {
        backupMatches = false;
      }
      if (!backupMatches) {
        console.warn(
          `[native-backend] the previous stage at ${backup} changed after it was moved; ` +
          'refusing to delete it. Inspect and remove it manually.',
        );
      } else {
        try {
          fs.rmSync(backup, { recursive: true, force: true });
        } catch (error) {
          console.warn(
            `[native-backend] the new stage is installed, but the old stage could not be ` +
            `removed from ${backup}: ${error.message}`,
          );
        }
      }
    }
    return { paths: nativePaths({ studioRoot, stageDir: destination, platform, target }), metadata };
  } catch (error) {
    if (candidateCreated) discardCandidate(candidate, destination);
    throw error;
  }
}

async function buildAndStageNative(options = {}) {
  const args = buildCargoArguments(options);
  console.log(`[native-backend] cargo ${args.join(' ')}`);
  await run('cargo', args, { cwd: options.repoRoot || REPO_ROOT });
  return stageNativeBackend(options);
}

function takeValue(remaining, argument) {
  const value = remaining.shift();
  if (value === undefined || value === '' || value.startsWith('--')) {
    throw new Error(`${argument} requires a value`);
  }
  return value;
}

function parseArguments(argv) {
  const options = {
    noBuild: false,
    stageDir: DEFAULT_STAGE_DIR,
    distDir: DEFAULT_DIST_DIR,
    target: undefined,
    noDefaultFeatures: false,
    features: undefined,
    binaryPath: undefined,
    occtLibDir: undefined,
    vendorFreeCad: true,
    requireFreeCadVendor: false,
    freeCadPython: undefined,
    freeCadPythonVersion: undefined,
    freeCadAbi: undefined,
    vendorPython: undefined,
    help: false,
  };
  const remaining = [...argv];
  while (remaining.length > 0) {
    const argument = remaining.shift();
    switch (argument) {
      case '--no-build':
        options.noBuild = true;
        break;
      case '--no-vendor-freecad':
        options.vendorFreeCad = false;
        break;
      case '--require-freecad-vendor':
        options.requireFreeCadVendor = true;
        break;
      case '--freecad-python':
        options.freeCadPython = path.resolve(takeValue(remaining, argument));
        break;
      case '--freecad-python-version':
        options.freeCadPythonVersion = takeValue(remaining, argument);
        break;
      case '--freecad-abi':
        options.freeCadAbi = takeValue(remaining, argument);
        break;
      case '--vendor-python':
        options.vendorPython = path.resolve(takeValue(remaining, argument));
        break;
      case '--stage-dir':
        options.stageDir = path.resolve(takeValue(remaining, argument));
        break;
      case '--dist':
        options.distDir = path.resolve(takeValue(remaining, argument));
        break;
      case '--target':
        options.target = takeValue(remaining, argument);
        break;
      case '--binary':
        options.binaryPath = path.resolve(takeValue(remaining, argument));
        break;
      case '--occt-lib-dir':
        options.occtLibDir = path.resolve(takeValue(remaining, argument));
        break;
      case '--no-default-features':
        options.noDefaultFeatures = true;
        break;
      case '--features':
        options.features = takeValue(remaining, argument);
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

function usage() {
  return [
    'Usage: node scripts/native-backend.cjs [options]',
    '',
    'Build and stage the native Kala Studio backend (kala-studio only), the',
    'Vite UI, the OCCT shared libraries it links, the FreeCAD adapter and the',
    "adapter's Python dependencies vendored for every supported FreeCAD Python",
    '(cp311, cp312, cp313, cp314) in per-ABI site-packages directories.',
    '',
    'Options:',
    '  --no-build               stage an existing release binary',
    '  --binary <path>          explicit kala-studio binary to stage',
    '  --stage-dir <path>       staging directory (default studio/backend-native)',
    '  --dist <path>            Vite UI directory (default studio/dist)',
    '  --target <triple>        cargo target triple (default host)',
    '  --occt-lib-dir <path>    explicit OCCT lib/DLL directory (Windows staging; Linux defaults to ldd)',
    '  --no-vendor-freecad      do not vendor the FreeCAD adapter Python dependencies',
    '  --require-freecad-vendor fail unless FreeCAD was detected, its ABI is supported and imports verified',
    '  --freecad-python <path>  explicit FreeCADCmd executable for dependency verification',
    '  --freecad-python-version <x.y>  vendor only this interpreter version (development aid)',
    '  --freecad-abi <abi>      vendor only this interpreter ABI (development aid)',
    '  --vendor-python <path>   pip-capable Python used to download/unpack wheels',
    '  --no-default-features    build without default features (the default set is empty)',
    '  --features <csv>         extra cargo features (`native-primitives` enables native STEP)',
    '  -h, --help               show this help',
  ].join('\n');
}

async function main(argv = process.argv.slice(2)) {
  let options;
  try {
    options = parseArguments(argv);
  } catch (error) {
    console.error(`[native-backend] ${error.message}`);
    console.error(usage());
    return 2;
  }
  if (options.help) {
    console.log(usage());
    return 0;
  }
  try {
    const shared = {
      stageDir: options.stageDir,
      distDir: options.distDir,
      target: options.target,
      noDefaultFeatures: options.noDefaultFeatures,
      features: options.features,
      binaryPath: options.binaryPath,
      occtLibDir: options.occtLibDir,
      freecadVendor: options.vendorFreeCad === false
        ? { mode: 'skip' }
        : {
            mode: options.requireFreeCadVendor ? 'require' : 'auto',
            executable: options.freeCadPython,
            pythonVersion: options.freeCadPythonVersion,
            abi: options.freeCadAbi,
            pipPython: options.vendorPython,
          },
    };
    const { paths, metadata } = options.noBuild
      ? await stageNativeBackend(shared)
      : await buildAndStageNative(shared);
    console.log(`[native-backend] staged ${paths.binaryPath}`);
    console.log(`[native-backend] static UI ${paths.staticDir}`);
    console.log(`[native-backend] FreeCAD adapter ${metadata.freeCadAdapter.fileCount} files`);
    const vendor = metadata.freeCadAdapter.vendor;
    if (vendor && vendor.state === 'vendored') {
      const wheels = vendor.supportedAbis
        .reduce((total, abi) => total + (vendor.abis[abi]?.wheels.length || 0), 0);
      console.log(
        `[native-backend] FreeCAD Python deps vendored for ${vendor.supportedAbis.join(', ')} ` +
        `(${wheels} wheels, ${vendor.fileCount} files, ${vendor.wheelPlatform}, ` +
        `${vendor.freecad && vendor.freecad.detected ? 'FreeCAD detected' : 'no build-host FreeCAD'})`,
      );
      if (vendor.verificationSkippedReason) {
        console.warn(`[native-backend] ${vendor.verificationSkippedReason}`);
      }
    } else {
      console.log('[native-backend] FreeCAD Python deps NOT vendored (--no-vendor-freecad)');
    }
    console.log(
      `[native-backend] OCCT libs ${metadata.sharedLibraries.length} (${metadata.sharedLibraryMode}), ` +
      `rpath ${metadata.rpath || 'unchanged (LD_LIBRARY_PATH fallback)'}`,
    );
    console.log(`[native-backend] sha256 ${metadata.binarySha256}`);
    console.log(
      `[native-backend] current-feature readiness: ${metadata.sourceParity.summary}`,
    );
    return 0;
  } catch (error) {
    console.error(`[native-backend] ${error.message}`);
    return 1;
  }
}

if (require.main === module) {
  main().then((code) => process.exit(code));
}

module.exports = {
  STUDIO_ROOT,
  REPO_ROOT,
  BACKEND_ROOT,
  DEFAULT_STAGE_DIR,
  DEFAULT_DIST_DIR,
  METADATA_NAME,
  STATIC_DIR_NAME,
  SHARED_LIB_DIR_NAME,
  ADAPTER_DIR_NAME,
  FREECAD_SITE_PACKAGES_PREFIX,
  FREECAD_PYTHON_ABIS,
  STAGE_OWNER,
  HELPER_ID,
  ORIGIN_LIB_RPATH,
  ORIGIN_RPATH,
  nativeBinaryName,
  cargoTargetDirectory,
  cargoBinaryPath,
  buildCargoArguments,
  nativePaths,
  assertSafeStageDir,
  assertNoSymlinkAncestors,
  assertSourceTreeNoSymlinks,
  assertRegularSourceFile,
  directoryStateDigest,
  captureStageState,
  sameStageState,
  assertStageStateUnchanged,
  fileSha256,
  treeSha256,
  binaryContainsNativeStep,
  validateNativeTree,
  verifyStageIntegrity,
  assertStageSourceParity,
  assertStageUiMatches,
  validateStageOwnership,
  stageFreeCadAdapter,
  verifyStagedFreeCadAddOn,
  FREECAD_ADDON_VERIFY_MARKER,
  validateBundledFreeCadAdapter,
  validateFreeCadVendorManifest,
  isFreeCadVendorPath,
  assertFreeCadAdapterBundled,
  FREECAD_VENDOR_MANIFEST,
  parseLddOutput,
  isOcctSharedLibraryName,
  isSystemLibraryPath,
  collectLddSharedLibraries,
  listExplicitLibraries,
  stageSharedLibraries,
  patchElfRpaths,
  currentSourceParity,
  stageNativeBackend,
  buildAndStageNative,
  parseArguments,
  main,
};
