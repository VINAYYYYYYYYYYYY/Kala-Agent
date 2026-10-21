// Kala Studio — Electron main process.
//
// Lifecycle:
//   1. Detect packaged vs development mode via `app.isPackaged`. In
//      packaged builds the native backend binary, its static UI and the
//      bundled OCCT/FreeCAD runtime live under `resources/backend/`
//      (`kala-studio[.exe]`, `studio_dist/`, `lib/`, `adapters/`). Packaged
//      mode launches only the native Rust `kala-studio`; Python environment
//      overrides are removed and never consulted. Development defaults to the
//      staged native slice; the Python reference is an explicit
//      `KALA_STUDIO_DEV_BACKEND=python` development fallback only.
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

const {
  RotatingLog,
  formatStartupFailure,
  reportStartupFailure,
  scrubSecrets,
} = require('./backend-log.cjs');

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

/**
 * Where the backend log lives.
 *
 * Electron's `app.getPath('logs')` is the documented place for a desktop
 * application's own logs; the user data directory is the fallback when
 * Electron cannot resolve one. The Studio is the only writer of the file and
 * the backend is never told where it is.
 */
function resolveLogsDirectory(appLike = app) {
  try {
    const logs = appLike && typeof appLike.getPath === 'function' ? appLike.getPath('logs') : null;
    if (logs) return path.resolve(logs);
  } catch {
    // fall through to the user data directory
  }
  try {
    const userData = appLike && typeof appLike.getPath === 'function' ? appLike.getPath('userData') : null;
    if (userData) return path.join(path.resolve(userData), 'logs');
  } catch {
    // fall through to the repository-local fallback
  }
  return path.join(REPO_ROOT, 'studio', 'logs');
}

/**
 * Create the bounded, rotating backend log.
 *
 * The bearer token is registered as a secret so that a backend which echoes
 * its own environment cannot write the token into the log or the error
 * dialog.
 */
