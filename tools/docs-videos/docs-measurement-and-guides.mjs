// Docs-fleet clip: Precision, measurement, and guides chapter
// (site/src/content/learn/measurement-and-guides.md).
// Silent, caption-carried. Beats: typed rect size, two parallel guides off
// two edges, a line snapped to their amber crossing, a point-to-point
// measurement readout, then an edge measured and re-typed to rescale the
// model. Top view throughout, so edges and guides read as clean
// horizontals/verticals and the amber crossing is unambiguous.
//   node docs-measurement-and-guides.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-measurement-and-guides`;
const h = await startCapture({ out: OUT, headless: true });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

// This chapter never runs push/pull, so no solid ever exists and the
// bottom-right "N objects ✓ solid" badge never renders (confirmed empty on
// every beat) — h.expectBadge has nothing to read. These fail-fast checks
// stand in for it: same contract (throw + screenshot on a silent miss
// immediately after the mutating action), aimed at the state this chapter
// actually produces (the outliner, the top-right VCB reading, the amber
// Intersection cue).
const assertText = async (locator, mustInclude, beat) => {
  const t = await locator.innerText().catch(() => '(missing)');
  console.log('CHECK', JSON.stringify(t), 'includes', JSON.stringify(mustInclude), 'after', beat);
  if (!t.includes(mustInclude)) {
    await page.screenshot({ path: `${OUT}/FAILED-${beat}.png` }).catch(() => {});
    throw new Error(`beat "${beat}": expected text containing "${mustInclude}", got "${t}"`);
  }
};
const assertVisible = async (locator, label, beat) => {
  const ok = await locator.isVisible().catch(() => false);
  console.log('CHECK visible', label, ok, 'after', beat);
  if (!ok) {
    await page.screenshot({ path: `${OUT}/FAILED-${beat}.png` }).catch(() => {});
    throw new Error(`beat "${beat}": expected "${label}" to be visible`);
  }
};
const vcbValue = page.locator('span:text-is("Distance") + span');

await h.showMark();
await h.caption('Precision, measurement, and guides.', 600);
await page.waitForTimeout(1000);
h.mark('scene-start');

// Top view: the rectangle, its edges, and every guide read as clean
// horizontals/verticals — no iso foreshortening to fight.
await page.getByRole('button', { name: 'Top', exact: true }).click();
await page.waitForTimeout(500);

// beat 1 — rectangle with typed exact size
// Corners land at TL(699,598) TR(1051,598) BL(699,834) BR(1051,834).
await h.caption('Type an exact size into any tool, in any display unit.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(699, 598, 350);
await h.click();
await h.glide(950, 750, 400);
await h.typeSlow('15,10');
await page.keyboard.press('Enter');
h.mark('rect');
await assertText(page.getByText('Sketch 1', { exact: true }), 'Sketch 1', 'rect');
await page.waitForTimeout(1300); await shot(1);

// beat 2 — tape measure: parallel guide off the bottom edge, pulled inward
await h.caption('Tape Measure: click an edge, pull sideways — a parallel guide.');
await page.keyboard.press('t');
await page.waitForTimeout(250);
await h.glide(875, 834, 350);
await h.click();
await h.glide(875, 784, 450);
await h.typeSlow('2');
await page.keyboard.press('Enter');
h.mark('guide1');
await assertText(vcbValue, '2', 'guide1');
await page.waitForTimeout(900); await shot(2);

// second guide off the right edge, pulled outward
await h.caption('A world axis or another guide works as the source edge too.');
await h.glide(1051, 716, 350);
await h.click();
await h.glide(1101, 716, 450);
await h.typeSlow('2');
await page.keyboard.press('Enter');
h.mark('guide2');
await assertText(vcbValue, '2', 'guide2');
await page.waitForTimeout(1300); await shot(3);

// beat 3 — draw a line snapped to the amber guide-crossing
await h.caption('Guides cross at an amber cue — click there to land exactly.');
await page.keyboard.press('l');
await page.waitForTimeout(250);
await h.glide(1051, 834, 350);
await h.click();
await h.glide(1098, 787, 400);
await assertVisible(page.getByText('Intersection', { exact: true }), 'Intersection cue', 'line-snap');
await h.click();
await page.keyboard.press('Escape');
h.mark('line');
await page.waitForTimeout(1300); await shot(4);

// beat 4 — tape measure a distance point-to-point (readout, no guide —
// the second click lands on real geometry)
await h.caption('Measure endpoint to endpoint and read the live distance.');
await page.keyboard.press('t');
await page.waitForTimeout(250);
await h.glide(699, 598, 350);
await h.click();
await h.glide(1051, 834, 450);
await page.waitForTimeout(500); await shot(5);
await h.click();
h.mark('measure');
await assertText(vcbValue, '18.03', 'measure');
await page.waitForTimeout(1300); await shot(6);
await page.keyboard.press('Escape');
await page.waitForTimeout(300);

// beat 5 — measure an edge, then type a new length after the fact to
// rescale the model (the confirm dialog this produces)
await h.caption('Type a new length after the fact and Hew offers to resize.');
await page.keyboard.press('t');
await page.waitForTimeout(250);
await h.glide(699, 598, 350);
await h.click();
await h.glide(1051, 598, 450);
await h.click();
await page.waitForTimeout(600); await shot(7);
await h.typeSlow('20');
await page.keyboard.press('Enter');
await page.waitForTimeout(900); await shot(8);
const resizeDialog = page.getByRole('dialog', { name: 'Resize the model' });
if (await resizeDialog.isVisible().catch(() => false)) {
  await assertText(resizeDialog, '20 cm', 'resize-dialog');
  await page.waitForTimeout(900);
  await shot(9);
  await page.getByRole('button', { name: 'Resize' }).click();
  h.mark('resize');
  const stillOpen = await resizeDialog.isVisible().catch(() => false);
  console.log('CHECK dialog closed after Resize click:', !stillOpen);
  if (stillOpen) {
    await page.screenshot({ path: `${OUT}/FAILED-resize-confirmed.png` }).catch(() => {});
    throw new Error('beat "resize-confirmed": dialog still open after clicking Resize');
  }
  await page.waitForTimeout(900); await shot(10);
} else {
  console.log('RESIZE DIALOG DID NOT APPEAR — dropping beat 5 content');
}
await page.keyboard.press('Escape');
await page.waitForTimeout(300);

// cleanup — Edit > Delete Guide Lines
await h.caption('Edit ▸ Delete Guide Lines clears every guide in one step.');
await page.getByRole('button', { name: 'Edit' }).click();
await page.waitForTimeout(300); await shot(11);
await page.getByTestId('menu-bar').getByText('Delete Guide Lines').click();
h.mark('cleanup');
await page.waitForTimeout(1300); await shot(12);

await h.caption('hew3d.com/learn/measurement-and-guides', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
