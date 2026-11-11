import { defineConfig, type Plugin } from 'vitest/config';
import { fileURLToPath } from 'node:url';
import { createReadStream, statSync } from 'node:fs';
import { resolve, sep } from 'node:path';

// The app imports fixtures from ../fixtures, so the dev server may read the repository root.
const repoRoot = fileURLToPath(new URL('..', import.meta.url));

/**
 * Serves sample packs at /packs/ (dev and preview servers) from $PRAXIS_PACKS, else the
 * repository's packs/ folder: the browser engine fetches a pack's files over HTTP. A deployed
 * web build serves the same folder next to index.html.
 */
function packs(): Plugin {
  const root = resolve(process.env.PRAXIS_PACKS || resolve(repoRoot, 'packs'));
  const handler = (req: { url?: string }, res: import('node:http').ServerResponse, next: () => void) => {
    let rel: string;
    try {
      rel = decodeURIComponent((req.url ?? '').split('?')[0]).replace(/^\/+/, '');
    } catch {
      next();
      return;
    }
    const file = resolve(root, rel);
    if (!rel || !(file + sep).startsWith(root + sep) || file === root) {
      res.statusCode = 404;
      res.end();
      return;
    }
    let size: number;
    try {
      const st = statSync(file);
      if (!st.isFile()) throw new Error('not a file');
      size = st.size;
    } catch {
      res.statusCode = 404;
      res.end();
      return;
    }
    res.setHeader('Content-Type', file.endsWith('.json') ? 'application/json' : 'application/octet-stream');
    res.setHeader('Content-Length', String(size));
    createReadStream(file).pipe(res);
  };
  return {
    name: 'praxis-packs',
    configureServer(server) {
      server.middlewares.use('/packs', handler);
    },
    configurePreviewServer(server) {
      server.middlewares.use('/packs', handler);
    },
  };
}

export default defineConfig(({ mode }) => ({
  // The web build (`--mode web`) may be hosted under any path.
  base: mode === 'web' ? './' : '/',
  plugins: [packs()],
  server: { port: 5173, strictPort: true, fs: { allow: [repoRoot] } },
  preview: { port: mode === 'web' ? 4174 : 4173, strictPort: true },
  build: {
    target: 'es2022',
    sourcemap: true,
    // The web build goes under the git-ignored target/ folder, so it never mixes with the
    // desktop shell's dist/.
    outDir: mode === 'web' ? resolve(repoRoot, 'target', 'praxis-web') : 'dist',
    emptyOutDir: true,
  },
  // The engine worker and the AudioWorklet are ES modules.
  worker: { format: 'es' as const },
  test: {
    include: ['tests/unit/**/*.test.ts'],
    environment: 'node',
  },
}));
