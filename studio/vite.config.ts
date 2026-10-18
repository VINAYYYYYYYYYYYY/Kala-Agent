import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Kala Studio Vite configuration.
//
// The desktop shell loads the bundled UI from the backend over a loopback HTTP
// origin (see `electron/main.cjs`). The backend serves `studio/dist` from the
// same origin that issued the bearer token, so no CORS, no cross-origin assets,
// and no public CDN are involved at runtime. We do not configure a separate
// dev-server port; the operator who wants live-reload runs `npm run dev` and
// opens the URL the launcher prints.

export default defineConfig({
  plugins: [react()],
  base: './',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
    sourcemap: true,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('node_modules/three')) return 'three';
          if (id.includes('node_modules/react') || id.includes('node_modules/react-dom') || id.includes('node_modules/scheduler')) return 'react';
          return undefined;
        },
      },
    },
  },
  server: {
    strictPort: true,
    host: '127.0.0.1',
    open: false,
  },
  preview: {
    strictPort: true,
    host: '127.0.0.1',
  },
});