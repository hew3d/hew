// Capture harness for the launch reel: the wordless, chrome-free, fast-cut
// clip on the hew3d.com landing page and (as an animated excerpt) the README.
//
// Differs from tools/docs-videos/capture-lib.mjs on purpose:
//   - the app chrome (menu bar, tool rail, panels, status bar, viewport HUD,
//     contextual dock) is hidden by injected CSS so the canvas fills the
//     whole 1920x1080 frame — the reel shows geometry, not UI;
//   - no captions, key chips, or watermark: only the cursor and click ripple;
//   - hotkey-less commands (booleans, standard views, axes/grid) are fired
//     through the hidden menu bar with h.menu('Object', 'Subtract');
//   - models load by dropping a .hew onto the app (h.loadHew(path)), which is
//     the app's own drag-and-drop path — no file picker;
//   - h.mark(name) records cut points; assemble-reel.py trims each shot
//     between marks and concatenates the shots into one clip.
//
// Every shot is its own script (shots/*.mjs) and its own capture, so a shot
// can be re-taken without re-running the rest.

import { createRequire } from 'module';
const require = createRequire(new URL('../../app/package.json', import.meta.url));
const { chromium } = require('@playwright/test');
import { mkdirSync, rmSync, writeFileSync, readFileSync } from 'fs';
import { basename } from 'path';
import { loadavg } from 'os';

export const APP = 'http://localhost:5199/';
export const W = 1920, H = 1080;

// Chrome to hide. The shell is inline-styled divs, so the panel column and
// status bar are addressed by position: <main> is [menu bar, row, status
// bar] and the row is [tool rail, viewport, resize handle, right panel]. The viewport HUD
// is the absolute div holding the Orbit/Top/Iso/Front chips.
// startCapture asserts the canvas really is W×H afterwards, so a shell
// re-layout fails loudly here instead of producing a half-cropped reel.
const CHROME_CSS = `
  [data-testid="menu-bar"],
  [role="radiogroup"][aria-label="Tools"],
  [data-testid="contextual-dock"],
  [role="separator"][aria-label="Resize panels"],
  main > div:nth-child(2) > div:last-child,
  main > div:nth-child(3),
  div:has(> button[title="Orbit"]) { display: none !important; }
`;

const OVERLAY = `
(() => {
  if (window.__reelOverlay) return; window.__reelOverlay = true;
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
      #cap-ripple.go { animation: capRip .4s ease-out; }
      @keyframes capRip { 0% { opacity: .85; transform: scale(.35); }
                          100% { opacity: 0; transform: scale(1.1); } }\`;
    document.head.appendChild(css);
    const cursor = document.createElement('div'); cursor.id = 'cap-cursor';
    cursor.innerHTML = '<svg viewBox="0 0 24 24" width="22" height="22">'
      + '<path d="M4 2 L4 19 L8.6 15.2 L11.4 21.4 L14.2 20.1 L11.5 14 L17.5 13.6 Z"'
      + ' fill="#F3EDE3" stroke="#1B1A17" stroke-width="1.4"/></svg>';
    const ripple = document.createElement('div'); ripple.id = 'cap-ripple';
    document.body.append(cursor, ripple);
    addEventListener('mousemove', e => {
      cursor.style.left = e.clientX + 'px'; cursor.style.top = e.clientY + 'px';
      ripple.style.left = e.clientX + 'px'; ripple.style.top = e.clientY + 'px';
    }, { capture: true, passive: true });
    addEventListener('mousedown', () => {
      ripple.classList.remove('go'); void ripple.offsetWidth; ripple.classList.add('go');
    }, { capture: true, passive: true });
  }
})();`;

const ease = t => t < .5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;

/**
 * Launch the app, hide its chrome, and return the gesture helpers.
 *   out      capture directory (video + capture-meta.json + stills)
 *   units    welcome-screen units selection
 *   sample   bundled sample to open instead of a blank document
 *   model    absolute path of a .hew to drop onto the blank document
 */
