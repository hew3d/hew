import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { WALL_SKIP, buildUnderTest, isProductionShape, previewFeatures, type RegistryFeature } from './helpers/preview'
import { settleFrame } from './helpers/render'
import { dumpPluginStore, seedPluginStore } from './helpers/pluginStore'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * The Preview wall in the browser (docs/design/preview-wall-build.md section
 * 3, layer 3). Run against the PRODUCTION-SHAPE build (`HEW_PREVIEW=0`: `vite
 * build` with the flag off, served by `vite preview`; the harness stays in so
 * the spec can build models). It walks `preview-features.json`: for every
 * entry, nothing of it reaches a regular user.
 *
 * In the preview-build run (`HEW_PREVIEW=1`) the production block skips and the
 * second block checks the other side: the mark, Settings > Preview, the tags.
 *
 * No feature is named here. A feature added to the registry is covered by
 * the next run.
 */

const HERE = dirname(fileURLToPath(import.meta.url))
const features = previewFeatures()
const FILLETED_BOX = Array.from(readFileSync(resolve(HERE, 'fixtures/filleted-box.hew')))

/** Escape a name for a whole-word, case-sensitive pattern. */
const word = (s: string): RegExp => new RegExp(`(^|[^A-Za-z])${s.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&')}([^A-Za-z]|$)`)

/** Every string that names a feature to a person: its name and its tools. */
const namesOf = (f: RegistryFeature): string[] => [f.name, ...f.tools]

const SRC = (rel: string): string => readFileSync(resolve(HERE, '../src', rel), 'utf8')

/** The chords a command had by default in a preview build (read from the
 *  defaults file, which is the same source in both builds). */
function defaultChords(id: string): string[] {
  const m = new RegExp(`'${id}':\\s*\\[([^\\]]*)\\]`).exec(SRC('keymap/defaults.ts'))
  if (m === null) return []
  return [...m[1].matchAll(/'([^']+)'/g)].map((x) => x[1])
}

/** A palette search term for each way a person could look for the entry: its
 *  name, its tools, and the synonyms its commands carry in the palette source. */
