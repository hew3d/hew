// Docs-fleet clip: Drawing chapter (site/src/content/learn/drawing.md).
// Silent, caption-carried. Beats: Rectangle (typed dims), Line (closes on
// its own start point via Endpoint snap), Circle (typed radius), Arc
// (two-point chord + bulge), Polygon (rail tool), finale Push/Pull turns
// one closed region into a solid. ~40 s.
//   node docs-drawing.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-drawing`;
const h = await startCapture({ out: OUT, headless: true });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

await h.showMark();
await h.caption('Drawing — five tools, one shared sketch.', 900);
await page.waitForTimeout(1200);
h.mark('scene-start');

// beat 1 — Rectangle, typed dimensions
await h.caption('A closed shape fills in as a sketch region, ready to extrude.');
await page.keyboard.press('r');
await page.waitForTimeout(250);
await h.glide(480, 680, 350);
await h.click();
await h.glide(630, 734, 350);
await h.typeSlow('12,8');
await page.keyboard.press('Enter');
h.mark('rectangle');
await page.waitForTimeout(1300); await shot(1);

// beat 2 — Line, a 4-segment loop that closes on its own start point
await h.caption('Glide back onto your start point — Endpoint snapping closes the loop.');
await page.keyboard.press('l');
await page.waitForTimeout(250);
await h.glide(430, 300, 350);
await h.click();                    // A — anchor
await h.glide(560, 240, 350);
await h.click();                    // B
await h.glide(680, 300, 350);
await h.click();                    // C
await h.glide(560, 380, 350);
await h.click();                    // D
await h.glide(430, 300, 450);       // glide back onto A — let the green dot show
await page.waitForTimeout(300); await shot(2);
await h.click();                    // closes the loop, forms the face
h.mark('line-loop');
await page.waitForTimeout(1300); await shot(3);

// beat 3 — Circle, typed radius
await h.caption('Circle: click a center, then type an exact radius.');
await page.keyboard.press('c');
await page.waitForTimeout(250);
await h.glide(1150, 300, 350);
await h.click();
await h.typeSlow('3');
await page.keyboard.press('Enter');
h.mark('circle');
await page.waitForTimeout(1300); await shot(4);

// beat 4 — Arc: two endpoints set the chord, then the bulge
await h.caption('Arc: two clicks set the chord, a third pulls out the bulge.');
await page.keyboard.press('a');
await page.waitForTimeout(250);
await h.glide(1080, 650, 400);
await h.click();                    // endpoint one
await h.glide(1280, 650, 400);
await h.click();                    // endpoint two — sets the chord
await h.glide(1180, 560, 400);
await h.click();                    // perpendicular — commits the bulge
h.mark('arc');
await page.waitForTimeout(1300); await shot(5);

// beat 5 — Polygon (rail tool), typed radius
await h.caption('Polygon: a regular N-sided shape — six sides by default.');
await h.clickRail('Polygon');
await h.glide(850, 900, 350);
await h.click();
await h.typeSlow('3');
await page.keyboard.press('Enter');
h.mark('polygon');
await page.waitForTimeout(1300); await shot(6);

// finale — Push/Pull turns one of the closed regions into a solid
await h.caption('Any closed region is one pull from a solid.');
await page.keyboard.press('p');
await page.waitForTimeout(200);
await h.glide(558, 305, 350);
await h.click();
await h.glide(558, 175, 350);
await h.typeSlow('4');
await page.keyboard.press('Enter');
h.mark('pulled');
await h.expectBadge('1 object', 'pulled');
await page.waitForTimeout(1300); await shot(7);

// close — orbit the result. Pivot from the model's screen center, not
// wherever the last click left the cursor — orbiting from the far edge
// swings the model off-frame (viewer feedback).
await page.keyboard.press('Escape');
await h.glide(820, 540, 400);
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
await h.caption('hew3d.com/learn/drawing', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);