function createBackendLogger({ token, directory, ...options } = {}) {
  return new RotatingLog({
    dir: directory || resolveLogsDirectory(),
    secrets: token ? [token] : [],
    ...options,
  });
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

const NATIVE_STAGE_DIR = path.join(REPO_ROOT, 'studio', 'backend-native');
const DEV_BACKEND_ENV = 'KALA_STUDIO_DEV_BACKEND';

function nativeBinaryName(platform = process.platform) {
  return platform === 'win32' ? 'kala-studio.exe' : 'kala-studio';
}

function isFile(candidate) {
  try {
    return fs.statSync(candidate).isFile();
  } catch {
    return false;
  }
}

function isExecutableFile(candidate, platform = process.platform) {
  try {
    if (!fs.statSync(candidate).isFile()) return false;
    if (platform !== 'win32') fs.accessSync(candidate, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function nativeInstructions({ packaged, candidate, platform }) {
  if (packaged) {
    return `the packaged native backend is missing at ${candidate}. ` +
      'Rebuild the desktop bundle with the native stage; packaged mode never uses Python.';
  }
  return `the native backend is not staged at ${candidate}. ` +
    'Run `npm run build:native` in studio/ (or `node scripts/native-backend.cjs`). ' +
    'Development defaults to this native slice; the Python reference is available only with ' +
    'KALA_STUDIO_DEV_BACKEND=python and KALA_STUDIO_PYTHON.';
}

const PACKAGED_ENV_KEYS_TO_REMOVE = [
  'PYTHONPATH',
  'PYTHONHOME',
  'KALA_STUDIO_PYTHON',
  'KALA_STUDIO_DEV_BACKEND',
  'KALA_STUDIO_GEOMETRY_WORKER',
  'KALA_STUDIO_REPO_ROOT',
  'KALA_STUDIO_DIST',
];

function isDirectory(candidate) {
  try {
    return fs.statSync(candidate).isDirectory();
  } catch {
    return false;
  }
}

/**
 * Build the backend child environment for a packaged launch.
 *
 * Packaged mode is native-only: Python selection variables are removed, so a
 * stale `KALA_STUDIO_PYTHON`/`KALA_STUDIO_DEV_BACKEND` in the operator's shell
 * can never select a Python interpreter. The bundled OCCT directory is added
 * to the platform loader path (this is the documented fallback when patchelf
 * was unavailable at staging time) and the FreeCAD adapter is exposed to the
 * provider's discovery path.
 */
function packagedBackendEnvironment({
  resourcesPath,
  environ = process.env,
  platform = process.platform,
} = {}) {
  const env = { ...environ };
  for (const key of PACKAGED_ENV_KEYS_TO_REMOVE) delete env[key];
  const backendDir = path.join(resourcesPath, 'backend');
  const libDir = path.join(backendDir, 'lib');
  if (platform === 'linux' && isDirectory(libDir)) {
    env.LD_LIBRARY_PATH = [libDir, env.LD_LIBRARY_PATH].filter(Boolean).join(path.delimiter);
  } else if (platform === 'win32') {
    env.PATH = [backendDir, isDirectory(libDir) ? libDir : null, env.PATH]
      .filter(Boolean)
      .join(path.delimiter);
  }
  const adapterSrc = path.join(backendDir, 'adapters', 'freecad', 'src');
  if (isDirectory(adapterSrc)) {
    env.KALA_FREECAD_SYS_PATH = adapterSrc;
  }
  return env;
}

// Find the backend binary. `app.isPackaged` is the source of truth;
// `process.resourcesPath` is set in dev as well, so checking its
// truthiness is wrong. We import `electron` lazily so this module loads
// under plain Node for unit tests. Optional arguments are a test seam:
// production callers use the live Electron app/environment.
//
// Packaged mode resolves ONLY `resources/backend/kala-studio[.exe]`; it
// never consults Python overrides. Development defaults to the staged native
// slice and fails with actionable build instructions when it is missing.
// `KALA_STUDIO_DEV_BACKEND=python` (or `python-reference`) is the explicit
// development-only reference mode; neither path falls back to an arbitrary
// `python` found on PATH.
function resolveBackendCommand(options = {}) {
  let isPackaged = options.isPackaged;
  if (isPackaged === undefined) {
    try {
      isPackaged = require('electron').app.isPackaged;
    } catch {
      isPackaged = false;
    }
  }
  const platform = options.platform || process.platform;
  const resourcesPath = options.resourcesPath || process.resourcesPath;
  const repoRoot = options.repoRoot || REPO_ROOT;
  const binaryName = nativeBinaryName(platform);

  if (isPackaged) {
    const packaged = path.join(resourcesPath, 'backend', binaryName);
    return {
      command: packaged,
      args: [],
      cwd: path.dirname(packaged),
      env: packagedBackendEnvironment({ resourcesPath, platform }),
      // A packaged resource may not carry a Unix execute bit through an
      // archive extraction. Presence is the packaging contract; a spawn
      // failure is surfaced by the readiness probe with the child error.
      exists: isFile(packaged),
      mode: 'packaged-native',
      instructions: nativeInstructions({ packaged: true, candidate: packaged, platform }),
    };
  }

  const devMode = String(options.devBackend ?? process.env[DEV_BACKEND_ENV] ?? '')
    .trim()
    .toLowerCase();
  const pythonOverride = String(options.python ?? process.env.KALA_STUDIO_PYTHON ?? '').trim();
  const native = path.join(repoRoot, 'studio', 'backend-native', binaryName);

  // An explicit interpreter override selects the development reference backend.
  if (pythonOverride) {
    const exists = isExecutableFile(pythonOverride, platform);
    return {
      command: pythonOverride,
      args: ['-m', 'kala_agent.studio'],
      cwd: repoRoot,
      env: { ...process.env, PYTHONPATH: path.join(repoRoot, 'src') },
      exists,
      mode: 'python-override',
      instructions: `the development-only Python reference backend is not executable at ${pythonOverride}. ` +
        'Point KALA_STUDIO_PYTHON at the intended interpreter or unset it to use the native backend.',
    };
  }

  if (devMode === 'python' || devMode === 'python-reference') {
    const venvPython = platform === 'win32'
      ? path.join(repoRoot, '.venv', 'Scripts', 'python.exe')
      : path.join(repoRoot, '.venv', 'bin', 'python');
    return {
      command: venvPython,
      args: ['-m', 'kala_agent.studio'],
      cwd: repoRoot,
      env: { ...process.env, PYTHONPATH: path.join(repoRoot, 'src') },
      exists: isExecutableFile(venvPython, platform),
      mode: 'python-reference',
      instructions: `the development reference backend is not executable at ${venvPython}. ` +
        'Create the repository virtualenv, set KALA_STUDIO_PYTHON to the intended interpreter, ' +
        'or unset KALA_STUDIO_DEV_BACKEND to use the native backend.',
    };
  }

  if (devMode === '' || devMode === 'native') {
    return {
      command: native,
      args: [],
      cwd: repoRoot,
      env: { ...process.env },
      exists: isExecutableFile(native, platform),
      mode: 'native',
      instructions: nativeInstructions({ packaged: false, candidate: native, platform }),
    };
  }

  return {
    command: native,
    args: [],
    cwd: repoRoot,
    env: { ...process.env },
    exists: false,
    mode: 'invalid',
    instructions: `unknown ${DEV_BACKEND_ENV}=${devMode}; use "native" or "python". ` +
      nativeInstructions({ packaged: false, candidate: native, platform }),
  };
}

class BackendProcess {
  constructor({ command, args, cwd, env, port, token, logger = null }) {
    this.command = command;
    this.args = args;
    this.cwd = cwd;
    this.env = env;
    this.port = port;
    this.token = token;
    this.logger = logger;
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
    // The child's output is the only thing the backend says about itself.
    // It goes to the bounded log and nowhere else; it is never echoed to a
    // console the user might share.
    this.child.stdout.on('data', (chunk) => { this.writeLog(chunk); });
    this.child.stderr.on('data', (chunk) => { this.writeLog(chunk); });
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

  writeLog(chunk) {
    if (!this.logger || typeof this.logger.write !== 'function') return;
    try {
      this.logger.write(chunk.toString('utf8'));
    } catch {
      // A log that cannot be written must never take the app down.
    }
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
  // The Studio is the only writer of the backend log. It is created before
  // anything can fail so that every failure below has a log to point at.
  const logger = createBackendLogger({ token });
  if (!resolved.exists) {
    logger.write(`[kala-studio] the backend binary is missing at ${resolved.command}\n`);
    reportStartupFailure({
      dialog,
      logger,
      title: 'Kala Studio could not start',
      reason: `${resolved.instructions}\n\nThe backend program was not found, so the workspace could not open.`,
    });
    app.exit(1);
    return;
  }
  if (resolved.mode === 'python-override' || resolved.mode === 'python-reference') {
    console.warn(
      '[kala-studio] development-only Python reference backend selected by an explicit ' +
      'KALA_STUDIO_DEV_BACKEND/KALA_STUDIO_PYTHON override; packaged builds always use the ' +
      'staged native backend.',
    );
  }
  const invocation = buildBackendInvocation({
    command: resolved.command,
    args: resolved.args,
    cwd: resolved.cwd,
    env: resolved.env,
    port,
    token,
  });
  const backend = new BackendProcess({ ...invocation, logger });
  backend.start();

  try {
    await backend.waitForHealth(15_000);
  } catch (cause) {
    backend.shutdown();
    logger.write(`[kala-studio] failed to start backend: ${cause && cause.message ? cause.message : cause}\n`);
    reportStartupFailure({
      dialog,
      logger,
      title: 'Kala Studio could not start',
      reason:
        'The Kala Studio backend did not become ready, so the workspace could not open.\n\n' +
        `${cause && cause.message ? cause.message : String(cause)}`,
    });
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
    logger.write(
      `[kala-studio] the backend exited unexpectedly after startup ` +
      `(code=${code ?? 'null'} signal=${signal ?? 'null'}); closing.\n`,
    );
    shutdownOnce();
    reportStartupFailure({
      dialog,
      logger,
      title: 'Kala Studio backend stopped',
      reason:
        'The Kala Studio backend process exited unexpectedly, so the workspace can no longer work. ' +
        'The application will now close.',
    });
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
  packagedBackendEnvironment,
  buildBackendInvocation,
  validateIpcSender,
  pickFreePort,
  generateToken,
  resolveLogsDirectory,
  createBackendLogger,
  formatStartupFailure,
  reportStartupFailure,
  scrubSecrets,
  RotatingLog,
  BackendProcess,
  attachSecurityPolicy,
  registerIpc,
  TRUSTED_HOST,
  PRELOAD_PATH,
  REPO_ROOT,
  NATIVE_STAGE_DIR,
  DEV_BACKEND_ENV,
};