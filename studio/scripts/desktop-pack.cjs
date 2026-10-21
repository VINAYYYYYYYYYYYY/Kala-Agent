#!/usr/bin/env node
// Packaging helper.
//
// Verifies the Vite UI and the native staged backend (`backend-native/`,
// produced by scripts/native-backend.cjs) are present and intact, checks the
// current-feature readiness gate, then runs the installed electron-builder CLI
// with the process's own argv forwarded (so conductor flags like `--dir`
// survive).
//
// Release safety:
//   * Binary/static SHA-256 metadata is recomputed; a corrupted or stale stage
//     is refused before electron-builder sees it.
//   * The stage must have been produced from the current Studio Python/Rust
//     sources (`sourceParity` digests). Current-feature native-design readiness
//     is the release gate: every public route/DTO the Python reference exposes
//     must be covered by the native front door. When it is pending, release
//     packaging/NSIS is refused. An unpacked directory build remains possible
//     only when the effective electron-builder targets (parsed by the installed
//     electron-builder configuration, not by string matching) are all the `dir`
//     target; it is labelled in `release/native-readiness.json`.
//   * The bundled FreeCAD adapter and OCCT shared libraries are part of the
//     package contract; a stage missing the adapter is refused.
//   * `--publish` is always forced to `never`; caller overrides are removed.
//     No upload API is invoked by this helper or its tests.
//
// The legacy Python staging path `studio/backend/` is never inspected or
// copied.

'use strict';

const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const nativeBackend = require('./native-backend.cjs');
const readiness = require('./native-readiness.cjs');

const studioRoot = path.resolve(__dirname, '..');
const READINESS_REPORT = path.join('release', 'native-readiness.json');

function exists(p) {
  try { return fs.statSync(p).isFile(); } catch { return false; }
}

function findBuilderCli() {
  // electron-builder exposes its CLI entrypoint at this path in 26.x.
  const candidates = [
    'electron-builder/out/cli/cli.js',
    'electron-builder/out/cli.js',
    'electron-builder/bin/electron-builder.js',
  ];
  for (const candidate of candidates) {
    try {
      return require.resolve(candidate);
    } catch {
      // try the next candidate
    }
  }
  throw new Error('electron-builder CLI entrypoint not found');
}

/**
 * Remove every caller-supplied publish flag (`--publish`, `--publish=...`,
 * `-p`) and append a single `--publish never`. A missing publish value must
 * not swallow the next legitimate flag.
 */
function sanitizePublishArguments(argv) {
  const cleaned = [];
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--publish' || argument === '-p') {
      const next = argv[index + 1];
      if (next !== undefined && !next.startsWith('-')) index += 1;
      continue;
    }
    if (argument.startsWith('--publish=')) continue;
    if (argument.startsWith('-p=')) continue;
    cleaned.push(argument);
  }
  cleaned.push('--publish', 'never');
  return cleaned;
}

function loadBuilderApi() {
  try {
    return require('electron-builder/out/builder.js');
  } catch (cause) {
    throw new Error(
      `electron-builder argument parser is unavailable (${cause.message}); run \`npm install\` in studio/`,
    );
  }
}

/**
 * Parse the exact arguments forwarded to electron-builder with the installed
 * electron-builder/yargs configuration, then normalize them through
 * electron-builder's own `normalizeOptions`.
 *
 * The packaging gate must never infer `--dir` by string presence: yargs
 * accepts `--dir=false`, `--no-dir`, repeated flags and explicit platform
 * targets, where the last occurrence wins. `--dir --win nsis` also produces a
 * real NSIS target because the explicit platform target replaces the unpacked
 * default. Only an invocation whose effective targets are all the electron-
 * builder `dir` target is an unpacked development build.
 */
