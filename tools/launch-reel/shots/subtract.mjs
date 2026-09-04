// Shot: Subtract. A box and an overlapping cylinder become one cut solid.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('subtract') });
await h.shot('layout');
h.mark('start');

// the box
await h.key('r');
await h.glide(760, 560, 300);
await h.click();
await h.glide(980, 650, 350);
await h.type('12,12');
await h.key('Enter', 500);
await h.shot('rect');
await h.key('p');
await h.glide(870, 605, 250);
await h.click();
await h.glide(870, 430, 400);
await h.type('8');
await h.key('Enter', 100);
await h.expectBadge('1 object', 'box');
await h.shot('box');

// the cylinder, overlapping the box's near corner
await h.key('c');
await h.glide(1060, 690, 350);
await h.click();
await h.type('4');
await h.key('Enter', 400);
await h.shot('cyl-circle');
await h.key('p');
await h.glide(1060, 690, 250);
await h.click();
await h.glide(1060, 500, 400);
await h.type('12');
await h.key('Enter', 100);
await h.expectBadge('2 objects', 'cylinder');
await h.shot('two-solids');

// frame the pair before the selection so the cut happens at full size
await h.key('Escape');
await h.glide(930, 600, 300);
await h.zoom(-5);
await h.wait(300);
await h.shot('framed-pair');

// select both, subtract
await h.key(' ');
await h.glide(800, 590, 350);
await h.click();
await h.wait(150);
await h.page.keyboard.down('Shift');
await h.glide(1080, 620, 350);
await h.click();
await h.page.keyboard.up('Shift');
await h.wait(200);
await h.shot('selected');
await h.menu('Object', 'Subtract');
h.mark('subtract');
await h.expectBadge('1 object', 'subtract');
await h.wait(700);
await h.shot('subtract');

await h.key('Escape');
await h.glide(950, 590, 350);
await h.orbit(220, 60, 2200);
h.mark('orbited');
await h.wait(400);
await h.glide(1500, 950, 300);
await h.shot('orbited');
await h.finish();
