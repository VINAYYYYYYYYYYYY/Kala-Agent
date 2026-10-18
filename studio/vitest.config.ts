import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'node',
    include: ['test/**/*.test.ts', 'test/**/*.test.tsx'],
    globals: false,
    environmentMatchGlobs: [
      ['test/**/*.dom.test.{ts,tsx}', 'happy-dom'],
    ],
  },
});