// Docs-fleet clip: The Hew interface chapter (site/src/content/learn/interface.md).
// Silent, caption-carried, READ-ONLY (no geometry is touched) — a guided
// cursor tour of the window itself, following the chapter's own section
// order: tool rail, command palette, contextual dock, panels tray, status
// bar + solid badge. Loads the bundled sample so the dock/panels/badge have
// real content to show (an empty document has no badge and empty panels).
// ~46 s.
//   node docs-interface.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-interface`;
const h = await startCapture({ out: OUT, headless: true, sample: 'Café Table' });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

await h.showMark();
await h.caption('The Hew interface — tool rail, palette, dock, panels, status bar.', 900);
await page.waitForTimeout(1200);
h.mark('scene-start');
await h.expectBadge('2 objects', 'loaded');

// beat 1 — glide down the tool rail (Draw, Modify, Inspect)
await h.caption('The left rail groups everyday tools — Draw, Modify, Inspect — each with a shortcut.', 600);
await h.glide(95, 129, 700);    // Select
await h.glide(95, 157, 320);    // Line
await h.glide(95, 185, 320);    // Rectangle
await h.glide(95, 213, 320);    // Circle
await h.glide(95, 241, 320);    // Polygon
await h.glide(95, 269, 320);    // Arc
await h.glide(95, 329, 450);    // Push/Pull
await h.glide(95, 357, 320);    // Follow Me
await h.glide(95, 385, 320);    // Offset
await h.glide(95, 413, 320);    // Move
await h.glide(95, 441, 320);    // Rotate
await h.glide(95, 469, 320);    // Scale
await h.glide(95, 529, 450);    // Tape Measure
await h.glide(95, 557, 320);    // Dimension
await h.glide(95, 585, 320);    // Paint
await h.glide(95, 613, 320);    // Section Plane
h.mark('rail');
await page.waitForTimeout(1300); await shot(1);

// beat 2 — the command palette: open it, type slowly, see the greyed
// "needs a selection" note on Subtract, then close
await h.caption('Type a few letters — every tool, action, and named object is searchable.', 600);
await h.glide(95, 60, 600);
await h.click();
await page.waitForTimeout(225); await shot(2);
await h.typeSlow('sub', 220);
await page.waitForTimeout(1300); await shot(3);
h.mark('palette');
await page.keyboard.press('Escape');
await page.waitForTimeout(225);

// beat 3 — the contextual dock follows the selection
await h.caption('The dock at the bottom of the viewport follows your selection with likely next actions.', 600);
await h.glide(900, 330, 650);
await h.click();
await page.waitForTimeout(225); await shot(4);
await h.glide(600, 995, 650);
await h.glide(1230, 995, 700);
h.mark('dock');
await page.waitForTimeout(1300); await shot(5);

// beat 4 — glide across the right panels tray
await h.caption('The right tray holds Object Info, the Outliner, Materials, Tags, and Scenes.', 600);
await h.glide(1767, 94, 650);    // Object Info — Name
await h.glide(1767, 137, 450);   // Type
await h.glide(1767, 177, 450);   // Geometry: Solid
await h.glide(1767, 216, 450);   // Bounding box
await h.glide(1767, 260, 450);   // Tags
await h.glide(1768, 366, 650);   // Outliner header
await h.glide(1768, 417, 400);   // Tabletop row
await h.glide(1768, 438, 400);   // Pen Cup row
await h.glide(1768, 681, 650);   // Materials header
await h.glide(1768, 717, 450);   // Tags header
await h.glide(1752, 753, 450);   // Scenes header
h.mark('panels');
await page.waitForTimeout(1300); await shot(6);

// beat 5 — the status bar always names the next step; the badge confirms
// the whole model is watertight
await h.caption('The status bar always names the active tool and what it expects next.', 600);
await h.glide(200, 1063, 650);
await h.glide(700, 1063, 650);
await h.glide(1859, 1062, 700);
h.mark('status-bar');
await page.waitForTimeout(1300); await shot(7);

// close
await h.caption('hew3d.com/learn/interface', 500);
await page.waitForTimeout(2500);
h.mark('scene-end');
await h.finish(OUT);
