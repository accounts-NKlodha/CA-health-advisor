# HealthierYou camera vision: vision LLM vs MediaPipe Pose Landmarker

20 Oct 2026 · Prepared by Claude for Rohit Lodha

## 1. Recommendation

Keep **MediaPipe Pose Landmarker lite, on the CPU, inside WebView2** as the only posture detector. It is already in the batch-1 build and it is the only option measured on your laptop within budget. A cloud vision LLM such as Claude should **not** be in the monitoring loop. At most it could be an optional, consented "review my desk setup" snapshot later. That is a scope and privacy change, so it is your call (see section 6).

| Criterion | MediaPipe pose lite (on device) | Vision LLM (Claude Haiku 5.5 / Sonnet 5.5, cloud) |
|---|---|---|
| Measured on your laptop | Yes: 4.1% CPU, 280 MB peak, 37 ms per frame | Not tested; would need internet and an API key |
| Fits camera budget (10% CPU, 350 MB) | Yes, about 6 points CPU and 70 MB headroom | Device load is small, but every frame is uploaded |
| Output | 33 numeric landmarks per frame, deterministic | Text judgement, varies between calls, no reliable angles |
| Latency | 37 ms per frame | About 1 to 3 s per frame (estimate, unverified) |
| Running cost | Nil | Haiku 5.5: about $0.00008 per 640x480 frame; Sonnet 5.5: about $0.0016 |
| Works offline | Yes | No |
| Privacy | Frames never leave the PC | Face images go to a cloud API; needs explicit consent |
| Licence | Apache-2.0 (code and model) | Commercial API terms |
| Best role | Continuous check-ins at 2 to 3 fps, fall back to 1 fps | Rare, user-triggered snapshot review or coaching text |

## 2. What batch 1 already measured

The batch-1 spike (build 701207e, run 4, 20 Oct, i5-1035G1, 7.8 GB, Windows 11) runs pose lite on the CPU delegate, set to 3 fps. From the run-4 logs:

| Measure | Value | Target |
|---|---|---|
| Frames achieved | 13 to 14 per 5 s, about 2.7 fps | 2 to 3 fps |
| Inference per frame | 37 ms mean, 44 ms worst | none set |
| Model load | 0.46 to 0.57 s | none set |
| Camera CPU, 10 min | 4.1% of machine | at most 10% |
| Camera memory | 280 MB peak private working set (369 MB private bytes) | at most 350 MB |
| Idle memory | 97.7 MB | at most 150 MB |

The two camera failures in run 4 (C3 light stayed on, C4 camera started despite Deny) come from the DirectShow capture path, not from MediaPipe. They would affect a vision LLM design equally, because it uses the same camera. Build 6ef6eb3 addresses them and is owned by the delivery thread.

The 4 GB PC remains unmeasured. 37 ms on this i5 suggests an older dual-core could take roughly 2 to 3 times longer per frame (estimate); the planned 1 fps fallback covers that.

## 3. Vision LLM (Claude with image input)

**What it can do.** Give a categorical judgement on a single image ("leaning towards the screen", "screen too low", "laptop on lap"), explain it in plain language, and comment on the desk setup (monitor height, lighting, chair) that landmarks cannot see.

**What it cannot do reliably.**
- Measure angles or distances. Anthropic's vision docs describe spatial reasoning and localisation as approximate and warn against uses that need precision.
- Stay consistent frame to frame. Answers vary, so thresholds would flicker and dwell or cooldown logic becomes unreliable.
- Get left and right right. ChatPose (arXiv 2311.18836) found GPT-4V confuses left and right body parts; Spatial457 (arXiv 2502.08636) found GPT-4o fails 3D orientation questions.
- No published benchmark of frontier models on webcam posture was found. That is a gap, not proof either way.

