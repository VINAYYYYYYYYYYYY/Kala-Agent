// Tests for the Electron main module. We do not load Electron here;
// instead we import the module and exercise its exported pure helpers
// (buildBackendInvocation, validateIpcSender, pickFreePort). The
// spawn path is exercised by spawning a tiny child process via
// `buildBackendInvocation` and asserting the captured argv matches
// the contract the backend CLI requires (`--host` and `--port`).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { EventEmitter } from 'node:events';
import { spawn } from 'node:child_process';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';
import net from 'node:net';

type Main = typeof import('../electron/main.cjs');

let mainModule: Main;
const temporaryDirectories: string[] = [];

function makeTemporaryDirectory(): string {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'kala-electron-main-'));
  temporaryDirectories.push(directory);
  return directory;
}

function makeExecutable(filePath: string, contents = 'native-backend'): void {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
  if (process.platform !== 'win32') fs.chmodSync(filePath, 0o755);
}

beforeEach(async () => {
  // Dynamic import of a CommonJS module under vitest returns
  // `{ default: <module.exports> }`. Unwrap to keep the helper surface
  // identical to the live Electron runtime.
  const mod = (await import('../electron/main.cjs' as string)) as { default?: Main } & Main;
  mainModule = (mod.default ?? mod) as Main;
});

