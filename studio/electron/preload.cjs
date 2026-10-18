// Kala Studio — preload script.
//
// Loaded in the trusted renderer context with `contextIsolation: true` and
// `sandbox: true`. Exposes exactly one narrow IPC surface: a synchronous
// (well, promise-returning) `getConnection()` that the renderer can call to
// obtain the per-launch base URL and bearer token. The main process checks
// the sender URL before honouring the request, so the renderer never has
// to trust arbitrary code paths.
//
// We do not expose any generic IPC channel, do not run code on the
// renderer's behalf, and do not read or write anything outside this typed
// bridge.

'use strict';

const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('kalaStudio', {
  async getConnection() {
    return ipcRenderer.invoke('kala-studio:get-connection');
  },
});