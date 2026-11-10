import { defineConfig } from 'vitest/config';
import { fileURLToPath } from 'node:url';

// The app imports fixtures from ../fixtures, so the dev server may read the repository root.
const repoRoot = fileURLToPath(new URL('..', import.meta.url));

export default defineConfig({
  server: { port: 5173, strictPort: true, fs: { allow: [repoRoot] } },
  preview: { port: 4173, strictPort: true },
  build: { target: 'es2022', sourcemap: true },
  test: {
    include: ['tests/unit/**/*.test.ts'],
    environment: 'node',
  },
});
