// Docs-fleet clip: Move, Rotate, and Scale chapter
// (site/src/content/learn/moving-and-transforming.md).
// Silent, caption-carried. Beats: box + pull, Move (click-move-click),
// Alt-copy + array (3x), Rotate a copy with a typed angle, Scale a copy
// with a grip drag. ~50 s.
//   node docs-moving-and-transforming.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-moving-and-transforming`;
const h = await startCapture({ out: OUT, headless: true });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

await h.showMark();
await h.caption('Move, Rotate, and Scale — with full snapping and typed values.', 900);
await page.waitForTimeout(1200);
h.mark('scene-start');

// beat 1 — a small box to work with
await h.caption('Start with a simple box.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(760, 610, 350);
await h.click();
await h.glide(940, 690, 400);
await h.typeSlow('8,8');
await page.keyboard.press('Enter');
await page.waitForTimeout(900);
await page.keyboard.press('p');
await page.waitForTimeout(250);
await h.glide(848, 650, 350);
await h.click();
await h.glide(848, 520, 400);
await h.typeSlow('6');
await page.keyboard.press('Enter');
h.mark('box');
await h.expectBadge('1 object', 'box');
await page.waitForTimeout(1300); await shot(1);

// beat 2 — Move: click a base point, click a destination
await h.caption('Move (M): click a base point, click a destination.');
await page.keyboard.press('m');
await page.waitForTimeout(250);
await h.glide(760, 518, 350);
await h.click();
await h.glide(1180, 680, 450);
await h.click();
h.mark('moved');
await h.expectBadge('1 object', 'moved');
await page.waitForTimeout(1300); await shot(2);

// beat 3 — copy: tap Alt mid-move, then array with 3x
// (the object stays selected from the move above, so this second move's
// base/destination just need to define a near-vertical drag — the app's
// axis inference locks that to Z and stacks the copy on top)
await h.caption('Tap Alt to copy instead of move — it stays on.');
await page.keyboard.press('m');
await page.waitForTimeout(250);
await h.glide(1092, 600, 350);
await h.click();
await page.keyboard.press('Alt');
await page.waitForTimeout(350);
await h.glide(1092, 500, 450);
await h.click();
h.mark('copy');
await h.expectBadge('2 objects', 'copy');
await page.waitForTimeout(900); await shot(3);
await h.caption('Type 3x right after — three copies march out at that spacing.', 500);
await h.typeSlow('3x');
await page.keyboard.press('Enter');
h.mark('array');
await h.expectBadge('4 objects', 'array');
await page.waitForTimeout(1300); await shot(4);

// beat 4 — Rotate one copy with a typed angle
// clear the array's multi-selection first so Rotate's own first click
// (pivot) is the thing that selects a single copy — the topmost one,
// fully clear of the others.
await h.caption('Rotate (Q): pivot, reference point, then a typed angle.');
await page.keyboard.press(' ');
await page.waitForTimeout(250);
await h.glide(600, 850, 350);
await h.click();
await page.waitForTimeout(300);
await page.keyboard.press('q');
await page.waitForTimeout(250);
await h.glide(1339, 230, 350);
await page.waitForTimeout(300);
await h.click();
await h.glide(1390, 260, 400);
await h.click();
await h.glide(1420, 240, 300);
await h.typeSlow('45');
await page.keyboard.press('Enter');
h.mark('rotated');
await h.expectBadge('4 objects', 'rotated');
await page.waitForTimeout(1300); await shot(5);

// beat 5 — Scale one copy: grab a corner grip, type an exact factor
// (the just-rotated copy is still selected; enter Scale directly.
// Grip coordinate measured directly off this scene's own beat-6 shot —
// the topmost corner grip of the rotated box's top face — since captures
// are deterministic this pixel stays valid run to run.)
await h.caption('Scale (S): grab a grip, then type an exact factor.');
await page.keyboard.press('s');
await page.waitForTimeout(250);
await h.glide(1415, 188, 400);
await h.click();
await page.waitForTimeout(350);
await h.typeSlow('1.4');
await page.keyboard.press('Enter');
h.mark('scaled');
await h.expectBadge('4 objects', 'scaled');
await page.waitForTimeout(1300); await shot(6);

// close — Escape, glide to the model's screen center, moderate orbit, then
// Camera > Zoom Extents guarantees a centered final frame (copied from
// docs-push-pull.mjs — a bare orbit pivots from wherever the cursor sits
// and drifts the model off-frame).
await h.caption('Every transform stays snapped, typed, and undoable.', 300);
await page.keyboard.press('Escape');
await h.glide(1050, 500, 400);
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
await h.caption('hew3d.com/learn/moving-and-transforming', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
