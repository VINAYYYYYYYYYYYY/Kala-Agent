// Regression coverage for the real Electron entry point.
//
// The native launch regression was: the app's main script gated startup
// on comparing the module against the process main entry, which is
// always false in Electron 44's browser process, so nothing started.
//
// We load the ACTUAL entry (`electron/bootstrap.cjs`) through Node's
// native CommonJS require (the same way Electron loads it) and inject a
// fake `electron` + `child_process` via a `Module._load` hook. That lets
// the real wiring run: bootstrap -> whenReady -> main -> pickFreePort ->
// spawn -> waitForHealth -> BrowserWindow -> IPC/security -> loadURL.
// The test asserts exactly one backend spawn with explicit --host/--port,
// a hardened window that loads the trusted loopback origin, the wired
// IPC/permission handlers, and app close on unexpected backend exit.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const nodeRequire = createRequire(import.meta.url);
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const Module = nodeRequire('node:module') as any;

interface Emitter {
  on(event: string, fn: (...args: unknown[]) => void): Emitter;
  once(event: string, fn: (...args: unknown[]) => void): Emitter;
  off(event: string, fn: (...args: unknown[]) => void): Emitter;
  emit(event: string, ...args: unknown[]): void;
}

function makeEmitter(): Emitter & Record<string, unknown> {
  const listeners = new Map<string, Array<(...args: unknown[]) => void>>();
  const api = {
    on(event: string, fn: (...args: unknown[]) => void) {
      const arr = listeners.get(event) ?? [];
      arr.push(fn);
      listeners.set(event, arr);
      return api;
    },
    once(event: string, fn: (...args: unknown[]) => void) {
      const wrap = (...args: unknown[]) => {
        api.off(event, wrap);
        fn(...args);
      };
      return api.on(event, wrap);
    },
    off(event: string, fn: (...args: unknown[]) => void) {
      const arr = listeners.get(event);
      if (arr) {
        const i = arr.indexOf(fn);
        if (i >= 0) arr.splice(i, 1);
      }
      return api;
    },
    emit(event: string, ...args: unknown[]) {
      for (const fn of [...(listeners.get(event) ?? [])]) fn(...args);
    },
  };
  return api as unknown as Emitter & Record<string, unknown>;
}

interface SpawnCall {
  command: string;
  args: string[];
  options: unknown;
}

interface Harness {
  spawnCalls: SpawnCall[];
  children: Array<Emitter & Record<string, unknown>>;
  windows: Array<Record<string, unknown>>;
  app: {
    whenReadyCalls: number;
    exitCalls: Array<number | undefined>;
    quitCalls: number;
  };
  dialogErrors: Array<{ title: string; message: string }>;
  ipcHandlers: Map<string, unknown>;
  permissionHandlers: unknown[];
  permissionCheckHandlers: unknown[];
  fakeElectron: unknown;
  fakeChildProcess: unknown;
}

