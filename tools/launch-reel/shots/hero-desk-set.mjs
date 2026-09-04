// Shot: hero orbit — the desk set (tray, pen cup, bin, phone stand) from
// the Getting Started tutorial. Tight framing, quick orbit.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('hero-desk-set') });
await h.loadHew(`${process.env.HOME}/Documents/Hew/desk-set.hew`);
await h.clean();
await h.view('Guides', false); // the tutorial model saves a construction guide
await h.zoomExtents();
await h.shot('extents');
await h.glide(960, 540, 200);
await h.zoom(-9, 120, 60);
await h.wait(400);
await h.shot('framed');
await h.glide(960, 560, 200);
h.mark('start');
await h.orbit(260, 40, 3200);
h.mark('orbited');
await h.wait(400);
await h.shot('orbited');
await h.finish();
