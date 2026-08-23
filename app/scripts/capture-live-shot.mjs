/**
 * capture-live-shot — take documentation-standard screenshots of a Hew session
 * you drive by hand.
 *
 * It opens the dev app in a real browser window at 1440x900 with a device pixel
 * ratio of 2 — the same frame capture-docs-screenshots.mjs uses — then waits.
 * You model, orbit and open whatever panels the shot needs; every time you press
 * Enter here, the page is captured to a 2880x1800 PNG that is pixel-for-pixel
 * the same kind of image as the generated shots under site/public/docs/.
 *
 * The device pixel ratio is the browser's, not the screen's, so this produces
 * true 2x images even on a non-Retina display, where a screen capture of the
 * desktop app can only be upsampled (see tools/macos-ui/hew-shot.sh).
 *
 * Usage:
 *   pnpm --dir app dev                               # or the 4173 preview server
 *   pnpm exec node scripts/capture-live-shot.mjs [outDir] [options]
 *
 *   --url URL        app to open (default: probes localhost and 127.0.0.1
 *                    on 5173 then 4173 — on macOS those two names do not
 *                    always resolve to the same interface)
 *   --size WxH       CSS viewport (default 1440x900)
 *   --scale N        device pixel ratio (default 2)
 *   --format F       png (default), jpeg, or webp. PNG is the docs standard;
 *                    jpeg/webp are for hero/marketing shots where file size
 *                    matters (webp shells out to cwebp — `brew install webp`)
 *   --quality N      jpeg/webp quality 0-100 (default 90; ignored for png)
 *   --delay S        seconds to wait before each capture (default 0)
 *   --cursor         draw the mouse cursor into the shot (see below)
 *   --light          light theme instead of dark
 *   --welcome        keep the welcome overlay up, as on a first launch
 *
 * At the `shot>` prompt:
 *   <name>           capture <name>.png
 *   <name> <secs>    capture after a one-off delay, overriding --delay
 *   <Enter>          repeat the last name with a numeric suffix
 *   cursor on|off    toggle the cursor overlay
 *   delay <secs>     change the standing delay
 *   q                quit
 *
 * SHOOTING THE CURSOR. A page screenshot is a render of the document, so the
 * real pointer — an OS compositor layer — is never in it. With --cursor the
 * script injects an overlay that tracks the pointer and draws it into the page,
 * where the capture can see it. It isn't a lookalike: it reads the computed
 * `cursor` of whatever is under the pointer, so the viewport's tool cursors
 * (built as `url(data:image/svg+xml,…)` values by src/tools/toolIcons.ts, copy
 * badge and all) are drawn from the very same image the app is showing, on the
 * same hotspot. Only the plain CSS keywords (arrow, hand, I-beam, resize) are
 * stand-ins, drawn to match their macOS shapes.
 *
 * Pair it with --delay to catch the pointer mid-gesture: press Enter, then take
 * hold of the mouse and be part-way through the push/pull drag when the shutter
 * goes. The overlay sits on top of the real pointer, so during the session you
 * are seeing double; only the drawn one lands in the PNG.
 */
import { chromium } from '@playwright/test'
import { mkdirSync, existsSync, rmSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createInterface } from 'node:readline/promises'
import { setTimeout as sleep } from 'node:timers/promises'

const APP_DIR = dirname(dirname(fileURLToPath(import.meta.url)))

const argv = process.argv.slice(2)
const flag = (name) => {
  const i = argv.indexOf(name)
  if (i === -1) return undefined
  return argv.splice(i, 2)[1]
}
const bool = (name) => {
  const i = argv.indexOf(name)
  if (i === -1) return false
  argv.splice(i, 1)
  return true
}

const url = flag('--url')
const size = flag('--size') ?? '1440x900'
const scale = Number(flag('--scale') ?? 2)
const format = flag('--format') ?? 'png'
const quality = Number(flag('--quality') ?? 90)
let delay = Number(flag('--delay') ?? 0)
let cursor = bool('--cursor')
const light = bool('--light')
const welcome = bool('--welcome')
const OUT = resolve(argv[0] ?? resolve(APP_DIR, '../site/public/docs'))

