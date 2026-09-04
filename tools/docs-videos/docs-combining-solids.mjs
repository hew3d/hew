// Docs-fleet clip: Combining and splitting solids chapter
// (site/src/content/learn/combining-solids.md).
// Silent, caption-carried. Beats: build a box, build a cylinder overlapping
// one of its corners, then Union / Subtract / Intersect the same pair,
// undoing between each so every command starts from the same two solids.
// ~48 s.
//   node docs-combining-solids.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-combining-solids`;
const h = await startCapture({ out: OUT, headless: true });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

// Object ▸ <item> — the menu bar trigger is a real button, but each item
// inside the dropdown is a plain div (see MenuBar.tsx), so it's picked by
// its text, not a role.
// A raw Playwright locator .click() moves the real mouse but never touches
// capture-lib's internal `cur` tracker, so it goes stale here — h.orbit()'s
// middle-drag would then start from wherever `cur` last was (e.g. still on
// the model) while the real cursor sits over the menu bar / left panel.
// Resync with a glide back into the viewport before returning.
const objectMenu = async label => {
  await page.getByRole('button', { name: 'Object', exact: true }).click();
  await page.waitForTimeout(200);
  await page.getByText(label, { exact: true }).click();
  await page.waitForTimeout(200);
  await h.glide(950, 700, 250);
};

// Select the box, then Shift-click the cylinder — same pair every time.
const selectBoth = async () => {
  await page.keyboard.press(' ');
  await page.waitForTimeout(250);
  await h.glide(760, 640, 450);
  await h.click();
  await page.waitForTimeout(150);
  await page.keyboard.down('Shift');
  await h.glide(1155, 600, 450);
  await h.click();
  await page.keyboard.up('Shift');
  await page.waitForTimeout(250);
};

await h.showMark();
await h.caption('Objects never merge just by touching.', 600);
await page.waitForTimeout(700);
h.mark('scene-start');

// beat 1 — the box
await h.caption('Draw a box and pull it into a solid.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(700, 590, 450);
await h.click();
await h.glide(830, 660, 450);
await h.typeSlow('12,12', 90);
await page.keyboard.press('Enter');
await page.waitForTimeout(900);
await shot(1);
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(760, 620, 400);
await h.click();
await h.glide(760, 500, 450);
await h.typeSlow('8', 90);
await page.keyboard.press('Enter');
h.mark('box');
await h.expectBadge('1 object', 'box');
await page.waitForTimeout(1300); await shot(2);

// beat 2 — the cylinder, its footprint overlapping the box's near corner.
// Center is clicked on open ground just past the box's right corner (measured
// at ~1130,610 from the flat sketch) so the click lands on the ground plane,
// not the box's face — the radius alone carries it back into the corner.
await h.caption('Draw a cylinder so it overlaps one corner of the box.');
await page.keyboard.press('c');
await page.waitForTimeout(250);
await h.glide(1155, 613, 450);
await h.click();
await h.typeSlow('4', 90);
await page.keyboard.press('Enter');
await page.waitForTimeout(900);
await shot(3);
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(1155, 613, 400);
await h.click();
await h.glide(1155, 440, 450);
await h.typeSlow('12', 90);
await page.keyboard.press('Enter');
h.mark('two-solids');
await h.expectBadge('2 objects', 'two-solids');
await page.waitForTimeout(1300); await shot(4);

// beat 3 — select both, Union
await h.caption('Click the first, Shift-click the second.');
await selectBoth();
await shot(5);
await h.caption('Union produces one watertight solid — the seam dissolves.');
await objectMenu('Union');
h.mark('union');
await h.expectBadge('1 object', 'union');
await page.waitForTimeout(1300); await shot(6);

await page.keyboard.press('Meta+z');
await page.waitForTimeout(400);
await h.expectBadge('2 objects', 'undo-union');

// beat 4 — select both again, Subtract (the money shot — orbit it)
await selectBoth();
await h.caption('Subtract cuts the second object away from the first.');
await objectMenu('Subtract');
h.mark('subtract');
await h.expectBadge('1 object', 'subtract');
await page.waitForTimeout(1300); await shot(7);
await h.orbit(180, 70, 1200);
await page.waitForTimeout(600); await shot(8);

await page.keyboard.press('Meta+z');
await page.waitForTimeout(400);
await h.expectBadge('2 objects', 'undo-subtract');
await h.iso();

// beat 5 — select both again, Intersect
await selectBoth();
await h.caption('Intersect keeps only the volume the two share.');
await objectMenu('Intersect');
h.mark('intersect');
await h.expectBadge('1 object', 'intersect');
await page.waitForTimeout(1300); await shot(9);
// the shared volume is small — zoom toward it before the closing orbit so
// the lens shape actually reads on screen.
await h.glide(1020, 590, 300);
for (let i = 0; i < 4; i++) { await page.mouse.wheel(0, -130); await page.waitForTimeout(120); }
await page.waitForTimeout(300);
await h.orbit(200, 70, 1200);
await page.waitForTimeout(600); await shot(10);

// close — orbit the result. Pivot from the model's screen center, not
// wherever the last click left the cursor — orbiting from the far edge
// swings the model off-frame (viewer feedback).
await h.caption('Every result is watertight — booleans undo in one step.', 300);
await page.keyboard.press('Escape');
await h.glide(950, 600, 400);
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
await h.caption('hew3d.com/learn/combining-solids', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