function buildHarness(): Harness {
  const spawnCalls: SpawnCall[] = [];
  const children: Array<Emitter & Record<string, unknown>> = [];
  const windows: Array<Record<string, unknown>> = [];
  const app = { whenReadyCalls: 0, exitCalls: [] as Array<number | undefined>, quitCalls: 0 };
  const dialogErrors: Array<{ title: string; message: string }> = [];
  const ipcHandlers = new Map<string, unknown>();
  const permissionHandlers: unknown[] = [];
  const permissionCheckHandlers: unknown[] = [];

  const fakeApp = {
    isPackaged: true,
    whenReady: () => {
      app.whenReadyCalls += 1;
      return Promise.resolve();
    },
    exit: (code?: number) => {
      app.exitCalls.push(code);
    },
    quit: () => {
      app.quitCalls += 1;
    },
    on: () => fakeApp,
  };

  const fakeSession = {
    defaultSession: {
      setPermissionRequestHandler: (fn: unknown) => {
        permissionHandlers.push(fn);
      },
      setPermissionCheckHandler: (fn: unknown) => {
        permissionCheckHandlers.push(fn);
      },
      webRequest: { onHeadersReceived: () => undefined },
    },
  };

  function FakeBrowserWindow(options: unknown) {
    const win = makeEmitter();
    win.options = options;
    const webContents = makeEmitter();
    webContents.mainFrame = { url: '' };
    webContents.session = fakeSession.defaultSession;
    webContents.isDestroyed = () => false;
    webContents.setWindowOpenHandler = () => undefined;
    win.webContents = webContents;
    const loadURLCalls: string[] = [];
    win.loadURLCalls = loadURLCalls;
    win.loadURL = (url: string) => {
      loadURLCalls.push(url);
    };
    win.showCalls = 0;
    win.show = () => {
      win.showCalls = (win.showCalls as number) + 1;
    };
    win.isDestroyed = () => false;
    windows.push(win);
    return win;
  }

  const fakeIpcMain = {
    handle: (channel: string, fn: unknown) => {
      ipcHandlers.set(channel, fn);
    },
  };

  const fakeDialog = {
    showErrorBox: (title: string, message: string) => {
      dialogErrors.push({ title, message });
    },
  };

  const fakeElectron = {
    app: fakeApp,
    BrowserWindow: FakeBrowserWindow,
    ipcMain: fakeIpcMain,
    session: fakeSession,
    dialog: fakeDialog,
  };

  const fakeChildProcess = {
    spawn: (command: string, args: string[], options: unknown) => {
      spawnCalls.push({ command, args, options });
      const child = makeEmitter();
      child.stdout = makeEmitter();
      child.stderr = makeEmitter();
      child.killedWith = null;
      child.kill = (signal: string) => {
        child.killedWith = signal;
        return true;
      };
      children.push(child);
      return child;
    },
  };

  return {
    spawnCalls,
    children,
    windows,
    app,
    dialogErrors,
    ipcHandlers,
    permissionHandlers,
    permissionCheckHandlers,
    fakeElectron,
    fakeChildProcess,
  };
}

let harness: Harness;
let restoreLoad: () => void;
let tmpRoot = '';
const originalResourcesPath = (process as { resourcesPath?: string }).resourcesPath;

async function waitFor(predicate: () => boolean, timeoutMs = 4000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  throw new Error('waitFor timed out');
}

// Load the real bootstrap entry exactly as Electron does (native
// require), with `electron`/`child_process` redirected to the harness.
function loadEntry(): void {
  const originalLoad = Module._load;
  Module._load = function (request: string, parent: unknown, isMain: boolean) {
    if (request === 'electron') return harness.fakeElectron;
    if (request === 'child_process' || request === 'node:child_process') return harness.fakeChildProcess;
    return originalLoad.call(this, request, parent, isMain);
  };
  restoreLoad = () => {
    Module._load = originalLoad;
  };
  for (const rel of ['../electron/main.cjs', '../electron/bootstrap.cjs']) {
    delete nodeRequire.cache[nodeRequire.resolve(rel)];
  }
  nodeRequire('../electron/bootstrap.cjs');
}

beforeEach(() => {
  harness = buildHarness();

  tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'kala-entry-'));
  const backendDir = path.join(tmpRoot, 'backend');
  fs.mkdirSync(backendDir, { recursive: true });
  fs.writeFileSync(path.join(backendDir, process.platform === 'win32' ? 'kala-studio.exe' : 'kala-studio'), '');
  Object.defineProperty(process, 'resourcesPath', { value: tmpRoot, configurable: true });

  vi.stubGlobal(
    'fetch',
    vi.fn(async () => ({
      ok: true,
      json: async () => ({ status: 'ok', service: 'kala-studio', version: '0.1.0' }),
    })),
  );
});

afterEach(() => {
  if (restoreLoad) restoreLoad();
  vi.unstubAllGlobals();
  if (typeof originalResourcesPath === 'string') {
    Object.defineProperty(process, 'resourcesPath', { value: originalResourcesPath, configurable: true });
  } else {
    delete (process as { resourcesPath?: string }).resourcesPath;
  }
  if (tmpRoot) fs.rmSync(tmpRoot, { recursive: true, force: true });
});

