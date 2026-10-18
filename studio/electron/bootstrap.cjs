// Kala Studio — Electron browser-process entry point.
//
// `package.json#main` points here. This file exists only to start the
// app wiring exported by `main.cjs`.
//
// Why a separate entry instead of a guard at the bottom of `main.cjs`:
// the previous guard compared the app module against Node's process
// main entry, which is always false in Electron 44's browser process —
// there the process main entry is Electron itself, not the app's main
// script. That guard therefore prevented the backend child and the
// window from ever being created. Electron only loads this file when it
// is running us as the app entry, so by the time we run we are, by
// definition, the browser process and startup must proceed.
//
// The dev launcher (`scripts/desktop.cjs`) also points Electron at this
// file explicitly, so `electron .` and `npm run desktop` share one path.

'use strict';

const { bootstrap } = require('./main.cjs');

bootstrap().catch((cause) => {
  // `bootstrap` already logs and exits on its own startup failures; this
  // is a last-resort guard so an unexpected throw cannot leave Electron
  // hanging with no window and no diagnostics.
  console.error('[kala-studio] bootstrap failed:', cause);
  try {
    const { app } = require('electron');
    app.exit(1);
  } catch {
    // Electron is not importable; there is nothing more we can do.
  }
});
