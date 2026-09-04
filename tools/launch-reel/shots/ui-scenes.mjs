// Shot: the trestle table, in the UI. Its three saved scenes switched from
// the Scenes panel (each restores camera, visibility, and section cut with
// an animated transition); then, back in the assembled view, the Outliner
// opened into the model's parts and one selected, and a tag hidden and
// shown again from the Tags panel.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('ui-scenes'), chrome: true });
const { page } = h;
await h.loadHew(`${process.env.HOME}/Documents/Hew/Trestle Table.hew`, 2000);

const rows = page.getByTestId('scene-row');
// the marks below name the scenes by position, so the file's scene order
// is asserted rather than assumed
const names = (await rows.allInnerTexts()).map(t => t.replace(/^[^\n]*\n/, '').trim());
const want = ['Assembled Table', 'Cut List', 'Vertical Section'];
if (JSON.stringify(names) !== JSON.stringify(want)) {
  throw new Error(`scene rows are ${JSON.stringify(names)}, expected ${JSON.stringify(want)}`);
}

// A panel row is found by its exact label text; `button` picks a control
// inside that row by its own text or title (the Outliner's ▸ chevron, the
// Tags panel's eye). Returns a locator for clickUi.
const rowControl = (label, button) => page.locator(
  `xpath=//*[normalize-space(text())=${JSON.stringify(label)}]/ancestor::*[.//button][1]//button[normalize-space(text())=${JSON.stringify(button)} or @title=${JSON.stringify(button)}]`
).first();
const rowLabel = label => page.getByText(label, { exact: true }).first();

await h.shot('loaded');
h.mark('start');
await h.clickUi(rows.nth(1), 500);
await h.wait(2200);
h.mark('cut-list');
await h.shot('cut-list');
await h.clickUi(rows.nth(2), 400);
await h.wait(2200);
h.mark('section');
await h.shot('section');
await h.clickUi(rows.nth(0), 400);
await h.wait(2200);
h.mark('assembled');
await h.shot('assembled');

// the Outliner: open the model's group, pick a part
await h.clickUi(rowControl('3D Model', '▸'), 500);
await h.wait(500);
await h.shot('outliner-open');
// a part whose row is in view without scrolling the list
await h.clickUi(rowLabel('Vertical Leg'), 500);
h.mark('outliner');
await h.wait(900);
await h.shot('outliner');

// the Tags panel: hide the Table Top tag, then show it again
const tagsHeader = page.getByRole('button', { name: 'Tags', exact: true }).first();
if ((await tagsHeader.getAttribute('aria-expanded')) !== 'true') await h.clickUi(tagsHeader, 450);
await h.wait(500);
await h.shot('tags-open');
const partsChevron = rowControl('Parts', '▸');
if (await partsChevron.count()) await h.clickUi(partsChevron, 400);
await h.wait(400);
await h.shot('tags-parts');
await h.clickUi(rowControl('Table Top', 'Hide tagged objects'), 500);
h.mark('tag-hidden');
await h.wait(1000);
await h.shot('tag-hidden');
await h.clickUi(rowControl('Table Top', 'Show tagged objects'), 400);
h.mark('tag-shown');
await h.wait(900);
await h.shot('tag-shown');
await h.finish();
