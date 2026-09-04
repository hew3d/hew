// Shot: cylinder-radius — a cylinder pulled up, then its wall pushed to
// change the whole solid's radius (fatter, then thinner).
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('cylinder-radius') });
await h.zoom(-1);
await h.wait(300);
await h.shot('layout');
h.mark('start');

// circle on the ground, then pull into a cylinder
await h.key('c');
await h.glide(900, 660, 300);
await h.click();
await h.type('4');
await h.key('Enter', 400);
await h.key('p');
await h.glide(900, 660, 250);
await h.click();
await h.glide(900, 480, 400);
await h.type('10');
await h.key('Enter', 100);
await h.expectBadge('1 object', 'cylinder');
h.mark('cylinder');
await h.wait(700);
await h.shot('cylinder');

await h.glide(890, 520, 300);
await h.zoom(-10);
await h.wait(300);
await h.shot('zoomed');

// push the curved wall outward — the whole cylinder gets fatter
await h.glide(1200, 520, 300);
await h.click();
await h.glide(1350, 520, 300);
await h.click();
await h.expectBadge('1 object', 'fatter');
h.mark('fatter');
await h.wait(700);
await h.shot('fatter');

// push the wall back inward — thinner
await h.glide(1300, 500, 300);
await h.click();
await h.glide(1050, 500, 300);
await h.click();
await h.expectBadge('1 object', 'thinner');
h.mark('thinner');
await h.wait(700);
await h.shot('thinner');

await h.glide(200, 900, 300);
await h.wait(300);
await h.finish();