**Cost.** Image tokens are about (width/28) x (height/28): 414 for 640x480 and 108 for 320x240. With a 150-token prompt and 50-token JSON answer, at Haiku 5.5 $0.10/$0.50 and Sonnet 5.5 $2/$10 per million tokens (Anthropic pricing, checked 20 Oct). Month is 22 working days of 8 h.

| Model, frame | 1 frame every 5 min, per user per month | 1 frame per min, per user per month | 1 frame per s, per user per month | 1 per min, 30 staff per month |
|---|---|---|---|---|
| Haiku 5.5, 640x480 | $0.17 | $0.86 | $51.6 | $25.8 |
| Haiku 5.5, 320x240 | $0.11 | $0.54 | $32.2 | $16.1 |
| Sonnet 5.5, 640x480 | $3.44 | $17.2 | $1,032 | $516 |
| Sonnet 5.5, 320x240 | $2.15 | $10.7 | $644 | $322 |

Per-frame arithmetic, Haiku 5.5 at 640x480: 564 x $0.10/1M + 50 x $0.50/1M = $0.0000814 (about ₹0.007). Token counts use the docs formula and were not measured against the API.

Cost is only small if frames are rare. Continuous use at the 2 to 3 fps the posture logic needs is out of the question.

**Privacy and operations.**
- Every frame (a face image of an employee) leaves the PC for a cloud API. Today the app can truthfully say no camera data leaves the PC; that message would be lost.
- Anthropic's vision FAQ says images are not used for training. API data is kept up to 30 days by default; zero retention needs an agreement. Confirm before any pilot use.
- India DPDP Act 2023 and Rules 2025: the "purposes of employment" ground in s.7(i) is untested for camera images. The safe course is explicit opt-in consent per use, a single purpose (self-coaching), no sharing with managers, and no storage. Commentary suggests the main consent provisions take effect 13 May 2027 (unverified); design for them now.
- Needs internet, so an office outage switches the feature off. An API key cannot sit inside a desktop app; it needs a small proxy on the office server or a hosted service, which is not in the current plan or budget.

## 4. MediaPipe Pose Landmarker review

**Platforms.** Web (JavaScript plus WebAssembly, CPU or WebGL GPU delegate), Python, Android and iOS. The web build works offline: point FilesetResolver at bundled wasm files and modelAssetPath at the bundled .task file, as the batch-1 build already does. WebView2 is Chromium, so it works there, though Google lists only Chrome and Safari officially. Python is irrelevant because the app ships no Python runtime.

**Models.**

| Variant | Model card size | Pixel 3 CPU | Accuracy (PDJ at 0.2) |
|---|---|---|---|
| Lite (in use) | 3 MB (5.8 MB .task) | about 44 fps | 87.0 |
| Full | 6 MB (9.4 MB .task) | about 18 fps | 91.8 |
| Heavy | 26 MB | about 4 fps | 94.2 |

Full is about 2.4 times slower than lite on the model card's phone CPU, so on your laptop it would likely cost about 90 ms per frame (estimate, unmeasured). Heavy is unsuitable for office PCs.

**Footprint.** No published desktop WASM CPU or memory figures exist; batch 1 is the best evidence available. The npm package is 42 MB unpacked, but only the wasm folder and one .task file need shipping. Known issues: memory leaks have been reported with the segmentation mask on (keep it off) and in GPU mode; integrated-GPU drivers are a risk, so stay on the CPU delegate. Create the landmarker once, run it in a worker, and soak-test for hours, since batch 1 covered 10 minutes.

**Landmarks for a seated user at a frontal webcam.** 33 points (face, shoulders, elbows, wrists, hands, hips, legs) with visibility scores, plus hip-centred world coordinates in metres. At a desk the hips are usually hidden, so hip, leg and world coordinates are guesses and must be ignored or gated on visibility.

