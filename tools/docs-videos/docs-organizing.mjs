// Docs-fleet clip: Organizing chapter (site/src/content/learn/organizing.md).
// Silent, caption-carried. Beats: build two objects, name one in Object
// Info, select-all + group via the Object menu, the Outliner shows the
// group, tag the group (hierarchical), toggle the tag's visibility eye in
// the Tags panel. Panel-heavy: every panel click is located by querying the
// live DOM (label text / aria-label / title), not guessed pixels — only the
// viewport geometry clicks (rectangle corners, push/pull grabs) use measured
// screen coordinates. ~58 s.
// Pacing: modeling/panel glides at hand speed (250-450 ms); panel-state
// dwells (Outliner, tags, named field) hold 1300 ms so the change reads.
//   node docs-organizing.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-organizing`;
const h = await startCapture({ out: OUT, headless: true });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

// ---------------------------------------------------------------------------
// Panel locators — find the live element by the same identity a person
// would (label text, aria-label, title), not a guessed pixel. Returns a
// screen-space center point, or throws so a wrong build fails loudly rather
// than clicking the wrong thing.
// ---------------------------------------------------------------------------

async function locate(fn, ...args) {
  const box = await page.evaluate(fn, ...args);
  if (!box) throw new Error(`locator not found: ${fn.name || fn.toString().slice(0, 60)}`);
  return box;
}

// Object Info's plain "Name" field (object/group — not "Definition Name" /
// "Instance Name", which are components-only and read differently).
const findNameInput = () => locate(() => {
  const label = [...document.querySelectorAll('div')]
    .find(d => d.children.length === 0 && d.textContent.trim() === 'Name');
  const input = label?.parentElement?.querySelector('input');
  if (!input) return null;
  const r = input.getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
});

// Object Info's "+" add-tag button (single-selection aria-label is exactly
// "Add tag"; the multi-selection variant is "Add tag to all selected").
const findAddTagButton = () => locate(() => {
  const b = [...document.querySelectorAll('button')].find(x => x.getAttribute('aria-label') === 'Add tag');
  if (!b) return null;
  const r = b.getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
});

// An Outliner row by its exact label text (the label <span> DocumentTree
// renders — never collides with Object Info's "Type: Group" text, which
// carries no index, or the Name field's placeholder, which is an attribute
// not a text node).
const findOutlinerRow = text => locate(label => {
  const span = [...document.querySelectorAll('span')]
    .find(s => s.children.length === 0 && s.textContent.trim() === label);
  const row = span?.closest('div');
  const el = row ?? span;
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
}, text);

// A Tags-panel row's eye toggle, found by the tag's own last-segment text
// (e.g. "Bins" for "Shop/Bins") — TagsPanel tags each row data-testid="tag-row".
const findTagEye = segment => locate(seg => {
  const rows = [...document.querySelectorAll('[data-testid="tag-row"]')];
  const row = rows.find(r => [...r.querySelectorAll('span')]
    .some(s => s.children.length === 0 && s.textContent.trim() === seg));
  if (!row) return null;
  const btn = [...row.querySelectorAll('button')]
    .find(b => (b.title || '').startsWith('Hide') || (b.title || '').startsWith('Show'));
  if (!btn) return null;
  const r = btn.getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
}, segment);

const clickAt = async (pt, ms = 700) => { await h.glide(pt.x, pt.y, ms); await h.click(); };

// ---------------------------------------------------------------------------

await h.showMark();
await h.caption('Organizing — names, tags, and the Outliner keep a model legible.', 700);
await page.waitForTimeout(1000);
h.mark('scene-start');