describe('electron bootstrap entry', () => {
  it('starts the app even though require.main is not the module (the native regression)', async () => {
    loadEntry();
    await waitFor(() => harness.windows.length === 1);

    expect(harness.app.whenReadyCalls).toBe(1);
    expect(harness.permissionHandlers).toHaveLength(1);
    // Both the async request handler and the synchronous check handler deny
    // by default; a page must never be able to obtain camera/mic/geolocation.
    expect(harness.permissionCheckHandlers).toHaveLength(1);
    expect((harness.permissionCheckHandlers[0] as () => boolean)()).toBe(false);
    expect(harness.ipcHandlers.has('kala-studio:get-connection')).toBe(true);
  });

  it('spawns exactly one backend with explicit --host 127.0.0.1 and --port <chosen>', async () => {
    loadEntry();
    await waitFor(() => harness.windows.length === 1);

    expect(harness.spawnCalls).toHaveLength(1);
    const call = harness.spawnCalls[0]!;
    const binary = path.join(tmpRoot, 'backend', process.platform === 'win32' ? 'kala-studio.exe' : 'kala-studio');
    expect(call.command).toBe(binary);
    expect(call.args[call.args.indexOf('--host') + 1]).toBe('127.0.0.1');
    const port = call.args[call.args.indexOf('--port') + 1]!;
    expect(/^\d+$/.test(port)).toBe(true);
    expect(call.args.join(' ')).not.toContain('--no-sandbox');

    const win = harness.windows[0]!;
    expect(win.loadURLCalls).toEqual([`http://127.0.0.1:${port}/`]);
  });

  it('creates the window hardened: sandbox + contextIsolation + static preload + no nodeIntegration', async () => {
    loadEntry();
    await waitFor(() => harness.windows.length === 1);
    const options = harness.windows[0]!.options as { webPreferences: Record<string, unknown> };
    expect(options.webPreferences.contextIsolation).toBe(true);
    expect(options.webPreferences.nodeIntegration).toBe(false);
    expect(options.webPreferences.sandbox).toBe(true);
    expect(options.webPreferences.webSecurity).toBe(true);
    expect(String(options.webPreferences.preload)).toMatch(/electron[/\\]preload\.cjs$/);
  });

  it('closes the app and shows a failure when the owned backend exits after readiness', async () => {
    loadEntry();
    await waitFor(() => harness.windows.length === 1);
    expect(harness.children).toHaveLength(1);

    harness.children[0]!.emit('exit', 1, null);

    expect(harness.dialogErrors).toHaveLength(1);
    expect(harness.dialogErrors[0]!.title).toContain('Kala Studio');
    expect(harness.app.quitCalls).toBe(1);
    // We never re-kill an already-dead child.
    expect(harness.children[0]!.killedWith).toBeNull();
  });
});

describe('electron entry wiring (static)', () => {
  const studioRoot = path.resolve(__dirname, '..');

  it('package.json points Electron at the bootstrap entry', () => {
    const pkg = JSON.parse(fs.readFileSync(path.join(studioRoot, 'package.json'), 'utf8')) as { main: string };
    expect(pkg.main).toBe('electron/bootstrap.cjs');
  });

  it('neither bootstrap.cjs nor main.cjs uses a require.main startup guard', () => {
    const bootstrapSrc = fs.readFileSync(path.join(studioRoot, 'electron', 'bootstrap.cjs'), 'utf8');
    const mainSrc = fs.readFileSync(path.join(studioRoot, 'electron', 'main.cjs'), 'utf8');
    expect(bootstrapSrc).not.toContain('require.main');
    expect(mainSrc).not.toContain('require.main');
    expect(bootstrapSrc).toContain('bootstrap()');
  });

  it('the packaging config bundles the bootstrap entry', () => {
    const builder = fs.readFileSync(path.join(studioRoot, 'electron-builder.json'), 'utf8');
    expect(builder).toContain('electron/bootstrap.cjs');
  });

  it('the dev launcher launches the bootstrap entry and never spawns a backend/token', () => {
    const launcher = fs.readFileSync(path.join(studioRoot, 'scripts', 'desktop.cjs'), 'utf8');
    expect(launcher).toContain("'bootstrap.cjs'");
    expect(launcher).toContain("require.resolve('electron/cli.js')");
    expect(launcher).not.toMatch(/spawn\([^)]*[Bb]ackend/);
    expect(launcher).not.toMatch(/writeFileSync[^)]*[Tt]oken|fs\.writeFileSync[^)]*token/i);
  });

  it('the preload stays a static, single-channel bridge', () => {
    const preload = fs.readFileSync(path.join(studioRoot, 'electron', 'preload.cjs'), 'utf8');
    expect(preload).toContain('contextBridge.exposeInMainWorld');
    expect(preload).toContain("ipcRenderer.invoke('kala-studio:get-connection')");
    expect(preload).not.toContain('ipcRenderer.on');
    expect(preload).not.toContain('nodeIntegration');
  });
});
