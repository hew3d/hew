// Shot: Follow Me lathe. A circle path + an upright profile become a sphere.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('lathe') });
await h.zoom(-8);
await h.wait(200);
await h.shot('layout');
h.mark('start');

// the path: a circle flat on the ground
await h.key('c');
await h.glide(960, 540, 300);
await h.click();
await h.type('6');
await h.key('Enter', 400);
await h.shot('path');

// the profile: a circle drawn upright through the same center — ArrowRight
// locks the red vertical plane, then the click snaps to the path's center
await h.key('c');
await h.glide(960, 540, 300);
await h.key('ArrowRight', 300);
await h.click();
await h.type('6');
await h.key('Enter', 400);
h.mark('setup');
await h.shot('setup');

// select the path (its rim, clear of the profile), Follow Me, click inside
// the upright profile's upper half
await h.key(' ');
await h.glide(1196, 545, 350);   // path rim: center + 6 cm at ~39 px/cm
await h.click();
await h.wait(300);
await h.shot('path-selected');
await h.menu('Tools', 'Follow Me');
await h.glide(960, 400, 350);
await h.click();
h.mark('sphere');
await h.expectBadge('1 object', 'sphere');
await h.wait(400);
await h.shot('sphere-badge');
await h.wait(500);
await h.shot('sphere');

await h.key('Escape');
await h.glide(960, 550, 300);
await h.zoom(-6);
await h.wait(200);
h.mark('orbited-begin');
await h.orbit(220, 60, 2200);
h.mark('orbited');
await h.wait(400);
await h.glide(1500, 950, 300);
await h.shot('orbited');
await h.finish();
