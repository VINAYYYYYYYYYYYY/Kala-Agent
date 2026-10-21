// Native backend build/staging, readiness and desktop packaging tests.
//
// These tests exercise the helper that produces studio/backend-native/,
// its transactional ownership/symlink guards, the Electron resolver that
// consumes it, and the electron-builder resource configuration. When a stage
// exists, the final test runs a real native child through the same
// BackendProcess class the Electron main process uses (health probe +
// graceful shutdown), with no Python on the command path.
//
// Destructive guard tests only use temporary directories and non-destructive
// probes against real project paths; no test deletes source/control data.

import { afterEach, describe, expect, it } from 'vitest';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const require = createRequire(import.meta.url);
const nativeBackend = require('../scripts/native-backend.cjs') as typeof import('../scripts/native-backend.cjs');
const nativeReadiness = require('../scripts/native-readiness.cjs') as typeof import('../scripts/native-readiness.cjs');
const desktopPack = require('../scripts/desktop-pack.cjs') as typeof import('../scripts/desktop-pack.cjs');
const electronMain = require('../electron/main.cjs') as typeof import('../electron/main.cjs');

const temporaryDirectories: string[] = [];

function makeTemporaryDirectory(): string {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'kala-native-backend-'));
  temporaryDirectories.push(directory);
  return directory;
}

function writeFile(filePath: string, contents: string | Buffer): void {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
}

function makeDist(directory: string): string {
  writeFile(path.join(directory, 'index.html'), '<html><body><div id="root"></div></body></html>');
  writeFile(path.join(directory, 'assets', 'app.js'), 'console.log("kala");\n');
  writeFile(path.join(directory, 'assets', 'app.css'), 'body { color: #fff; }\n');
  return directory;
}

function makeBinary(filePath: string, contents = 'native-binary ISO-10303-21;'): string {
  writeFile(filePath, contents);
  if (process.platform !== 'win32') fs.chmodSync(filePath, 0o755);
  return filePath;
}

function stageFixture(root: string, name = 'backend-native'): {
  stage: string;
  binary: string;
  dist: string;
} {
  const binary = makeBinary(path.join(root, 'cargo-target', 'release', 'kala-studio'));
  const dist = makeDist(path.join(root, 'dist'));
  return { stage: path.join(root, name), binary, dist };
}

function sortedFiles(root: string): string[] {
  return fs.readdirSync(root).sort();
}

/**
 * Write the multi-ABI vendored dependency layout a real staging run produces:
 * `src/kala_agent` once plus one `site-packages-cpNNN` per supported ABI and a
 * schema-2 `vendor.json`.
 */
function writeVendorMatrix(adapter: string, options: { keepManifest?: boolean } = {}): void {
  const abis = ['cp311', 'cp312', 'cp313', 'cp314'];
  writeFile(path.join(adapter, 'src', 'kala_agent', '__init__.py'), '');
  const entries: Record<string, unknown> = {};
  for (const abi of abis) {
    const directory = path.join(adapter, `site-packages-${abi}`);
    writeFile(path.join(directory, 'pydantic', '__init__.py'), '');
    writeFile(path.join(directory, 'pydantic_core', '__init__.py'), '');
    writeFile(
      path.join(directory, 'pydantic_core', `_pydantic_core.cpython-${abi.slice(2)}-x86_64-linux-gnu.so`),
      'elf',
    );
    writeFile(path.join(directory, 'pydantic-2.13.5.dist-info', 'METADATA'), 'Name: pydantic');
    entries[abi] = {
      python: `3.${abi.slice(3)}`,
      abi,
      wheelPlatform: 'manylinux2014_x86_64',
      wheels: [
        {
          name: `pydantic_core-2.46.5-${abi}-${abi}-manylinux_2_17_x86_64.whl`,
          bytes: 4,
          sha256: 'a'.repeat(64),
        },
      ],
      packages: ['pydantic', 'pydantic_core', 'kala_agent'],
      fileCount: 4,
      sourceDigest: 'b'.repeat(64),
    };
  }
  if (options.keepManifest) return;
  writeFile(
    path.join(adapter, 'vendor.json'),
    JSON.stringify({
      schemaVersion: 2,
      kind: 'staged-vendor',
      layout: 'site-packages-{abi}',
      platform: 'manylinux2014_x86_64',
      wheelPlatform: 'manylinux2014_x86_64',
      supportedAbis: abis,
      abis: entries,
      requirements: ['pydantic==2.13.5'],
      kalaAgent: { path: 'kala_agent', source: '/repo/src/kala_agent', files: 1, treeSha256: 'c'.repeat(64) },
      fileCount: 17,
      sourceDigest: 'd'.repeat(64),
    }),
  );
}

interface RustRouteGroup {
  path: string;
  methods: string[];
  proxied?: boolean;
}

/**
 * Build the r2 `RouteKind` route table shape the readiness scanner parses:
 * `classify(path)` returns a kind, `allowed_methods(kind)` returns its
 * methods, and the `route` dispatch names the kinds that call `proxy_route`.
 */
function syntheticRustRouteTable(groups: RustRouteGroup[]): string {
  const variants = groups.map((_, index) => `Route${index}`);
  const classifierArms = groups
    .map((group, index) => `        "${group.path}" => RouteKind::${variants[index]},`)
    .join('\n');
  const methodArms = groups
    .map((group, index) => {
      const methods = group.methods.map((method) => `Method::${method}`).join(', ');
      return `        RouteKind::${variants[index]} => &[${methods}],`;
    })
    .join('\n');
  const proxiedVariants = groups
    .map((group, index) => (group.proxied ? `RouteKind::${variants[index]}` : null))
    .filter((variant): variant is string => variant !== null);
  const nativeVariants = groups
    .map((group, index) => (group.proxied ? null : `RouteKind::${variants[index]}`))
    .filter((variant): variant is string => variant !== null);
  const dispatchArms = [
    ...nativeVariants.map((variant) => `        ${variant} => 200,`),
    ...(proxiedVariants.length > 0
      ? [
        `        ${proxiedVariants.join(' | ')} => {`,
        '            proxy_route(kind)',
        '        }',
      ]
      : []),
  ].join('\n');
  return [
    'enum RouteKind {',
    ...variants.map((variant) => `    ${variant},`),
    '}',
    'fn classify(path: &str) -> Option<RouteKind> {',
    '    let kind = match path {',
    classifierArms,
    '        _ => return None,',
    '    };',
    '    Some(kind)',
    '}',
    "fn allowed_methods(kind: RouteKind) -> &'static [Method] {",
    '    match kind {',
    methodArms,
    '    }',
    '}',
    'fn route(kind: RouteKind) -> u16 {',
    '    match kind {',
    dispatchArms,
    '    }',
    '}',
    ...(proxiedVariants.length > 0
      ? ['fn proxy_route(kind: RouteKind) -> u16 {', '    let _ = kind;', '    200', '}']
      : []),
  ].join('\n');
}

/** A deterministic not-ready report, so packaging-gate tests do not depend on
 * the current source readiness. */
function pendingReadinessReport(): ReturnType<typeof nativeReadiness.checkStudioReadiness> {
  return {
    ...nativeReadiness.checkStudioReadiness(),
    ready: false,
    missingRouteMethods: ['POST /api/studio/design'],
    missingCapabilities: ['health.design_available'],
    pendingFeatures: [{ id: 'design.compile', label: 'POST /design' }],
    summary: 'synthetic pending readiness (test fixture)',
  };
}

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

