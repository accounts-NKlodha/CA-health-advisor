# Posture check model (V2-1)

Bundled with the app so the camera page never fetches anything at run time
(architecture 4.5). Vite copies this folder to `dist/mediapipe/`; the WebAssembly runtime
is copied next to it, into `dist/mediapipe/wasm/`, from the pinned npm package by
`vite.config.ts`.

| File | What | Source | Licence |
| --- | --- | --- | --- |
| `pose_landmarker_lite.task` | MediaPipe Pose Landmarker, lite, float16, version 1 | `https://storage.googleapis.com/mediapipe-models/pose_landmarker/pose_landmarker_lite/float16/1/pose_landmarker_lite.task` (last modified 2023-04-27) | Apache-2.0 (MediaPipe models; see `LICENSE`) |
| `LICENSE` | Apache License, Version 2.0 | `https://www.apache.org/licenses/LICENSE-2.0.txt` | - |
| `wasm/*` (build output) | `@mediapipe/tasks-vision` 1.1.1 runtime, SIMD build only: `vision_wasm_internal.js` and `.wasm` (the bundled WebView2 always has WebAssembly SIMD) | npm, pinned in `package.json` and `package-lock.json` (integrity checked by `npm ci`) | Apache-2.0 (package metadata) |

`pose_landmarker_lite.task`: 5,777,746 bytes, SHA-256
`59929e1d1ee95287735ddd833b19cf4ac46d29bc7afddbbf6753c459690d574a`. The self-test loads
it on the Windows CI runner; to replace it, change the file, this table and the hash
together.

The model runs on the CPU in a Web Worker on this PC. Frames and landmarks never leave
the camera page. The library also contains a usage-metrics sender (to
`odml.pa.googleapis.com`); the camera page's content policy blocks it like any other
network request, and the self-test checks that the network is unreachable from the
worker. See `docs/V2_Camera.md`.
