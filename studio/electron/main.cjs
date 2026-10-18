// Kala Studio — Electron main process.
//
// Lifecycle:
//   1. Detect packaged vs development mode via `app.isPackaged`. In
//      packaged builds the backend binary lives at
//      `resources/backend/kala-studio[.exe]`; we refuse to start
//      otherwise rather than fall back to an arbitrary Python on the
//      operator's machine.
//   2. Pick a free loopback port via `pickFreePort`.
//   3. Spawn the backend with the exact argv shape the backend CLI
//      expects: `--host 127.0.0.1 --port <port>`. The backend reads
//      `--port` from argv, NOT from `KALA_STUDIO_PORT`; that env var
//      only configures where the token is sourced. Without the explicit
//      `--port` flag the backend would default to 8765 and our readiness
//      probe would hang.
//   4. Wait for `/api/studio/health` (bounded).
//   5. Construct the BrowserWindow with the static `electron/preload.cjs`
//      path set on `webPreferences.preload` BEFORE construction, plus
//      `contextIsolation: true`, `nodeIntegration: false`, `sandbox: true`.
//   6. Validate IPC sender URL exactly via `URL#origin` (not `startsWith`).
//      Require both `sender === win.webContents` and
//      `senderFrame === win.webContents.mainFrame` so a child frame cannot
//      trick the bridge.
//   7. Deny every window open (`setWindowOpenHandler` returns deny) and
//      every navigation except within the trusted origin.
//   8. Tear down the backend child on window close, app quit, unexpected
//      child exit, or startup failure. We never touch unrelated processes.
//   9. Only ONE backend process per session — the Electron main process
//      is the sole spawner. The legacy launcher-managed path is removed
//      because the previous dual-spawn approach did not pass `--port`
//      and could not work.
//
// No temporary token files are written from the Electron flow itself.
// No JS is written into `userData`. No `shell.openExternal` is invoked;
// the renderer cannot open anything.

'use strict';

const { app, BrowserWindow, ipcMain, session, dialog } = require('electron');
const { spawn } = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const net = require('node:net');

const REPO_ROOT = path.resolve(__dirname, '..', '..');
const PRELOAD_PATH = path.join(__dirname, 'preload.cjs');
const TRUSTED_HOST = '127.0.0.1';

function pickFreePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, TRUSTED_HOST, () => {
      const address = server.address();
      if (!address || typeof address === 'string') {
        server.close();
        reject(new Error('failed to acquire loopback port'));
        return;
      }
      server.close(() => resolve(address.port));
    });
  });
}

function generateToken() {
  return crypto.randomBytes(24).toString('base64url');
}

// Build the argv and environment for the backend. The backend CLI takes
// `--host` and `--port` as real flags; it does NOT read the port from
// `KALA_STUDIO_PORT`. The env var only affects where the token file is
// read from.
function buildBackendInvocation({ command, args, cwd, env, port, token }) {
  const fullArgs = [
    ...args,
    '--host', TRUSTED_HOST,
    '--port', String(port),
  ];
  return {
    command,
    args: fullArgs,
    cwd,
    env: { ...env, KALA_STUDIO_TOKEN: token },
    port,
    token,
  };
}

// Find the backend binary. `app.isPackaged` is the source of truth;
// `process.resourcesPath` is set in dev as well, so checking its
// truthiness is wrong. We import `electron` lazily so this module loads
// under plain Node for unit tests.
function resolveBackendCommand() {
  let isPackaged = false;
  try {
    isPackaged = require('electron').app.isPackaged;
  } catch {
    isPackaged = false;
  }
  if (isPackaged) {
    const packaged = path.join(
      process.resourcesPath,
      'backend',
      process.platform === 'win32' ? 'kala-studio.exe' : 'kala-studio',
    );
    return { command: packaged, args: [], exists: fs.existsSync(packaged) };
  }
  // Dev: prefer the project's .venv interpreter when present so the
  // operator doesn't need a globally-installed Python.
  const repoVenvBin = process.platform === 'win32'
    ? path.join(REPO_ROOT, '.venv', 'Scripts', 'python.exe')
    : path.join(REPO_ROOT, '.venv', 'bin', 'python');
  const venvPython = fs.existsSync(repoVenvBin) ? repoVenvBin : null;
  const python = process.env.KALA_STUDIO_PYTHON || venvPython || 'python';
  return {
    command: python,
    args: ['-m', 'kala_agent.studio'],
    cwd: REPO_ROOT,
    env: { ...process.env, PYTHONPATH: path.join(REPO_ROOT, 'src') },
    exists: true,
  };
}