describe('native backend helper', () => {
  it('uses platform-native binary names, including cross-target Windows', () => {
    expect(nativeBackend.nativeBinaryName({ platform: 'linux' })).toBe('kala-studio');
    expect(nativeBackend.nativeBinaryName({ platform: 'win32' })).toBe('kala-studio.exe');
    expect(nativeBackend.nativeBinaryName({ platform: 'linux', target: 'x86_64-pc-windows-msvc' }))
      .toBe('kala-studio.exe');
  });

  it('builds only kala-studio with locked release arguments and no shell', () => {
    const args = nativeBackend.buildCargoArguments({
      backendRoot: '/repo/backend',
      targetDir: '/repo/backend/target',
      target: 'x86_64-unknown-linux-gnu',
    });
    expect(args[0]).toBe('build');
    expect(args).toContain('--manifest-path');
    expect(args).toContain(path.join('/repo/backend', 'Cargo.toml'));
    expect(args).toContain('--package');
    expect(args[args.indexOf('--package') + 1]).toBe('kala-studio');
    expect(args).toContain('--release');
    expect(args).toContain('--locked');
    expect(args).toContain('--target-dir');
    expect(args[args.indexOf('--target-dir') + 1]).toBe(path.resolve('/repo/backend/target'));
    expect(args[args.indexOf('--target') + 1]).toBe('x86_64-unknown-linux-gnu');
    expect(args).not.toContain('--workspace');
    expect(args.join(' ')).not.toContain(';');
  });

  it('stages a clean native binary, static UI, ownership and digest metadata', async () => {
    const root = makeTemporaryDirectory();
    const { stage, binary, dist } = stageFixture(root);
    const result = await nativeBackend.stageNativeBackend({
      stageDir: stage,
      distDir: dist,
      binaryPath: binary,
      platform: 'linux',
    });

    expect(result.paths.binaryPath).toBe(path.join(stage, 'kala-studio'));
    expect(fs.readFileSync(result.paths.binaryPath, 'utf8')).toBe('native-binary ISO-10303-21;');
    expect(fs.readFileSync(result.paths.indexFile, 'utf8')).toContain('id="root"');
    expect(fs.existsSync(path.join(stage, 'studio_dist', 'assets', 'app.js'))).toBe(true);
    expect(result.metadata.binarySha256).toBe(await nativeBackend.fileSha256(binary));
    expect(result.metadata.staticDistSha256).toMatch(/^[a-f0-9]{64}$/);
    // r2 parks the native geometry behind the non-default `native-primitives`
    // feature; the default front door proxies geometry to the Python worker.
    expect(result.metadata.nativeStep).toBe(false);
    expect(result.metadata.features).toBe('default');
    expect(result.metadata.package).toBe('kala-studio');
    expect(result.metadata.schemaVersion).toBe(3);
    expect(result.metadata.stageOwner).toBe(nativeBackend.STAGE_OWNER);
    expect(result.metadata.stageDir).toBe(stage);
    expect(result.metadata.freeCadAdapter).toMatchObject({
      path: path.join('adapters', 'freecad'),
      srcPath: path.join('adapters', 'freecad', 'src', 'kala_agent_freecad'),
      modPath: path.join('adapters', 'freecad', 'mod', 'Kala'),
    });
    expect(result.metadata.freeCadAdapter!.pythonFileCount).toBeGreaterThan(10);
    expect(result.metadata.freeCadAdapter!.treeSha256).toMatch(/^[a-f0-9]{64}$/);
    expect(result.metadata.sharedLibraries).toEqual([]);
    expect(
      fs.existsSync(path.join(stage, 'adapters', 'freecad', 'src', 'kala_agent_freecad', '__init__.py')),
    ).toBe(true);
    expect(
      fs.existsSync(path.join(stage, 'adapters', 'freecad', 'mod', 'Kala', 'InitGui.py')),
    ).toBe(true);
    expect(nativeBackend.assertFreeCadAdapterBundled(result.paths).pythonFileCount)
      .toBe(result.metadata.freeCadAdapter!.pythonFileCount);
    expect(result.metadata.sourceParity.pythonSourceSha256).toMatch(/^[a-f0-9]{64}$/);
    expect(result.metadata.sourceParity.nativeSourceSha256).toMatch(/^[a-f0-9]{64}$/);
    expect(nativeBackend.validateNativeTree(result.paths)).toBe(true);
    await expect(nativeBackend.verifyStageIntegrity(result.paths)).resolves.toBeTruthy();
  });

  it('bundles the FreeCAD adapter and refuses to package without it', async () => {
    const root = makeTemporaryDirectory();
    const { stage, binary, dist } = stageFixture(root);
    const result = await nativeBackend.stageNativeBackend({
      stageDir: stage,
      distDir: dist,
      binaryPath: binary,
      platform: 'linux',
    });
    expect(nativeBackend.assertFreeCadAdapterBundled(result.paths).root)
      .toBe(path.join(stage, 'adapters', 'freecad'));
    expect(() => nativeBackend.assertFreeCadAdapterBundled({
      ...result.paths,
      stageDir: path.join(root, 'missing-adapter-stage'),
      adapterDir: path.join(root, 'missing-adapter-stage', 'adapters'),
    })).toThrow(/FreeCAD adapter module/);

    // Bytecode never belongs in the adapter subtree.
    const cache = path.join(
      stage,
      'adapters',
      'freecad',
      'src',
      'kala_agent_freecad',
      '__pycache__',
    );
    writeFile(path.join(cache, 'contracts.cpython-314.pyc'), 'bytecode');
    expect(() => nativeBackend.validateNativeTree(result.paths)).toThrow(/__pycache__/);
    fs.rmSync(cache, { recursive: true, force: true });
    expect(nativeBackend.validateNativeTree(result.paths)).toBe(true);
  });

  it('accepts a four-ABI FreeCAD vendor tree and refuses interpreter payloads', async () => {
    const root = makeTemporaryDirectory();
    const { stage, binary, dist } = stageFixture(root);
    const result = await nativeBackend.stageNativeBackend({
      stageDir: stage,
      distDir: dist,
      binaryPath: binary,
      platform: 'linux',
    });
    const adapter = path.join(stage, 'adapters', 'freecad');
    writeVendorMatrix(adapter);
    expect(nativeBackend.isFreeCadVendorPath('adapters/freecad/site-packages-cp311/pydantic/main.py'))
      .toBe(true);
    for (const abi of ['cp311', 'cp312', 'cp313', 'cp314']) {
      expect(
        nativeBackend.isFreeCadVendorPath(`adapters/freecad/site-packages-${abi}/pydantic_core/__init__.py`),
      ).toBe(true);
    }
    expect(nativeBackend.isFreeCadVendorPath('adapters/freecad/site-packages-cp310/pydantic/main.py'))
      .toBe(false);
    expect(nativeBackend.isFreeCadVendorPath('adapters/freecad/src/kala_agent/__init__.py')).toBe(true);
    expect(nativeBackend.isFreeCadVendorPath('adapters/freecad/src/pydantic/main.py')).toBe(false);
    expect(nativeBackend.isFreeCadVendorPath('adapters/freecad/src/kala_agent_freecad/worker.py'))
      .toBe(false);
    const manifest = nativeBackend.validateFreeCadVendorManifest(adapter)!;
    expect(manifest.schemaVersion).toBe(2);
    expect(manifest.supportedAbis).toEqual(['cp311', 'cp312', 'cp313', 'cp314']);
    expect(nativeBackend.validateNativeTree(result.paths)).toBe(true);

    // Every pinned ABI is present in the checked dependency matrix.
    const pins = JSON.parse(
      fs.readFileSync(path.join(__dirname, '..', 'scripts', 'freecad-python-deps.json'), 'utf8'),
    );
    expect(pins.python.supported.map((entry: { abi: string }) => entry.abi))
      .toEqual(['cp311', 'cp312', 'cp313', 'cp314']);
    expect(nativeBackend.FREECAD_SITE_PACKAGES_PREFIX).toBe('site-packages-');

    // A missing ABI set is refused with the exact directory.
    const removed = path.join(adapter, 'site-packages-cp313');
    fs.rmSync(removed, { recursive: true, force: true });
    expect(() => nativeBackend.validateFreeCadVendorManifest(adapter)).toThrow(
      /site-packages-cp313.*pydantic.*is missing/,
    );
    expect(() => nativeBackend.validateNativeTree(result.paths)).toThrow(/site-packages-cp313/);
    writeVendorMatrix(adapter, { keepManifest: true });

    // A Python interpreter or a vendor site-packages tree is still refused,
    // even inside the one subtree where adapter .py/.so files are legitimate.
    const interpreter = path.join(adapter, 'site-packages-cp314', 'pydantic', 'python3.14');
    writeFile(interpreter, 'not an interpreter');
    expect(() => nativeBackend.validateNativeTree(result.paths)).toThrow(/Python\/OCP/);
    fs.rmSync(interpreter);
    const sitePackages = path.join(adapter, 'site-packages-cp314', 'pydantic', 'site-packages');
    writeFile(path.join(sitePackages, 'evil.py'), '');
    expect(() => nativeBackend.validateNativeTree(result.paths)).toThrow(/Python\/OCP/);
    fs.rmSync(sitePackages, { recursive: true, force: true });
    expect(nativeBackend.validateNativeTree(result.paths)).toBe(true);
  });

  it('records Linux and Windows wheel digests for every supported ABI', () => {
    const matrixPath = path.join(__dirname, '..', '..', 'adapters', 'freecad', 'vendor.json');
    const matrix = JSON.parse(fs.readFileSync(matrixPath, 'utf8'));
    expect(matrix.kind).toBe('dependency-matrix');
    expect(matrix.supportedAbis).toEqual(['cp311', 'cp312', 'cp313', 'cp314']);
    expect(matrix.layout).toBe('site-packages-{abi}');
    expect(Object.keys(matrix.targets).sort()).toEqual(['linux-x86_64', 'win_amd64']);
    for (const target of Object.values(matrix.targets) as Array<{
      wheelPlatform: string;
      abis: Record<string, { wheels: Array<{ name: string; sha256: string }> }>;
    }>) {
      expect(Object.keys(target.abis).sort()).toEqual(['cp311', 'cp312', 'cp313', 'cp314']);
      for (const [abi, entry] of Object.entries(target.abis)) {
        expect(entry.wheels.length).toBeGreaterThanOrEqual(5);
        const core = entry.wheels.find((wheel) => wheel.name.includes('pydantic_core'));
        expect(core?.name).toContain(`-${abi}-`);
        for (const wheel of entry.wheels) {
          expect(wheel.sha256).toMatch(/^[a-f0-9]{64}$/);
        }
      }
    }
    expect(matrix.targets['win_amd64'].wheelPlatform).toBe('win_amd64');
    expect(matrix.targets['linux-x86_64'].wheelPlatform).toBe('manylinux2014_x86_64');
  });

  it('verifies the bundled adapter tree digest and detects tampering', async () => {    const root = makeTemporaryDirectory();
    const { stage, binary, dist } = stageFixture(root);
    const result = await nativeBackend.stageNativeBackend({
      stageDir: stage,
      distDir: dist,
      binaryPath: binary,
      platform: 'linux',
    });
    const target = path.join(
      stage,
      'adapters',
      'freecad',
      'src',
      'kala_agent_freecad',
      'contracts.py',
    );
    fs.appendFileSync(target, '\n# tampered\n');
    await expect(nativeBackend.verifyStageIntegrity(result.paths)).rejects.toThrow(
      /FreeCAD adapter digest/,
    );
  });

  it('stages explicit Windows OCCT DLLs next to the executable', async () => {
    const root = makeTemporaryDirectory();
    const binary = makeBinary(path.join(root, 'cargo-target', 'release', 'kala-studio.exe'));
    const dist = makeDist(path.join(root, 'dist'));
    const occt = path.join(root, 'vcpkg-bin');
    writeFile(path.join(occt, 'TKernel.dll'), 'kernel-dll');
    writeFile(path.join(occt, 'TKMath.dll'), 'math-dll');
    writeFile(path.join(occt, 'unrelated.txt'), 'not a dll');

    const result = await nativeBackend.stageNativeBackend({
      stageDir: path.join(root, 'backend-native'),
      distDir: dist,
      binaryPath: binary,
      platform: 'win32',
      occtLibDir: occt,
    });

    expect(result.metadata.sharedLibraryMode).toBe('explicit');
    expect(result.metadata.sharedLibraries!.map((library) => library.name).sort())
      .toEqual(['TKMath.dll', 'TKernel.dll']);
    expect(fs.existsSync(path.join(result.paths.stageDir, 'TKernel.dll'))).toBe(true);
    expect(fs.existsSync(path.join(result.paths.stageDir, 'unrelated.txt'))).toBe(false);
    await expect(nativeBackend.verifyStageIntegrity(result.paths, { requireBundles: true }))
      .resolves.toBeTruthy();
  });

  it('parses ldd output into loader names and resolved paths', () => {
    const parsed = nativeBackend.parseLddOutput([
      '\tlinux-vdso.so.1 (0x00007ffd)',
      '\tlibTKernel.so.7 => /usr/lib/libTKernel.so.7.9.3 (0x00007f00)',
      '\tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f01)',
      '\tlibmissing.so.1 => not found',
    ].join('\n'));
    expect(parsed.entries).toEqual([
      { name: 'libTKernel.so.7', path: '/usr/lib/libTKernel.so.7.9.3' },
      { name: 'libc.so.6', path: '/lib/x86_64-linux-gnu/libc.so.6' },
    ]);
    expect(parsed.missing).toEqual(['libmissing.so.1']);
    expect(nativeBackend.isOcctSharedLibraryName('libTKernel.so.7')).toBe(true);
    expect(nativeBackend.isOcctSharedLibraryName('libc.so.6')).toBe(false);
    expect(nativeBackend.isSystemLibraryPath('/usr/lib/x86_64-linux-gnu/libc.so.6')).toBe(true);
    expect(nativeBackend.isSystemLibraryPath('/opt/occt/lib/libTKernel.so.7')).toBe(false);
  });

  it('rejects Python/OCP/vendor artifacts in the native stage', async () => {
    const root = makeTemporaryDirectory();
    const { stage, binary, dist } = stageFixture(root);
    const result = await nativeBackend.stageNativeBackend({
      stageDir: stage,
      distDir: dist,
      binaryPath: binary,
      platform: 'linux',
    });
    writeFile(path.join(stage, 'site-packages', 'ocp', 'python.exe'), 'not native');
    expect(() => nativeBackend.validateNativeTree(result.paths)).toThrow(/Python\/OCP/);
  });

  it('treats native primitives as the non-default feature and still requires its marker', async () => {
    const root = makeTemporaryDirectory();
    const { stage, dist } = stageFixture(root);
    const proxyOnly = makeBinary(path.join(root, 'kala-studio'), 'ELF-without-step');

    // The default r2 build links no native geometry, so the marker is not
    // required and the metadata must not claim native STEP.
    const staged = await nativeBackend.stageNativeBackend({
      stageDir: stage,
      distDir: dist,
      binaryPath: proxyOnly,
      platform: 'linux',
    });
    expect(staged.metadata.nativeStep).toBe(false);

    // A build that opts into `native-primitives` claims native STEP, so the
    // marker is still mandatory; a binary without it is refused.
    const primitivesStage = path.join(root, 'native-primitives-stage');
    await expect(nativeBackend.stageNativeBackend({
      stageDir: primitivesStage,
      distDir: dist,
      binaryPath: proxyOnly,
      platform: 'linux',
      features: 'native-primitives',
    })).rejects.toThrow(/native primitives/);
    expect(fs.existsSync(primitivesStage)).toBe(false);

    const nativeStage = path.join(root, 'native-step-stage');
    const nativeBinary = makeBinary(path.join(root, 'kala-studio-native-step'));
    const nativeStaged = await nativeBackend.stageNativeBackend({
      stageDir: nativeStage,
      distDir: dist,
      binaryPath: nativeBinary,
      platform: 'linux',
      features: 'native-step',
    });
    expect(nativeStaged.metadata.nativeStep).toBe(true);
  });

  it('refuses protected project/control paths without creating or deleting anything', () => {
    expect(() => nativeBackend.assertSafeStageDir(nativeBackend.DEFAULT_DIST_DIR))
      .toThrow(/protected path/);
    expect(() => nativeBackend.assertSafeStageDir(
      path.join(nativeBackend.STUDIO_ROOT, 'backend'),
    )).toThrow(/protected path/);
    for (const candidate of [
      path.join(nativeBackend.REPO_ROOT, '.git'),
      path.join(nativeBackend.STUDIO_ROOT, 'src'),
      path.join(nativeBackend.REPO_ROOT, 'docs'),
      path.join(nativeBackend.REPO_ROOT, 'packaging'),
    ]) {
      expect(() => nativeBackend.assertSafeStageDir(candidate)).toThrow(/protected path/);
    }
  });

  it('refuses a destination created while the replacement is being built and preserves it', async () => {
    const root = makeTemporaryDirectory();
    const { stage, binary, dist } = stageFixture(root);
    const sentinel = path.join(stage, 'user-sentinel.txt');

    await expect(nativeBackend.stageNativeBackend({
      stageDir: stage,
      distDir: dist,
      binaryPath: binary,
      platform: 'linux',
      onBeforeInstall: () => {
        fs.mkdirSync(stage);
        writeFile(sentinel, 'concurrent user data\n');
      },
    })).rejects.toThrow(/appeared/);

    expect(fs.readFileSync(sentinel, 'utf8')).toBe('concurrent user data\n');
    expect(sortedFiles(stage)).toEqual(['user-sentinel.txt']);
    expect(fs.readdirSync(root).filter((name) => /\.(?:staging|backup)-/.test(name))).toEqual([]);
  });

  it('refuses a managed stage replaced or modified concurrently and preserves both copies', async () => {
    const root = makeTemporaryDirectory();
    const fixture = stageFixture(root);
    const first = await nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    });
    const oldMetadata = fs.readFileSync(first.paths.metadataPath, 'utf8');
    const oldBinary = fs.readFileSync(first.paths.binaryPath);

    // Add a file to the authorized stage while the candidate is being built.
    const sentinel = path.join(fixture.stage, 'user-sentinel.txt');
    await expect(nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
      onBeforeInstall: () => writeFile(sentinel, 'concurrent modification\n'),
    })).rejects.toThrow(/modified/);
    expect(fs.readFileSync(sentinel, 'utf8')).toBe('concurrent modification\n');
    expect(fs.readFileSync(first.paths.metadataPath, 'utf8')).toBe(oldMetadata);
    expect(fs.readFileSync(first.paths.binaryPath)).toEqual(oldBinary);

    // Replace the whole stage with an unmanaged directory between validation
    // and installation. The replacement must be left untouched too.
    const displaced = path.join(root, 'displaced-managed-stage');
    const replacementSentinel = path.join(fixture.stage, 'replacement-sentinel.txt');
    fs.rmSync(sentinel);
    await expect(nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
      onBeforeInstall: () => {
        fs.renameSync(fixture.stage, displaced);
        fs.mkdirSync(fixture.stage);
        writeFile(replacementSentinel, 'replacement user data\n');
      },
    })).rejects.toThrow(/replaced/);

    expect(fs.readFileSync(replacementSentinel, 'utf8')).toBe('replacement user data\n');
    expect(sortedFiles(fixture.stage)).toEqual(['replacement-sentinel.txt']);
    expect(fs.readFileSync(path.join(displaced, 'native-build.json'), 'utf8')).toBe(oldMetadata);
    expect(fs.readFileSync(path.join(displaced, 'kala-studio'))).toEqual(oldBinary);
  });

  it('keeps a backup that changed after the swap instead of deleting it', async () => {
    const root = makeTemporaryDirectory();
    const fixture = stageFixture(root);
    await nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    });

    let backupPath = '';
    const result = await nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
      onBeforeBackupRemoval: ({ backup }) => {
        backupPath = backup;
        writeFile(path.join(backup, 'late-sentinel.txt'), 'keep this\n');
      },
    });

    expect(result.paths.binaryPath).toBe(path.join(fixture.stage, 'kala-studio'));
    expect(backupPath).not.toBe('');
    expect(fs.readFileSync(path.join(backupPath, 'late-sentinel.txt'), 'utf8')).toBe('keep this\n');
    await expect(nativeBackend.verifyStageIntegrity(result.paths)).resolves.toBeTruthy();
  });

  it('accepts a new or empty custom directory outside the project', async () => {
    const root = makeTemporaryDirectory();
    const { binary, dist } = stageFixture(root);
    const stage = path.join(root, 'empty-custom-stage');
    fs.mkdirSync(stage);
    const result = await nativeBackend.stageNativeBackend({
      stageDir: stage,
      distDir: dist,
      binaryPath: binary,
      platform: 'linux',
    });
    expect(result.paths.stageDir).toBe(stage);
    expect(fs.existsSync(path.join(stage, 'kala-studio'))).toBe(true);
  });

  it('never deletes a nonempty unmanaged/custom directory', async () => {
    const root = makeTemporaryDirectory();
    const { binary, dist } = stageFixture(root);
    const custom = path.join(root, 'unrelated-project');
    const sentinel = path.join(custom, 'keep-me.txt');
    writeFile(sentinel, 'important user data\n');

    await expect(nativeBackend.stageNativeBackend({
      stageDir: custom,
      distDir: dist,
      binaryPath: binary,
      platform: 'linux',
    })).rejects.toThrow(/unmanaged directory/);

    expect(fs.readFileSync(sentinel, 'utf8')).toBe('important user data\n');
    expect(sortedFiles(custom)).toEqual(['keep-me.txt']);
  });

  it('refuses a managed stage with a pre-existing unknown top-level file instead of deleting it', async () => {
    const root = makeTemporaryDirectory();
    const fixture = stageFixture(root);
    const first = await nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    });
    const sentinel = path.join(fixture.stage, 'user-sentinel.txt');
    writeFile(sentinel, 'pre-existing user data\n');
    const oldMetadata = fs.readFileSync(first.paths.metadataPath, 'utf8');

    await expect(nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    })).rejects.toThrow(/unexpected top-level entries/);

    expect(fs.readFileSync(sentinel, 'utf8')).toBe('pre-existing user data\n');
    expect(fs.readFileSync(first.paths.metadataPath, 'utf8')).toBe(oldMetadata);
    await expect(nativeBackend.verifyStageIntegrity(first.paths)).rejects.toThrow(
      /unexpected top-level entries/,
    );
  });

  it('refuses an owned stage copied to a different path because metadata is path-bound', async () => {
    const root = makeTemporaryDirectory();
    const fixture = stageFixture(root);
    const result = await nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    });
    const copied = path.join(root, 'copied-stage');
    fs.cpSync(result.paths.stageDir, copied, { recursive: true });

    await expect(nativeBackend.stageNativeBackend({
      stageDir: copied,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    })).rejects.toThrow(/unmanaged directory/);
    expect(fs.existsSync(path.join(copied, 'kala-studio'))).toBe(true);
  });

  it('preserves a previous valid managed stage when replacement validation fails', async () => {
    const root = makeTemporaryDirectory();
    const fixture = stageFixture(root);
    const first = await nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    });
    const oldMetadata = fs.readFileSync(first.paths.metadataPath, 'utf8');
    const oldBinary = fs.readFileSync(first.paths.binaryPath);

    // A binary without the native-STEP marker defeats candidate validation
    // when the stage opts into `native-primitives`.
    const badBinary = makeBinary(path.join(root, 'bad-binary'), 'ELF-without-step');
    await expect(nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: badBinary,
      platform: 'linux',
      features: 'native-primitives',
    })).rejects.toThrow(/native primitives/);

    expect(fs.readFileSync(first.paths.metadataPath, 'utf8')).toBe(oldMetadata);
    expect(fs.readFileSync(first.paths.binaryPath)).toEqual(oldBinary);
    await expect(nativeBackend.verifyStageIntegrity(first.paths)).resolves.toBeTruthy();
  });

  it.skipIf(process.platform === 'win32')(
    'preserves a previous valid managed stage when sibling staging cannot be created',
    async () => {
      const root = makeTemporaryDirectory();
      const fixture = stageFixture(root);
      const first = await nativeBackend.stageNativeBackend({
        stageDir: fixture.stage,
        distDir: fixture.dist,
        binaryPath: fixture.binary,
        platform: 'linux',
      });
      const oldMetadata = fs.readFileSync(first.paths.metadataPath, 'utf8');
      fs.chmodSync(root, 0o555);
      try {
        await expect(nativeBackend.stageNativeBackend({
          stageDir: fixture.stage,
          distDir: fixture.dist,
          binaryPath: fixture.binary,
          platform: 'linux',
        })).rejects.toThrow(/EACCES|permission/i);
      } finally {
        fs.chmodSync(root, 0o755);
      }
      expect(fs.readFileSync(first.paths.metadataPath, 'utf8')).toBe(oldMetadata);
      await expect(nativeBackend.verifyStageIntegrity(first.paths)).resolves.toBeTruthy();
    },
  );

  it('preserves a previous valid managed stage when the replacement inputs are missing', async () => {
    const root = makeTemporaryDirectory();
    const fixture = stageFixture(root);
    const first = await nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    });
    const oldMetadata = fs.readFileSync(first.paths.metadataPath, 'utf8');

    await expect(nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: path.join(root, 'missing-dist'),
      binaryPath: fixture.binary,
      platform: 'linux',
    })).rejects.toThrow(/npm run build/);

    expect(fs.readFileSync(first.paths.metadataPath, 'utf8')).toBe(oldMetadata);
  });

  it('rejects symlinked input UI roots/components/assets before copying outside', async () => {
    const root = makeTemporaryDirectory();
    const fixture = stageFixture(root);
    const outside = path.join(root, 'outside');
    const secret = path.join(outside, 'secret.txt');
    writeFile(secret, 'do not copy me\n');

    fs.symlinkSync(outside, path.join(fixture.dist, 'assets', 'linked-dir'), 'dir');
    await expect(nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    })).rejects.toThrow(/symlink/);
    expect(fs.existsSync(fixture.stage)).toBe(false);
    expect(fs.readFileSync(secret, 'utf8')).toBe('do not copy me\n');

    fs.rmSync(path.join(fixture.dist, 'assets', 'linked-dir'));
    fs.symlinkSync(secret, path.join(fixture.dist, 'assets', 'linked-file.txt'), 'file');
    await expect(nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    })).rejects.toThrow(/symlink/);
    expect(fs.existsSync(fixture.stage)).toBe(false);
    expect(fs.readFileSync(secret, 'utf8')).toBe('do not copy me\n');
  });

  it('rejects symlinked staging roots before traversal and leaves the target intact', async () => {
    const root = makeTemporaryDirectory();
    const fixture = stageFixture(root);
    const realTarget = path.join(root, 'real-target');
    writeFile(path.join(realTarget, 'keep.txt'), 'keep\n');
    const linkedStage = path.join(root, 'linked-stage');
    fs.symlinkSync(realTarget, linkedStage, 'dir');

    await expect(nativeBackend.stageNativeBackend({
      stageDir: linkedStage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    })).rejects.toThrow(/symlink/);
    expect(fs.readFileSync(path.join(realTarget, 'keep.txt'), 'utf8')).toBe('keep\n');
    expect(sortedFiles(realTarget)).toEqual(['keep.txt']);

    const linkedParent = path.join(root, 'linked-parent');
    fs.symlinkSync(realTarget, linkedParent, 'dir');
    await expect(nativeBackend.stageNativeBackend({
      stageDir: path.join(linkedParent, 'child-stage'),
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    })).rejects.toThrow(/symlink/);
    expect(fs.readFileSync(path.join(realTarget, 'keep.txt'), 'utf8')).toBe('keep\n');
    expect(sortedFiles(realTarget)).toEqual(['keep.txt']);
  });

  it('refuses corrupted binary/static trees whose digest no longer matches metadata', async () => {
    const root = makeTemporaryDirectory();
    const fixture = stageFixture(root);
    const result = await nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    });
    fs.appendFileSync(result.paths.binaryPath, 'tamper');
    await expect(nativeBackend.verifyStageIntegrity(result.paths)).rejects.toThrow(/mismatch|digest/);

    const secondRoot = makeTemporaryDirectory();
    const second = stageFixture(secondRoot);
    const staged = await nativeBackend.stageNativeBackend({
      stageDir: second.stage,
      distDir: second.dist,
      binaryPath: second.binary,
      platform: 'linux',
    });
    fs.writeFileSync(path.join(staged.paths.staticDir, 'assets', 'app.js'), 'tampered-ui');
    await expect(nativeBackend.verifyStageIntegrity(staged.paths)).rejects.toThrow(/static UI digest/);
  });

  it('refuses a stage built from different source digests', async () => {
    const root = makeTemporaryDirectory();
    const fixture = stageFixture(root);
    const result = await nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    });
    const current = nativeReadiness.checkStudioReadiness();
    expect(() => nativeBackend.assertStageSourceParity(result.metadata, current)).not.toThrow();
    const stale = {
      ...result.metadata,
      sourceParity: {
        ...result.metadata.sourceParity,
        pythonSourceSha256: 'deadbeef',
      },
    };
    expect(() => nativeBackend.assertStageSourceParity(stale, current)).toThrow(/stale/);
    const missing = { ...result.metadata } as Record<string, unknown>;
    delete missing.sourceParity;
    expect(() => nativeBackend.assertStageSourceParity(
      missing as unknown as typeof result.metadata,
      current,
    )).toThrow(/no current-source parity/);
  });

  it('refuses a stage whose static UI is stale relative to studio/dist', async () => {
    const root = makeTemporaryDirectory();
    const fixture = stageFixture(root);
    const result = await nativeBackend.stageNativeBackend({
      stageDir: fixture.stage,
      distDir: fixture.dist,
      binaryPath: fixture.binary,
      platform: 'linux',
    });
    await expect(nativeBackend.assertStageUiMatches(result.metadata, {
      distDir: fixture.dist,
    })).resolves.toMatch(/^[a-f0-9]{64}$/);
    fs.writeFileSync(path.join(fixture.dist, 'assets', 'app.js'), 'new-ui-bundle');
    await expect(nativeBackend.assertStageUiMatches(result.metadata, {
      distDir: fixture.dist,
    })).rejects.toThrow(/stale relative to studio\/dist/);
  });

  it('fails with actionable errors for a missing binary or UI build', async () => {
    const root = makeTemporaryDirectory();
    const dist = makeDist(path.join(root, 'dist'));
    await expect(nativeBackend.stageNativeBackend({
      stageDir: path.join(root, 'stage-missing-binary'),
      distDir: dist,
      binaryPath: path.join(root, 'missing-binary'),
      platform: 'linux',
    })).rejects.toThrow(/npm run build:native/);

    const binary = makeBinary(path.join(root, 'kala-studio'));
    await expect(nativeBackend.stageNativeBackend({
      stageDir: path.join(root, 'stage-missing-ui'),
      distDir: path.join(root, 'missing-dist'),
      binaryPath: binary,
      platform: 'linux',
    })).rejects.toThrow(/npm run build/);
  });

  it('parses no-build and target arguments without invoking a shell', () => {
    const parsed = nativeBackend.parseArguments([
      '--no-build', '--stage-dir', '/tmp/stage', '--target', 'x86_64-pc-windows-msvc',
      '--binary', '/tmp/kala-studio.exe', '--occt-lib-dir', '/tmp/vcpkg/bin',
    ]);
    expect(parsed.noBuild).toBe(true);
    expect(parsed.stageDir).toBe(path.resolve('/tmp/stage'));
    expect(parsed.target).toBe('x86_64-pc-windows-msvc');
    expect(parsed.binaryPath).toBe(path.resolve('/tmp/kala-studio.exe'));
    expect(parsed.occtLibDir).toBe(path.resolve('/tmp/vcpkg/bin'));
  });

  it('rejects missing flag values before any staging mutation', () => {
    for (const argument of [
      '--stage-dir', '--dist', '--target', '--features', '--binary', '--occt-lib-dir',
    ]) {
      expect(() => nativeBackend.parseArguments([argument])).toThrow(/requires a value/);
      expect(() => nativeBackend.parseArguments([argument, '--no-build'])).toThrow(/requires a value/);
    }
  });
});

