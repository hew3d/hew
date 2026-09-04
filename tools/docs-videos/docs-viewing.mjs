// Docs-fleet clip: Viewing your model chapter (site/src/content/learn/viewing.md).
// Silent, caption-carried, READ-ONLY (no geometry is touched). Beats: orbit,
// pan, zoom with the mouse, the Top/Front/Iso viewport chips, Camera > Zoom
// Extents. ~51 s.
//   node docs-viewing.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-viewing`;
const h = await startCapture({ out: OUT, headless: true, sample: 'Café Table' });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

// Read-only scene: we track the on-screen cursor ourselves (the harness's
// own position tracking is private to `h`) so the hand-rolled right-button
// pan below can start from wherever the last glide/orbit left off.
const ease = t => t < .5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
let pos = { x: 960, y: 540 }; // matches capture-lib's initial cursor position
const glide = async (x, y, ms) => { await h.glide(x, y, ms); pos = { x, y }; };
const orbit = async (dx, dy, ms) => { await h.orbit(dx, dy, ms); pos = { x: pos.x + dx, y: pos.y + dy }; };
// Right-button drag, eased the same way h.orbit eases its middle-button one.
const pan = async (dx, dy, ms = 1400) => {
  const x0 = pos.x, y0 = pos.y, x1 = pos.x + dx, y1 = pos.y + dy;
  await page.mouse.down({ button: 'right' });
  const steps = Math.max(12, Math.round(ms / 16));
  for (let i = 1; i <= steps; i++) {
    const t = ease(i / steps);
    await page.mouse.move(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
    await page.waitForTimeout(ms / steps);
  }
  await page.mouse.up({ button: 'right' });
  pos = { x: x1, y: y1 };
};
// Glide the cursor to a real <button> (viewport chips, menu triggers) by
// its accessible name, then click — visible travel, not a teleporting cursor.
const clickButton = async (name, exact = true) => {
  const box = await page.getByRole('button', { name, exact }).boundingBox();
  await glide(box.x + box.width / 2, box.y + box.height / 2, 380);
  await h.click();
};

await h.showMark();
await h.caption('Camera navigation never interrupts your work.', 700);
await page.waitForTimeout(900);
h.mark('scene-start');
await h.expectBadge('2 objects', 'loaded');

// beat 1 — orbit (middle-button drag)
await h.caption('Orbit — drag with the middle mouse button.');
await glide(905, 540, 700);
await orbit(230, -110, 2200);
h.mark('orbit');
await page.waitForTimeout(1300); await shot(1);

// beat 2 — pan (right-button drag)
await h.caption('Pan — drag with the right mouse button.');
await page.waitForTimeout(300);
await pan(-260, 90, 1800);
h.mark('pan');
await page.waitForTimeout(1300); await shot(2);

// beat 3 — zoom (scroll wheel, toward the cursor)
await h.caption('Zoom — the scroll wheel zooms toward the cursor.');
await glide(905, 560, 700);
await page.waitForTimeout(200);
for (let i = 0; i < 7; i++) {
  await page.mouse.wheel(0, -110);
  await page.waitForTimeout(220);
}
h.mark('zoom');
await page.waitForTimeout(1300); await shot(3);

// beat 4 — standard views: the viewport's top-left Top/Front/Iso chips
await h.caption("The viewport's top-left chips jump straight to Top, Front, or Iso.");
await clickButton('Top');
await page.waitForTimeout(1300); await shot(4);
await clickButton('Front');
await page.waitForTimeout(1300); await shot(5);
await clickButton('Iso');
h.mark('standard-views');
await page.waitForTimeout(1300); await shot(6);

// beat 5 — Camera > Zoom Extents
await h.caption('Camera ▸ Zoom Extents frames everything — the fastest way back.');
await page.getByTestId('menu-bar').getByRole('button', { name: 'Camera' }).click();
await page.waitForTimeout(300); await shot(7);
await page.getByTestId('menu-bar').getByText('Zoom Extents', { exact: true }).click();
h.mark('zoom-extents');
await page.waitForTimeout(1300); await shot(8);

await h.caption('hew3d.com/learn/viewing', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