afterEach(() => {
  vi.restoreAllMocks();
  for (const directory of temporaryDirectories.splice(0)) {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

describe('buildBackendInvocation', () => {
  it('always appends --host 127.0.0.1 and --port <chosen>', () => {
    const invocation = mainModule.buildBackendInvocation({
      command: 'python',
      args: ['-m', 'kala_agent.studio'],
      cwd: '/tmp',
      env: {},
      port: 4321,
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
    });
    expect(invocation.command).toBe('python');
    expect(invocation.args).toContain('--host');
    const hostIndex = invocation.args.indexOf('--host');
    expect(invocation.args[hostIndex + 1]).toBe('127.0.0.1');
    expect(invocation.args).toContain('--port');
    const portIndex = invocation.args.indexOf('--port');
    expect(invocation.args[portIndex + 1]).toBe('4321');
    // Token must travel via env, NOT via argv, so it never appears on
    // the process command line.
    expect(invocation.args.join(' ')).not.toContain('unit-test-token');
    expect(invocation.env.KALA_STUDIO_TOKEN).toBe('unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa');
  });

  it('passes the launch nonce through the environment, never argv', () => {
    const invocation = mainModule.buildBackendInvocation({
      command: 'python',
      args: ['-m', 'kala_agent.studio'],
      cwd: '/tmp',
      env: {},
      port: 4321,
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      nonce: 'unit-test-nonce-bbbbbbbbbbbbbbbbbbbbbbbb',
    });
    expect(invocation.args.join(' ')).not.toContain('unit-test-nonce');
    expect(invocation.env.KALA_STUDIO_LAUNCH_NONCE).toBe(
      'unit-test-nonce-bbbbbbbbbbbbbbbbbbbbbbbb',
    );
    expect(invocation.nonce).toBe('unit-test-nonce-bbbbbbbbbbbbbbbbbbbbbbbb');
  });

  it('preserves any existing args before appending --host and --port', () => {
    const invocation = mainModule.buildBackendInvocation({
      command: 'kala-studio',
      args: ['--data-dir', '/var/lib/kala'],
      cwd: '/',
      env: {},
      port: 9000,
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
    });
    expect(invocation.args[0]).toBe('--data-dir');
    expect(invocation.args[1]).toBe('/var/lib/kala');
    expect(invocation.args).toContain('--port');
    expect(invocation.args[invocation.args.indexOf('--port') + 1]).toBe('9000');
  });

  it('produces argv that, when spawned against a real CLI parser, binds to the chosen port', async () => {
    // Spin up a tiny node script that prints argv, spawn it with our
    // invocation shape, and confirm the captured argv matches what the
    // backend CLI would see. We use a Node script (rather than the
    // Python backend) so the test does not require the backend to be
    // installed in CI.
    const inspectorPath = path.join(os.tmpdir(), `argv-inspector-${process.pid}-${Date.now()}.cjs`);
    fs.writeFileSync(inspectorPath, "process.stdout.write(JSON.stringify(process.argv.slice(2)));\n");
    const invocation = mainModule.buildBackendInvocation({
      command: process.execPath,
      args: [inspectorPath],
      cwd: process.cwd(),
      env: {},
      port: 8765,
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
    });
    const captured = await new Promise<string>((resolve, reject) => {
      const child = spawn(invocation.command, invocation.args, { env: invocation.env });
      let stdout = '';
      child.stdout.on('data', (chunk) => { stdout += chunk.toString('utf8'); });
      child.on('exit', (code) => {
        if (code === 0) resolve(stdout);
        else reject(new Error(`inspector exited ${code}`));
      });
      child.on('error', reject);
    });
    fs.unlinkSync(inspectorPath);
    const argv = JSON.parse(captured) as string[];
    expect(argv).toEqual(expect.arrayContaining([
      '--host', '127.0.0.1',
      '--port', '8765',
    ]));
    // The token must NOT be in argv.
    expect(captured).not.toContain('unit-test-token');
  });
});

describe('pickFreePort', () => {
  it('returns a port in the loopback range that is currently free', async () => {
    const port = await mainModule.pickFreePort();
    expect(Number.isInteger(port)).toBe(true);
    expect(port).toBeGreaterThan(0);
    expect(port).toBeLessThanOrEqual(65_535);
  });

  it('releases the port it picks before returning', async () => {
    const port = await mainModule.pickFreePort();
    // If we never released, this would EADDRINUSE.
    await new Promise<void>((resolve, reject) => {
      const server = net.createServer();
      server.once('error', reject);
      server.listen(port, '127.0.0.1', () => {
        server.close(() => resolve());
      });
    });
  });
});

describe('generateToken', () => {
  it('produces a base64url token of at least 16 characters', () => {
    const token = mainModule.generateToken();
    expect(typeof token).toBe('string');
    expect(token.length).toBeGreaterThanOrEqual(16);
    expect(/^[A-Za-z0-9_-]+$/.test(token)).toBe(true);
  });

  it('returns a different token on each call', () => {
    const a = mainModule.generateToken();
    const b = mainModule.generateToken();
    expect(a).not.toBe(b);
  });
});

describe('validateIpcSender', () => {
  function makeWin(url: string) {
    const webContents = {
      mainFrame: { url },
      isDestroyed: () => false,
    };
    return { webContents };
  }
  // Sender must be the SAME object reference as win.webContents for the
  // sender check to pass and let later checks run. We assign sender
  // directly to win.webContents.
  function makeEvent(win: ReturnType<typeof makeWin>, frameUrl: string, destroyed = false) {
    const sender = Object.assign(win.webContents, { isDestroyed: () => destroyed });
    // The sender frame must be the SAME object reference as
    // win.webContents.mainFrame; we mutate the existing object so the
    // identity check passes.
    win.webContents.mainFrame.url = frameUrl;
    const senderFrame = win.webContents.mainFrame;
    return { sender, senderFrame };
  }

  it('rejects a sender that does not match the trusted window', () => {
    const fakeWin = makeWin('http://127.0.0.1:8765/');
    const fakeEvent = {
      sender: { isDestroyed: () => false },
      senderFrame: { url: 'http://127.0.0.1:8765/' },
    };
    expect(() => mainModule.validateIpcSender(fakeEvent, fakeWin as never, 'http://127.0.0.1:8765')).toThrow(
      /not the trusted window/,
    );
  });

  it('rejects a frame that does not match the trusted main frame', () => {
    const fakeWin = makeWin('http://127.0.0.1:8765/');
    const sender = Object.assign(fakeWin.webContents, { isDestroyed: () => false });
    // Different object reference for senderFrame — that is the trigger.
    const senderFrame = { url: 'http://127.0.0.1:9999/' };
    expect(() => mainModule.validateIpcSender({ sender, senderFrame }, fakeWin as never, 'http://127.0.0.1:8765')).toThrow(
      /main frame/,
    );
  });

  it('rejects an unparseable frame url', () => {
    const fakeWin = makeWin('http://127.0.0.1:8765/');
    const fakeEvent = makeEvent(fakeWin, 'not-a-url');
    expect(() => mainModule.validateIpcSender(fakeEvent, fakeWin as never, 'http://127.0.0.1:8765')).toThrow(
      /not parseable/,
    );
  });

  it('rejects a frame whose origin does not match the trusted origin', () => {
    const fakeWin = makeWin('http://localhost:8765/');
    const fakeEvent = makeEvent(fakeWin, 'http://localhost:8765/');
    expect(() => mainModule.validateIpcSender(fakeEvent, fakeWin as never, 'http://127.0.0.1:8765')).toThrow(
      /origin mismatch/,
    );
  });

  it('accepts a frame whose origin matches the trusted origin exactly', () => {
    const fakeWin = makeWin('http://127.0.0.1:8765/');
    const fakeEvent = makeEvent(fakeWin, 'http://127.0.0.1:8765/');
    expect(() => mainModule.validateIpcSender(fakeEvent, fakeWin as never, 'http://127.0.0.1:8765')).not.toThrow();
  });

  it('rejects a destroyed sender', () => {
    const fakeWin = makeWin('http://127.0.0.1:8765/');
    const fakeEvent = makeEvent(fakeWin, 'http://127.0.0.1:8765/', true);
    expect(() => mainModule.validateIpcSender(fakeEvent, fakeWin as never, 'http://127.0.0.1:8765')).toThrow(
      /sender unavailable/,
    );
  });
});

describe('BackendProcess (mocked spawn)', () => {
  it('marks itself exited when spawn errors with ENOENT', async () => {
    const proc = new mainModule.BackendProcess({
      command: path.join(os.tmpdir(), 'definitely-not-a-real-binary-' + Date.now()),
      args: [],
      cwd: process.cwd(),
      env: {},
      port: 8765,
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
    });
    proc.start();
    // Allow the spawn error event to propagate to the 'error' handler.
    await new Promise((resolve) => setTimeout(resolve, 250));
    await expect(proc.waitForHealth(500)).rejects.toThrow(/exited/);
    expect(proc.error).toBeTruthy();
  });

  it('records exit code and signal when the child exits unexpectedly', async () => {
    // Use a tiny node script that exits 0 immediately so the parent
    // can observe the exit event.
    const exitScript = path.join(os.tmpdir(), `exit-${Date.now()}.cjs`);
    fs.writeFileSync(exitScript, 'process.exit(0);');
    const proc = new mainModule.BackendProcess({
      command: process.execPath,
      args: [exitScript],
      cwd: process.cwd(),
      env: {},
      port: 8765,
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
    });
    proc.start();
    await new Promise((resolve) => setTimeout(resolve, 100));
    await expect(proc.waitForHealth(500)).rejects.toThrow(/exited/);
    fs.unlinkSync(exitScript);
    expect(proc.exited).toBe(true);
  });

  it('shutdown() is idempotent and does not throw on a process that never started', () => {
    const proc = new mainModule.BackendProcess({
      command: process.execPath,
      args: [],
      cwd: process.cwd(),
      env: {},
      port: 8765,
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
    });
    expect(() => proc.shutdown()).not.toThrow();
    expect(() => proc.shutdown()).not.toThrow();
  });
});

describe('resolveBackendCommand', () => {
  it('packaged mode resolves only resources/backend/kala-studio and ignores Python overrides', () => {
    const root = makeTemporaryDirectory();
    const resources = path.join(root, 'resources');
    const packaged = path.join(resources, 'backend', 'kala-studio');
    makeExecutable(packaged);
    const resolved = mainModule.resolveBackendCommand({
      isPackaged: true,
      platform: 'linux',
      resourcesPath: resources,
      python: '/definitely/not/the/python',
    });
    expect(resolved.mode).toBe('packaged-native');
    expect(resolved.command).toBe(packaged);
    expect(resolved.args).toEqual([]);
    expect(resolved.exists).toBe(true);
  });

  it('packaged mode fails closed when the native binary is missing', () => {
    const root = makeTemporaryDirectory();
    const resources = path.join(root, 'resources');
    const resolved = mainModule.resolveBackendCommand({
      isPackaged: true,
      platform: 'linux',
      resourcesPath: resources,
      python: '/usr/bin/python3',
      devBackend: 'python',
    });
    expect(resolved.exists).toBe(false);
    expect(resolved.mode).toBe('packaged-native');
    expect(resolved.command).toBe(path.join(resources, 'backend', 'kala-studio'));
    expect(resolved.instructions).toMatch(/packaged native backend/);
    expect(resolved.args).not.toContain('-m');
  });

  it('development prefers the staged native binary without a Python dependency', () => {
    const root = makeTemporaryDirectory();
    const native = path.join(root, 'studio', 'backend-native', 'kala-studio');
    makeExecutable(native);
    const resolved = mainModule.resolveBackendCommand({
      isPackaged: false,
      platform: 'linux',
      repoRoot: root,
      devBackend: 'native',
      python: '',
    });
    expect(resolved.mode).toBe('native');
    expect(resolved.command).toBe(native);
    expect(resolved.args).toEqual([]);
    expect(resolved.exists).toBe(true);
    expect(resolved.cwd).toBe(root);
  });

  it('development fails with actionable build instructions when native is missing', () => {
    const root = makeTemporaryDirectory();
    const resolved = mainModule.resolveBackendCommand({
      isPackaged: false,
      platform: 'linux',
      repoRoot: root,
      devBackend: 'native',
      python: '',
    });
    expect(resolved.mode).toBe('native');
    expect(resolved.exists).toBe(false);
    expect(resolved.command).toBe(path.join(root, 'studio', 'backend-native', 'kala-studio'));
    expect(resolved.instructions).toContain('npm run build:native');
    expect(resolved.instructions).toContain('KALA_STUDIO_PYTHON');
    // Never silently falls back to an arbitrary `python` on PATH.
    expect(resolved.command).not.toBe('python');
  });

  it('honours an explicit KALA_STUDIO_PYTHON override as a development-only reference', () => {
    const root = makeTemporaryDirectory();
    const python = path.join(root, 'python-reference');
    makeExecutable(python);
    const resolved = mainModule.resolveBackendCommand({
      isPackaged: false,
      platform: 'linux',
      repoRoot: root,
      devBackend: 'native',
      python,
    });
    expect(resolved.mode).toBe('python-override');
    expect(resolved.command).toBe(python);
    expect(resolved.args).toEqual(['-m', 'kala_agent.studio']);
    expect(resolved.env.PYTHONPATH).toBe(path.join(root, 'src'));
    expect(resolved.exists).toBe(true);
  });

  it('development defaults to the staged native binary, not Python', () => {
    const root = makeTemporaryDirectory();
    const native = path.join(root, 'studio', 'backend-native', 'kala-studio');
    makeExecutable(native);
    const resolved = mainModule.resolveBackendCommand({
      isPackaged: false,
      platform: 'linux',
      repoRoot: root,
      devBackend: '',
      python: '',
    });
    expect(resolved.mode).toBe('native');
    expect(resolved.command).toBe(native);
    expect(resolved.args).toEqual([]);
    expect(resolved.exists).toBe(true);
  });

  it('development fails closed when the default native binary is missing', () => {
    const root = makeTemporaryDirectory();
    const resolved = mainModule.resolveBackendCommand({
      isPackaged: false,
      platform: 'linux',
      repoRoot: root,
      devBackend: '',
      python: '',
    });
    expect(resolved.mode).toBe('native');
    expect(resolved.exists).toBe(false);
    expect(resolved.command).toBe(path.join(root, 'studio', 'backend-native', 'kala-studio'));
    expect(resolved.instructions).toContain('npm run build:native');
    expect(resolved.instructions).toContain('KALA_STUDIO_DEV_BACKEND=python');
    // Never silently falls back to an arbitrary `python` on PATH.
    expect(resolved.command).not.toBe('python');
    expect(resolved.args).not.toContain('-m');
  });

  it('selects the Python reference only behind an explicit development opt-in', () => {
    const root = makeTemporaryDirectory();
    const venv = path.join(root, '.venv', 'bin', 'python');
    makeExecutable(venv);
    const resolved = mainModule.resolveBackendCommand({
      isPackaged: false,
      platform: 'linux',
      repoRoot: root,
      devBackend: 'python',
      python: '',
    });
    expect(resolved.mode).toBe('python-reference');
    expect(resolved.command).toBe(venv);
    expect(resolved.args).toEqual(['-m', 'kala_agent.studio']);
    expect(resolved.env.PYTHONPATH).toBe(path.join(root, 'src'));
    expect(resolved.exists).toBe(true);
  });

  it('development still supports the explicit native opt-in', () => {
    const root = makeTemporaryDirectory();
    const venv = path.join(root, '.venv', 'bin', 'python');
    makeExecutable(venv);
    const native = path.join(root, 'studio', 'backend-native', 'kala-studio');
    makeExecutable(native);
    const resolved = mainModule.resolveBackendCommand({
      isPackaged: false,
      platform: 'linux',
      repoRoot: root,
      devBackend: 'native',
      python: '',
    });
    expect(resolved.mode).toBe('native');
    expect(resolved.command).toBe(native);
    expect(resolved.exists).toBe(true);
  });

  it('rejects an unknown development backend mode instead of guessing', () => {
    const root = makeTemporaryDirectory();
    const resolved = mainModule.resolveBackendCommand({
      isPackaged: false,
      platform: 'linux',
      repoRoot: root,
      devBackend: 'rust-please',
      python: '',
    });
    expect(resolved.mode).toBe('invalid');
    expect(resolved.exists).toBe(false);
    expect(resolved.instructions).toContain('unknown KALA_STUDIO_DEV_BACKEND=rust-please');
    expect(resolved.args).not.toContain('-m');
  });

  it('resolves the Windows .exe name in packaged mode', () => {
    const root = makeTemporaryDirectory();
    const resources = path.join(root, 'resources');
    const packaged = path.join(resources, 'backend', 'kala-studio.exe');
    makeExecutable(packaged);
    const resolved = mainModule.resolveBackendCommand({
      isPackaged: true,
      platform: 'win32',
      resourcesPath: resources,
      python: '',
    });
    expect(resolved.command).toBe(packaged);
    expect(resolved.exists).toBe(true);
  });

  it('packaged mode removes Python selection variables and exposes bundled loader paths', () => {
    const root = makeTemporaryDirectory();
    const resources = path.join(root, 'resources');
    const backend = path.join(resources, 'backend');
    makeExecutable(path.join(backend, 'kala-studio'));
    fs.mkdirSync(path.join(backend, 'lib'), { recursive: true });
    fs.mkdirSync(path.join(backend, 'adapters', 'freecad', 'src'), { recursive: true });

    const saved = {
      PYTHONPATH: process.env.PYTHONPATH,
      KALA_STUDIO_PYTHON: process.env.KALA_STUDIO_PYTHON,
      KALA_STUDIO_DEV_BACKEND: process.env.KALA_STUDIO_DEV_BACKEND,
      KALA_STUDIO_GEOMETRY_WORKER: process.env.KALA_STUDIO_GEOMETRY_WORKER,
      KALA_STUDIO_REPO_ROOT: process.env.KALA_STUDIO_REPO_ROOT,
      KALA_STUDIO_DIST: process.env.KALA_STUDIO_DIST,
      LD_LIBRARY_PATH: process.env.LD_LIBRARY_PATH,
    };
    process.env.PYTHONPATH = '/tmp/must-not-leak';
    process.env.KALA_STUDIO_PYTHON = '/usr/bin/python3';
    process.env.KALA_STUDIO_DEV_BACKEND = 'python';
    process.env.KALA_STUDIO_GEOMETRY_WORKER = '/tmp/worker';
    process.env.KALA_STUDIO_REPO_ROOT = '/tmp/repo';
    process.env.KALA_STUDIO_DIST = '/tmp/ui';
    process.env.LD_LIBRARY_PATH = '/opt/system-libs';
    try {
      const env = mainModule.packagedBackendEnvironment({
        resourcesPath: resources,
        platform: 'linux',
      });
      expect(env.PYTHONPATH).toBeUndefined();
      expect(env.KALA_STUDIO_PYTHON).toBeUndefined();
      expect(env.KALA_STUDIO_DEV_BACKEND).toBeUndefined();
      expect(env.KALA_STUDIO_GEOMETRY_WORKER).toBeUndefined();
      expect(env.KALA_STUDIO_REPO_ROOT).toBeUndefined();
      expect(env.KALA_STUDIO_DIST).toBeUndefined();
      expect(env.LD_LIBRARY_PATH).toBe(
        [path.join(backend, 'lib'), '/opt/system-libs'].join(path.delimiter),
      );
      expect(env.KALA_FREECAD_SYS_PATH).toBe(path.join(backend, 'adapters', 'freecad', 'src'));
    } finally {
      for (const [key, value] of Object.entries(saved)) {
        if (value === undefined) delete process.env[key];
        else process.env[key] = value;
      }
    }
  });

  it('packaged mode keeps inherited Python variables on a Windows environment only by removing them', () => {
    const root = makeTemporaryDirectory();
    const backend = path.join(root, 'resources', 'backend');
    fs.mkdirSync(path.join(backend, 'lib'), { recursive: true });
    const env = mainModule.packagedBackendEnvironment({
      resourcesPath: path.join(root, 'resources'),
      platform: 'win32',
      environ: { PATH: 'C:\\Windows', PYTHONPATH: 'C:\\leak' },
    });
    expect(env.PYTHONPATH).toBeUndefined();
    expect(env.PATH).toBeDefined();
    expect(env.PATH!.startsWith(backend)).toBe(true);
  });
});

describe('desktop-pack argv forwarding', () => {
  it('argv builder script forwards caller argv and injects --publish never', () => {
    const helperPath = path.resolve(__dirname, '..', 'scripts', 'desktop-pack.cjs');
    const source = fs.readFileSync(helperPath, 'utf8');
    expect(source).toContain('process.argv.slice(2)');
    expect(source).toContain('--publish');
    expect(source).toContain("'never'");
    expect(source).toContain('require.resolve');
  });
});

describe('desktop launcher argv forwarding', () => {
  it('desktop.cjs forwards process.argv to the Electron CLI without spawning a backend', () => {
    const launcherPath = path.resolve(__dirname, '..', 'scripts', 'desktop.cjs');
    const source = fs.readFileSync(launcherPath, 'utf8');
    // The launcher must NOT spawn the backend itself (it is the main
    // process's job) and must NOT write a token file to disk.
    expect(source).not.toMatch(/spawn\([^)]*[Bb]ackend|[Bb]ackend[^)]*spawn\(/);
    expect(source).not.toMatch(/writeFileSync[^)]*[Tt]oken|fs\.writeFileSync[^)]*token/i);
    // The launcher must forward argv and resolve Electron via its
    // installed entry.
    expect(source).toContain('process.argv.slice(2)');
    expect(source).toContain("require.resolve('electron/cli.js')");
  });
});

function studioRootPath(): string {
  // Resolve relative to the test file. The test runs from
  // studio/, so ../.. is the repo root and ../ is studio/.
  return path.resolve(__dirname, '..');
}

type FakeWebContents = EventEmitter & {
  session: { webRequest: { onHeadersReceived: (listener: () => void) => void } };
  setWindowOpenHandler: (handler: () => { action: string }) => void;
};

function makeFakeWindow(): { webContents: FakeWebContents } {
  const webContents = new EventEmitter() as FakeWebContents;
  webContents.session = { webRequest: { onHeadersReceived: () => undefined } };
  webContents.setWindowOpenHandler = vi.fn();
  return { webContents };
}

const TRUSTED_ORIGIN = 'http://127.0.0.1:8765';

describe('attachSecurityPolicy', () => {
  it('accepts navigation inside the trusted origin', () => {
    const win = makeFakeWindow();
    mainModule.attachSecurityPolicy(win as never, TRUSTED_ORIGIN);
    const event = { preventDefault: vi.fn() };

    win.webContents.emit('will-navigate', event, `${TRUSTED_ORIGIN}/index.html`);
    win.webContents.emit('will-redirect', event, `${TRUSTED_ORIGIN}/settings?tab=keys`);

    expect(event.preventDefault).not.toHaveBeenCalled();
  });

  it.each([
    'http://127.0.0.1:8765@attacker.example/',
    'http://127.0.0.1:8765.attacker.example/',
    'https://attacker.example/',
    'javascript:alert(1)',
    'not a url',
  ])('refuses off-origin navigation to %s', (target) => {
    const win = makeFakeWindow();
    mainModule.attachSecurityPolicy(win as never, TRUSTED_ORIGIN);
    const event = { preventDefault: vi.fn() };

    win.webContents.emit('will-navigate', event, target);

    expect(event.preventDefault).toHaveBeenCalledTimes(1);
  });

  it('refuses an off-origin server redirect before it becomes a navigation', () => {
    const win = makeFakeWindow();
    mainModule.attachSecurityPolicy(win as never, TRUSTED_ORIGIN);
    const event = { preventDefault: vi.fn() };

    win.webContents.emit('will-redirect', event, 'https://attacker.example/landing');

    expect(event.preventDefault).toHaveBeenCalledTimes(1);
  });

  it('refuses <webview> attachment and every window open', () => {
    const win = makeFakeWindow();
    mainModule.attachSecurityPolicy(win as never, TRUSTED_ORIGIN);
    const event = { preventDefault: vi.fn() };

    win.webContents.emit('will-attach-webview', event);

    expect(event.preventDefault).toHaveBeenCalledTimes(1);
    expect(win.webContents.setWindowOpenHandler).toHaveBeenCalledTimes(1);
    const handler = (win.webContents.setWindowOpenHandler as unknown as { mock: { calls: unknown[][] } })
      .mock.calls[0]![0] as () => { action: string };
    expect(handler()).toEqual({ action: 'deny' });
  });
});

describe('waitForHealth launch nonce', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  function backendProcess(requireLaunchNonce: boolean): InstanceType<typeof mainModule.BackendProcess> {
    return new mainModule.BackendProcess({
      command: process.execPath,
      args: [],
      cwd: process.cwd(),
      env: {},
      port: 8765,
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      nonce: 'unit-test-nonce-bbbbbbbbbbbbbbbbbbbbbbbb',
      requireLaunchNonce,
    });
  }

  it('accepts a health response that echoes the nonce', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        new Response(JSON.stringify({ status: 'ok' }), {
          status: 200,
          headers: { 'X-Kala-Launch-Nonce': 'unit-test-nonce-bbbbbbbbbbbbbbbbbbbbbbbb' },
        }),
      ),
    );
    const proc = backendProcess(true);

    await expect(proc.waitForHealth(1_000)).resolves.toBeUndefined();
    expect(proc.ready).toBe(true);
  });

  it('never reports ready for a responder that cannot echo the nonce', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response(JSON.stringify({ status: 'ok' }), { status: 200 })),
    );
    const proc = backendProcess(true);

    await expect(proc.waitForHealth(50)).rejects.toThrow(/readiness window/);
    expect(proc.ready).toBe(false);
  });

  it('does not require the echo from a backend mode that does not send one', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response(JSON.stringify({ status: 'ok' }), { status: 200 })),
    );
    const proc = backendProcess(false);

    await expect(proc.waitForHealth(1_000)).resolves.toBeUndefined();
    expect(proc.ready).toBe(true);
  });
});
