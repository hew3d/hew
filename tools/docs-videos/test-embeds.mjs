// Cross-engine validation of the <docs-video> embeds on the local site dev
// server: autoplay fires, the right source is chosen, theme toggle swaps
// the clip live and playback continues. Screenshots per engine+theme.
import { createRequire } from 'module';
const require = createRequire(new URL('../../app/package.json', import.meta.url));
const pw = require('@playwright/test');
const OUT = (process.env.TMPDIR ?? '/tmp') + '/hew-docs-videos/embed-test';
import { mkdirSync } from 'fs';
mkdirSync(OUT, { recursive: true });

const PAGE = 'http://localhost:4321/learn/push-pull/';
const results = [];
for (const engine of ['chromium', 'firefox', 'webkit']) {
  const browser = await pw[engine].launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  try {
    await page.goto(PAGE, { waitUntil: 'networkidle' });
    const vid = page.locator('docs-video video');
    await vid.waitFor({ timeout: 10000 });
    await page.waitForTimeout(2500);
    const probe = async () => await vid.evaluate(v => ({
      paused: v.paused, time: +v.currentTime.toFixed(2),
      src: (v.currentSrc || '').split('/').pop(), ready: v.readyState,
    }));
    const p1 = await probe();
    await page.waitForTimeout(1500);
    const p2 = await probe();
    const advancing = p2.time > p1.time;
    await vid.scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${OUT}/${engine}-light.png` });
    // flip theme via the site's own toggle
    await page.locator('#theme-toggle').first().click();
    await page.waitForTimeout(2000);
    const p3 = await probe();
    await page.waitForTimeout(1200);
    const p4 = await probe();
    await page.screenshot({ path: `${OUT}/${engine}-dark.png` });
    results.push({ engine, autoplay: !p1.paused && advancing, lightSrc: p1.src,
      darkSrc: p3.src, swapKeptPlaying: !p4.paused && p4.time > p3.time,
      resumedNear: Math.abs(p3.time - p2.time) < 4 });
  } catch (e) {
    results.push({ engine, error: e.message.split('\n')[0] });
  }
  await browser.close();
}
console.log(JSON.stringify(results, null, 1));
