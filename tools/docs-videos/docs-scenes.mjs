// Docs-fleet clip: Scenes chapter (site/src/content/learn/scenes.md).
// Silent, caption-carried. Beats: add a Scene, change the view + hide an
// object and add a second Scene, then click back and forth between them —
// the camera glides and visibility switches. ~48 s.
// Pacing: the Scene-to-Scene camera glides (the 4400/3800 ms holds after
// scene1-activate/scene2-activate) are the feature demo — left untouched;
// only the two panel "scene added" result dwells were trimmed to 1300 ms.
//   node docs-scenes.mjs   (DEBUG_SHOTS=1 for per-beat stills)

import { startCapture } from './capture-lib.mjs';

const OUT = process.env.CAPTURE_OUT ??
  `${process.env.TMPDIR ?? '/tmp'}/hew-docs-videos/docs-scenes`;
const h = await startCapture({ out: OUT, headless: true, sample: 'Café Table' });
const { page } = h;
const shot = async n => { if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${OUT}/beat-${n}.png` }); };

await h.showMark();
await h.caption('Scenes — save a view, come back to it in one click.', 2200);
await page.waitForTimeout(1800);
h.mark('scene-start');
await h.expectBadge('2 objects', 'start');

// beat 1 — add a Scene from the current (default) view
await h.caption("A Scene saves the camera, what's hidden, and the section cut.", 2200);
await page.waitForTimeout(600);
await h.caption('Set up the view, then click Add Scene to capture it.', 1100);
await h.glide(1683, 841, 200);
await h.click();
await page.waitForTimeout(400);
await page.keyboard.press('Escape');
h.mark('scene1-added');
await h.expectBadge('2 objects', 'scene1-added');
await page.waitForTimeout(1300); await shot(1);

// beat 2 — orbit to a different angle, zoom in, hide the Pen Cup, add Scene 2
await h.caption('Orbit to a new angle, zoom in, and hide what shouldn’t show.', 1100);
await page.mouse.move(905, 540);
await h.orbit(-380, 90, 200);
await page.waitForTimeout(200);
await page.mouse.move(905, 540);
await page.mouse.wheel(0, -180);
await page.waitForTimeout(1300);
await shot(2);
await h.glide(1898, 438, 200);
await h.click();
await page.waitForTimeout(1300);
await h.glide(1898, 753, 200);
await h.click();
await page.waitForTimeout(400);
await page.keyboard.press('Escape');
h.mark('scene2-added');
await h.expectBadge('2 objects', 'scene2-added');
await page.waitForTimeout(1300); await shot(3);

// beat 3 — click Scene 1: the camera glides back, the cup returns (money shot)
await h.caption('Click a Scene: the camera glides back, hidden objects return.', 1100);
await h.glide(1700, 785, 200);
await h.click();
h.mark('scene1-activate');
await page.waitForTimeout(4400); await shot(4);

// beat 4 — click Scene 2: glides forward again, cup hides again
await h.caption('Each Scene remembers its own camera and visibility.', 1100);
await h.glide(1700, 815, 200);
await h.click();
h.mark('scene2-activate');
await page.waitForTimeout(3800); await shot(5);

// close
await h.caption('Scenes are saved in the file — they travel with your model.', 3600);
await page.waitForTimeout(600);
await h.caption('hew3d.com/learn/scenes', 300);
await page.waitForTimeout(3000);
h.mark('scene-end');
await h.finish(OUT);
