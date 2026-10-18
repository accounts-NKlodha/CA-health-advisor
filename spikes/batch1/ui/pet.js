// 128 px pet animated at 12 frames a second. Clicking makes it jump; the
// time from click to the first painted reaction frame is logged (P4).
const invoke = (kind, data) => window.__TAURI__.core.invoke('report', { kind, data }).catch(() => {});
const canvas = document.getElementById('pet');
const ctx = canvas.getContext('2d');
const FPS = 12;
let dpr = 0, tick = 0, jumpUntil = 0, pendingClick = null, firstPaint = true;
let frames = 0, maxGap = 0, drawMsSum = 0, lastTick = performance.now();

function resize() {
  dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(128 * dpr);
  canvas.height = Math.round(128 * dpr);
  invoke('pet_scale', { devicePixelRatio: dpr });
}

function draw() {
  const t0 = performance.now();
  maxGap = Math.max(maxGap, t0 - lastTick);
  lastTick = t0;
  if (canvas.width !== Math.round(128 * (window.devicePixelRatio || 1))) resize();
  const s = canvas.width / 128;
  ctx.setTransform(s, 0, 0, s, 0, 0);
  ctx.clearRect(0, 0, 128, 128);
  const jumping = tick < jumpUntil;
  const bob = jumping ? -18 * Math.sin(Math.PI * (jumpUntil - tick) / 6) : 3 * Math.sin(tick / 2);
  ctx.fillStyle = jumping ? '#f2a541' : '#5cb884';
  ctx.beginPath();
  ctx.ellipse(64, 76 + bob, 46, 40, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = '#1e283c';
  const blink = tick % 36 === 0 ? 1 : 6;
  ctx.beginPath(); ctx.ellipse(48, 68 + bob, 5, blink, 0, 0, Math.PI * 2); ctx.fill();
  ctx.beginPath(); ctx.ellipse(80, 68 + bob, 5, blink, 0, 0, Math.PI * 2); ctx.fill();
  tick++;
  frames++;
  drawMsSum += performance.now() - t0;
  if (pendingClick !== null && jumping) {
    const clickAt = pendingClick;
    pendingClick = null;
    requestAnimationFrame(() => invoke('click', { latency_ms: Math.round(performance.now() - clickAt) }));
  }
  if (firstPaint) {
    firstPaint = false;
    requestAnimationFrame(() => invoke('pet_visible', { since_navigation_ms: Math.round(performance.now()) }));
  }
}

canvas.addEventListener('pointerdown', (e) => {
  pendingClick = e.timeStamp || performance.now();
  jumpUntil = tick + 6;
});

setInterval(() => {
  invoke('pet_frames', { frames, expected: FPS * 5, max_gap_ms: Math.round(maxGap), mean_draw_ms: frames ? +(drawMsSum / frames).toFixed(2) : 0 });
  frames = 0; maxGap = 0; drawMsSum = 0;
}, 5000);

resize();
setInterval(draw, 1000 / FPS);
