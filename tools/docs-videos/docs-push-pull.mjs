// Docs-fleet clip: Push/Pull chapter (site/src/content/learn/push-pull.md).
// Silent, caption-carried. Beats: sketch→solid, reshape a face, recess,
// through-cut, cylinder radius. ~45 s.
// Pacing: glides at hand speed (~300-450 ms) — viewer feedback said the
// slow cursor reads as glacial; the typing and result-holds carry the
// legibility, not the mouse travel.
//   node docs-push-pull.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-push-pull`;
const h = await startCapture({ out: OUT, headless: true });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

await h.showMark();
await h.caption('Push/Pull — one gesture, three jobs.', 600);
await page.waitForTimeout(1000);
h.mark('scene-start');

// beat 1 — sketch → solid
await h.caption('Pull a closed sketch region into a new watertight solid.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(700, 600, 350);
await h.click();
await h.glide(950, 690, 450);
await h.typeSlow('20,14');
await page.keyboard.press('Enter');
await page.waitForTimeout(1000);
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(825, 645, 300);
await h.click();
await h.glide(825, 520, 400);
await h.typeSlow('6');
await page.keyboard.press('Enter');
h.mark('solid');
await h.expectBadge('1 object', 'solid');
await page.waitForTimeout(1300); await shot(1);

// beat 2 — reshape an existing face
await h.caption('Click a face of an existing solid to reshape it.');
await h.glide(825, 505, 350);
await h.click();
await h.glide(825, 440, 350);
await h.typeSlow('3');
await page.keyboard.press('Enter');
h.mark('reshaped');
await h.expectBadge('1 object', 'reshaped');
await page.waitForTimeout(1300); await shot(2);

// beat 3 — recess (draw on the face, push in)
await h.caption('Draw on a face and push inward — a recess.');
await page.keyboard.press('c');
await page.waitForTimeout(250);
await h.glide(950, 475, 350);
await h.click();
await h.typeSlow('1.5');
await page.keyboard.press('Enter');
await page.waitForTimeout(900);
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(950, 475, 250);
await h.click();
await h.typeSlow('-2');
await page.keyboard.press('Enter');
h.mark('recess');
await h.expectBadge('1 object', 'recess');
await page.waitForTimeout(1300); await shot(3);

// beat 4 — through-cut
await h.caption('Push all the way through and the material is removed.');
await page.keyboard.press('c');
await page.waitForTimeout(250);
await h.glide(1125, 430, 350);
await h.click();
await h.typeSlow('1.5');
await page.keyboard.press('Enter');
await page.waitForTimeout(900);
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(1125, 430, 250);
await h.click();
await h.typeSlow('-9');
await page.keyboard.press('Enter');
h.mark('through');
await h.expectBadge('1 object', 'through');
await page.waitForTimeout(1300); await shot(4);

// beat 5 — cylinder wall = radius change
await h.caption("A cylinder's wall is special — push/pull changes its radius.");
await page.keyboard.press('c');
await page.waitForTimeout(250);
await h.glide(1390, 800, 400);
await h.click();
await h.typeSlow('3');
await page.keyboard.press('Enter');
await page.waitForTimeout(900);
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(1390, 800, 250);
await h.click();
await h.glide(1390, 670, 350);
await h.typeSlow('8');
await page.keyboard.press('Enter');
await h.expectBadge('2 objects', 'cylinder');
await page.waitForTimeout(600);
// pull the wall — whole cylinder gets fatter
await h.glide(1462, 715, 350);
await h.click();
await h.glide(1552, 715, 450);
await h.click();
h.mark('radius');
await page.waitForTimeout(1300); await shot(5);

// close — orbit the result. Pivot from the model's screen center, not
// wherever the last click left the cursor — orbiting from the far edge
// swings the model off-frame (viewer feedback).
await h.caption('Every result is a watertight solid.', 300);
await page.keyboard.press('Escape');
await h.glide(1000, 620, 400);
await h.orbit(220, 90, 1400);
await page.waitForTimeout(300);
// any orbit drifts the model off-center; Camera > Zoom Extents guarantees
// a centered final frame (and echoes the viewing chapter's lesson)
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
await h.caption('hew3d.com/learn/push-pull', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