describe('current-feature native readiness', () => {
  it('reports the current Python surface and the computed Rust-native/geometry-seam split', () => {
    const report = nativeReadiness.checkStudioReadiness();
    expect(report.python.routeMethodCount).toBe(16);
    expect(report.python.internalRouteMethodCount).toBe(4);
    expect(report.python.internalRouteMethods).toEqual([
      'GET /internal/v1/intent/schema',
      'POST /internal/v1/intent/parse',
      'POST /internal/v1/intent/validate',
      'POST /internal/v1/journal/append',
    ]);
    expect(report.python.routeMethods.every((route) => route.includes(' /api/studio/'))).toBe(true);

    // r2: the native front door serves the public surface itself; the
    // geometry, design, journal and import routes reach geometry through the
    // geometry-backend seam, which the default build answers natively. Both
    // halves are read from the Rust route table, never pinned to a hardcoded
    // readiness.
    expect(report.native.rustNativeRouteMethods).toEqual([
      'GET /api/studio/health',
      'GET /api/studio/integrations',
      'GET /api/studio/providers',
      'GET /api/studio/providers/{}/models',
      'GET /api/studio/settings',
      'POST /api/studio/chat',
      'POST /api/studio/settings',
    ]);
    expect(report.native.geometrySeamRouteMethods).toEqual([
      'GET /api/studio/design/{}/artifact/{}',
      'GET /api/studio/design/{}/drawing',
      'GET /api/studio/design/{}/section',
      'GET /api/studio/journal',
      'GET /api/studio/journal/verify',
      'GET /api/studio/providers/cad',
      'POST /api/studio/design',
      'POST /api/studio/export',
      'POST /api/studio/handoff',
      'POST /api/studio/import',
      'POST /api/studio/preview',
    ]);
    expect(report.native.rustNativeRouteMethodCount).toBe(7);
    expect(report.native.geometrySeamRouteMethodCount).toBe(11);
    expect(report.native.routeMethodCount).toBe(18);
    // The old "proxied" field name survives as a compatible alias with the
    // same values, so an older consumer keeps working.
    expect(report.native.proxiedRouteMethods).toEqual(
      report.native.geometrySeamRouteMethods,
    );
    expect(report.native.proxiedRouteMethodCount).toBe(
      report.native.geometrySeamRouteMethodCount,
    );
    expect(report.native.rustNativeRouteMethodCount + report.native.geometrySeamRouteMethodCount)
      .toBe(report.native.routeMethodCount);
    // The wording never claims the Python worker answers those routes.
    expect(report.summary).toContain('answered natively by default');
    expect(report.summary).not.toContain('proxied');
    expect(nativeReadiness.formatReadinessReport(report)).toContain(
      'geometry-backend seam route-methods (answered natively by default; Python worker is a development opt-in): 11',
    );
    expect(nativeReadiness.formatReadinessReport(report)).not.toContain('proxied route-methods');

    expect(report.missingRouteMethods).toEqual([]);
    // The native front door carries two routes the Python reference does
    // not have: the STEP import and the provider model catalog.
    expect(report.extraNativeRouteMethods).toEqual([
      'GET /api/studio/providers/{}/models',
      'POST /api/studio/import',
    ]);
    expect(report.ready).toBe(
      report.missingRouteMethods.length === 0 && report.missingCapabilities.length === 0,
    );
    expect(report.pendingFeatures).toEqual([]);
    // Every capability the Python DTOs require is identified, and the missing
    // list is exactly the required-but-absent set.
    expect(report.capabilities.filter((row) => row.requiredByPython).map((row) => row.id))
      .toEqual(expect.arrayContaining([
        'health.design_available',
        'chat.design_result',
        'export.design_id',
      ]));
    expect(report.missingCapabilities).toEqual(
      report.capabilities
        .filter((row) => row.requiredByPython && !row.presentInNative)
        .map((row) => row.id),
    );
  });

  it('counts a worker-only internal route separately and never as public parity', () => {
    const pythonSource = [
      '// FILE server.py',
      '@router.get("/health")',
      'def health(): ...',
      '// FILE internal_api.py',
      'INTERNAL_PREFIX: Final[str] = "/internal/v1"',
      'router = APIRouter(prefix=INTERNAL_PREFIX, tags=["internal"])',
      '@router.post("/intent/parse")',
      'def intent_parse(): ...',
    ].join('\n');
    const nativeSource = syntheticRustRouteTable([
      { path: '/api/studio/health', methods: ['GET'] },
    ]);

    const analysis = nativeReadiness.analyzeStudioSources({ pythonSource, nativeSource });
    expect(analysis.pythonRouteMethods).toEqual(['GET /api/studio/health']);
    expect(analysis.internalRouteMethods).toEqual(['POST /internal/v1/intent/parse']);
    expect(analysis.internalRouteMethodCount).toBe(1);
    expect(analysis.missingRouteMethods).toEqual([]);
    expect(analysis.ready).toBe(true);
  });

  it('separates Rust-native routes from routes through the geometry-backend seam', () => {
    const pythonSource = [
      '// FILE server.py',
      '@router.get("/health")',
      'def health(): ...',
      '@router.get("/journal")',
      'def journal(): ...',
    ].join('\n');
    const nativeSource = syntheticRustRouteTable([
      { path: '/api/studio/health', methods: ['GET'] },
      { path: '/api/studio/journal', methods: ['GET'], proxied: true },
    ]);

    const surface = nativeReadiness.parseRustRouteSurface(nativeSource);
    expect(surface.routeMethods).toEqual([
      'GET /api/studio/health',
      'GET /api/studio/journal',
    ]);
    expect(surface.rustNativeRouteMethods).toEqual(['GET /api/studio/health']);
    expect(surface.geometrySeamRouteMethods).toEqual(['GET /api/studio/journal']);

    const analysis = nativeReadiness.analyzeStudioSources({ pythonSource, nativeSource });
    expect(analysis.rustNativeRouteMethodCount).toBe(1);
    expect(analysis.geometrySeamRouteMethodCount).toBe(1);
    expect(analysis.proxiedRouteMethodCount).toBe(1);
    expect(analysis.missingRouteMethods).toEqual([]);
    expect(analysis.ready).toBe(true);
  });

  it('refuses to guess a route table it does not understand', () => {
    // The pre-r2 `allowed_methods(path)` shape is no longer the route table.
    expect(() => nativeReadiness.parseRustRoutes([
      'fn allowed_methods(path: &str) -> Option<Vec<Method>> {',
      '    let methods = match path {',
      '        "/api/studio/health" => { vec![Method::GET] }',
      '        _ => return None,',
      '    };',
      '    Some(methods)',
      '}',
    ].join('\n'))).toThrow(/could not find `fn classify`/);

    // A classifier kind with no method arm is an inconsistent table, not an
    // empty route surface to report as ready.
    const inconsistent = syntheticRustRouteTable([
      { path: '/api/studio/health', methods: ['GET'] },
    ]).replace('        RouteKind::Route0 => &[Method::GET],\n', '');
    expect(() => nativeReadiness.parseRustRoutes(inconsistent))
      .toThrow(/method table|empty surface/);
  });

  it('still fails the diff when a new public route is added', () => {
    const pythonSource = [
      '// FILE server.py',
      '@router.get("/health")',
      'def health(): ...',
      '@router.post("/new-public-route")',
      'def new_public_route(): ...',
    ].join('\n');
    const nativeSource = syntheticRustRouteTable([
      { path: '/api/studio/health', methods: ['GET'] },
    ]);

    const analysis = nativeReadiness.analyzeStudioSources({ pythonSource, nativeSource });
    expect(analysis.ready).toBe(false);
    expect(analysis.missingRouteMethods).toEqual(['POST /api/studio/new-public-route']);
  });

  it('is computed from sources, not hardcoded false: a complete synthetic surface is ready', () => {
    const pythonRoutes = nativeReadiness.checkStudioReadiness().python.routeMethods;
    const pythonSource = [
      'class HealthResponse:', '    design_available: bool',
      'class ChatResponse:', '    design: DesignResult | None',
      'class ExportRequest:', '    design_id: DesignId | None',
      ...pythonRoutes.map((route) => {
        const [method, absolute] = route.split(' ') as [string, string];
        const relative = absolute.replace('/api/studio', '').replace('{}', '{id}');
        return `@router.${method.toLowerCase()}("${relative}")\ndef handler(): ...`;
      }),
    ].join('\n');
    const groups = new Map<string, string[]>();
    for (const route of pythonRoutes) {
      const [method, absolute] = route.split(' ') as [string, string];
      groups.set(absolute, [...(groups.get(absolute) ?? []), method]);
    }
    const nativeSource = [
      syntheticRustRouteTable([...groups.entries()].map(([routePath, methods], index) => ({
        path: routePath,
        methods,
        proxied: index === 0,
      }))),
      'struct NativeCapabilities {',
      '    design_available: bool,',
      '    design: Option<String>,',
      '    design_id: Option<String>,',
      '}',
    ].join('\n');

    const analysis = nativeReadiness.analyzeStudioSources({ pythonSource, nativeSource });
    expect(analysis.ready).toBe(true);
    expect(analysis.missingRouteMethods).toEqual([]);
    expect(analysis.missingCapabilities).toEqual([]);
    expect(analysis.rustNativeRouteMethodCount + analysis.geometrySeamRouteMethodCount)
      .toBe(analysis.nativeRouteMethods.length);
  });

  it('blocks release/NSIS packaging but permits a labelled --dir development artifact', () => {
    const pending = pendingReadinessReport();
    expect(() => desktopPack.decidePackaging({
      argv: ['--win', 'nsis', '--x64'],
      report: pending,
    })).toThrow(/release packaging\/NSIS is blocked/);

    const dirDecision = desktopPack.decidePackaging({ argv: ['--dir'], report: pending });
    expect(dirDecision.classification).toBe('development-unpacked-artifact');
    expect(dirDecision.releaseEligible).toBe(false);
    expect(dirDecision.warning).toMatch(/must not be distributed/);

    const readyDecision = desktopPack.decidePackaging({
      argv: ['--win', 'nsis', '--x64'],
      report: { ...pending, ready: true, missingRouteMethods: [], missingCapabilities: [] },
    });
    expect(readyDecision.classification).toBe('release');
    expect(readyDecision.releaseEligible).toBe(true);
    // The release warning names the seam honestly: the default build answers
    // those routes natively, and the Python worker is a development opt-in.
    expect(readyDecision.warning).toContain('go through the geometry-backend seam');
    expect(readyDecision.warning).toContain('development opt-in');
    expect(readyDecision.warning).not.toContain('delegated through');
    expect(readyDecision.warning).not.toContain('proxied');

    // An older readiness report carries only the deprecated field name; the
    // alias still produces the same warning.
    const legacyNative = {
      ...pending.native,
      geometrySeamRouteMethodCount: undefined,
      geometrySeamRouteMethods: undefined,
      proxiedRouteMethodCount: 10,
      proxiedRouteMethods: pending.native.proxiedRouteMethods,
    };
    const legacyDecision = desktopPack.decidePackaging({
      argv: ['--win', 'nsis', '--x64'],
      report: {
        ...pending,
        ready: true,
        missingRouteMethods: [],
        missingCapabilities: [],
        native: legacyNative,
      },
    });
    expect(legacyDecision.warning).toContain('10 route-method(s) go through the geometry-backend seam');
  });

  it('classifies the effective installed builder targets, not raw --dir strings', () => {
    const report = pendingReadinessReport();
    const blocked = [
      ['--dir', '--dir=false', '--win', 'nsis'],
      ['--dir', '--no-dir', '--win', 'nsis'],
      ['--dir', '--win', 'nsis'],
      ['--win', 'nsis', '--dir'],
      ['-w', 'nsis', '--dir'],
      ['--windows', 'nsis', '--dir'],
      ['--dir=true', '--win=nsis'],
      ['--dir=false', '--win', 'nsis'],
      ['--no-dir', '--win', 'nsis'],
      ['--dir=false'],
      ['--no-dir'],
      ['--dir', '--dir=false'],
      ['--dir', '--dir=true', '--dir=false'],
      ['--dir', '--win', 'nsis', '--win', 'dir'],
    ];
    for (const argv of blocked) {
      expect(desktopPack.normalizeBuilderInvocation(argv).unpackedOnly).toBe(false);
      expect(() => desktopPack.decidePackaging({ argv, report })).toThrow(
        /release packaging\/NSIS is blocked/,
      );
    }

    const allowed = [
      ['--dir'],
      ['--dir=true'],
      ['--dir', '--dir'],
      ['--dir=false', '--dir'],
      ['--dir', '--win'],
      ['--dir', '--win', 'dir'],
      ['--dir', '--config', 'electron-builder.json'],
      ['--dir', '--publish', 'always'],
    ];
    for (const argv of allowed) {
      expect(desktopPack.normalizeBuilderInvocation(argv).unpackedOnly).toBe(true);
      expect(desktopPack.decidePackaging({ argv, report }).classification)
        .toBe('development-unpacked-artifact');
    }

    // A ready report does not turn an installer target into a dev artifact
    // just because --dir was also present.
    const ready = { ...report, ready: true, missingRouteMethods: [], missingCapabilities: [] };
    expect(desktopPack.decidePackaging({
      argv: ['--dir', '--win', 'nsis'],
      report: ready,
    }).classification).toBe('release');
    const readyDir = desktopPack.decidePackaging({ argv: ['--dir'], report: ready });
    expect(readyDir.classification).toBe('development-unpacked-artifact');
    expect(readyDir.releaseEligible).toBe(false);
  });

  it('matches the installed electron-builder parser and fails closed on invalid arguments', () => {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const builder = require('electron-builder/out/builder.js') as {
      createYargs: () => any;
      configureBuildCommand: (yargs: any) => any;
      normalizeOptions: (argv: any) => { targets: Map<unknown, Map<unknown, string[]>> };
    };
    const cases = [
      ['--dir'],
      ['--dir', '--dir=false', '--win', 'nsis'],
      ['--dir', '--no-dir', '--win', 'nsis'],
      ['--dir', '--win', 'nsis'],
      ['--win', 'nsis', '--dir'],
      ['-w', 'nsis', '--dir'],
      ['--windows', 'nsis', '--dir'],
      ['--dir=true', '--win=nsis'],
      ['--dir=true', '--win'],
      ['--dir', '--win', 'dir'],
      ['--dir=false', '--dir'],
      ['--no-dir', '--linux', 'AppImage'],
      ['--dir=false', '--linux', 'AppImage'],
    ];
    for (const argv of cases) {
      const parsed = builder.configureBuildCommand(
        builder.createYargs().exitProcess(false).strict().showHelpOnFail(false),
      ).parse([...argv]);
      const normalized = builder.normalizeOptions(parsed);
      const effectiveTypes = [...normalized.targets.values()]
        .flatMap((archToType) => [...archToType.values()])
        .flat()
        .sort();
      const gate = desktopPack.normalizeBuilderInvocation(argv);
      expect(gate.dir).toBe(parsed.dir === true);
      expect(gate.targets.map((target) => target.type).sort()).toEqual(effectiveTypes);
    }

    expect(() => desktopPack.decidePackaging({
      argv: ['--dir', '--not-a-real-electron-builder-flag'],
      report: pendingReadinessReport(),
    })).toThrow(/invalid or ambiguous/);
  });
});

