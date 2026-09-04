// Docs-fleet clip: Core concepts chapter (site/src/content/learn/core-concepts.md).
// Silent, caption-carried. Beats: two boxes drawn flush, dragged apart, dragged
// back into overlap, then Union — demonstrating that touching/overlapping
// Objects never fuse until you explicitly combine them. ~42 s.
//   node docs-core-concepts.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-core-concepts`;
const h = await startCapture({ out: OUT, headless: true });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

// Object ▸ <item> — the menu bar trigger is a real button, but each item
// inside the dropdown is a plain div (see MenuBar.tsx), so it's picked by
// its text, not a role. Resync the cursor into the viewport afterward.
const objectMenu = async label => {
  await page.getByRole('button', { name: 'Object', exact: true }).click();
  await page.waitForTimeout(200);
  await page.getByText(label, { exact: true }).click();
  await page.waitForTimeout(200);
  await h.glide(950, 700, 250);
};

await h.showMark();
await h.caption('Core concepts — the rules everything else follows.', 700);
await page.waitForTimeout(1000);
h.mark('scene-start');

// beat 1a — box A
await h.caption('Draw a box and pull it into a solid.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(760, 610, 350);
await h.click();
await h.glide(940, 690, 400);
await h.typeSlow('8,8');
await page.keyboard.press('Enter');
await page.waitForTimeout(900);
await shot(0);
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(848, 650, 300);
await h.click();
await h.glide(848, 520, 400);
await h.typeSlow('6');
await page.keyboard.press('Enter');
h.mark('boxA');
await h.expectBadge('1 object', 'boxA');
await page.waitForTimeout(1300); await shot(1);

// beat 1b — box B, flush against A's visible right edge (endpoint snap on
// the base-right vertex, same 8x8 footprint, same 6 pull)
await h.caption('Draw a second box flush against the first.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(1048, 610, 350);
await h.click();
await h.glide(1125, 725, 400);
await h.typeSlow('8,8');
await page.keyboard.press('Enter');
await page.waitForTimeout(900);
await shot(2);
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(1051, 686, 300);
await h.click();
await h.glide(1051, 556, 400);
await h.typeSlow('6');
await page.keyboard.press('Enter');
h.mark('boxB');
await h.expectBadge('2 objects', 'boxB');
await page.waitForTimeout(1300); await shot(3);
await h.caption('Touching solids never weld — even edge to edge, they stay two.', 1600);
await shot(4);

// beat 2 — Move (M): drag B away, clean separation
await h.caption('Move (M): drag it away — nothing was ever fused.');
await page.keyboard.press('m');
await page.waitForTimeout(250);
await h.glide(1052, 760, 350);
await h.click();
await h.glide(1400, 850, 450);
await h.click();
h.mark('separated');
await h.expectBadge('2 objects', 'separated');
await page.waitForTimeout(1300); await shot(5);

// beat 3 — drag B back into A: overlap is fine too
await h.caption('Drag it back in — solids can overlap without merging.');
await page.keyboard.press('m');
await page.waitForTimeout(250);
await h.glide(1400, 850, 350);
await h.click();
await h.glide(976, 730, 450);
await h.click();
h.mark('overlap');
await h.expectBadge('2 objects', 'overlap');
await page.waitForTimeout(1300); await shot(6);

// beat 4 — select both, Object > Union
await h.caption('Select both, then Object ▸ Union — combining is always explicit.');
await page.keyboard.press(' ');
await page.waitForTimeout(250);
await h.glide(790, 570, 300);
await h.click();
await page.waitForTimeout(150);
await page.keyboard.down('Shift');
await h.glide(1100, 690, 300);
await h.click();
await page.keyboard.up('Shift');
await page.waitForTimeout(300); await shot(7);
await objectMenu('Union');
h.mark('union');
await h.expectBadge('1 object', 'union');
await page.waitForTimeout(1300); await shot(8);
await h.glide(400, 280, 250);
await h.click();
await page.waitForTimeout(300);

// close — orbit the result. Pivot from the model's screen center, not
// wherever the last click left the cursor — orbiting from the far edge
// swings the model off-frame (viewer feedback).
await h.caption('One watertight solid — exactly what you asked for, nothing more.', 400);
await page.keyboard.press('Escape');
await h.glide(950, 650, 400);
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
await h.caption('hew3d.com/learn/core-concepts', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
