// Shot: Section Plane — place it on a theater wall, then sweep the cut
// through the room (Tools > Section Plane; click a face to place, click the
// widget and glide the cursor to sweep, click again to set).
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('section') });
await h.loadHew(`${process.env.HOME}/Documents/Hew/theater-test-5.hew`, 4000);
await h.clean();
await h.zoomExtents();
await h.shot('extents');
await h.glide(960, 540, 200);
await h.zoom(-6, 120, 60);
await h.wait(400);
await h.shot('framed');

await h.menu('Tools', 'Section Plane');
h.mark('start');
// click the near-front interior wall face to place the section plane there
await h.glide(650, 620, 350);
await h.click();
await h.wait(500);
await h.shot('placed');
h.mark('placed');

// click the widget, then sweep the cursor along its normal through the room
await h.click();
h.mark('sweep-start');
await h.glide(900, 600, 2000);
await h.shot('mid-sweep');
await h.click();
h.mark('sweep-end');
await h.wait(700);
await h.shot('sweep-end');

await h.finish();
