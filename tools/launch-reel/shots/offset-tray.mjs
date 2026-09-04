// Shot: offset-tray — a flat slab gets an offset rim, pulled into a tray.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('offset-tray') });
await h.zoom(-1);
await h.wait(300);
await h.shot('layout');
h.mark('start');

// slab
await h.key('r');
await h.glide(790, 600, 300);
await h.click();
await h.glide(1060, 680, 350);
await h.type('20,12');
await h.key('Enter', 400);
await h.key('p');
await h.glide(930, 630, 250);
await h.click();
await h.glide(930, 590, 300);
await h.type('2');
await h.key('Enter', 100);
await h.expectBadge('1 object', 'slab');
h.mark('slab');
await h.wait(700);
await h.shot('slab');

await h.glide(1150, 640, 300);
await h.zoom(-3);
await h.wait(300);
await h.shot('zoomed');

// offset: click the top face, glide inward, type the inset distance
await h.key('f');
await h.glide(1165, 680, 250);
await h.click();
await h.glide(1210, 650, 300);
await h.type('1');
await h.key('Enter', 300);
await h.expectBadge('1 object', 'offset');
h.mark('offset');
await h.wait(700);
await h.shot('offset');

// pull the inner face down to hollow it into a tray
await h.key('p');
await h.glide(1200, 660, 250);
await h.click();
await h.type('-1.5');
await h.key('Enter', 100);
await h.expectBadge('1 object', 'tray');
h.mark('tray');
await h.wait(700);
await h.shot('tray');

await h.glide(450, 850, 300);
await h.wait(300);
await h.finish();
