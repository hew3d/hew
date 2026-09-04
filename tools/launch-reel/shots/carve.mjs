// Shot: carve — a box gets a recess and a through-hole, drawn on its top face.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('carve') });
await h.zoom(-1);
await h.wait(300);
await h.shot('layout');
h.mark('start');

// box
await h.key('r');
await h.glide(790, 600, 300);
await h.click();
await h.glide(1060, 680, 350);
await h.type('20,12');
await h.key('Enter', 400);
await h.key('p');
await h.glide(930, 630, 250);
await h.click();
await h.glide(930, 470, 350);
await h.type('8');
await h.key('Enter', 100);
await h.expectBadge('1 object', 'box');
h.mark('box');
await h.wait(700);
await h.shot('box');

// frame tight on the top face before carving it
await h.glide(1140, 470, 300);
await h.zoom(-3);
await h.wait(300);
await h.shot('zoomed');

// recess: circle, then push inward
await h.key('c');
await h.glide(1030, 425, 300);
await h.click();
await h.type('2.5');
await h.key('Enter', 300);
await h.key('p');
await h.glide(1030, 425, 250);
await h.click();
await h.type('-3');
await h.key('Enter', 100);
await h.expectBadge('1 object', 'recess');
h.mark('recess');
await h.wait(700);
await h.shot('recess');

// through-hole: second circle, then push all the way through
await h.key('c');
await h.glide(1300, 470, 300);
await h.click();
await h.type('2');
await h.key('Enter', 300);
await h.key('p');
await h.glide(1300, 470, 250);
await h.click();
await h.type('-8');
await h.key('Enter', 100);
await h.expectBadge('1 object', 'hole');
h.mark('hole');
await h.wait(700);
await h.shot('hole');

await h.glide(450, 850, 300);
await h.wait(300);
await h.finish();