class BackendProcess {
  constructor({ command, args, cwd, env, port, token }) {
    this.command = command;
    this.args = args;
    this.cwd = cwd;
    this.env = env;
    this.port = port;
    this.token = token;
    this.child = null;
    this.exited = false;
    // `ready` flips true once the readiness probe succeeds. Only a child
    // that exited AFTER readiness is "unexpected"; a child that dies
    // during the probe is reported by `waitForHealth`.
    this.ready = false;
    // `expected` flips true the moment we decide to shut the child down,
    // so the exit handler will not treat our own SIGTERM as a crash.
    this.expected = false;
    // Optional callback invoked when the child exits after readiness and
    // we did not ask it to. `main()` wires this to close the window.
    this.onUnexpectedExit = null;
    this.error = null;
    this.exitCode = null;
    this.exitSignal = null;
  }

  start() {
    try {
      this.child = spawn(this.command, this.args, {
        cwd: this.cwd,
        env: this.env,
        stdio: ['ignore', 'pipe', 'pipe'],
        windowsHide: true,
      });
    } catch (cause) {
      this.error = cause;
      this.exited = true;
      return;
    }
    this.child.on('error', (cause) => {
      this.error = cause;
      this.exited = true;
    });
    this.child.stdout.on('data', () => undefined);
    this.child.stderr.on('data', () => undefined);
    this.child.on('exit', (code, signal) => {
      this.exited = true;
      this.exitCode = code;
      this.exitSignal = signal;
      if (this.ready && !this.expected && typeof this.onUnexpectedExit === 'function') {
        try {
          this.onUnexpectedExit({ code, signal });
        } catch {
          // Never let an observer throw out of a child-process event.
        }
      }
    });
  }

