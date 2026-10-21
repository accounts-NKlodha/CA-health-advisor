import { resolve } from 'node:path';
import { defineConfig } from 'vite';

// Two pages (architecture 4.1 and 7): the always-present pet page in plain TypeScript and
// the on-demand screens page in Preact. JSX is compiled by esbuild using tsconfig's
// `jsxImportSource: "preact"`, so no Babel plugin is needed.
// Assets are local only; the Tauri CSP forbids remote and inline scripts.
export default defineConfig({
  root: import.meta.dirname,
  base: './',
  clearScreen: false,
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
    sourcemap: false,
    modulePreload: { polyfill: false },
    rollupOptions: {
      input: {
        pet: resolve(import.meta.dirname, 'pet.html'),
        screens: resolve(import.meta.dirname, 'screens.html'),
      },
    },
  },
});
