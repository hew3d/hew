// Shared capture harness for Hew docs/launch videos.
// Provides: browser/context setup with the brand overlay (cursor, click
// ripple, keystroke chips, caption band, corner mark), eased mouse glides,
// slow typing, beat marks, and capture-meta.json output.
//
// The overlay adds two docs-fleet extras over the launch-video harness:
//   caption(text)  — brand caption card, top-center; empty string hides it
//   showMark()     — small Hew mark, bottom-right, low opacity (watermark)

import { createRequire } from 'module';
const require = createRequire(new URL('../../app/package.json', import.meta.url));
const { chromium } = require('@playwright/test');
import { mkdirSync, writeFileSync } from 'fs';
import { loadavg } from 'os';

export const APP = 'http://localhost:5199/';
export const W = 1920, H = 1080;

// hud layouts (CAPTURE_HUD): 'bottom' (DEFAULT, viewer-tested winner) =
// caption sits directly above the pill chip near the bottom, so caption and
// keys read simultaneously; 'top' = caption top + a KEYCAP row (one
// bordered key per press) right under it — kept as the stronger
// "these are key presses" variant if viewers ever ask for it again.
const overlayFor = (theme, hud = 'bottom') => {
  const dark = theme === 'dark';
  const CAP_BG = dark ? '#29261E' : '#FBF8F2';
  const CAP_FG = dark ? '#F3EDE3' : '#1B1A17';
  const CHIP_BG = dark ? '#F3EDE3' : '#1B1A17';
  const CHIP_FG = dark ? '#1B1A17' : '#F3EDE3';
  const CHIP_HL = dark ? '#9A3D22' : '#E08561';
  const KEY_EDGE = dark ? '#B7AD9B' : '#5E5B54';
  const CAP_POS = hud === 'bottom' ? 'bottom: 232px; top: auto;' : 'top: 60px;';
  const KEYS_POS = hud === 'top' ? 'top: 152px; bottom: auto;' : 'bottom: 170px;';
  const KEYCAPS = hud === 'top';
  return `
(() => {
  if (window.__hewCaptureOverlay) return; window.__hewCaptureOverlay = true;
  const boot = () => { if (!document.body) return requestAnimationFrame(boot); install(); };
  boot();
  function install() {
  const css = document.createElement('style');
  css.textContent = \`
    #cap-cursor { position: fixed; z-index: 2147483647; pointer-events: none;
      width: 22px; height: 22px; margin: -2px 0 0 -2px; }
    #cap-ripple { position: fixed; z-index: 2147483646; pointer-events: none;
      width: 44px; height: 44px; margin: -22px 0 0 -22px; border-radius: 50%;
      border: 3px solid #C45D3C; opacity: 0; transform: scale(.4); }
    #cap-ripple.go { animation: capRip .45s ease-out; }
    @keyframes capRip { 0% { opacity: .85; transform: scale(.35); }
                        100% { opacity: 0; transform: scale(1.1); } }
    #cap-keys { position: fixed; z-index: 2147483647; pointer-events: none;
      left: 50%; ${KEYS_POS} transform: translateX(-50%); display: flex;
      align-items: center; font-family: "Hanken Grotesk", system-ui, sans-serif; }
    .cap-chip { background: ${CHIP_BG}; color: ${CHIP_FG}; border-radius: 8px;
      padding: 8px 14px; font-size: 22px; font-weight: 600; opacity: 0;
      transition: opacity .15s; box-shadow: 0 2px 10px rgb(0 0 0 / .25); }
    .cap-chip.on { opacity: .92; }
    .cap-chip b { color: ${CHIP_HL}; font-weight: 800; }
    .cap-key { display: inline-block; min-width: 22px; padding: 6px 10px;
      margin: 0 3px; background: ${CHIP_BG}; color: ${CHIP_FG};
      border-radius: 8px; text-align: center; font-size: 22px; font-weight: 700;
      border: 2px solid ${KEY_EDGE}; border-bottom-width: 5px;
      box-shadow: 0 3px 8px rgb(0 0 0 / .3); animation: capKey .12s ease-out; }
    .cap-key.enter { color: ${CHIP_HL}; }
    @keyframes capKey { 0% { transform: translateY(-5px) scale(1.12); }
                        100% { transform: none; } }
    #cap-keys.row { opacity: 0; transition: opacity .15s; }
    #cap-keys.row.on { opacity: 1; }
    #cap-caption { position: fixed; z-index: 2147483647; pointer-events: none;
      ${CAP_POS} left: 50%; transform: translateX(-50%); max-width: 56%;
      display: flex; align-items: stretch; opacity: 0; transition: opacity .3s;
      font-family: "Hanken Grotesk", system-ui, sans-serif;
      box-shadow: 0 3px 14px rgb(27 26 23 / .14); border-radius: 10px; overflow: hidden; }
    #cap-caption.on { opacity: 1; }
    #cap-caption .tick { width: 6px; background: #C45D3C; flex: none; }
    #cap-caption .txt { background: ${CAP_BG}; color: ${CAP_FG}; padding: 12px 20px;
      font-size: 24px; font-weight: 600; line-height: 1.35; }
    #cap-mark { position: fixed; z-index: 2147483647; pointer-events: none;
      right: 22px; bottom: 56px; width: 44px; opacity: 0; transition: opacity .5s; }
    #cap-mark.on { opacity: .45; }\`;
  document.head.appendChild(css);
  const cursor = document.createElement('div'); cursor.id = 'cap-cursor';
  cursor.innerHTML = '<svg viewBox="0 0 24 24" width="22" height="22">'
    + '<path d="M4 2 L4 19 L8.6 15.2 L11.4 21.4 L14.2 20.1 L11.5 14 L17.5 13.6 Z"'
    + ' fill="#1B1A17" stroke="#F3EDE3" stroke-width="1.4"/></svg>';
  const ripple = document.createElement('div'); ripple.id = 'cap-ripple';
  const KEYCAPS = ${KEYCAPS};
  const keys = document.createElement('div'); keys.id = 'cap-keys';
  const chip = document.createElement('div'); chip.className = 'cap-chip';
  if (KEYCAPS) keys.classList.add('row'); else keys.appendChild(chip);
  const cap = document.createElement('div'); cap.id = 'cap-caption';
  cap.innerHTML = '<div class="tick"></div><div class="txt"></div>';
  const mark = document.createElement('div'); mark.id = 'cap-mark';
  mark.innerHTML = '<svg viewBox="-50 -50 100 100"><g fill="none" stroke="#C45D3C"'
    + ' stroke-width="4.6" stroke-linejoin="round" stroke-linecap="round">'
    + '<polygon points="0,-34 29.44,-17 29.44,17 0,34 -29.44,17 -29.44,-17"></polygon>'
    + '<line x1="0" y1="0" x2="0" y2="-34"></line>'
    + '<line x1="0" y1="0" x2="-29.44" y2="-17"></line>'
    + '<line x1="0" y1="0" x2="29.44" y2="-17"></line></g></svg>';
  document.body.append(cursor, ripple, keys, cap, mark);
  window.__cap = {
    caption(t) {
      if (!t) { cap.classList.remove('on'); return; }
      cap.querySelector('.txt').textContent = t;
      cap.classList.add('on');
    },
    showMark() { mark.classList.add('on'); },
  };
  addEventListener('mousemove', e => {
    cursor.style.left = e.clientX + 'px'; cursor.style.top = e.clientY + 'px';
    ripple.style.left = e.clientX + 'px'; ripple.style.top = e.clientY + 'px';
  }, { capture: true, passive: true });
  addEventListener('mousedown', () => {
    ripple.classList.remove('go'); void ripple.offsetWidth; ripple.classList.add('go');
  }, { capture: true, passive: true });
  let hideTimer, buf = '';
  const NAME = { ' ': 'Space', Enter: '⏎ Enter', Escape: 'Esc', Shift: '⇧ Shift',
    Meta: '⌘', ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→' };
  const KEYNAME = { ' ': 'space', Enter: '⏎', Escape: 'esc', Shift: '⇧',
    Meta: '⌘', ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→' };
  const clearRow = () => { keys.querySelectorAll('.cap-key').forEach(k => k.remove()); };
  addEventListener('keydown', e => {
    const printable = e.key.length === 1 && e.key !== ' ';
    if (KEYCAPS) {
      // one bordered keycap per press — reads as a keyboard, not as
      // letters popping up; the typed value accumulates as a key row
      const k = document.createElement('span');
      k.className = 'cap-key';
      if (printable) k.textContent = /[a-z]/.test(e.key) ? e.key.toUpperCase() : e.key;
      else { k.textContent = KEYNAME[e.key] ?? e.key; if (e.key === 'Enter') k.classList.add('enter'); }
      if (!printable && e.key !== 'Enter') clearRow();
      keys.appendChild(k);
      while (keys.querySelectorAll('.cap-key').length > 10) keys.querySelector('.cap-key').remove();
      keys.classList.add('on');
      clearTimeout(hideTimer);
      hideTimer = setTimeout(() => { keys.classList.remove('on'); setTimeout(clearRow, 200); }, 2600);
      return;
    }
    let label;
    if (printable) {
      if (buf.length < 14) buf += e.key;
      label = buf.replace(/</g, '&lt;');
    } else if (e.key === 'Enter' && buf) {
      // keep the typed value on screen; append the confirm instead of
      // replacing it (viewers lost "20,14" the instant Enter landed)
      label = buf.replace(/</g, '&lt;') + ' ⏎'; buf = '';
    } else {
      label = NAME[e.key] ?? e.key; buf = '';
    }
    chip.innerHTML = '<b>' + label + '</b>';
    chip.classList.add('on');
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => { chip.classList.remove('on'); buf = ''; }, 2600);
  }, { capture: true });
  // a typed value always starts after a click — reset the buffer there so
  // the long-lived chip doesn't glue a tool hotkey onto the value ("r20,14")
  addEventListener('mousedown', () => {
    buf = '';
    if (KEYCAPS) { keys.classList.remove('on'); setTimeout(clearRow, 200); }
  }, { capture: true, passive: true });
  }
})();`;
};