function searchTerms(f: RegistryFeature): string[] {
  const palette = SRC('palette/registry.ts') + SRC('palette/curveEntries.ts')
  const terms = new Set<string>([f.name, ...f.tools])
  for (const tool of f.tools) {
    const m = new RegExp(`'${tool}':\\s*\\[([^\\]]*)\\]`).exec(palette)
    if (m !== null) for (const x of m[1].matchAll(/t`([^`]+)`/g)) terms.add(x[1])
  }
  for (const id of f.actions) {
    const at = palette.indexOf(`id: '${id}'`)
    if (at < 0) continue
    const end = palette.indexOf('}', at)
    const syn = /synonyms:\s*\[([^\]]*)\]/.exec(palette.slice(at, end + 1))
    if (syn !== null) for (const x of syn[1].matchAll(/t`([^`]+)`/g)) terms.add(x[1])
    const label = /label:\s*t`([^`]+)`/.exec(palette.slice(at, end + 1))
    if (label !== null) terms.add(label[1])
  }
  return [...terms]
}

async function boot(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 90_000 })
}

/** The rail's checked tool. */
const activeTool = (page: Page) => page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { checked: true })

async function openPalette(page: Page) {
  await page.getByRole('button', { name: 'Search tools, actions, help' }).click()
  const palette = page.getByRole('dialog', { name: /command palette/i })
  await expect(palette).toBeVisible()
  return palette
}

/** The text of every open menu entry of every top-level menu in the bar. */
async function allMenuText(page: Page): Promise<string> {
  const bar = page.getByTestId('menu-bar')
  const triggers = await bar.getByRole('button').all()
  const seen: string[] = []
  for (const trigger of triggers) {
    const label = (await trigger.getAttribute('aria-label')) ?? (await trigger.textContent()) ?? ''
    if (/settings/i.test(label)) continue
    await trigger.click()
    seen.push(await bar.innerText())
    await page.keyboard.press('Escape')
    await bar.click({ position: { x: 2, y: 2 } }).catch(() => undefined)
  }
  return seen.join('\n')
}

test.describe('production-shape build', () => {
  test.skip(!isProductionShape, `${WALL_SKIP}the production block runs against the production-shape build (HEW_PREVIEW=0); this run is ${buildUnderTest}`)

  test.beforeEach(async ({ page }) => {
    await boot(page)
  })

  test('runs on the production flavor of the WASM module', async ({ page }) => {
    const flavor = await page.evaluate(() => window.__hew_test!.wasmFlavor())
    test.skip(flavor === 'unknown', `${WALL_SKIP}waiting on the rebuilt WASM: app/src/wasm/pkg has no preview_build() (it predates the Rust gate)`)
    expect(flavor).toBe('production')
  })

  test('is a production-shape build: no Preview mark, no Settings > Preview, no Preview in the title', async ({ page }) => {
    await expect(page.getByTestId('preview-mark')).toHaveCount(0)
    expect(await page.title()).not.toMatch(/preview/i)
    await page.getByRole('button', { name: 'Settings' }).click()
    const tabs = page.getByRole('tab')
    await expect(tabs.first()).toBeVisible()
    const names = await tabs.allTextContents()
    expect(names.length).toBeGreaterThan(5)
    expect(names.map((n) => n.toLowerCase())).not.toContain('preview')
    for (const f of features) expect(names, f.name).not.toContain(f.name)
    // The controls list carries no withheld command either.
    await page.getByRole('tab', { name: 'Controls' }).click()
    const pane = page.getByRole('tabpanel')
    for (const f of features) for (const n of namesOf(f)) await expect(pane.getByText(word(n))).toHaveCount(0)
  })

  test('browser storage a plugin store would use is never read, written or listed, and Settings has no Plugins tab', async ({ page }) => {
    // What a preview build's plugin store leaves behind: a record, a file, a
    // staging entry and a settings key. The production app must not know the
    // store exists, so after a fresh load every byte is where it was.
    await seedPluginStore(page)
    const before = await dumpPluginStore(page)
    expect(Object.keys(before)).toHaveLength(5)
    await page.reload()
    await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 90_000 })
    await page.getByRole('button', { name: 'Settings' }).click()
    const names = (await page.getByRole('tab').allTextContents()).map((n) => n.toLowerCase())
    expect(names.length).toBeGreaterThan(5)
    expect(names).not.toContain('plugins')
    // Even with the switch stored on, a production build shows nothing.
    await page.evaluate(() => localStorage.setItem('hew.settings.preview', JSON.stringify({ off: [], on: ['plugins'] })))
    await page.reload()
    await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 90_000 })
    await page.getByRole('button', { name: 'Settings' }).click()
    expect((await page.getByRole('tab').allTextContents()).map((n) => n.toLowerCase())).not.toContain('plugins')
    expect(await dumpPluginStore(page)).toEqual(before)
    await page.evaluate(() => localStorage.removeItem('hew.settings.preview'))
  })

  for (const f of features) {
    test.describe(`entry "${f.id}" (${f.name})`, () => {
      test('is on no surface the app builds its lists from', async ({ page }) => {
        const s = await page.evaluate(() => window.__hew_test!.wallSurfaces())
        for (const t of f.tools) {
          for (const list of [s.rail, s.tools]) expect(list, t).not.toContain(t)
        }
        for (const id of f.actions) {
          for (const list of [s.toolCommands, s.palette, s.nativeCommands, s.bound]) expect(list, id).not.toContain(id)
        }
        for (const id of [...f.dockChips, ...f.actions]) expect(s.dock, id).not.toContain(id)
      })

      test('has no rail button, and nothing of it in any menu', async ({ page }) => {
        const rail = await page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio').evaluateAll((els) => els.map((e) => e.getAttribute('aria-label')))
        for (const t of f.tools) expect(rail, t).not.toContain(t)
        const menus = await allMenuText(page)
        for (const n of namesOf(f)) expect(menus, n).not.toMatch(word(n))
      })

      test('is not found by palette search under its name, its tools or its synonyms', async ({ page }) => {
        const gated = new Set([...f.actions])
        const palette = await openPalette(page)
        const input = palette.getByPlaceholder(/search/i)
        for (const term of searchTerms(f)) {
          // The ranking the UI uses, over the entries this build has.
          const ids = await page.evaluate((q) => window.__hew_test!.paletteSearch(q), term)
          for (const id of ids) expect(gated.has(id), `"${term}" found ${id}`).toBe(false)
        }
        // And by real typing: the rows shown never carry its name or a tool's.
        for (const term of [f.name, ...f.tools]) {
          await input.fill(term)
          const rows = await palette.getByRole('option').allTextContents()
          for (const row of rows) for (const n of namesOf(f)) expect(row, `"${term}" row`).not.toMatch(word(n))
        }
      })

      test('its default shortcut does not arm it, and a saved binding does nothing', async ({ page }) => {
        for (const id of f.actions) {
          for (const chord of defaultChords(id)) {
            const mapped = chord.replace(/^Mod\+/, 'Control+').replace(/\+Mod\+/, '+Control+')
            if (chord.includes('Mod+')) continue // a Cmd-combo of a command with a native item; the keymap check below covers it
            await page.mouse.move(400, 300)
            await page.keyboard.press(mapped)
            await expect(activeTool(page)).toHaveAccessibleName('Select')
            expect(await page.evaluate((c) => window.__hew_test!.commandForChord(c), chord), `${id}: ${chord}`).toBeUndefined()
          }
        }
        // A binding saved while the feature was on (the setting survives the build): ignored.
        await page.evaluate((ids) => {
          const chords = ['Shift+J', 'Shift+K', 'Shift+Y', 'Shift+U', 'Shift+X', 'Shift+V', 'Shift+N', 'Shift+H', 'Shift+G', 'Shift+D', 'Shift+W', 'Shift+Z']
          const saved: Record<string, string[]> = {}
          ids.forEach((id, i) => { saved[id] = [chords[i % chords.length]] })
          localStorage.setItem('hew.settings.keybindings', JSON.stringify(saved))
        }, f.actions)
        await page.reload()
        await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 90_000 })
        for (const chord of ['Shift+J', 'Shift+K', 'Shift+Y', 'Shift+U', 'Shift+X', 'Shift+V', 'Shift+N', 'Shift+H', 'Shift+G', 'Shift+D', 'Shift+W', 'Shift+Z']) {
          const owner = await page.evaluate((c) => window.__hew_test!.commandForChord(c), chord)
          expect(f.actions, `${chord} runs ${owner}`).not.toContain(owner)
        }
        await page.keyboard.press('Shift+J')
        await expect(activeTool(page)).toHaveAccessibleName('Select')
      })

      test('has no chip on the dock for a selected solid or a selected sketch', async ({ page }) => {
        const labels = (): Promise<string[]> =>
          page.getByTestId('contextual-dock').getByRole('button').evaluateAll((els) => els.map((e) => `${e.getAttribute('title') ?? ''} ${e.textContent ?? ''}`))
        await page.evaluate(() => {
          const h = window.__hew_test!
          const id = h.drawBox([0, 0, 0], [1, 1, 0], 1)
          h.selectObjects([id])
        })
        await expect(page.getByTestId('contextual-dock')).toBeVisible()
        for (const text of await labels()) for (const n of namesOf(f)) expect(text, 'solid').not.toMatch(word(n))
        await page.evaluate(() => {
          const h = window.__hew_test!
          for (const id of h.getObjectIds()) h.deleteObject(id)
          h.drawRectangle([3, 0, 0], [4, 1, 0])
          h.selectAll()
        })
        await expect(page.getByTestId('contextual-dock')).toBeVisible()
        for (const text of await labels()) for (const n of namesOf(f)) expect(text, 'sketch').not.toMatch(word(n))
      })

      test('its API commands refuse with preview_only', async ({ page }) => {
        test.skip(f.apiCommands.length === 0, `${WALL_SKIP}the entry names no API command`)
        // The refusal comes from the Rust side. A module built before that
        // gate existed (it has no preview_build()) cannot give it: wait for
        // the rebuilt WASM rather than fake the answer.
        const flavor = await page.evaluate(() => window.__hew_test!.wasmFlavor())
        test.skip(flavor === 'unknown', `${WALL_SKIP}waiting on the rebuilt WASM: app/src/wasm/pkg has no preview_build() (it predates the Rust gate)`)
        expect(flavor, 'the production-shape build must run the production WASM module').toBe('production')
        for (const method of f.apiCommands) {
          const reply = await page.evaluate((m) => window.__hew_test!.apiDispatch({ method: m, params: {} }), method)
          expect(JSON.stringify(reply.error ?? reply.result), method).toContain('preview_only')
        }
      })
    })
  }

  test('a file with a fillet in it opens, renders and saves', async ({ page }) => {
    await settleFrame(page)
    const empty = await page.locator('canvas').first().screenshot()
    const before = await page.evaluate((bytes) => {
      const h = window.__hew_test!
      h.load(bytes)
      return { objects: h.getObjectIds().length, blends: h.blendFeatures(h.getObjectIds()[0]).length, solid: h.isObjectSolid(h.getObjectIds()[0]), hash: h.getModelStateHash() }
    }, FILLETED_BOX)
    expect(before.objects).toBe(1)
    expect(before.blends).toBeGreaterThan(0)
    expect(before.solid).toBe(true)
    await page.evaluate(() => window.__hew_test!.setCamera({ position: [4, -4, 3], target: [1, 0.5, 0.5], up: [0, 0, 1], fovDeg: 45 }))
    await settleFrame(page)
    const drawn = await page.locator('canvas').first().screenshot()
    expect(drawn.equals(empty)).toBe(false)
    // Saved bytes read back to the same model, fillet included.
    const after = await page.evaluate(() => {
      const h = window.__hew_test!
      const saved = h.save()
      h.load(saved)
      return { saved: saved.length, blends: h.blendFeatures(h.getObjectIds()[0]).length, hash: h.getModelStateHash() }
    })
    expect(after.saved).toBeGreaterThan(1000)
    expect(after.blends).toBe(before.blends)
    expect(after.hash).toBe(before.hash)
  })
  test('a loader root is not there: its hash shows the ordinary app, stored switch or not', async ({ page }) => {
    for (const f of features.filter((x) => x.loader !== undefined)) {
      await page.evaluate((id) => localStorage.setItem('hew.settings.preview', JSON.stringify({ off: [], on: [id] })), f.id)
      for (const root of f.loader!.roots) {
        await page.goto(`/#${root}`)
        await page.reload()
        await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 90_000 })
        await expect(page.getByTestId('menu-bar')).toBeVisible()
        await expect(page.locator('#probe-mount')).toHaveCount(0)
      }
      await page.evaluate(() => localStorage.removeItem('hew.settings.preview'))
    }
  })
})

