// Docs-fleet clip: Materials chapter (site/src/content/learn/materials.md).
// Silent, caption-carried. Beats: expand the collapsed Materials panel, pick
// up Paint, choose a swatch, repaint the Pen Cup, then the Tabletop. ~44 s.
//   node docs-materials.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-materials`;
const h = await startCapture({ out: OUT, headless: true, sample: 'Café Table' });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

// Materials panel header, right-hand tray (collapsed at load).
const MATERIALS_HEADER = { x: 1765, y: 680 };
const SWATCH_WALNUT = { x: 1649, y: 696 };
const SWATCH_TERRACOTTA = { x: 1649, y: 742 };
const PEN_CUP = { x: 915, y: 345 };
const TABLETOP = { x: 1050, y: 330 };

async function ctrlClick() {
  await page.keyboard.down('Control');
  await h.click();
  await page.keyboard.up('Control');
}

await h.showMark();
await h.caption('Materials — a palette of colors and textures, per document.', 700);
await page.waitForTimeout(900);
h.mark('scene-start');

// beat 1 — expand the collapsed Materials panel
await h.caption('Expand Materials in the right-hand tray.', 700);
await h.glide(MATERIALS_HEADER.x, MATERIALS_HEADER.y, 400);
await h.click();
await page.waitForTimeout(1300);
h.mark('panel-expanded');
await shot(1);

// beat 2 — pick up Paint
await h.caption('Press B, or click a swatch, to pick up Paint.', 700);
await page.keyboard.press('b');
await page.waitForTimeout(1300);
h.mark('paint-tool');
await shot(2);

// beat 3 — pick a swatch, then repaint the Pen Cup (whole-object base material)
await h.caption('Click a swatch to make it the current material.', 700);
await h.glide(SWATCH_WALNUT.x, SWATCH_WALNUT.y, 400);
await h.click();
await page.waitForTimeout(1100);
await shot(3);
await h.caption("Ctrl/Cmd-click sets the whole object's base material.", 800);
await h.glide(PEN_CUP.x, PEN_CUP.y, 400);
await ctrlClick();
h.mark('paint-cup');
await h.expectBadge('2 objects', 'paint-cup');
await page.waitForTimeout(1300);
await shot(4);

// beat 4 — a different swatch, then the Tabletop
await h.caption('Paint the Tabletop with a different swatch.', 700);
await h.glide(SWATCH_TERRACOTTA.x, SWATCH_TERRACOTTA.y, 400);
await h.click();
await page.waitForTimeout(800);
await h.glide(TABLETOP.x, TABLETOP.y, 400);
await ctrlClick();
h.mark('paint-tabletop');
await h.expectBadge('2 objects', 'paint-tabletop');
await page.waitForTimeout(1300);
await shot(5);

// close — Escape, glide to the model's screen center, moderate orbit, then
// Camera > Zoom Extents guarantees a centered final frame (copied from
// docs-push-pull.mjs — a bare orbit pivots from wherever the cursor sits
// and drifts the model off-frame).
await h.caption('Materials survive push/pull, splitting, and booleans.', 300);
await page.keyboard.press('Escape');
await h.glide(960, 450, 400);
await h.orbit(220, 90, 1400);
await page.waitForTimeout(300);
const menuTarget = async loc => {
  const b = await loc.boundingBox();
  await h.glide(b.x + b.width / 2, b.y + b.height / 2, 450);
  await h.click();
  await page.waitForTimeout(250);
};
await menuTarget(page.getByTestId('menu-bar').getByRole('button', { name: 'Camera' }));
await menuTarget(page.getByTestId('menu-bar').getByText('Zoom Extents', { exact: true }));
await page.waitForTimeout(900);
await page.waitForTimeout(600);
await h.caption('hew3d.com/learn/materials', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