const ease = t => t < .5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;

export async function startCapture({ out, headless = true, units = 'Centimeters',
                                     sample = null,
                                     hud = process.env.CAPTURE_HUD ?? 'bottom',
                                     theme = process.env.CAPTURE_THEME ?? 'light' }) {
  // Contention gate: Playwright records wall-clock, so a busy machine
  // stretches every gesture (measured 8-15% at load ~6). Refuse to start a
  // capture that would bake the stretch into the published clip.
  const load = loadavg()[0];
  const maxLoad = Number(process.env.CAPTURE_MAX_LOAD ?? 3);
  if (load > maxLoad) {
    throw new Error(`1-min load average ${load.toFixed(1)} exceeds ${maxLoad} — ` +
      `captures stretch under CPU contention. Wait for a quiet machine, or ` +
      `override with CAPTURE_MAX_LOAD (pilot/layout runs only, never goldens).`);
  }
  mkdirSync(out, { recursive: true });
  const browser = await chromium.launch({ headless });
  const ctx = await browser.newContext({
    viewport: { width: W, height: H },
    recordVideo: { dir: out, size: { width: W, height: H } },
  });
  const page = await ctx.newPage();
  await page.addInitScript(`try { localStorage.setItem('hew.settings.theme', ${JSON.stringify(theme)}) } catch {}`);
  await page.addInitScript(overlayFor(theme, hud));
  await page.goto(APP);
  await page.getByRole('button', { name: 'Start modeling' }).waitFor({ timeout: 20000 });
  await page.locator('select').first().selectOption({ label: units });
  if (sample) {
    await page.getByText(sample, { exact: false }).first().click();
    await page.waitForTimeout(3500);   // sample fetch + tessellate + frame
  } else {
    await page.getByRole('button', { name: 'Start modeling' }).click();
    await page.waitForTimeout(1200);
  }
  const dismiss = page.getByRole('button', { name: 'Dismiss' });
  if (await dismiss.isVisible().catch(() => false)) await dismiss.click();

  const t0 = Date.now();
  const beats = {};
  const cur = { x: W / 2, y: H / 2 };
  await page.mouse.move(cur.x, cur.y);

  const h = {
    page, browser, ctx, beats,
    mark(name) { beats[name] = (Date.now() - t0) / 1000; console.log('BEAT', name, beats[name]); },
    async glide(x, y, ms = 700) {
      const steps = Math.max(12, Math.round(ms / 16));
      const from = { ...cur };
      for (let i = 1; i <= steps; i++) {
        const t = ease(i / steps);
        await page.mouse.move(from.x + (x - from.x) * t, from.y + (y - from.y) * t);
        await page.waitForTimeout(ms / steps);
      }
      cur.x = x; cur.y = y;
    },
    async typeSlow(text, delay = 120) {
      for (const ch of text) { await page.keyboard.type(ch); await page.waitForTimeout(delay); }
    },
    async click() { await page.mouse.down(); await page.mouse.up(); },
    async orbit(dx, dy, ms = 1200) {
      await page.mouse.down({ button: 'middle' });
      const steps = Math.round(ms / 16), from = { ...cur };
      for (let i = 1; i <= steps; i++) {
        const t = ease(i / steps);
        await page.mouse.move(from.x + dx * t, from.y + dy * t);
        await page.waitForTimeout(ms / steps);
      }
      await page.mouse.up({ button: 'middle' });
      cur.x = from.x + dx; cur.y = from.y + dy;
    },
    async caption(text, holdMs = 400) {
      await page.evaluate(t => window.__cap.caption(t), text);
      if (holdMs) await page.waitForTimeout(holdMs);
    },
    async showMark() { await page.evaluate(() => window.__cap.showMark()); },
    async iso() { await page.getByRole('button', { name: 'Iso', exact: true }).click(); await page.waitForTimeout(900); },
    // glide the visible cursor to a tool-rail entry and click it (rail
    // entries are role=radio, and the on-camera cursor travel reads well)
    async clickRail(label) {
      const r = await page.evaluate(l => {
        const b = [...document.querySelectorAll('button')].find(x => (x.textContent || '').trim() === l);
        if (!b) return null;
        const q = b.getBoundingClientRect();
        return { x: q.x + q.width / 2, y: q.y + q.height / 2 };
      }, label);
      if (!r) throw new Error(`rail entry not found: ${label}`);
      await h.glide(r.x, r.y, 600);
      await h.click();
      await page.waitForTimeout(250);
    },
    // fail fast: assert the status-bar solid badge, e.g. expectBadge('2 objects')
    async expectBadge(prefix, beat) {
      const t = await page.getByText(/✓ solid|leaky/).last().innerText().catch(() => '(no badge)');
      console.log('BADGE', JSON.stringify(t), 'after', beat);
      if (!t.startsWith(prefix)) {
        await page.screenshot({ path: `${process.env.CAPTURE_OUT ?? '.'}/FAILED-${beat}.png` }).catch(() => {});
        throw new Error(`beat "${beat}": expected badge "${prefix}…", got "${t}"`);
      }
    },
    async finish(outDir) {
      await page.screenshot({ path: `${outDir}/final-frame.png` });
      await ctx.close();
      const video = await page.video().path();
      writeFileSync(`${outDir}/capture-meta.json`,
        JSON.stringify({ video, beats, width: W, height: H }, null, 2));
      console.log('VIDEO', video);
      await browser.close();
      return video;
    },
  };
  return h;
}