if (!['png', 'jpeg', 'webp'].includes(format))
  throw new Error(`--format wants png, jpeg, or webp, got ${format}`)
if (!Number.isFinite(quality) || quality < 0 || quality > 100)
  throw new Error('--quality wants 0-100')
if (format === 'webp' && spawnSync('cwebp', ['-version']).error)
  throw new Error('--format webp needs cwebp on the PATH (brew install webp)')
const EXT = format === 'jpeg' ? 'jpg' : format

const m = /^(\d+)[xX](\d+)$/.exec(size)
if (!m) throw new Error(`--size wants WxH, got ${size}`)
const viewport = { width: Number(m[1]), height: Number(m[2]) }
if (!Number.isFinite(delay) || delay < 0) throw new Error('--delay wants seconds')

mkdirSync(OUT, { recursive: true })

// ---------------------------------------------------------------------------
// Cursor overlay
// ---------------------------------------------------------------------------

/**
 * Installed into the page (and reinstalled after any reload). Mirrors the live
 * pointer into a DOM element so a page screenshot can see it.
 *
 * Keyword cursors are drawn here; anything the app sets as `url(...)` is used
 * verbatim, hotspot included, so tool cursors come out exactly right.
 */
function installCursorOverlay() {
  if (window.__shotCursor) return
  const NS = 'http://www.w3.org/2000/svg'

  // macOS-shaped stand-ins for the CSS keywords, in their own coordinate space
  // with the hotspot given in the same units. Each is drawn dark over a white
  // halo, the same trick toolIcons.ts uses, so it reads on any background.
  const halo = (body) =>
    `<g fill="#111" stroke="#fff" stroke-width="1.6" stroke-linejoin="round" paint-order="stroke fill">${body}</g>`
  const stroked = (body) =>
    `<g fill="none" stroke="#fff" stroke-width="3.4" stroke-linecap="round">${body}</g>` +
    `<g fill="none" stroke="#111" stroke-width="1.4" stroke-linecap="round">${body}</g>`

  const ARROW = halo('<polygon points="1,1 1,23.4 6.6,18.5 10.1,26.9 13.6,25.5 10.4,17.4 17.1,17.4"/>')
  const HAND = halo(
    '<path d="M9 2.5a1.5 1.5 0 0 1 3 0V10h1V7.2a1.5 1.5 0 0 1 3 0V10h1V8.5a1.5 1.5 0 0 1 3 0V10h1a1.5 1.5 0 0 1 1.5 1.5v4.7c0 3.5-2.9 6.3-6.4 6.3h-2.2a6.3 6.3 0 0 1-5.1-2.6L5 15.3a1.5 1.5 0 0 1 2.3-1.9L9 15z"/>',
  )
  const ARROW2 = halo('<polygon points="0,8 5,2.5 5,5.6 13,5.6 13,2.5 18,8 13,13.5 13,10.4 5,10.4 5,13.5"/>')
  const CROSS = stroked('<path d="M11 1v20M1 11h20"/>')
  const IBEAM = stroked('<path d="M3.5 1.5h7M7 1.5v17M3.5 18.5h7"/>')
  const MOVE = halo(
    '<polygon points="12,0 16.5,5 13.4,5 13.4,10.4 18.8,10.4 18.8,7.3 24,12 18.8,16.7 18.8,13.6 13.4,13.6 13.4,19 16.5,19 12,24 7.5,19 10.6,19 10.6,13.6 5.2,13.6 5.2,16.7 0,12 5.2,7.3 5.2,10.4 10.6,10.4 10.6,5 7.5,5"/>',
  )
  const NO = halo('<path d="M11 1a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm0 3.2a6.8 6.8 0 0 1 4 1.3l-9.5 9.5a6.8 6.8 0 0 1 5.5-10.8zm0 13.6a6.8 6.8 0 0 1-4-1.3l9.5-9.5a6.8 6.8 0 0 1-5.5 10.8z"/>')

  const svg = (body, w, h) =>
    `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="${NS}" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">${body}</svg>`)}`

  const rotated = (deg) => svg(`<g transform="rotate(${deg} 12 12) translate(3 4)">${ARROW2}</g>`, 24, 24)

  const KEYWORDS = {
    default: { src: svg(ARROW, 19, 28), hx: 1, hy: 1 },
    pointer: { src: svg(HAND, 25, 24), hx: 10.5, hy: 1 },
    text: { src: svg(IBEAM, 14, 20), hx: 7, hy: 10 },
    crosshair: { src: svg(CROSS, 22, 22), hx: 11, hy: 11 },
    move: { src: svg(MOVE, 24, 24), hx: 12, hy: 12 },
    'ew-resize': { src: rotated(0), hx: 12, hy: 12 },
    'col-resize': { src: rotated(0), hx: 12, hy: 12 },
    'ns-resize': { src: rotated(90), hx: 12, hy: 12 },
    'row-resize': { src: rotated(90), hx: 12, hy: 12 },
    'nwse-resize': { src: rotated(45), hx: 12, hy: 12 },
    'nesw-resize': { src: rotated(-45), hx: 12, hy: 12 },
    'not-allowed': { src: svg(NO, 22, 22), hx: 11, hy: 11 },
    grab: { src: svg(HAND, 25, 24), hx: 12, hy: 12 },
    grabbing: { src: svg(HAND, 25, 24), hx: 12, hy: 12 },
  }
  KEYWORDS.auto = KEYWORDS.default
  KEYWORDS.none = null

  const host = document.createElement('div')
  host.setAttribute('data-shot-cursor', '')
  host.style.cssText =
    'position:fixed;left:0;top:0;width:0;height:0;pointer-events:none;z-index:2147483647;display:none'
  const img = document.createElement('img')
  img.style.cssText = 'position:absolute;left:0;top:0;max-width:none'
  host.appendChild(img)

  let lastSpec = null

  /** Resolve the effective cursor at a point into an image plus its hotspot.
   * `cursor` is an inherited property, so the computed value of the topmost
   * element is already the one the browser is honouring. */
  function specAt(x, y) {
    const el = document.elementFromPoint(x, y)
    const value = el ? getComputedStyle(el).cursor : 'default'
    const url = /url\((['"]?)(.*?)\1\)(?:\s+([\d.]+)\s+([\d.]+))?/.exec(value)
    if (url) return { src: url[2], hx: Number(url[3] ?? 0), hy: Number(url[4] ?? 0) }
    // Fall back through the comma-separated list to the first keyword we draw.
    for (const part of value.split(',')) {
      const key = part.trim()
      if (key in KEYWORDS) return KEYWORDS[key]
    }
    return KEYWORDS.default
  }

  function draw(x, y) {
    const spec = specAt(x, y)
    if (!spec) {
      host.style.display = 'none'
      return
    }
    if (!lastSpec || spec.src !== lastSpec.src) img.src = spec.src
    lastSpec = spec
    host.style.display = ''
    host.style.transform = `translate(${x - spec.hx}px, ${y - spec.hy}px)`
  }

  let x = -1000
  let y = -1000
  const onMove = (e) => {
    x = e.clientX
    y = e.clientY
    draw(x, y)
  }
  // Capture phase, so a handler that stops propagation can't blind the overlay,
  // and pointermove specifically so a drag with pointer capture still reports.
  window.addEventListener('pointermove', onMove, { capture: true, passive: true })
  window.addEventListener('mousemove', onMove, { capture: true, passive: true })
  document.addEventListener('pointerleave', () => (host.style.display = 'none'), true)

  window.__shotCursor = {
    enable() {
      document.body.appendChild(host)
      // Re-resolve at the last known point: a tool change swaps the cursor
      // without the pointer having moved.
      if (x >= 0) draw(x, y)
    },
    disable() {
      host.remove()
    },
    refresh() {
      if (host.isConnected && x >= 0) draw(x, y)
    },
  }
}

// ---------------------------------------------------------------------------
// Find the app
// ---------------------------------------------------------------------------

/** Candidate dev-server URLs, most likely first.
 *
 * Both spellings of loopback are tried, because on macOS they are not the same
 * interface: `pnpm dev` leaves Vite on its default host, which binds [::1] only,
 * so `localhost` answers and `127.0.0.1` is refused outright. Only an explicit
 * `--host 127.0.0.1` (what capture-docs-screenshots.mjs asks for on 4173) binds
 * IPv4. The extra 517x ports are Vite's own fallbacks when 5173 is taken. */
function candidates() {
  if (url) return [url]
  const out = []
  for (const port of [5173, 5174, 5175, 4173])
    for (const host of ['localhost', '127.0.0.1']) out.push(`http://${host}:${port}/`)
  return out
}

