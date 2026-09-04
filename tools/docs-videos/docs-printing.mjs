// Docs-fleet clip: Printing chapter (site/src/content/learn/printing.md).
// Silent, caption-carried, READ-ONLY (no geometry is touched) — a guided
// tour of the Print Layout dialog: Standard vs Scaled, the scale ladder's
// plain-language reading, a scale that tiles across lettered pages, the
// cut list, and Cancel (never Print). ~56 s.
// Pacing: cursor travel to dialog controls runs at hand speed (250-450 ms);
// every dialog-result dwell (scale/tiling/cut-list previews, dropdown
// clicks) is left as authored — that's the beat, not the travel to it.
//   node docs-printing.mjs   (DEBUG_SHOTS=1 for per-beat stills)
//
// Trap for the next person: the Scale control is a native <select>. Chromium
// headless does not paint the native option popup into page.screenshot(), so
// there is no on-camera "dropdown open" frame — clicking it visibly moves and
// clicks the cursor, and the value change itself (plus the preview's reading
// line / tile grid reacting) carries the beat. Also: pressing Escape while
// focus is inside that select closes the WHOLE Print Layout dialog, not just
// the (invisible) popup — confirmed by direct test. Never press Escape here
// except in the final, deliberate close beat.

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-printing`;
const h = await startCapture({ out: OUT, headless: true, sample: 'Café Table' });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

// fail fast on a silent miss, same spirit as h.expectBadge but for dialog
// state (this scene never changes the object count, so the solid badge
// can't tell us anything went wrong — these attribute checks can)
async function assertAttr(locator, attr, expected, beat) {
  const val = await locator.first().getAttribute(attr).catch(() => null);
  console.log('ASSERT', beat, attr, '=', JSON.stringify(val), 'expected', JSON.stringify(expected));
  if (val !== expected) {
    await page.screenshot({ path: `${OUT}/FAILED-${beat}.png` }).catch(() => {});
    throw new Error(`beat "${beat}": expected [${attr}]=${expected}, got ${val}`);
  }
}
async function assertChecked(locator, expected, beat) {
  const val = await locator.first().isChecked().catch(() => null);
  console.log('ASSERT', beat, 'checked =', val, 'expected', expected);
  if (val !== expected) {
    await page.screenshot({ path: `${OUT}/FAILED-${beat}.png` }).catch(() => {});
    throw new Error(`beat "${beat}": expected checked=${expected}, got ${val}`);
  }
}

const dialog = () => page.locator('[role="dialog"]');

await h.showMark();
await h.caption('Printing — on paper, at scale, or into a PDF.', 1400);
await page.waitForTimeout(1400);
h.mark('scene-start');
await h.expectBadge('2 objects', 'start');

// beat 1 — File > Print… opens Print Layout
await h.caption('File ▸ Print… opens Print Layout — preview left, controls right.', 900);
await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click();
await page.waitForTimeout(600); await shot(0);
await page.getByTestId('menu-bar').getByText('Print…', { exact: true }).click();
await page.waitForTimeout(1900);
h.mark('dialog-open');
await assertAttr(dialog(), 'aria-label', 'Print Layout', 'dialog-open');
await shot(1);

// beat 2 — Scaled mode
await h.caption('Scaled mode prints a parallel-projection drawing at a ratio you choose.', 1400);
await h.glide(295, 74, 300);
await h.click();
await page.waitForTimeout(1400);
h.mark('scaled-mode');
await assertAttr(
  page.getByTestId('print-mode').getByRole('button', { name: 'Scaled', exact: true }),
  'aria-pressed', 'true', 'scaled-mode');
await shot(2);

// beat 3 — the scale ladder, plain language
await h.caption('The Scale control is a ladder of preset ratios for your unit family.', 1200);
await h.glide(1717, 184, 350);
await h.click();
await page.waitForTimeout(900); await shot(3);   // DEBUG: native <select> popup (not rendered by Chromium headless)
await page.locator('#print-scale-select').selectOption({ label: '1:5 — 1 cm = 5 cm' });
await page.waitForTimeout(1300);
h.mark('scale-1-5');
await assertAttr(dialog(), 'data-scale', '1:5', 'scale-1-5');
await h.caption('Each row spells out the ratio in plain language: 1 cm on paper = 5 cm in the model.', 2400);
await shot(4);

// beat 4 — a scale too big for one page tiles automatically
await h.caption('Too big for one sheet? It tiles — lettered A1, A2, B1, B2, reading left to right.', 1200);
await h.glide(1717, 184, 350);
await h.click();
await page.waitForTimeout(400);
await page.locator('#print-scale-select').selectOption({ label: '1:2 — 1 cm = 2 cm' });
await page.waitForTimeout(2000);
h.mark('scale-tiled');
await assertAttr(dialog(), 'data-pages', '4', 'scale-tiled');
await shot(5);
await h.caption('Trim the dashed lines, overlap the marked bands, and glue up a full-size template.', 2600);
await shot(6);

// beat 5 — Cut list page
await h.caption('Cut list page appends a table of every part — Part, Qty, L, W, H.', 1200);
await h.glide(1645, 514, 350);
await h.click();
await page.waitForTimeout(1500);
h.mark('cut-list');
await assertChecked(
  page.locator('label.hwprint__check', { hasText: 'Cut list page' }).locator('input'),
  true, 'cut-list');
await shot(7);

// beat 6 — Cancel: never actually print
await h.caption('Escape or Cancel closes the dialog. Nothing prints until you choose to.', 1700);
await page.keyboard.press('Escape');
await page.waitForTimeout(1000);
h.mark('cancelled');
await dialog().waitFor({ state: 'detached', timeout: 3000 });
await h.expectBadge('2 objects', 'cancelled');
await shot(8);

// close — orbit the result, then Camera ▸ Zoom Extents guarantees a
// centered final frame (copied from docs-push-pull.mjs's approved close).
await page.keyboard.press('Escape');
await h.glide(905, 540, 400);
await h.orbit(260, 90, 2200);
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
await h.caption('hew3d.com/learn/printing', 500);
await page.waitForTimeout(3000);
h.mark('scene-end');
await h.finish(OUT);
