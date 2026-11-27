// Docs-fleet clip: "Drawing on other planes" section of the Drawing chapter
// (site/src/content/learn/drawing.md, slug drawing-planes). Silent,
// caption-carried. Beats: a box to work against; Circle drawn on its face;
// Rectangle with a pip click locking the red plane (padlock in the pip),
// then pulled into a wall; Shift pinning a face's plane; Line with ↑ lifting
// a segment off the ground and closing a vertical face, then pulled. The view
// die and its four pips (top-right of the viewport) stay in frame throughout.
//   node docs-drawing-planes.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-drawing-planes`;
const h = await startCapture({ out: OUT, headless: true });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

await h.showMark();
await h.caption('Drawing on other planes — a face, a locked plane, or a line into space.', 900);
await page.waitForTimeout(1200);
h.mark('scene-start');

// beat 1 — a box to work against (same footprint as the push/pull scene)
await h.caption('Over the ground the blue pip is filled: that is where your next click draws.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(700, 600, 350);
await page.waitForTimeout(900); await shot('1a');
await h.click();
await h.glide(950, 690, 400);
await h.typeSlow('12,8');
await page.keyboard.press('Enter');
await page.waitForTimeout(1000);
await h.caption('Push/Pull turns the rectangle into a solid box.', 0);
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(790, 640, 300);
await h.click();
await h.glide(790, 520, 400);
await h.typeSlow('6');
await page.keyboard.press('Enter');
h.mark('box');
await h.expectBadge('1 object', 'box');
await page.waitForTimeout(1300); await shot(1);

// beat 2 — Circle drawn on the box's front face (the shape cuts the face)
await h.caption('Point a tool at a face: its pip lights and the shape is drawn on that face.');
await page.keyboard.press('c');
await page.waitForTimeout(250);
await h.glide(800, 610, 400);
await page.waitForTimeout(1100); await shot('2a');
await h.click();
await h.typeSlow('1.5');
await page.keyboard.press('Enter');
h.mark('face-circle');
await h.expectBadge('1 object', 'face-circle');
await page.waitForTimeout(1300); await shot(2);

// beat 3 — Rectangle: click the red pip to lock that plane (the mouse form
// of →), then stand a rectangle up on it through the first click
await h.caption('Click a pip, or press → ↑ ←, to lock that plane — the pip holds a padlock.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(1150, 640, 400);
await h.diePip('red');
await page.waitForTimeout(1000); await shot('3a');
await h.glide(1150, 640, 400);
await h.click();
await h.glide(1220, 560, 400);
await h.typeSlow('8,5');
await page.keyboard.press('Enter');
await page.waitForTimeout(1000); await shot('3b');
await h.diePip('red');                          // click the held pip again: release
await page.waitForTimeout(600); await shot(3);
await h.caption('An ordinary sketch region: Push/Pull extrudes it sideways into a wall.');
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(1185, 600, 300);
await h.click();
await h.glide(1120, 640, 400);
await h.typeSlow('3');
await page.keyboard.press('Enter');
h.mark('locked-rect');
await h.expectBadge('2 objects', 'locked-rect');
await page.waitForTimeout(1300); await shot(4);

// beat 3b — Shift pins the plane under the cursor; the pip holds it
await h.caption('Shift pins the plane under the cursor; press it again to release.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(740, 550, 400);
await page.waitForTimeout(700);
await page.keyboard.press('Shift');
await page.waitForTimeout(500);
await h.glide(500, 760, 450);                   // off the face, over the ground
await page.waitForTimeout(1100); await shot('3c');
await page.keyboard.press('Shift');
await page.waitForTimeout(900); await shot('3d');
await page.keyboard.press('Escape');
await page.waitForTimeout(300);

// beat 4 — Line into space: a ground segment first (it stays in the
// ground sketch), then ↑ lifts the chain onto a fresh vertical sketch whose
// own start point B closes the face; the return leg stops a centimetre
// short of A so the chain never revisits the ground sketch's vertex.
await h.caption('Line: ↑ lifts the next segment off the ground; the pip shows the new plane.');
await page.keyboard.press('l');
await page.waitForTimeout(250);
await h.glide(560, 700, 400);
await h.click();                                // A — on the ground
await page.keyboard.press('ArrowRight');
await page.waitForTimeout(400);
await h.glide(440, 630, 400);                   // along red, away from the box
await h.typeSlow('5');
await page.keyboard.press('Enter');             // B — 5 cm along red
await page.waitForTimeout(1000);
await page.keyboard.press('ArrowUp');
await page.waitForTimeout(400);
await h.glide(464, 500, 400);
await h.typeSlow('5');
await page.keyboard.press('Enter');             // C — 5 cm straight up
await page.waitForTimeout(1000);
await page.keyboard.press('ArrowRight');
await page.waitForTimeout(400);
await h.glide(600, 620, 400);                   // back along red, toward A
await h.typeSlow('4');
await page.keyboard.press('Enter');             // D — 4 cm back (short of A)
await page.waitForTimeout(1000);
await page.keyboard.press('ArrowUp');
await page.waitForTimeout(400);
await h.glide(548, 780, 400);                   // straight down
await h.typeSlow('5');
await page.keyboard.press('Enter');             // E — back on the ground
await page.waitForTimeout(1000);
await page.keyboard.press('ArrowDown');         // release the lock
await page.waitForTimeout(300);
await shot('5a');
await h.glide(497, 640, 450);                   // onto B — Endpoint closes the face
await page.waitForTimeout(300); await shot(5);
await h.click();
await page.waitForTimeout(1000);
await shot('5b');
await h.caption('A closed outline off the ground is a face like any other.');
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(512, 605, 300);
await h.click();
await h.glide(600, 560, 400);
await h.typeSlow('2');
await page.keyboard.press('Enter');
h.mark('line-face');
await h.expectBadge('3 objects', 'line-face');
await page.waitForTimeout(1300); await shot(6);

// close — a gentle orbit from the model's screen center, then Zoom
// Extents via the menu so the final frame is centered (the close from
// docs-push-pull.mjs; this scene is wide, so the orbit stays small)
await page.keyboard.press('Escape');
await h.glide(800, 600, 400);
await h.orbit(90, 20, 1200);
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
await h.caption('hew3d.com/learn/drawing', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