async function answers(candidate) {
  try {
    const res = await fetch(candidate, { signal: AbortSignal.timeout(1500) })
    return res.ok
  } catch {
    return false
  }
}

const browser = await chromium.launch({ headless: false })
const page = await browser.newPage({ viewport, deviceScaleFactor: scale })

await page.addInitScript(
  ([theme, showWelcome]) => {
    // Seed before any app module loads, exactly as the generated shots do: the
    // theme drives both the CSS chrome and the WebGL clear colour.
    localStorage.setItem('hew.settings.theme', theme)
    localStorage.setItem('hew.settings.showWelcome', showWelcome)
  },
  [light ? 'light' : 'dark', String(welcome)],
)
await page.addInitScript(installCursorOverlay)

const rl = createInterface({ input: process.stdin, output: process.stdout })

const found = []
for (const candidate of candidates()) if (await answers(candidate)) found.push(candidate)

// Several dev servers can be up at once — a stale one from another worktree on
// 5173 alongside today's on 5174 is easy to end up with, and shooting the wrong
// build is not something you'd catch in the PNG. Pick deliberately.
if (found.length > 1) {
  console.log('\n  more than one dev server is answering:')
  found.forEach((c, n) => console.log(`    ${n + 1}. ${c}`))
  const pick = Number((await rl.question('  which? [1] ')).trim() || '1')
  if (Number.isInteger(pick) && pick >= 2 && pick <= found.length) found.unshift(found.splice(pick - 1, 1)[0])
}

