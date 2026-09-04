// Shot: paint — a blank box, built and then painted from gray to a wood
// brown. The Materials panel is chrome-hidden, so the swatch is reached by
// adding a color material through the DOM (same effect as clicking a
// swatch: it becomes the current material) — the paint gesture itself
// (picking up Paint, Ctrl-clicking the object) is a real mouse action.
import { startCapture, outDir } from '../reel-lib.mjs';
const h = await startCapture({ out: outDir('paint') });
const { page } = h;

await h.zoom(-3);
await h.wait(300);
await h.shot('layout');

h.mark('start');
await h.key('r');
await h.glide(830, 560, 300);
await h.click();
await h.glide(1180, 700, 350);
await h.type('20,12');
await h.key('Enter', 500);
await h.key('p');
await h.glide(1000, 630, 250);
await h.click();
await h.glide(1000, 380, 500);
await h.type('8');
await h.key('Enter', 100);
await h.expectBadge('1 object', 'solid');
await h.shot('solid-gray');

// Reach the (chrome-hidden) Materials panel through the DOM: expand it,
// expand "Add color", set a wood-brown hex on the native color input, and
// click "+ Add color" — same swatch-picking effect as MaterialPalette's own
// onClick handlers, without ever needing the invisible panel to be clickable
// by mouse. onMaterialCreated makes the new material current but does NOT
// switch tools, so Paint is picked up with the real hotkey below.
const expandMaterials = await page.evaluate(() => {
  const btn = [...document.querySelectorAll('button')].find(b => (b.textContent || '').trim().startsWith('Materials'));
  if (!btn) return 'missing';
  btn.click();
  return 'ok';
});
if (expandMaterials !== 'ok') throw new Error(`paint: Materials header not found in DOM (${expandMaterials})`);
await page.waitForTimeout(200);

const expandAddColor = await page.evaluate(() => {
  const btn = [...document.querySelectorAll('button')].find(b => (b.textContent || '').trim().startsWith('Add color'));
  if (!btn) return 'missing';
  btn.click();
  return 'ok';
});
if (expandAddColor !== 'ok') throw new Error(`paint: Add color header not found in DOM (${expandAddColor})`);
await page.waitForTimeout(200);

const setColor = await page.evaluate((hex) => {
  const colorInput = document.querySelector('input[type="color"]');
  if (!colorInput) return 'missing';
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  setter.call(colorInput, hex);
  colorInput.dispatchEvent(new Event('input', { bubbles: true }));
  colorInput.dispatchEvent(new Event('change', { bubbles: true }));
  return colorInput.value === hex ? 'ok' : `mismatch:${colorInput.value}`;
}, '#8b5a2b');
if (setColor !== 'ok') throw new Error(`paint: color input not set (${setColor})`);
await page.waitForTimeout(200);

const addColor = await page.evaluate(() => {
  const btn = [...document.querySelectorAll('button')].find(b => (b.textContent || '').trim() === '+ Add color');
  if (!btn) return 'missing';
  if (btn.disabled) return 'disabled';
  btn.click();
  return 'ok';
});
if (addColor !== 'ok') throw new Error(`paint: + Add color button not clickable (${addColor})`);
await page.waitForTimeout(400);

// The visible gesture: pick up Paint, Ctrl-click the box to set its base
// material, watch it turn from gray to wood brown.
await h.key('b');
await h.glide(1150, 650, 350);
await page.keyboard.down('Control');
await h.click();
await page.keyboard.up('Control');
h.mark('painted');
await h.expectBadge('1 object', 'painted');
await h.wait(700);
await h.shot('painted');

await h.key('Escape');
await h.glide(1750, 130, 350);            // park off the geometry
await h.wait(300);
await h.finish();