function normalizeBuilderInvocation(argv) {
  const { createYargs, configureBuildCommand, normalizeOptions } = loadBuilderApi();
  let parsed;
  try {
    parsed = configureBuildCommand(
      createYargs().exitProcess(false).strict().showHelpOnFail(false),
    ).parse([...argv]);
  } catch (cause) {
    throw new Error(
      `electron-builder arguments are invalid or ambiguous and cannot be safety-classified: ` +
      `${cause.message}`,
    );
  }
  const normalized = normalizeOptions(parsed);
  const targets = [];
  for (const [platform, archToType] of normalized.targets) {
    for (const [, types] of archToType) {
      for (const type of types) {
        targets.push({ platform: platform.name, type });
      }
    }
  }
  const dir = parsed.dir === true;
  const unpackedOnly =
    dir && targets.length > 0 && targets.every((target) => target.type === 'dir');
  return {
    argv: [...argv],
    dir,
    win: parsed.win,
    mac: parsed.mac,
    linux: parsed.linux,
    targets,
    unpackedOnly,
  };
}

function isDevelopmentDirectoryBuild(argv) {
  try {
    return normalizeBuilderInvocation(argv).unpackedOnly;
  } catch {
    return false;
  }
}

/**
 * Decide whether this invocation may produce an artifact.
 *
 * Only an invocation whose effective electron-builder targets are all the
 * unpacked `dir` target is treated as a development review artifact while
 * current-feature parity is pending. Every installer/release target (including
 * one requested alongside `--dir`) requires a ready report. Invalid or
 * ambiguous arguments fail closed before electron-builder is spawned.
 */
function decidePackaging({ argv, report }) {
  const invocation = normalizeBuilderInvocation(argv);
  if (!report || report.ready !== true) {
    if (!invocation.unpackedOnly) {
      const missing = [];
      if (report && Array.isArray(report.missingRouteMethods)) {
        missing.push(...report.missingRouteMethods);
      }
      if (report && Array.isArray(report.missingCapabilities)) {
        missing.push(...report.missingCapabilities);
      }
      const detail = missing.length > 0 ? ` Missing: ${missing.join(', ')}.` : '';
      const effectiveTargets = invocation.targets.length > 0
        ? invocation.targets.map((target) => `${target.platform}:${target.type}`).join(', ')
        : 'no unpacked dir target';
      throw new Error(
        'current-feature native readiness is not satisfied; release packaging/NSIS is blocked. ' +
        `Effective electron-builder targets: ${effectiveTargets}.` +
        `${detail} Keep the Python reference as the working default until the expanded native ` +
        'design/journal port passes compatibility gates, or use an effective `--dir`-only ' +
        'invocation for a clearly labelled development review artifact.',
      );
    }
    return {
      dirMode: true,
      classification: 'development-unpacked-artifact',
      releaseEligible: false,
      warning:
        'current-feature native parity is pending; this unpacked directory is a development ' +
        'review artifact and must not be distributed as a release.',
    };
  }
  const geometrySeam = report && report.native
    && Number.isInteger(report.native.geometrySeamRouteMethodCount)
    ? report.native.geometrySeamRouteMethodCount
    : (report && report.native && Number.isInteger(report.native.proxiedRouteMethodCount)
      ? report.native.proxiedRouteMethodCount
      : 0);
  const nativeDesignWarning = geometrySeam > 0
    ? `${geometrySeam} route-method(s) go through the geometry-backend seam; the default ` +
      'build answers them from the native kala-studio backend (the supervised Python worker is a ' +
      'development opt-in). The packaged AppImage smoke with --require-geometry/--require-freecad ' +
      'is the release evidence.'
    : null;
  return {
    dirMode: invocation.unpackedOnly,
    classification: invocation.unpackedOnly ? 'development-unpacked-artifact' : 'release',
    releaseEligible: !invocation.unpackedOnly,
    warning: invocation.unpackedOnly
      ? 'unpacked directory build for development review; no installer was produced.'
      : nativeDesignWarning,
  };
}

function writeReadinessArtifact(report, decision) {
  const target = path.join(studioRoot, READINESS_REPORT);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  const payload = {
    schemaVersion: readiness.SCHEMA_VERSION,
    kind: readiness.KIND,
    generatedAt: new Date().toISOString(),
    classification: decision.classification,
    releaseEligible: decision.releaseEligible,
    warning: decision.warning,
    readiness: report,
  };
  fs.writeFileSync(target, `${JSON.stringify(payload, null, 2)}\n`, 'utf8');
  return target;
}