export async function startCapture({ out, units = 'Centimeters', sample = null, model = null,
                                     chrome = false, headless = true }) {
  const load = loadavg()[0];
  const maxLoad = Number(process.env.CAPTURE_MAX_LOAD ?? 3);
  if (load > maxLoad) {
    throw new Error(`1-min load average ${load.toFixed(1)} exceeds ${maxLoad} — captures ` +
      `stretch under CPU contention. Wait for a quiet machine, or override with ` +
      `CAPTURE_MAX_LOAD (layout runs only, never the final take).`);
  }
  rmSync(out, { recursive: true, force: true });   // one take per directory
  mkdirSync(out, { recursive: true });
  // Headless Chromium defaults to SwiftShader (software GL), which stalls
  // the event loop on large models and stretches every gesture. These flags
  // hand rendering to the real GPU through ANGLE/Metal; measured 2.5x faster
  // orbits on the theater model, and the recorded frames are identical.
  const browser = await chromium.launch({ headless, args: [
    '--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist', '--enable-gpu-rasterization',
  ] });
  const ctx = await browser.newContext({
    viewport: { width: W, height: H },
    recordVideo: { dir: out, size: { width: W, height: H } },
  });
  const page = await ctx.newPage();
  await page.addInitScript(`try { localStorage.setItem('hew.settings.theme', 'dark') } catch {}`);
  await page.addInitScript(OVERLAY);
  await page.goto(APP);
  await page.getByRole('button', { name: 'Start modeling' }).waitFor({ timeout: 20000 });
  await page.locator('select').first().selectOption({ label: units });
  if (sample) {
    await page.getByText(sample, { exact: false }).first().click();
    await page.waitForTimeout(3500);
  } else {
    await page.getByRole('button', { name: 'Start modeling' }).click();
    await page.waitForTimeout(1000);
  }
  const dismiss = page.getByRole('button', { name: 'Dismiss' });
  if (await dismiss.isVisible().catch(() => false)) await dismiss.click();

  // The chrome-hiding stylesheet is a single element so it can be pulled
  // back out mid-shot: hideChrome() / showChrome() below. `chrome: true`
  // starts with the UI visible (shots that work the panels).
  const hideChrome = async () => {
    await page.evaluate(css => {
      if (document.getElementById('reel-chrome')) return;
      const el = document.createElement('style');
      el.id = 'reel-chrome'; el.textContent = css;
      document.head.appendChild(el);
    }, CHROME_CSS);
    await page.waitForTimeout(600);
    const canvas = await page.evaluate(() => {
      const r = document.querySelector('canvas').getBoundingClientRect();
      return { x: r.x, y: r.y, w: r.width, h: r.height };
    });
    if (canvas.x !== 0 || canvas.y !== 0 || canvas.w !== W || canvas.h !== H) {
      throw new Error(`chrome not fully hidden: canvas is ${JSON.stringify(canvas)}, expected ${W}x${H} at 0,0`);
    }
  };
  const showChrome = async () => {
    await page.evaluate(() => document.getElementById('reel-chrome')?.remove());
    await page.waitForTimeout(600);
  };
  if (!chrome) await hideChrome();

  const t0 = Date.now();
  const marks = {};
  const cur = { x: W / 2, y: H / 2 };
  await page.mouse.move(cur.x, cur.y);

  // The status bar is hidden but still in the DOM, so its solid badge is
  // still the assertion of record after any mutating beat.
  const badge = () => page.evaluate(() => {
    const leaves = [...document.querySelectorAll('span, div')].filter(e => e.children.length === 0);
    const hits = leaves.map(e => (e.textContent || '').trim()).filter(t => /✓ solid|leaky/.test(t));
    return hits.length ? hits[hits.length - 1] : '(no badge)';
  });

  const h = {
    page, browser, ctx, marks, hideChrome, showChrome,
    // screen center of an element, for gliding the real cursor onto UI
    async center(locator) {
      const b = await locator.boundingBox();
      if (!b) throw new Error('element not visible: ' + String(locator));
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    },
    // glide to a UI element and click it (the rail, a panel row, a button)
    async clickUi(locator, ms = 350) {
      const c = await h.center(locator);
      await h.glide(c.x, c.y, ms);
      await h.click();
      await page.waitForTimeout(200);
    },
    mark(name) { marks[name] = (Date.now() - t0) / 1000; console.log('MARK', name, marks[name].toFixed(2)); },
    async wait(ms) { await page.waitForTimeout(ms); },
    // Gesture steps run at ~30 Hz, not per frame: each mouse.move is a CDP
    // round trip that waits on a render, so on a heavy model a 60 Hz loop
    // stretches a 3 s orbit to 10 s of wall clock (and of recording).
    async glide(x, y, ms = 300) {
      const steps = Math.max(8, Math.round(ms / 33));
      const from = { ...cur };
      for (let i = 1; i <= steps; i++) {
        const t = ease(i / steps);
        await page.mouse.move(from.x + (x - from.x) * t, from.y + (y - from.y) * t);
        await page.waitForTimeout(ms / steps);
      }
      cur.x = x; cur.y = y;
    },
    async type(text, delay = 70) {
      for (const ch of text) { await page.keyboard.type(ch); await page.waitForTimeout(delay); }
    },
    async key(k, after = 180) { await page.keyboard.press(k); await page.waitForTimeout(after); },
    async click() { await page.mouse.down(); await page.mouse.up(); },
    // Drags ease in and out by default (a hand gesture); `linear: true`
    // holds a constant rate for a long camera move such as a full lap.
    // The viewport's orbit turns 360° per viewport-height of drag (1080 px
    // here) and damps with a ~1.5 s tail, so a move reads as finished
    // about that long after the button comes up.
    async drag(button, dx, dy, ms, { linear = false } = {}) {
      await page.mouse.down({ button });
      const steps = Math.max(8, Math.round(ms / 33)), from = { ...cur };
      for (let i = 1; i <= steps; i++) {
        const t = linear ? i / steps : ease(i / steps);
        await page.mouse.move(from.x + dx * t, from.y + dy * t);
        await page.waitForTimeout(ms / steps);
      }
      await page.mouse.up({ button });
      cur.x = from.x + dx; cur.y = from.y + dy;
    },
    async orbit(dx, dy, ms = 1200, opts) { await h.drag('middle', dx, dy, ms, opts); },
    async pan(dx, dy, ms = 1000, opts) { await h.drag('right', dx, dy, ms, opts); },
    // camera pose from the dev build's test hook (null in a build without it):
    // polar 90° is horizontal, azimuth turns with the lap
    async camera() {
      return page.evaluate(() => {
        const t = window.__hew_test;
        if (!t) return null;
        const c = t.getCamera();
        const d = [c.position[0] - c.target[0], c.position[1] - c.target[1], c.position[2] - c.target[2]];
        const r = Math.hypot(...d);
        return { target: c.target.map(v => +v.toFixed(2)), height: +c.position[2].toFixed(2), dist: +r.toFixed(2),
          polarDeg: +(Math.acos(d[2] / r) * 180 / Math.PI).toFixed(1),
          azDeg: +(Math.atan2(d[1], d[0]) * 180 / Math.PI).toFixed(1) };
      });
    },
    // wheel zoom toward the cursor; negative = zoom in
    async zoom(notches, step = 120, gap = 90) {
      for (let i = 0; i < Math.abs(notches); i++) {
        await page.mouse.wheel(0, notches < 0 ? -step : step);
        await page.waitForTimeout(gap);
      }
    },
    // Fire a menu-bar command through the hidden menu bar. The trigger is a
    // real button (click), but the dropdown items act on mousedown and mix
    // a check glyph, the label text node, and a shortcut span, so each item
    // is matched on its own text nodes. Intermediate labels are submenus
    // (Camera > Standard Views > Iso): a submenu mounts its flyout only
    // while hovered, so those get a mouseover first. The bar is
    // display:none, so none of this shows on camera.
    async menu(menuLabel, ...path) {
      const err = await page.evaluate(m => {
        const bar = document.querySelector('[data-testid="menu-bar"]');
        const btn = [...bar.querySelectorAll('button')].find(b => (b.textContent || '').trim() === m);
        if (!btn) return `menu not found: ${m}`;
        btn.click();
        return null;
      }, menuLabel);
      if (err) throw new Error(err);
      await page.waitForTimeout(60);
      for (let i = 0; i < path.length; i++) {
        const last = i === path.length - 1;
        const err2 = await page.evaluate(([label, last, trail]) => {
          const bar = document.querySelector('[data-testid="menu-bar"]');
          const own = e => [...e.childNodes].filter(n => n.nodeType === 3).map(n => n.textContent.trim()).join('');
          const el = [...bar.querySelectorAll('*')].find(e => own(e) === label);
          if (!el) return `item not found: ${trail}`;
          const types = last ? ['mousedown', 'mouseup', 'click'] : ['mouseover'];
          for (const type of types) {
            el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0 }));
          }
          return null;
        }, [path[i], last, [menuLabel, ...path.slice(0, i + 1)].join(' > ')]);
        if (err2) { await page.keyboard.press('Escape'); throw new Error(err2); }
        await page.waitForTimeout(last ? 120 : 60);
      }
    },
    // Set a View-menu check item (Axes, Grid, Guides) to a known state by
    // reading its ✓ glyph — a blind toggle can be undone by a document's own
    // saved view state (a scene loading after the toggle, say).
    async view(label, on) {
      await page.evaluate(() => {
        const bar = document.querySelector('[data-testid="menu-bar"]');
        [...bar.querySelectorAll('button')].find(b => (b.textContent || '').trim() === 'View').click();
      });
      await page.waitForTimeout(60);
      const changed = await page.evaluate(([label, on]) => {
        const bar = document.querySelector('[data-testid="menu-bar"]');
        const own = e => [...e.childNodes].filter(n => n.nodeType === 3).map(n => n.textContent.trim()).join('');
        const el = [...bar.querySelectorAll('*')].find(e => own(e) === label);
        if (!el) return `item not found: View > ${label}`;
        const checked = (el.textContent || '').includes('✓');
        if (checked === on) return false;
        for (const type of ['mousedown', 'mouseup', 'click']) {
          el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0 }));
        }
        return true;
      }, [label, on]);
      if (typeof changed === 'string') { await page.keyboard.press('Escape'); throw new Error(changed); }
      if (!changed) await page.keyboard.press('Escape');
      await page.waitForTimeout(120);
    },
    // Hide the inference tooltip chip ("Ground", "Endpoint"…) for hero
    // orbits, where a label riding the cursor over a finished model is
    // clutter. Modeling shots keep it — the snap labels are part of the
    // draw-on-anything feel. The chip carries no id or class, so it is
    // matched by its inline style signature; the viewport's "Editing …" and
    // "Inserting …" status pills share that signature and go with it, which
    // is the right call for a chrome-free frame.
    async hideInference() {
      await page.addStyleTag({ content: `
        main > div:nth-child(2) > div:nth-child(2) >
          div[style*="pointer-events: none"][style*="border-radius: 7px"][style*="white-space: nowrap"]
          { display: none !important; }` });
    },
    // show or hide the on-camera cursor (a long multi-drag orbit reads
    // better without a cursor jumping back between drags)
    async cursor(show) {
      await page.evaluate(v => { document.getElementById('cap-cursor').style.display = v ? '' : 'none'; }, show);
    },
    // the usual hero-shot cleanup: no axes, no grid, no inference chip
    async clean() { await h.view('Axes', false); await h.view('Grid', false); await h.hideInference(); },
    async iso() { await h.menu('Camera', 'Standard Views', 'Iso'); await page.waitForTimeout(700); },
    async zoomExtents() { await h.menu('Camera', 'Zoom Extents'); await page.waitForTimeout(700); },
    // Drop a .hew onto the app — the real drag-and-drop open path. Resolves
    // once the document title carries the file name, then waits `settle` ms
    // for tessellation and the first frame (a fully grouped model shows no
    // solid badge, so the title is the load signal of record).
    async loadHew(path, settle = 1500) {
      const b64 = readFileSync(path).toString('base64');
      const name = basename(path);
      const dt = await page.evaluateHandle(([b64, name]) => {
        const bin = atob(b64);
        const arr = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
        const dt = new DataTransfer();
        dt.items.add(new File([arr], name));
        return dt;
      }, [b64, name]);
      // the drop handler lives on the viewport container, not <main>
      await page.dispatchEvent('main > div:nth-child(2) > div:nth-child(2)', 'drop', { dataTransfer: dt });
      const deadline = Date.now() + 120000;
      while (Date.now() < deadline) {
        if ((await page.title()).startsWith(name)) {
          await page.waitForTimeout(settle);
          console.log('LOADED', name, await badge());
          return;
        }
        await page.waitForTimeout(200);
      }
      throw new Error(`loadHew(${name}): document title never changed`);
    },
    async expectBadge(prefix, beat) {
      const t = await badge();
      console.log('BADGE', JSON.stringify(t), 'after', beat);
      if (!t.startsWith(prefix)) {
        await page.screenshot({ path: `${out}/FAILED-${beat}.png` }).catch(() => {});
        throw new Error(`beat "${beat}": expected badge "${prefix}…", got "${t}"`);
      }
    },
    async shot(name) {
      if (process.env.DEBUG_SHOTS) await page.screenshot({ path: `${out}/still-${name}.png` });
    },
    async finish() {
      h.mark('end');
      await page.screenshot({ path: `${out}/final-frame.png` });
      await ctx.close();
      const video = await page.video().path();
      writeFileSync(`${out}/capture-meta.json`,
        JSON.stringify({ video, marks, width: W, height: H }, null, 2));
      console.log('VIDEO', video);
      await browser.close();
      return video;
    },
  };
  return h;
}

/** Output directory for a shot script: tools/launch-reel/out/<shot>/ (gitignored). */
export const outDir = shot =>
  process.env.CAPTURE_OUT ?? new URL(`./out/${shot}`, import.meta.url).pathname;