describe('desktop packaging publish safety', () => {
  it('always forces --publish never and removes caller overrides', () => {
    expect(desktopPack.sanitizePublishArguments(['--dir'])).toEqual([
      '--dir', '--publish', 'never',
    ]);
    expect(desktopPack.sanitizePublishArguments(['--publish', 'always', '--dir'])).toEqual([
      '--dir', '--publish', 'never',
    ]);
    expect(desktopPack.sanitizePublishArguments(['--publish=always', '--win', 'nsis'])).toEqual([
      '--win', 'nsis', '--publish', 'never',
    ]);
    expect(desktopPack.sanitizePublishArguments(['--publish'])).toEqual([
      '--publish', 'never',
    ]);
    expect(desktopPack.sanitizePublishArguments(['--publish', '--dir'])).toEqual([
      '--dir', '--publish', 'never',
    ]);
    expect(desktopPack.sanitizePublishArguments(['-p', 'always', '--dir'])).toEqual([
      '--dir', '--publish', 'never',
    ]);
    expect(desktopPack.sanitizePublishArguments(['-p=always', '--dir'])).toEqual([
      '--dir', '--publish', 'never',
    ]);
  });

  it('desktop-pack validates stage integrity and source parity before invoking the builder', () => {
    const source = fs.readFileSync(
      path.resolve(__dirname, '..', 'scripts', 'desktop-pack.cjs'),
      'utf8',
    );
    expect(source).toContain('verifyStageIntegrity');
    expect(source).toContain('assertStageSourceParity');
    expect(source).toContain('sanitizePublishArguments');
    expect(source).toContain('requireBundles: true');
    expect(source).toContain('assertFreeCadAdapterBundled');
    // The release gate is the native-design readiness report, not the old
    // `nativeStep` feature marker.
    expect(source).not.toContain('nativeStep !== true');
    expect(source).not.toMatch(/path\.join\(studioRoot,\s*'backend'/);
  });
});

describe('desktop packaging configuration', () => {
  it('packages only the native stage into resources/backend', () => {
    const configPath = path.resolve(__dirname, '..', 'electron-builder.json');
    const source = fs.readFileSync(configPath, 'utf8');
    // The config only uses full-line comments; strip them before parsing.
    const stripped = source
      .split('\n')
      .filter((line) => !line.trimStart().startsWith('//'))
      .join('\n');
    const config = JSON.parse(stripped) as {
      extraResources: Array<{ from: string; to: string }>;
    };
    expect(config.extraResources).toEqual([{ from: 'backend-native', to: 'backend' }]);
    expect(source).not.toContain('"to": "app/dist"');
    expect(source).not.toMatch(/"from":\s*"backend"/);
  });

  it('desktop-pack checks the native stage, never the Python staging directory', () => {
    const source = fs.readFileSync(
      path.resolve(__dirname, '..', 'scripts', 'desktop-pack.cjs'),
      'utf8',
    );
    expect(source).toContain('nativePaths');
    expect(source).toContain('validateNativeTree');
    expect(source).not.toMatch(/path\.join\(studioRoot,\s*'backend'/);
    expect(source).not.toContain("'kala-studio.exe' : 'kala-studio'");
  });

  it('exposes the native build/readiness scripts and no hidden Python fallback', () => {
    const packageJson = JSON.parse(
      fs.readFileSync(path.resolve(__dirname, '..', 'package.json'), 'utf8'),
    ) as { scripts: Record<string, string> };
    expect(packageJson.scripts['build:native']).toBe('node scripts/native-backend.cjs');
    expect(packageJson.scripts['check:native-readiness']).toBe('node scripts/native-readiness.cjs');
    const launcher = fs.readFileSync(
      path.resolve(__dirname, '..', 'scripts', 'desktop.cjs'),
      'utf8',
    );
    expect(launcher).toContain('buildAndStageNative');
    expect(launcher).toContain('pythonReferenceRequested');
    expect(launcher).not.toMatch(/spawn\([^)]*[Bb]ackend|[Bb]ackend[^)]*spawn\(/);
    expect(launcher).not.toMatch(/writeFileSync[^)]*[Tt]oken|fs\.writeFileSync[^)]*token/i);
  });
});