  async waitForHealth(timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    const url = `http://${TRUSTED_HOST}:${this.port}/api/studio/health`;
    while (Date.now() < deadline) {
      if (this.exited) {
        const cause = this.error ? this.error.message : `exit=${this.exitCode} signal=${this.exitSignal}`;
        throw new Error(`backend exited before reporting health (${cause})`);
      }
      try {
        const response = await fetch(url, {
          headers: { Accept: 'application/json' },
          signal: AbortSignal.timeout(1500),
        });
        if (response.ok) {
          const payload = await response.json();
          if (payload && payload.status === 'ok') {
            this.ready = true;
            return;
          }
        }
      } catch {
        // retry
      }
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    throw new Error(`backend did not respond within the readiness window (port=${this.port})`);
  }

  shutdown() {
    this.expected = true;
    if (!this.child || this.exited) return;
    try {
      this.child.kill('SIGTERM');
    } catch {
      // ignore
    }
    const forceAfter = setTimeout(() => {
      if (this.child && !this.exited) {
        try { this.child.kill('SIGKILL'); } catch { /* ignore */ }
      }
    }, 3_000);
    this.child.once('exit', () => clearTimeout(forceAfter));
  }
}

function attachSecurityPolicy(win, trustedOrigin) {
  win.webContents.on('will-navigate', (event, target) => {
    if (target !== trustedOrigin && !target.startsWith(trustedOrigin + '/')) {
      event.preventDefault();
    }
  });
  win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  win.webContents.session.webRequest.onHeadersReceived((details, callback) => {
    callback({
      responseHeaders: {
        ...details.responseHeaders,
        'Content-Security-Policy': [
          "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self' data:; worker-src 'self' blob:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ],
        'X-Content-Type-Options': ['nosniff'],
        'Referrer-Policy': ['no-referrer'],
      },
    });
  });
}

function validateIpcSender(event, win, trustedOrigin) {
  const sender = event.sender;
  const frame = event.senderFrame;
  if (!sender || sender.isDestroyed()) {
    throw new Error('ipc sender unavailable');
  }
  if (sender !== win.webContents) {
    throw new Error('ipc sender is not the trusted window');
  }
  if (!frame || frame !== win.webContents.mainFrame) {
    throw new Error('ipc sender frame is not the main frame');
  }
  if (!frame.url) {
    throw new Error('ipc sender frame url is unavailable');
  }
  let origin;
  try {
    origin = new URL(frame.url).origin;
  } catch {
    throw new Error('ipc sender frame url is not parseable');
  }
  if (origin !== trustedOrigin) {
    throw new Error(`ipc sender origin mismatch (got ${origin})`);
  }
}

function registerIpc(win, trustedOrigin, baseUrl, token) {
  ipcMain.handle('kala-studio:get-connection', (event) => {
    validateIpcSender(event, win, trustedOrigin);
    return { baseUrl, token };
  });
}

async function main() {
  const port = await pickFreePort();
  const token = generateToken();
  const resolved = resolveBackendCommand();
  if (!resolved.exists) {
    console.error(
      '[kala-studio] packaged backend missing at resources/backend/kala-studio' +
      (process.platform === 'win32' ? '.exe' : '') +
      '; refusing to start an arbitrary Python fallback in production.',
    );
    app.exit(1);
    return;
  }
  const invocation = buildBackendInvocation({
    command: resolved.command,
    args: resolved.args,
    cwd: resolved.cwd,
    env: resolved.env,
    port,
    token,
  });
  const backend = new BackendProcess(invocation);
  backend.start();

  try {
    await backend.waitForHealth(15_000);
  } catch (cause) {
    backend.shutdown();
    console.error('[kala-studio] failed to start backend:', cause);
    app.exit(1);
    return;
  }

  const trustedOrigin = `http://${TRUSTED_HOST}:${port}`;

  const win = new BrowserWindow({
    width: 1280,
    height: 820,
    minWidth: 960,
    minHeight: 640,
    title: 'Kala Studio',
    backgroundColor: '#0c1116',
    show: false,
    webPreferences: {
      preload: PRELOAD_PATH,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
      allowRunningInsecureContent: false,
      experimentalFeatures: false,
      spellcheck: false,
      javascript: true,
    },
  });

  registerIpc(win, trustedOrigin, trustedOrigin, token);
  attachSecurityPolicy(win, trustedOrigin);

  let shuttingDown = false;
  function shutdownOnce() {
    if (shuttingDown) return;
    shuttingDown = true;
    backend.shutdown();
  }

  // If the backend dies after it reported healthy, the workspace is
  // unusable: surface the failure and close the app rather than leaving
  // a dead window behind. `shutdownOnce` reaps the (already dead) child
  // and clears the force-kill timer.
  backend.onUnexpectedExit = ({ code, signal }) => {
    console.error(
      `[kala-studio] backend exited unexpectedly after startup (code=${code ?? 'null'} signal=${signal ?? 'null'}); closing.`,
    );
    shutdownOnce();
    try {
      dialog.showErrorBox(
        'Kala Studio backend stopped',
        'The Kala Studio backend process exited unexpectedly, so the workspace can no longer work. The application will now close.',
      );
    } catch {
      // Headless environments may not offer a dialog; still close.
    }
    app.quit();
  };

  win.once('ready-to-show', () => win.show());
  win.on('closed', shutdownOnce);

  app.on('window-all-closed', () => {
    shutdownOnce();
    if (process.platform !== 'darwin') app.quit();
  });

  app.on('before-quit', shutdownOnce);

  win.loadURL(`${trustedOrigin}/`);
}

// Start the application. `electron/bootstrap.cjs` is the real entry and
// calls this after `app.whenReady()` is available; it is exported (rather
// than run from a top-level process-main-entry guard) because in
// Electron 44 the browser-process main entry is Electron itself, so that
// comparison is false for the app main script and previously blocked all
// startup.
function bootstrap() {
  return app.whenReady().then(() => {
    session.defaultSession.setPermissionRequestHandler((_wc, _permission, callback) => callback(false));
    return main().catch((cause) => {
      console.error('[kala-studio] failed to start:', cause);
      app.exit(1);
    });
  });
}

module.exports = {
  bootstrap,
  resolveBackendCommand,
  buildBackendInvocation,
  validateIpcSender,
  pickFreePort,
  generateToken,
  BackendProcess,
  attachSecurityPolicy,
  registerIpc,
  TRUSTED_HOST,
  PRELOAD_PATH,
  REPO_ROOT,
};