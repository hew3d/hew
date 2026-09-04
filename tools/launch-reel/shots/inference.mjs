// Shot: inference — snap chips (Endpoint, Midpoint, On Edge, On Face) glide
// along the Café Table's round tabletop rim, then in tight on the pen cup's
// own rim for a second Endpoint/Midpoint pair. No clicks — nothing is drawn;
// the point of the shot is the snap dots and labels riding the cursor.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('inference'), sample: 'Café Table' });
await h.expectBadge('2 objects', 'loaded');
await h.zoomExtents();
await h.glide(975, 305, 200);
await h.zoom(-8);
await h.wait(300);
await h.shot('framed');

await h.key('l');
h.mark('start');
await h.glide(830, 205, 600);           // tabletop rim vertex — Endpoint
await h.wait(400);
h.mark('endpoint');
await h.shot('endpoint');

await h.glide(1155, 500, 600);          // tabletop rim segment midpoint — Midpoint
await h.wait(400);
h.mark('midpoint');
await h.shot('midpoint');

await h.glide(700, 470, 600);           // along the rim, off the vertex — On Edge
await h.wait(400);
h.mark('on-edge');
await h.shot('on-edge');

await h.glide(950, 517, 600);           // clearly inside the tabletop — On Face
await h.wait(400);
h.mark('on-face');
await h.shot('on-face');

await h.glide(975, 300, 400);           // move onto the cup before zooming to it
await h.zoom(-6);
await h.wait(300);
await h.shot('cup-framed');

await h.glide(965, 225, 600);           // cup rim vertex — Endpoint
await h.wait(400);
h.mark('cup-endpoint');
await h.shot('cup-endpoint');

await h.glide(955, 230, 600);           // cup rim segment midpoint — Midpoint
await h.wait(700);
h.mark('cup-midpoint');
await h.shot('cup-midpoint');

await h.key('Escape');
await h.glide(100, 80, 400);            // park off the geometry
await h.wait(300);
await h.finish();
