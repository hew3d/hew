// Shot: the move, then the whole app. Rectangle typed to size and pulled into
// a solid with no chrome; then the UI appears around the same model and the
// next steps go through it — both tools picked from the rail.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('ui-tour') });
const { page } = h;
await h.zoom(-1);
await h.wait(300);
h.mark('start');
await h.key('r');
await h.glide(790, 600, 300);
await h.click();
await h.glide(1060, 680, 350);
await h.type('20,12');
await h.key('Enter', 500);
await h.key('p');
await h.glide(930, 630, 250);
await h.click();
await h.glide(930, 470, 500);
await h.type('8');
await h.key('Enter', 100);
h.mark('solid');
await h.expectBadge('1 object', 'solid');
await h.glide(1560, 300, 400);
await h.wait(700);
await h.shot('solid');

// the app appears around the model
await h.showChrome();
h.mark('chrome');
await h.wait(900);
await h.shot('chrome');

// a circle on the top face, both tools picked from the rail
await h.clickUi(page.getByRole('radio', { name: 'Circle' }));
await h.glide(1110, 480, 400);   // top-face center once the viewport shrinks
await h.click();
await h.type('2');
await h.key('Enter', 500);
await h.shot('circle');
await h.clickUi(page.getByRole('radio', { name: 'Push/Pull' }));
await h.glide(1110, 480, 350);
await h.click();
await h.type('-3');
await h.key('Enter', 100);
h.mark('recess');
await h.expectBadge('1 object', 'recess');
await h.wait(600);
await h.shot('recess');

await h.key('Escape');
await h.glide(1300, 900, 300);
await h.wait(400);
await h.finish();