async function main(argv = process.argv.slice(2)) {
  // The gate must classify the exact arguments that electron-builder will
  // parse. Sanitize publication first, then classify the forwarded argv.
  const forwarded = sanitizePublishArguments(argv);
  const distIndex = path.join(studioRoot, 'dist', 'index.html');
  if (!exists(distIndex)) {
    console.error('[desktop-pack] UI dist missing. Run `npm run build` first.');
    return 1;
  }
  const nativePaths = nativeBackend.nativePaths({ studioRoot });
  if (!exists(nativePaths.binaryPath)) {
    console.error(
      `[desktop-pack] native backend binary missing at ${nativePaths.binaryPath}. ` +
      'Run `npm run build:native` first; the packaged resources must contain only ' +
      'the native runtime/UI, never the Python staging directory.',
    );
    return 1;
  }
  if (!exists(nativePaths.indexFile)) {
    console.error(
      `[desktop-pack] native static UI missing at ${nativePaths.indexFile}. ` +
      'Run `npm run build` and `npm run build:native` first.',
    );
    return 1;
  }

  let metadata;
  let report;
  let decision;
  try {
    if (!exists(nativePaths.metadataPath)) {
      throw new Error(
        `native build metadata missing at ${nativePaths.metadataPath}; ` +
        'stage the backend with `npm run build:native`',
      );
    }
    metadata = await nativeBackend.verifyStageIntegrity(nativePaths, { requireBundles: true });
    nativeBackend.validateNativeTree(nativePaths, { requireBundles: true });
    const adapter = nativeBackend.assertFreeCadAdapterBundled(nativePaths);
    if (!adapter.vendor) {
      throw new Error(
        'the FreeCAD adapter was staged without its Python dependencies; the installed app ' +
        'could not run the FreeCAD provider. Restage with `npm run build:native` (FreeCAD ' +
        'must be installed, or pass --freecad-python-version/--freecad-abi).',
      );
    }
    report = readiness.checkStudioReadiness();
    nativeBackend.assertStageSourceParity(metadata, report);
    await nativeBackend.assertStageUiMatches(metadata, {
      distDir: path.join(studioRoot, 'dist'),
    });
    decision = decidePackaging({ argv: forwarded, report });
  } catch (cause) {
    console.error(`[desktop-pack] ${cause instanceof Error ? cause.message : cause}`);
    return 1;
  }

  const reportPath = writeReadinessArtifact(report, decision);
  console.log(`[desktop-pack] ${report.summary}`);
  console.log(`[desktop-pack] readiness report: ${reportPath}`);
  if (decision.warning) {
    console.warn(`[desktop-pack] ${decision.classification.toUpperCase()}: ${decision.warning}`);
  }

  let builderCli;
  try {
    builderCli = findBuilderCli();
  } catch (cause) {
    console.error('[desktop-pack]', cause.message);
    return 1;
  }

  const args = [builderCli, '--config', 'electron-builder.json', ...forwarded];
  console.log('[desktop-pack] argv:', args.join(' '));

  return new Promise((resolve) => {
    const child = spawn(process.execPath, args, {
      cwd: studioRoot,
      stdio: 'inherit',
      env: process.env,
      windowsHide: true,
    });
    child.on('error', (cause) => {
      console.error('[desktop-pack] electron-builder spawn failed:', cause);
      resolve(1);
    });
    child.on('exit', (code, signal) => {
      if (signal) {
        console.error(`[desktop-pack] electron-builder exited via ${signal}`);
        resolve(1);
        return;
      }
      resolve(code ?? 1);
    });
  });
}

if (require.main === module) {
  main()
    .then((code) => process.exit(code))
    .catch((error) => {
      console.error('[desktop-pack] unexpected failure:', error);
      process.exit(1);
    });
}

module.exports = {
  sanitizePublishArguments,
  normalizeBuilderInvocation,
  isDevelopmentDirectoryBuild,
  decidePackaging,
  writeReadinessArtifact,
  sanitizePublishArgs: sanitizePublishArguments,
  main,
  READINESS_REPORT,
};
