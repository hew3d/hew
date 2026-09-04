// Docs-fleet clip: 3D Text chapter (site/src/content/learn/text-3d.md).
// Silent, caption-carried. Beats: open the dialog from the draw dock, type
// text + size, place it on the ground as one component instance, explode it
// to reveal the genuine watertight solids underneath (proven by the real
// object-count badge — a bare placement never shows one, since the result
// is a component with no top-level Objects), then push/pull a letter face
// like any other solid. ~58 s.
//   node docs-text-3d.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-text-3d`;
const h = await startCapture({ out: OUT, headless: true });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

// Glide the visible cursor to a plain <button> found by its exact trimmed
// text (dock verbs and the dialog's OK/Cancel are ordinary buttons, not the
// role=radio tool rail — capture-lib's clickRail is for the rail only).
async function clickButton(label) {
  const r = await page.evaluate(l => {
    const b = [...document.querySelectorAll('button')].find(x => (x.textContent || '').trim() === l);
    if (!b) return null;
    const q = b.getBoundingClientRect();
    return { x: q.x + q.width / 2, y: q.y + q.height / 2 };
  }, label);
  if (!r) throw new Error(`button not found: ${label}`);
  await h.glide(r.x, r.y, 400);
  await h.click();
}

await h.showMark();
await h.caption('3D Text turns typed characters into real, watertight solids.', 700);
await page.waitForTimeout(1000);
h.mark('scene-start');

// beat 1 — open the dialog from the draw dock (blank doc, nothing selected)
await h.caption('Open 3D Text from the draw dock.');
await clickButton('3D Text…');
await page.getByRole('dialog', { name: '3D Text' }).waitFor();
await page.waitForTimeout(1300);
await shot(1);

// beat 2 — type the word (mixed case: keeps a real counter in the "e"),
// keep the bundled font, set a compact sign-scale size
await h.caption('Type your text — pick a font, set Height and Extrusion depth.');
const textArea = page.locator('#text-3d-input');
await textArea.click();
await page.keyboard.press('Meta+a');
await page.keyboard.press('Backspace');
await h.typeSlow('Hew', 160);
await page.waitForTimeout(400);
const heightField = page.locator('#text-3d-height');
await heightField.click();
await page.keyboard.press('Meta+a');
await h.typeSlow('9', 120);
const depthField = page.locator('#text-3d-depth');
await depthField.click();
await page.keyboard.press('Meta+a');
await h.typeSlow('1.5', 120);
await page.waitForTimeout(500);
await shot(2);

await h.caption('Height is the type size — same measurement a word processor uses.');
await page.waitForTimeout(600);
await shot(3);

await h.caption('Click OK, then click where you want it.');
await clickButton('OK');
await page.waitForTimeout(1300);
await shot(4);

// beat 3 — place it on the ground plane
await h.glide(870, 560, 400);
await h.click();
await page.waitForTimeout(1300);
h.mark('placed');
// A placed 3D Text is ONE component instance, not a top-level Object — the
// aggregate watertight badge (which only ever counts top-level Objects)
// stays hidden after a bare placement, so a literal expectBadge('1 object')
// can never pass here. Fail fast a different way instead: the Outliner and
// Object Info must show the new "3D Text "Hew"" component, or the click
// missed the ground plane and nothing was placed.
{
  const label = await page.getByText('3D Text "Hew"').first().innerText().catch(() => '(missing)');
  console.log('PLACED_LABEL', JSON.stringify(label), 'after placed');
  if (label !== '3D Text "Hew"') {
    await page.screenshot({ path: `${OUT}/FAILED-placed.png` }).catch(() => {});
    throw new Error(`beat "placed": expected the Outliner/Object Info to show 3D Text "Hew", got "${label}"`);
  }
}
await shot(5);

// beat 4 — explode reveals the real, separate watertight solids underneath
// (docs/text-3d.md's "Booleans with text" section: this is the same
// explode a boolean runs automatically). It's already selected from
// placement, so the dock is showing the instance ("COMPNT") row.
await h.caption("It's one placed component — but real geometry underneath.");
await page.waitForTimeout(500);
await shot(6);
await h.caption('Explode reveals it: separate, watertight solids, one per letter.');
await clickButton('Explode');
h.mark('exploded');
await h.expectBadge('3 objects', 'exploded');
await page.waitForTimeout(1300);
await shot(7);

// beat 5 — push/pull a letter face, exactly like any other solid. Top view
// first: straight down on the letters' top faces is a huge, unambiguous
// click target (the iso angle forshortens them into thin, edge-crowded
// strips where a click reliably lands on a neighboring edge instead of the
// face — verified by probing every candidate point in each view).
await h.caption('Push/Pull still works on any face — the same geometry as anything else you model.');
await page.getByRole('button', { name: 'Top', exact: true }).click();
await page.waitForTimeout(500);
await page.keyboard.press('p');
await page.waitForTimeout(250);
await shot('pp-armed');
await h.glide(522, 550, 350);
await h.click();
await h.glide(522, 530, 400);
await h.typeSlow('4');
await page.keyboard.press('Enter');
h.mark('pushed');
await h.expectBadge('3 objects', 'pushed');
await page.waitForTimeout(600);
await shot(8);

// back to Iso to actually see the letter's new height before the close orbit
await h.iso();
await h.caption('Same watertight rule as everywhere else in Hew — no exceptions for text.');
await page.waitForTimeout(1300);
await shot(9);

// close — Escape, glide to the model's screen center, moderate orbit, then
// Camera > Zoom Extents guarantees a centered final frame (copied from
// docs-push-pull.mjs — a bare orbit pivots from wherever the cursor sits
// and drifts the model off-frame).
await h.caption('Watertight from the start — exports to STL with no repair needed.', 300);
await page.keyboard.press('Escape');
await h.glide(900, 550, 400);
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
await h.caption('hew3d.com/learn/text-3d', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
