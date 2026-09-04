// Docs-fleet clip: Follow Me chapter (site/src/content/learn/follow-me.md).
// Beats: sweep a circle profile along a 2-segment path (auto stand-up +
// mitered corner), then the sphere lathe (profile reaching the axis closes
// poles). ~50 s.
//   node docs-follow-me.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-follow-me`;
const h = await startCapture({ out: OUT, headless: true });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

await h.showMark();
await h.caption('Follow Me — sweep a profile along a path.', 600);
await page.waitForTimeout(1000);
h.mark('scene-start');

// beat 1 — the path: two connected lines on the ground
await h.caption('Draw a path — any connected run of edges.');
await page.keyboard.press('l');
await page.waitForTimeout(250);
await h.glide(650, 600, 350);
await h.click();
await h.glide(850, 660, 400);
await h.click();
await h.glide(1000, 580, 400);
await h.click();
await page.keyboard.press('Escape');
await page.waitForTimeout(600);
h.mark('path');
await shot(1);

// the profile: a small circle drawn FLAT beside the path start
await h.caption('Draw the profile flat beside the path.');
await page.keyboard.press('c');
await page.waitForTimeout(250);
await h.glide(560, 640, 350);
await h.click();
await h.typeSlow('0.8');
await page.keyboard.press('Enter');
await page.waitForTimeout(700);
h.mark('profile');
await shot(2);

// select the path (one click picks the whole connected run), Follow Me, click profile
await h.caption('Pick the path — one click takes the whole connected run.');
await page.keyboard.press(' ');
await page.waitForTimeout(250);
await h.glide(750, 630, 350);
await h.click();
await page.waitForTimeout(900);
await shot(3);
await h.caption('Follow Me stands the flat profile upright — and sweeps.');
await h.clickRail('Follow Me');
await h.glide(560, 640, 400);
await h.click();
h.mark('pipe');
await h.expectBadge('1 object', 'pipe');
await page.waitForTimeout(600);
await h.caption('A pipe that turns corners — one watertight solid, mitered clean.');
await page.waitForTimeout(1300);
await shot(4);

// beat 2 — the sphere lathe
await h.caption('A drawn circle as the path makes a lathe.');
await page.keyboard.press('c');
await page.waitForTimeout(250);
await h.glide(1300, 700, 400);
await h.click();
await h.typeSlow('3');
await page.keyboard.press('Enter');
await page.waitForTimeout(700);
await shot(5);
await h.caption('Draw the profile upright through the center — reaching the axis.');
await page.keyboard.press('c');
await page.waitForTimeout(250);
await h.glide(1300, 700, 400);
await page.keyboard.press('ArrowRight');       // lock the red (vertical) plane
await page.waitForTimeout(400);
await h.click();                               // center snaps to the path circle's center
await h.typeSlow('3');
await page.keyboard.press('Enter');
await page.waitForTimeout(700);
h.mark('sphere-setup');
await shot(6);
await h.caption('Sweep the circle around the circle…');
await page.keyboard.press(' ');
await page.waitForTimeout(250);
await h.glide(1355, 728, 350);                 // path circle rim, clear of the profile
await h.click();
await page.waitForTimeout(700);
await shot(7);
await h.clickRail('Follow Me');
await h.glide(1300, 650, 350);                 // inside the upright profile's upper half
await h.click();
h.mark('sphere');
await h.expectBadge('2 objects', 'sphere');
await page.waitForTimeout(400);
await h.caption('…and the poles close: a sphere, one watertight solid.');
await page.waitForTimeout(1300);
await shot(8);

// close — orbit the result. Pivot from the model's screen center, not
// wherever the last click left the cursor — orbiting from the far edge
// swings the model off-frame (viewer feedback).
await h.caption('Every result is an ordinary solid — push/pull it, boolean it, print it.', 300);
await page.keyboard.press('Escape');
await h.glide(1050, 660, 400);
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
await h.caption('hew3d.com/learn/follow-me', 300);
await page.waitForTimeout(2000);
h.mark('scene-end');
await h.finish(OUT);