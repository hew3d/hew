// Docs-fleet clip: Dimensions and text chapter
// (site/src/content/learn/dimensions-and-text.md).
// Silent, caption-carried. Beats: build a box, dimension one edge, dimension
// a second edge on another axis, leader text on a face. ~43 s.
// Pacing: modeling glides at hand speed (250-450 ms); the camera zoom-in
// and dimension-tool glides that follow keep the same range.
//   node docs-dimensions-and-text.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-dimensions-and-text`;
const h = await startCapture({ out: OUT, headless: true });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };
// Dimension/leader creation never changes the "N objects" badge, so
// expectBadge alone can't catch a silent miss here. Their labels are also
// no help: annotations render as a rasterized canvas texture in the
// three.js scene (SceneRenderer.refreshAnnotations), not DOM text, so
// getByText can never see them — confirmed by hand, not guessed. The real
// DOM signal is the tool's own status-bar hint: a commit (success or a
// kernel-level failure) calls cancel() and the hint returns to idle; a
// degenerate-offset refusal leaves the gesture parked mid-stage instead.
const assertVisible = async (locator, label, beat) => {
  const ok = await locator.first().isVisible().catch(() => false);
  console.log('CHECK visible', label, ok, 'after', beat);
  if (!ok) {
    await page.screenshot({ path: `${OUT}/FAILED-${beat}.png` }).catch(() => {});
    throw new Error(`beat "${beat}": expected "${label}" to be visible`);
  }
};
const idleHint = page.getByText(/Click a point to dimension from/);
const assertDimensionCommitted = async beat => {
  await assertVisible(idleHint, 'Dimension tool back to idle (gesture committed)', beat);
  const toast = await page.getByText(/Couldn't create dimension|Drag away from the line/).first()
    .isVisible().catch(() => false);
  console.log('CHECK no refusal toast after', beat, '->', !toast);
  if (toast) {
    await page.screenshot({ path: `${OUT}/FAILED-${beat}.png` }).catch(() => {});
    throw new Error(`beat "${beat}": dimension was refused (toast visible)`);
  }
};

await h.showMark();
await h.caption('Dimensions and text annotate a model without becoming part of it.', 600);
await page.waitForTimeout(1000);
h.mark('scene-start');

// beat 0 — build the box: 10x10 rect, pull 5
await h.caption('Draw a box, ten by six, and pull it up five.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(760, 620, 350);
await h.click();
await h.glide(960, 700, 450);
await h.typeSlow('10,6');
await page.keyboard.press('Enter');
await page.waitForTimeout(900);
await page.keyboard.press('p');
await page.waitForTimeout(250);
await h.glide(860, 660, 300);
await h.click();
await h.glide(860, 560, 400);
await h.typeSlow('5');
await page.keyboard.press('Enter');
h.mark('box');
await h.expectBadge('1 object', 'box');
await page.waitForTimeout(1300); await shot(0);

// Zoom in on the box so its vertices are large on screen — sub-10px pixel
// targeting on the un-zoomed box was landing just off the true Endpoint
// (measured lengths of 8-15cm instead of 10). Zoom is centered under the
// cursor, so park it over the box first.
await h.glide(950, 620, 300);
for (let i = 0; i < 5; i++) { await page.mouse.wheel(0, -200); await page.waitForTimeout(150); }
await page.waitForTimeout(500); await shot('zoomed');

// The snap chip is real DOM (InferenceTooltip), so every dimension pick is
// ASSERTED to be an Endpoint snap before the click — the tool commits the
// snapped vertex, not the raw cursor, so glide targets only need to land
// within the snap radius. Un-snapped picks were the "10.01 cm" slop.
const assertSnap = async (label, beat) => {
  const ok = await page.getByText(label, { exact: true }).first().isVisible().catch(() => false);
  console.log('CHECK snap', label, ok, 'before', beat);
  if (!ok) {
    await page.screenshot({ path: `${OUT}/FAILED-${beat}.png` }).catch(() => {});
    throw new Error(`beat "${beat}": expected ${label} snap before clicking`);
  }
};

// The two top edges (10 cm and 6 cm), both laid flat in the BLUE plane —
// the arrow keys name the locked plane's NORMAL axis (DimensionTool.ts:
// ArrowUp = blue-normal = the flat plane), so ↑ pins both dimensions
// horizontal like a drawing sheet, and the lock persists across gestures
// (the status bar names it) so the second dimension inherits it.
// Probed exact vertex snaps at this zoom: left (575,401), near (919,608),
// right (1191,484). Clicks need only land inside the snap radius — the
// tool commits the SNAPPED vertex, so the readings are exactly 10/6 cm.

// beat 1 — the 10 cm front-left edge, endpoint to endpoint, placed
// outward (down-left, away from the face).
await h.caption('Dimension: click endpoint to endpoint — ↑ lays the line flat.');
await h.clickRail('Dimension');
await h.glide(575, 401, 350);
await assertSnap('Endpoint', 'dim1-p1');
await h.click();
await h.glide(919, 608, 400);
await assertSnap('Endpoint', 'dim1-p2');
await h.click();
await page.keyboard.press('ArrowUp');
await page.waitForTimeout(400);
await h.glide(620, 545, 450);
await h.click();
h.mark('dim1');
await assertDimensionCommitted('dim1');
await page.waitForTimeout(1300); await shot(1);

// beat 2 — the 6 cm front-right edge; the blue-plane lock is still on,
// so this one lies flat beside the box too.
await h.caption('The dimension’s plane is anchored to the model, not the camera.');
await h.clickRail('Dimension');
await h.glide(919, 608, 350);
await assertSnap('Endpoint', 'dim2-p1');
await h.click();
await h.glide(1191, 484, 400);
await assertSnap('Endpoint', 'dim2-p2');
await h.click();
await h.glide(1215, 620, 450);
await h.click();
h.mark('dim2');
await assertDimensionCommitted('dim2');
await page.waitForTimeout(1300); await shot(2);

// beat 3 — leader text on the top face (Text is menu/palette-only, no rail
// slot — Tools menu ▸ Text).
await h.caption('Text: click the target, drag out, click again, and type.');
await page.getByTestId('menu-bar').getByRole('button', { name: 'Tools' }).click();
await page.waitForTimeout(300);
await page.getByTestId('menu-bar').getByText('Text', { exact: true }).click();
await page.waitForTimeout(300);
await h.glide(1000, 420, 350);
await h.click();
await h.glide(1140, 330, 400);
await h.click();
await page.waitForTimeout(300);
// The in-viewport AnnotationEditor is a real DOM <input> — check the typed
// value there before Enter hands it off to the canvas-rendered annotation
// (same "committed annotations aren't DOM text" reason as the dimensions
// above: nothing to query for "Top face" once it's on the canvas).
const editor = page.getByTestId('annotation-editor');
await assertVisible(editor, 'annotation text editor', 'leader-editor');
await editor.pressSequentially('Top face', { delay: 120 });
const editorValue = await editor.inputValue();
console.log('CHECK annotation editor value', JSON.stringify(editorValue));
if (editorValue !== 'Top face') {
  await page.screenshot({ path: `${OUT}/FAILED-leader-editor.png` }).catch(() => {});
  throw new Error(`beat "leader-editor": expected editor value "Top face", got "${editorValue}"`);
}
await page.keyboard.press('Enter');
h.mark('leader');
await page.waitForTimeout(1300); await shot(3);
await h.expectBadge('1 object', 'leader');

// close — orbit the result, then Camera ▸ Zoom Extents guarantees a
// centered final frame (copied from docs-push-pull.mjs's approved close).
await h.caption('Annotations follow the geometry they measure.', 300);
await page.keyboard.press('Escape');
await page.keyboard.press(' ');
await page.waitForTimeout(200);
await h.glide(950, 480, 400);
await h.orbit(220, 90, 1600);
await page.waitForTimeout(300);
const menuTarget = async loc => {
  const b = await loc.boundingBox();
  await h.glide(b.x + b.width / 2, b.y + b.height / 2, 450);
  await h.click();
  await page.waitForTimeout(250);
};
await menuTarget(page.getByTestId('menu-bar').getByRole('button', { name: 'Camera' }));
await menuTarget(page.getByTestId('menu-bar').getByText('Zoom Extents', { exact: true }));
// Zoom Extents frames the geometry only — the screen-space leader label
// ("Top face") sits above it; park the cursor at viewport center (wheel
// zoom is cursor-centered) and back out enough to bring the label in
await h.glide(905, 540, 350);
await page.mouse.wheel(0, 300);
await page.waitForTimeout(400);
await h.glide(95, 800, 400);
await page.waitForTimeout(500);
await page.waitForTimeout(600);
await h.caption('hew3d.com/learn/dimensions-and-text', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
