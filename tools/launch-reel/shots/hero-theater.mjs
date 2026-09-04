// Shot: hero orbit — the home theater, textured, roof off, seen from above.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('hero-theater') });
await h.loadHew(`${process.env.HOME}/Documents/Hew/theater-test-5.hew`, 4000);
await h.clean();
await h.zoomExtents();
await h.shot('extents');
await h.glide(960, 540, 200);
await h.zoom(-6, 120, 60);
await h.wait(400);
await h.shot('framed');
await h.glide(960, 560, 200);
h.mark('start');
await h.orbit(260, 40, 3200);
h.mark('orbited');
await h.wait(400);
await h.shot('orbited');
await h.finish();
