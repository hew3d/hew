// Shot: the guest house, the largest model in the set — whole-house framing
// imported from SketchUp. Opens high on the hip roof's rafters, tilts down
// to eye level in front of the patio, then laps the entire house once in a
// single constant-rate drag.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('hero-framing') });
const { page } = h;
const log = async label => console.log('CAM', label, JSON.stringify(await h.camera()));
await h.loadHew(`${process.env.HOME}/Documents/Hew/Guest House - 2021-07-10.hew`, 9000);
await h.clean();
await h.zoomExtents();
await h.glide(980, 470, 200);
await h.zoom(-5, 120, 60);
await h.wait(400);
await h.shot('high');
await log('high');
await h.glide(960, 500, 200);
h.mark('start');
await h.wait(500);
// down to eye level: the framed view sits at a polar angle of ~60°, and
// the orbit turns 1/3° per px, so 90 px of upward drag lands at ~90°
// (horizontal). The orbit target is the house's center, ~2.6 m up; a
// pan then drops it so the camera itself stands at a person's eye height
// while still looking level.
await h.orbit(0, -90, 2400);
await h.wait(1500);
await log('tilted');
await h.glide(960, 500, 200);
await h.pan(0, -70, 900);
await h.wait(1200);
// push in (the wheel zooms toward the cursor, which sits on the house)
await h.glide(960, 470, 200);
await h.zoom(-3, 120, 120);
await h.wait(600);
await log('eye-level');
h.mark('eye-level');
await h.shot('eye-level');
// one lap: 1080 px of drag is exactly 360°, in one linear drag from the
// left of the frame to the right. The cursor is hidden and the drag runs
// above the roofline so no snap dot rides on the model during the lap.
await h.cursor(false);
await h.glide(420, 120, 300);
h.mark('lap');
const t0 = Date.now();
const samples = [];
const sampler = setInterval(async () => { const c = await h.camera().catch(() => null); if (c) samples.push([((Date.now() - t0) / 1000).toFixed(1), c.azDeg]); }, 500);
await h.orbit(1080, 0, 6500, { linear: true });
await h.wait(1800);
clearInterval(sampler);
console.log('LAP azimuth by 0.5 s:', samples.map(s => `${s[0]}s:${s[1]}`).join(' '));
await log('lap-end');
h.mark('lap-end');
await h.wait(300);
await h.shot('lap-end');
await h.finish();