// setup — two plain boxes, side by side
await h.caption('Build a couple of objects to organize.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(700, 590, 250);
await h.click();
await h.glide(820, 660, 250);
await h.typeSlow('6,6', 90);
await page.keyboard.press('Enter');
await page.waitForTimeout(900);
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(760, 625, 450);
await h.click();
await h.glide(760, 505, 250);
await h.typeSlow('5', 90);
await page.keyboard.press('Enter');
await page.waitForTimeout(350);
h.mark('box1');
await h.expectBadge('1 object', 'box1');
await shot(1);

await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(1050, 590, 250);
await h.click();
await h.glide(1170, 660, 250);
await h.typeSlow('6,6', 90);
await page.keyboard.press('Enter');
await page.waitForTimeout(900);
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(1110, 625, 450);
await h.click();
await h.glide(1110, 505, 250);
await h.typeSlow('5', 90);
await page.keyboard.press('Enter');
h.mark('boxes');
await h.expectBadge('2 objects', 'boxes');
await page.waitForTimeout(1300); await shot(2);

// beat 1 — name one object in Object Info
await h.caption('Object Info names the selected thing — type a name, press Enter.');
await page.keyboard.press(' ');
await page.waitForTimeout(250);
await h.glide(760, 640, 350);
await h.click();
await page.waitForTimeout(400); await shot(3);
const nameField = await findNameInput();
await clickAt(nameField, 300);
await page.waitForTimeout(200);
await page.keyboard.press('Meta+a');
await h.typeSlow('Bin A');
await page.keyboard.press('Enter');
h.mark('named');
await h.expectBadge('2 objects', 'named');
await page.waitForTimeout(1300); await shot(4);

// beat 2 — select everything, group it (Object menu)
await h.caption('Group things that belong together — Object ▸ Group.');
await page.keyboard.press('Meta+a');
await page.waitForTimeout(400); await shot(5);
await page.getByRole('button', { name: 'Object', exact: true }).click();
await page.waitForTimeout(250);
// Scoped to the menu bar: a floating contextual dock also shows a "Group"
// quick-action button while 2+ objects are selected, and an unscoped text
// query resolves to both (strict-mode violation).
await page.getByTestId('menu-bar').getByText('Group', { exact: true }).click();
await page.waitForTimeout(250);
h.mark('grouped');
await h.expectBadge('2 objects', 'grouped');
await page.waitForTimeout(1300); await shot(6);

// beat 3 — the Outliner shows the group; glide over it
await h.caption('The Outliner lists every object, group, and sketch in the model.');
const groupRow = await findOutlinerRow('Group 1');
await h.glide(groupRow.x, groupRow.y, 450);
await page.waitForTimeout(1300); await shot(7);

// beat 4 — tag the group (hierarchical)
await h.caption('Add a tag with + — use / to nest, like Shop/Bins.');
const addTagBtn = await findAddTagButton();
await clickAt(addTagBtn, 300);
await page.waitForTimeout(300);
await h.typeSlow('Shop/Bins');
await page.keyboard.press('Enter');
h.mark('tagged');
await h.expectBadge('2 objects', 'tagged');
await page.waitForTimeout(1300); await shot(8);

// beat 5 — the Tags panel: toggle the tag's visibility eye off, then on
await h.caption("The Tags panel's eye toggle hides everything under that tag.");
await page.keyboard.press('Meta+Shift+T');
await page.waitForTimeout(500); await shot(9);
const eyeOff = await findTagEye('Bins');
await clickAt(eyeOff, 300);
h.mark('tag-hidden');
await page.waitForTimeout(1300); await shot(10);
const eyeOn = await findTagEye('Bins');
await clickAt(eyeOn, 300);
h.mark('tag-shown');
await h.expectBadge('2 objects', 'tag-shown');
await page.waitForTimeout(1300); await shot(11);

// close — orbit the result, then Camera ▸ Zoom Extents guarantees a
// centered final frame (copied from docs-push-pull.mjs's approved close).
await h.caption('Names, groups, and tags — a growing model stays easy to find.', 300);
await page.keyboard.press('Escape');
await h.glide(950, 590, 400);
await h.orbit(220, 80, 1400);
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
await h.caption('hew3d.com/learn/organizing', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