test.describe('preview build', () => {
  test.skip(isProductionShape, `${WALL_SKIP}the preview block runs against the preview build (HEW_PREVIEW=1)`)

  test.beforeEach(async ({ page }) => {
    await boot(page)
  })

  test('shows the Preview mark, with its tooltip', async ({ page }) => {
    const mark = page.getByTestId('preview-mark')
    await expect(mark).toBeVisible()
    await expect(mark).toHaveAttribute('title', /unfinished features/)
  })

  test('Settings > Preview lists every feature with a switch, on unless it loads code', async ({ page }) => {
    await page.getByRole('button', { name: 'Settings' }).click()
    await page.getByRole('tab', { name: 'Preview' }).click()
    const pane = page.getByRole('tabpanel')
    for (const f of features) {
      await expect(pane.getByText(f.name, { exact: true })).toBeVisible()
      await expect(pane.getByText(f.state, { exact: false })).toBeVisible()
      await expect(pane.locator(`[data-preview-switch="${f.id}"]`)).toBeChecked({ checked: f.defaultOn !== false })
    }
  })

  test('a loader root shows the ordinary app until its feature is switched on, then its own page after a reload', async ({ page }) => {
    for (const f of features.filter((x) => x.loader !== undefined)) {
      for (const root of f.loader!.roots) {
        await page.goto(`/#${root}`)
        await page.reload()
        await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 90_000 })
        await expect(page.getByTestId('menu-bar')).toBeVisible()
        // The switch, through the real Settings pane; it applies at the next start.
        await page.getByRole('button', { name: 'Settings' }).click()
        await page.getByRole('tab', { name: 'Preview' }).click()
        await page.getByRole('tabpanel').locator(`[data-preview-switch="${f.id}"]`).check()
        await expect(page.getByTestId('menu-bar')).toBeVisible()
        await page.goto(`/#${root}?rows=none`)
        await page.reload()
        await expect(page.getByTestId('menu-bar')).toHaveCount(0)
        await expect(page.locator('#probe-mount')).toBeAttached()
        await page.evaluate(() => localStorage.removeItem('hew.settings.preview'))
      }
    }
  })

  test('the rail tooltip of a Preview tool carries the tag, and the tool is there', async ({ page }) => {
    for (const f of features) {
      for (const t of f.tools) {
        const spec = await page.evaluate((name) => window.__hew_test!.wallSurfaces().rail.includes(name), t)
        if (!spec) continue
        await expect(page.getByRole('radio', { name: t, exact: true })).toHaveAttribute('title', /· Preview$/)
      }
    }
    await expect(page.getByRole('radio', { name: 'Push/Pull', exact: true })).not.toHaveAttribute('title', /Preview/)
  })

  test('runs on the preview flavor of the WASM module', async ({ page }) => {
    const flavor = await page.evaluate(() => window.__hew_test!.wasmFlavor())
    test.skip(flavor === 'unknown', `${WALL_SKIP}waiting on the rebuilt WASM: app/src/wasm/pkg has no preview_build() (it predates the Rust gate)`)
    expect(flavor).toBe('preview')
  })

  test('every registry tool is on a surface in this build', async ({ page }) => {
    const s = await page.evaluate(() => window.__hew_test!.wallSurfaces())
    for (const f of features) {
      for (const t of f.tools) expect(s.tools, t).toContain(t)
      for (const id of f.actions) expect(s.palette, id).toContain(id)
    }
  })
})
