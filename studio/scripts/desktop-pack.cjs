#!/usr/bin/env node
// Packaging helper.
//
// Verifies the UI build and the packaged backend binary are present,
// then runs the installed electron-builder CLI with the process's own
// argv forwarded (so conductor flags like `--dir` survive). We always
// inject `--publish never` to ensure an accidental `npm run desktop:pack`
// never tries to upload to a remote registry.

'use strict';

const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const studioRoot = path.resolve(__dirname, '..');

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

function main() {
  const distIndex = path.join(studioRoot, 'dist', 'index.html');
  if (!exists(distIndex)) {
    console.error('[desktop-pack] UI dist missing. Run `npm run build` first.');
    process.exit(1);
    return;
  }
  const packaged = path.join(studioRoot, 'backend', process.platform === 'win32' ? 'kala-studio.exe' : 'kala-studio');
  if (!exists(packaged)) {
    console.error(
      `[desktop-pack] packaged backend binary missing at ${packaged}. ` +
      'The delivery cell is expected to drop the binary here before packaging.',
    );
    process.exit(1);
    return;
  }

  let builderCli;
  try {
    builderCli = findBuilderCli();
  } catch (cause) {
    console.error('[desktop-pack]', cause.message);
    process.exit(1);
    return;
  }

  // Forward the caller's argv verbatim, but always force `--publish never`
  // so an accidental invocation cannot upload to a public registry.
  const forwarded = process.argv.slice(2);
  if (!forwarded.some((arg) => arg === '--publish' || arg.startsWith('--publish='))) {
    forwarded.push('--publish', 'never');
  }

  const args = [builderCli, '--config', 'electron-builder.json', ...forwarded];
  console.log('[desktop-pack] argv:', args.join(' '));

  const child = spawn(process.execPath, args, {
    cwd: studioRoot,
    stdio: 'inherit',
    env: process.env,
    windowsHide: true,
  });
  child.on('error', (cause) => {
    console.error('[desktop-pack] electron-builder spawn failed:', cause);
    process.exit(1);
  });
  child.on('exit', (code, signal) => {
    if (signal) {
      console.error(`[desktop-pack] electron-builder exited via ${signal}`);
      process.exit(1);
    }
    process.exit(code ?? 1);
  });
}

main();