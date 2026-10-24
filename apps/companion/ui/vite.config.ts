import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { defineConfig, type Plugin } from 'vite';

// Seven pages (architecture 4.1 and 7): the always-present pet page and the on-demand
// keyboard panel in plain TypeScript, and the on-demand settings, review, recovery-day
// plan, setup and camera pages in Preact. JSX is compiled by esbuild using tsconfig's
// `jsxImportSource: "preact"`, so no Babel plugin is needed.
// Assets are local only; the Tauri CSP forbids remote and inline scripts.

/**
 * The posture check's WebAssembly runtime (V2-1), copied from the pinned
 * `@mediapipe/tasks-vision` package into `dist/mediapipe/wasm/` so the app never fetches
 * it. Only the SIMD build is copied: the bundled WebView2 runtime always has WebAssembly
 * SIMD (a runtime without it gets "Posture model could not load", nothing worse). The model
 * itself is committed under `public/mediapipe/` (see its README for its source and
 * licence).
 */
function mediapipeRuntime(): Plugin {
  const require = createRequire(import.meta.url);
  const files = [
    'vision_wasm_internal.js',
    'vision_wasm_internal.wasm',
  ];
  return {
    name: 'hy-mediapipe-runtime',
    apply: 'build',
    generateBundle() {
      for (const f of files) {
        this.emitFile({
          type: 'asset',
          fileName: `mediapipe/wasm/${f}`,
          // Each file is one of the package's exported subpaths.
          source: readFileSync(require.resolve(`@mediapipe/tasks-vision/${f}`)),
        });
      }
      this.emitFile({
        type: 'asset',
        fileName: 'mediapipe/wasm/LICENSE.txt',
        source:
          '@mediapipe/tasks-vision 1.1.1 (Apache-2.0), copyright Google LLC.\n' +
          'Licence text: mediapipe/LICENSE (Apache License, Version 2.0).\n',
      });
    },
  };
}

export default defineConfig({
  root: import.meta.dirname,
  base: './',
  clearScreen: false,
  plugins: [mediapipeRuntime()],
  // The camera page's pose worker is a classic worker (MediaPipe loads its runtime with
  // importScripts), inlined into the page so it inherits the page's content policy.
  worker: { format: 'iife' },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
    sourcemap: false,
    modulePreload: { polyfill: false },
    rollupOptions: {
      input: {
        pet: resolve(import.meta.dirname, 'pet.html'),
        panel: resolve(import.meta.dirname, 'panel.html'),
        screens: resolve(import.meta.dirname, 'screens.html'),
        review: resolve(import.meta.dirname, 'review.html'),
        plan: resolve(import.meta.dirname, 'plan.html'),
        setup: resolve(import.meta.dirname, 'setup.html'),
        camera: resolve(import.meta.dirname, 'camera.html'),
      },
    },
  },
});