// Node's fetch and the browser can disagree about a hostname, so a candidate
// that answered the probe still has to survive a real navigation.
let base = null
for (const candidate of found) {
  try {
    await page.goto(candidate)
    base = candidate
    break
  } catch (err) {
    console.log(`  ${candidate} answered but wouldn't load in the browser (${err.message.split('\n')[0]})`)
  }
}
if (!base) {
  rl.close()
  await browser.close()
  throw new Error(`no dev server at ${candidates().join(', ')} — start one, or pass --url`)
}

// Hold the prompt until the kernel has loaded, so an eager first Enter can't
// capture the loading screen. The semantic harness only exists in dev builds;
// against anything else, fall through after the timeout.
await page
  .waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 30_000 })
  .catch(() => console.log('  (no test harness on this build — check the window before capturing)'))

const setCursor = async (on) => {
  cursor = on
  await page.evaluate((enable) => window.__shotCursor?.[enable ? 'enable' : 'disable'](), on).catch(() => {})
}
// A reload drops the overlay element; the init script reinstalls the API, so
// just switch it back on.
page.on('load', () => {
  if (cursor) setCursor(true)
})
if (cursor) await setCursor(true)

console.log(`\n  ${base} at ${viewport.width}x${viewport.height} @${scale}x`)
console.log(`  shots land in ${OUT} at ${viewport.width * scale}x${viewport.height * scale}`)
console.log(`  cursor ${cursor ? 'on' : 'off'}, delay ${delay}s`)
console.log('  <name> [secs] to capture, Enter to repeat, "cursor on|off", "delay <secs>", q to quit\n')

