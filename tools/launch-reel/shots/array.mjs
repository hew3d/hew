// Shot: array. Move-copy with Alt, then a typed "3x" fans out a row.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('array') });
// pre-frame: zoom in a touch from default so the finished row of 4 fills
// the frame without a mid-shot camera jump — the row runs along the red
// (+X) axis to screen-right.
await h.zoom(-2);
await h.wait(200);
await h.shot('layout');
h.mark('start');

// a small box
await h.key('r');
await h.glide(700, 560, 300);
await h.click();
await h.glide(790, 620, 350);
await h.type('8,8');
await h.key('Enter', 500);
await h.key('p');
await h.glide(745, 590, 250);
await h.click();
await h.glide(745, 500, 400);
await h.type('6');
await h.key('Enter', 100);
await h.expectBadge('1 object', 'box');
h.mark('box');
await h.shot('box');

// move-copy: base point on the box, Alt arms the copy, destination ~10cm
// along the red (+X) axis
await h.key('m');
await h.glide(745, 555, 300);
await h.click();
await h.page.keyboard.press('Alt');
await h.wait(250);
await h.glide(880, 555, 350);
await h.click();
await h.expectBadge('2 objects', 'copy');
h.mark('copy');
await h.shot('copy');
await h.type('3x');
await h.key('Enter', 200);
await h.expectBadge('4 objects', 'array');
h.mark('array');
await h.wait(700);
await h.shot('array');

await h.key('Escape');
await h.glide(1600, 950, 300);
await h.wait(400);
await h.shot('final');
await h.finish();