| Metric | Usable from a frontal webcam? | How |
|---|---|---|
| Presence and away from desk | Yes | Nose, eye and shoulder visibility |
| Shoulder elevation or asymmetry | Yes | Left versus right shoulder height, scaled by shoulder width |
| Head tilt (side to side) | Yes | Angle of the eye or ear line |
| Leaning towards the screen | Relative only | Shoulder width or eye spacing growing against the user's calibration |
| Slouch | Relative only | Ear-to-shoulder vertical gap shrinking against calibration |
| Forward head (neck angle) | Poor | Needs a side view; depth (z) is not metric. Face Landmarker head pose can add a pitch proxy |

These are drift-from-calibration signals, not clinical angles, which matches PRD 0.3's V2 quality gate. The model card says the model is not intended for life-critical decisions and is optimised for full-body, single-person, phone-camera images, so app wording should stay as wellness nudges.

**Licence and maintenance.** Code and model are Apache-2.0. Latest @mediapipe/tasks-vision is 1.1.1 (14 Oct 2026); batch 1 pins 1.1.1. The lite model shows the widest accuracy spread across regions on the model card (83.2 to 89.7), another reason to rely on per-user calibration.

## 5. Fit with the measured budget

| Option | CPU (laptop) | Memory | Within 10% and 350 MB? |
|---|---|---|---|
| Pose lite at 2.7 fps (measured) | 4.1% | 280 MB | Yes |
| Pose lite at 1 fps (fallback) | lower, estimate 2% | about the same | Yes |
| Pose full at 2 to 3 fps | estimate 7 to 9% | estimate +10 to 20 MB | Probably, unmeasured |
| Pose lite plus Face Landmarker at 1 fps | estimate 5 to 7% | estimate +20 to 40 MB | Probably, unmeasured |
| Vision LLM, 1 snapshot per 5 min | camera only, near idle | about idle plus camera | Yes on device; adds cloud cost, internet and consent |

Estimates are scaled from the run-4 inference time and model card ratios and need a measurement before use.

## 6. Next steps

1. No change to the delivery plan: V2 stays on pose lite (CPU, 2 to 3 fps, 1 fps fallback).
2. For slice V2-2, define slouch and leaning as drift from calibration (ear-to-shoulder gap, shoulder width, shoulder asymmetry, head roll) and ignore hip and leg points.
3. Add Face Landmarker only if the V2-2 desk test misses the forward-head target, and measure it at 1 fps first.
4. Your decision: whether a Claude "review my desk setup" snapshot should go in the backlog as an optional, consented feature. It needs a server proxy, consent text and budget, none of which is in the $200 plan.

## Sources

- MediaPipe Pose Landmarker: https://developers.google.com/edge/mediapipe/solutions/vision/pose_landmarker and web guide https://developers.google.com/edge/mediapipe/solutions/vision/pose_landmarker/web_js
- BlazePose GHUM 3D model card: https://storage.googleapis.com/mediapipe-assets/Model%20Card%20BlazePose%20GHUM%203D.pdf
- npm @mediapipe/tasks-vision: https://registry.npmjs.org/@mediapipe/tasks-vision
- MediaPipe issue #6216 (VIDEO mode memory): https://github.com/google-ai-edge/mediapipe/issues/6216
- Claude vision: https://platform.claude.com/docs/en/build-with-claude/vision ; pricing: https://platform.claude.com/docs/en/about-claude/pricing ; data retention: https://platform.claude.com/docs/en/manage-claude/api-and-data-retention
- ChatPose: https://arxiv.org/pdf/2311.18836 ; Spatial457: https://arxiv.org/pdf/2502.08636
- DPDP commentary: https://www.ey.com/content/dam/ey-unified-site/ey-com/en-in/pdf/2025/11/ey-india-dpdp-act-2023-and-rules-2025-pov.pdf ; https://ksandk.com/data-protection-and-data-privacy/can-employers-monitor-employees-dpdp-act/
- Batch-1 run 4: HealthierYou/batch1/Batch1_Results_Run4_Laptop.md and raw zip 0c7d792e