let last = null

// Queue every line rather than reading with rl.question(): a question only
// catches a line while it is pending, so anything typed during a countdown
// would be dropped on the floor. Installed after the server-choice question so
// that answer doesn't land in the queue.
const queued = []
let waiting = null
rl.setPrompt('shot> ')
rl.on('line', (line) => {
  if (waiting) {
    const resolve = waiting
    waiting = null
    resolve(line)
  } else {
    queued.push(line)
  }
})
const nextLine = () =>
  queued.length ? Promise.resolve(queued.shift()) : new Promise((resolve) => (waiting = resolve))

// A page that has gone away (window closed) should end the session rather than
// throw on the next capture.
let alive = true
const stop = () => {
  alive = false
  if (waiting) {
    const resolve = waiting
    waiting = null
    resolve(null)
  }
}
page.on('close', () => {
  stop()
  rl.close()
})
rl.on('close', stop)

while (alive) {
  rl.prompt()
  const line = await nextLine()
  if (line === null) break // stdin closed, or the browser window went away
  const answer = line.trim()
  if (answer === 'q' || answer === 'quit') break

  const toggle = /^cursor\s+(on|off)$/i.exec(answer)
  if (toggle) {
    await setCursor(toggle[1].toLowerCase() === 'on')
    console.log(`  cursor ${cursor ? 'on' : 'off'}`)
    continue
  }
  const restanding = /^delay\s+([\d.]+)$/i.exec(answer)
  if (restanding) {
    delay = Number(restanding[1])
    console.log(`  delay ${delay}s`)
    continue
  }

  // "<name> <secs>" — a one-off delay for this shot only.
  let wait = delay
  let name = answer
  const withDelay = /^(.*\S)\s+([\d.]+)$/.exec(answer)
  if (withDelay) {
    name = withDelay[1]
    wait = Number(withDelay[2])
  }

  const repeat = !name
  if (repeat) name = last
  if (!name) {
    console.log('  give the first shot a name')
    continue
  }
  name = name.replace(/\.(png|jpe?g|webp)$/i, '')
  if (repeat) {
    // Bump to the first free numeric suffix rather than overwrite.
    let n = 2
    while (existsSync(`${OUT}/${name}-${n}.${EXT}`)) n++
    name = `${name}-${n}`
  } else {
    last = name
  }

  if (wait > 0) {
    for (let left = Math.ceil(wait); left > 0; left--) {
      process.stdout.write(`  ${left}… `)
      await sleep(Math.min(1, wait - (Math.ceil(wait) - left)) * 1000)
    }
    process.stdout.write('\n')
  }

  try {
    // The pointer may have crossed into a different cursor region while we
    // waited without generating a move event we saw; re-resolve before firing.
    if (cursor) await page.evaluate(() => window.__shotCursor?.refresh())
    const out = `${OUT}/${name}.${EXT}`
    if (format === 'webp') {
      // Playwright captures png and jpeg only; capture png, hand it to cwebp.
      const tmp = `${OUT}/.${name}.capture.png`
      await page.screenshot({ path: tmp })
      const res = spawnSync('cwebp', ['-q', String(quality), '-m', '6', tmp, '-o', out])
      rmSync(tmp, { force: true })
      if (res.status !== 0) throw new Error(`cwebp failed: ${res.stderr}`)
    } else if (format === 'jpeg') {
      await page.screenshot({ path: out, type: 'jpeg', quality })
    } else {
      await page.screenshot({ path: out })
    }
    console.log(`  wrote ${out}`)
  } catch (err) {
    console.log(`  capture failed: ${err.message}`)
  }
}

rl.close()
await browser.close()
