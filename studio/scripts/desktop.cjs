#!/usr/bin/env node
// Dev launcher for Kala Studio.
//
// The launcher does ONE thing: prepare the UI bundle (if missing) and
// start the Electron main process. The main process is the sole owner
// of the backend lifecycle; the launcher never spawns the backend and
// never writes the token to disk. The token stays in the main process
// and reaches the renderer only via the narrow `kala-studio:get-connection`
// IPC handler.

'use strict';

const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const studioRoot = path.resolve(__dirname, '..');

function ensureUiBuilt() {
  const distIndex = path.join(studioRoot, 'dist', 'index.html');
  if (fs.existsSync(distIndex)) return Promise.resolve();
  // Build the UI by invoking the installed Vite JS entrypoint
  // directly. We deliberately do NOT shell out to `npm run build`
  // because this package does not depend on npm and the .cmd shim
  // breaks on Windows.
  const viteEntry = path.join(studioRoot, 'node_modules', 'vite', 'bin', 'vite.js');
  if (!fs.existsSync(viteEntry)) {
    return Promise.reject(new Error(
      `[kala-studio] UI dist missing and Vite is not installed at ${viteEntry}. ` +
      'Run `npm install` from the studio directory, then `npm run build`.',
    ));
  }
  console.log('[kala-studio] UI not built yet — running vite build…');
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [viteEntry, 'build'], {
      cwd: studioRoot,
      stdio: 'inherit',
      env: process.env,
    });
    child.on('exit', (code) => {
      if (code === 0) resolve(undefined);
      else reject(new Error(`vite build exited with code ${code}`));
    });
    child.on('error', reject);
  });
}

function launchElectron(forwardArgs) {
  // Resolve electron via its installed entrypoint so we never shell out
  // to npm.cmd / npx.cmd (which break on Windows).
  let electronCli;
  try {
    electronCli = require.resolve('electron/cli.js');
  } catch (error) {
    throw new Error('electron is not installed; run `npm install` first');
  }
  const mainEntry = path.join(studioRoot, 'electron', 'bootstrap.cjs');
  const args = [electronCli, mainEntry, ...forwardArgs];
  const child = spawn(process.execPath, args, {
    cwd: studioRoot,
    stdio: 'inherit',
    env: process.env,
    windowsHide: true,
  });
  child.on('error', (cause) => {
    console.error('[kala-studio] electron spawn failed:', cause);
    process.exit(1);
  });
  child.on('exit', (code, signal) => {
    if (signal) {
      console.log(`[kala-studio] electron exited via ${signal}`);
      process.exit(0);
    }
    process.exit(code ?? 0);
  });
}

async function main() {
  try {
    await ensureUiBuilt();
  } catch (cause) {
    console.error('[kala-studio] failed to build UI:', cause);
    process.exit(1);
    return;
  }
  const forwardArgs = process.argv.slice(2);
  launchElectron(forwardArgs);
}

main();