const staged = nativeBackend.nativePaths({});
const hasStagedNative = fs.existsSync(staged.binaryPath) && fs.existsSync(staged.indexFile);

describe.skipIf(!hasStagedNative)('real native backend lifecycle', () => {
  it('starts the staged native child, passes the health probe, and shuts down once', async () => {
    const resolved = electronMain.resolveBackendCommand({
      isPackaged: false,
      platform: process.platform,
      repoRoot: electronMain.REPO_ROOT,
      devBackend: 'native',
      python: '',
    });
    expect(resolved.mode).toBe('native');
    expect(resolved.exists).toBe(true);
    expect(resolved.command).toBe(staged.binaryPath);

    const port = await electronMain.pickFreePort();
    const token = electronMain.generateToken();
    const dataDir = makeTemporaryDirectory();
    const invocation = electronMain.buildBackendInvocation({
      command: resolved.command,
      args: resolved.args,
      cwd: resolved.cwd,
      env: { ...resolved.env, PATH: '', KALA_STUDIO_DATA_DIR: dataDir },
      port,
      token,
    });
    const backend = new electronMain.BackendProcess(invocation);
    try {
      backend.start();
      await backend.waitForHealth(20_000);
      expect(backend.ready).toBe(true);
      expect(backend.exited).toBe(false);
    } finally {
      backend.shutdown();
    }
    const deadline = Date.now() + 5_000;
    while (!backend.exited && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    expect(backend.exited).toBe(true);
    expect(backend.expected).toBe(true);
  });
});
