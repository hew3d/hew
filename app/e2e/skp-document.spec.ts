import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * `.skp` as a DOCUMENT format — the real pipeline in a real browser with the
 * real WASM kernel: File ▸ Open of a SketchUp file makes that file the
 * document's own (title `box.skp`, clean), Ctrl+S writes it back in ITS
 * container through the picked file handle, Save As offers Hew / SketchUp /
 * SketchUp 2017 through an in-app format dialog, and Export writes either
 * `.skp` flavor as a copy.
 *
 * The File System Access pickers are stubbed (as unified-open.spec.ts and
 * stl-import.spec.ts do): the open picker serves a fixture `.skp` through a
 * handle whose `createWritable` CAPTURES what is written, and the save
 * picker hands back a capturing handle for whatever name the test chose.
 * Everything else — menus, dialogs, tools, keyboard shortcuts — is real UI.
 */

// The stream releases, newest to oldest (src/io/docFormat.ts; not imported
// here because that module needs the lingui macro transform).
const SKP_STREAM_FORMATS = [
  'skp2020', 'skp2019', 'skp2018', 'skp2017', 'skp2016', 'skp2015', 'skp2014',
  'skp2013', 'skp8', 'skp7', 'skp6', 'skp5', 'skp4', 'skp3',
] as const

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
    __skp?: SkpStubState
  }
}

interface WrittenFile {
  /** Which handle received the write: the opened file or a save-picker one. */
  handle: string
  name: string
  bytes: number[]
}

interface SkpStubState {
  openName: string
  openBytes: number[]
  /** Name the next save picker will return. */
  saveName: string
  written: WrittenFile[]
  saveCalls: Array<{ suggestedName: string; extensions: string[] }>
  openCalls: number
}

const fixture = (relPath: string): number[] =>
  Array.from(readFileSync(resolve(process.cwd(), relPath)))

const BOX_2017 = 'e2e/fixtures/skp/box.skp'
const BOX_2013 = 'e2e/fixtures/skp/box-v2013.skp'
const BOX_2026 = 'e2e/fixtures/skp/2026/box.skp'

/** UTF-16LE "SketchUp Model" header every `.skp` (both containers) begins with. */
const SKP_HEADER = [0xff, 0xfe, 0xff, 0x0e, 0x53, 0x00, 0x6b, 0x00, 0x65, 0x00, 0x74, 0x00]

const startsWith = (bytes: number[], prefix: number[]): boolean =>
  prefix.every((b, i) => bytes[i] === b)
const isSkp = (bytes: number[]): boolean => startsWith(bytes, SKP_HEADER)
/** A `.hew` is a zip ("PK"). */
const isHew = (bytes: number[]): boolean => startsWith(bytes, [0x50, 0x4b])

/** Install capturing stubs for both File System Access pickers. */
async function installPickers(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const state: SkpStubState = {
      openName: '',
      openBytes: [],
      saveName: 'Saved.skp',
      written: [],
      saveCalls: [],
      openCalls: 0,
    }
    window.__skp = state
    const handleFor = (id: string, name: string, bytes: () => number[]) => ({
      kind: 'file',
      name,
      getFile: async () => {
        const buf = new Uint8Array(bytes())
        return { name, arrayBuffer: async () => buf.buffer }
      },
      queryPermission: async () => 'granted',
      requestPermission: async () => 'granted',
      createWritable: async () => {
        const chunks: number[] = []
        return {
          write: async (data: ArrayBufferView | ArrayBuffer) => {
            const view = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
            for (const b of view) chunks.push(b)
          },
          close: async () => {
            state.written.push({ handle: id, name, bytes: chunks })
          },
        }
      },
    })
    let openHandleSeq = 0
    // @ts-expect-error test override of the File System Access API
    window.showOpenFilePicker = async () => {
      state.openCalls += 1
      const name = state.openName
      const snapshot = state.openBytes
      return [handleFor(`open-${++openHandleSeq}`, name, () => snapshot)]
    }
    // @ts-expect-error test override of the File System Access API
    window.showSaveFilePicker = async (opts: {
      suggestedName?: string
      types?: Array<{ accept: Record<string, string[]> }>
    }) => {
      state.saveCalls.push({
        suggestedName: opts.suggestedName ?? '',
        extensions: (opts.types ?? []).flatMap((t) => Object.values(t.accept).flat()),
      })
      return handleFor(`save-${state.saveCalls.length}`, state.saveName, () => [])
    }
  })
}

async function boot(page: Page): Promise<void> {
  await installPickers(page)
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 20_000 })
}

async function serve(page: Page, name: string, bytes: number[]): Promise<void> {
  await page.evaluate(([n, b]) => {
    window.__skp!.openName = n as string
    window.__skp!.openBytes = b as number[]
  }, [name, bytes] as [string, number[]])
}

/** File ▸ Open… through the real menu, then dismiss the open report. */
async function openViaMenu(page: Page, name: string, bytes: number[]): Promise<void> {
  await serve(page, name, bytes)
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Open…', { exact: true }).click()
  // Opened as the document's own file, not imported: the report says so.
  const report = page.getByRole('dialog', { name: /open report/i })
  await expect(report).toBeVisible({ timeout: 20_000 })
  await expect(report).toContainText('Open Complete')
  await report.getByRole('button', { name: 'OK' }).click()
  await expect(report).toHaveCount(0)
}

const objectCount = (page: Page): Promise<number> =>
  page.evaluate(() => window.__hew_test!.getObjectCount())

const written = (page: Page): Promise<WrittenFile[]> => page.evaluate(() => window.__skp!.written)

const containerOf = (page: Page, bytes: number[]): Promise<string> =>
  page.evaluate((b) => window.__hew_test!.skpContainer(b), bytes)

/** Draw a rectangle on the ground and push/pull it into a solid, with real
 * tools and real pointer events over a pinned camera. Adds one Object. */
async function drawAndPush(page: Page): Promise<void> {
  await page.evaluate(() =>
    window.__hew_test!.setCamera({ position: [14, -16, 12], target: [3, 3, 0], fovDeg: 45 }),
  )
  await page.waitForTimeout(200)
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  const toPage = async (world: [number, number, number]) => {
    const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), world)
    return { x: canvas.x + p.x, y: canvas.y + p.y }
  }
  const hover = async (world: [number, number, number]) => {
    const p = await toPage(world)
    await page.mouse.move(p.x - 6, p.y - 4)
    await page.waitForTimeout(50)
    await page.mouse.move(p.x, p.y)
    await page.waitForTimeout(160)
    return p
  }
  const click = async (world: [number, number, number]) => {
    await hover(world)
    await page.mouse.down()
    await page.mouse.up()
    await page.waitForTimeout(150)
  }

  // Rectangle tool: two corners on the ground, well clear of the fixture box.
  await page.keyboard.press('r')
  await click([6, 6, 0])
  await click([8, 8, 0])
  // Push/Pull tool: click the new face, then drag up and click to commit.
  await page.keyboard.press('p')
  await page.locator('text=Click a face').first().waitFor({ timeout: 5000 })
  await hover([7, 7, 0])
  await page.mouse.down()
  await page.mouse.up()
  const up = await toPage([7, 7, 1.5])
  await page.mouse.move(up.x, up.y, { steps: 10 })
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(200)
}

for (const [label, path, expected] of [
  ['2013 container', BOX_2013, 'skp2013'],
  ['2017 container', BOX_2017, 'skp2017'],
  ['2026 container', BOX_2026, 'skp2026'],
] as const) {
  test(`open ${label}: titled box.skp and clean; edit, Ctrl+S writes the same container back; reopen has the edit`, async ({ page }) => {
    await boot(page)
    const bytes = fixture(path)
    expect(await containerOf(page, bytes)).toBe(expected)

    await openViaMenu(page, 'box.skp', bytes)
    await expect(page).toHaveTitle(/^box\.skp — Hew$/)
    const before = await objectCount(page)
    expect(before).toBeGreaterThan(0)

    await drawAndPush(page)
    await expect.poll(() => objectCount(page)).toBe(before + 1)
    await expect(page).toHaveTitle(/^• box\.skp — Hew$/)

    await page.keyboard.press('ControlOrMeta+s')
    await expect(page).toHaveTitle(/^box\.skp — Hew$/)
    const files = await written(page)
    expect(files).toHaveLength(1)
    const saved = files[0]!
    expect(saved.handle).toMatch(/^open-/) // in place, through the opened handle
    expect(isSkp(saved.bytes)).toBe(true)
    expect(isHew(saved.bytes)).toBe(false)
    expect(await containerOf(page, saved.bytes)).toBe(expected)

    // Reopen the written bytes: the edit survived.
    await openViaMenu(page, 'box.skp', saved.bytes)
    await expect(page).toHaveTitle(/^box\.skp — Hew$/)
    expect(await objectCount(page)).toBe(before + 1)

    // A second save is still the same container, still through a handle.
    await page.keyboard.press('ControlOrMeta+s')
    // Nothing dirty -> a save still rewrites; whatever lands is SketchUp, never .hew.
    for (const f of await written(page)) expect(isHew(f.bytes)).toBe(false)
  })
}

test('a .skp the reader refuses leaves the open document as it was', async ({ page }) => {
  await boot(page)
  await openViaMenu(page, 'box.skp', fixture(BOX_2017))
  expect(await objectCount(page)).toBe(1)
  // The same file cut short and stamped as written by a release Skpwright does
  // not list ("{17." -> "{25." in the UTF-16 version record): the body is a
  // stream container, so Skpwright is asked, and it refuses what it cannot
  // read (a whole stamped file would open, with a warning).
  const old = fixture(BOX_2017).slice(0, 80)
  expect(old.slice(36, 44)).toEqual([0x7b, 0, 0x31, 0, 0x37, 0, 0x2e, 0])
  old[38] = 0x32
  old[40] = 0x35
  await serve(page, 'old.skp', old)
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Open…', { exact: true }).click()
  await expect(page.getByText(/unsupported SketchUp version \{25\./)).toBeVisible({ timeout: 20_000 })
  await expect(page.getByRole('dialog', { name: /open report/i })).toHaveCount(0)
  expect(await objectCount(page)).toBe(1)
  await expect(page).toHaveTitle(/^box\.skp/)
})

test('an open on a hidden page, which runs no animation frames, still finishes', async ({ page }) => {
  await boot(page)
  // A background tab or a covered window: hidden, and no frames ever run.
  // The open steps between paints, and waiting for one here would wait
  // until someone looked.
  await page.evaluate(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' })
    window.requestAnimationFrame = () => 0
  })
  await openViaMenu(page, 'box.skp', fixture(BOX_2017))
  expect(await objectCount(page)).toBe(1)
})

test('Save As: the format dialog defaults to the document format; each choice writes that format', async ({ page }) => {
  await boot(page)
  await openViaMenu(page, 'box.skp', fixture(BOX_2017))

  const choose = async (optionName: RegExp, saveName: string, release?: string) => {
    await page.evaluate((n) => { window.__skp!.saveName = n }, saveName)
    await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
    await page.getByText('Save As…', { exact: true }).click()
    const dialog = page.getByRole('dialog', { name: 'Save As' })
    await expect(dialog).toBeVisible()
    await dialog.getByRole('radio', { name: optionName }).check()
    if (release !== undefined) await dialog.getByTestId('save-older-release').selectOption(release)
    await dialog.getByRole('button', { name: 'Continue' }).click()
    await expect(dialog).toHaveCount(0)
  }

  // Default selection = the current document's format (the 2017 file).
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Save As…', { exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'Save As' })
  // The older releases share one entry, its list on the file's own release.
  await expect(dialog.getByRole('radio', { name: /Older SketchUp versions/ })).toBeChecked()
  await expect(dialog.getByTestId('save-older-release')).toHaveValue('skp2017')
  await expect(dialog.getByRole('radio', { name: /^SketchUp 2021 and later/ })).not.toBeChecked()
  // Escape cancels with no picker.
  await page.keyboard.press('Escape')
  await expect(dialog).toHaveCount(0)
  expect(await page.evaluate(() => window.__skp!.saveCalls.length)).toBe(0)

  // SketchUp (2026 container)
  await choose(/^SketchUp 2021 and later/, 'As2026.skp')
  await expect(page).toHaveTitle(/^As2026\.skp — Hew$/)
  let files = await written(page)
  expect(files).toHaveLength(1)
  expect(await containerOf(page, files[0]!.bytes)).toBe('skp2026')
  let calls = await page.evaluate(() => window.__skp!.saveCalls)
  expect(calls[0]!.suggestedName).toBe('box.skp') // never box.skp.hew
  expect(calls[0]!.extensions).toEqual(['.skp'])

  // SketchUp 2017
  await choose(/Older SketchUp versions/, 'As2017.skp', 'skp2017')
  await expect(page).toHaveTitle(/^As2017\.skp — Hew$/)
  files = await written(page)
  expect(files).toHaveLength(2)
  expect(await containerOf(page, files[1]!.bytes)).toBe('skp2017')

  // Hew document
  await choose(/Hew document/, 'As.hew')
  await expect(page).toHaveTitle(/^As\.hew — Hew$/)
  files = await written(page)
  expect(files).toHaveLength(3)
  expect(isHew(files[2]!.bytes)).toBe(true)
  calls = await page.evaluate(() => window.__skp!.saveCalls)
  expect(calls[2]!.suggestedName).toBe('As2017.hew') // extension swapped, not stacked
  expect(calls[2]!.extensions).toEqual(['.hew'])

  // Now bound to a .hew: Ctrl+S writes .hew bytes (never SketchUp) to that handle.
  await page.evaluate(() => window.__hew_test!.drawBox([20, 20, 0], [21, 21, 0], 1))
  await page.keyboard.press('ControlOrMeta+s')
  await expect.poll(async () => (await written(page)).length).toBe(4)
  const last = (await written(page))[3]!
  expect(last.handle).toBe('save-3')
  expect(isHew(last.bytes)).toBe(true)
})

test('Save As of a new document defaults to Hew and writes .hew', async ({ page }) => {
  await boot(page)
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate(() => { window.__skp!.saveName = 'Fresh.hew' })
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Save As…', { exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'Save As' })
  await expect(dialog.getByRole('radio', { name: /Hew document/ })).toBeChecked()
  await page.keyboard.press('Enter')
  await expect(dialog).toHaveCount(0)
  await expect(page).toHaveTitle(/^Fresh\.hew — Hew$/)
  const files = await written(page)
  expect(files).toHaveLength(1)
  expect(isHew(files[0]!.bytes)).toBe(true)
})

test('Export writes every SketchUp flavor as a copy and leaves the document bound to its file', async ({ page }) => {
  await boot(page)
  await openViaMenu(page, 'box.skp', fixture(BOX_2026))

  for (const [option, saveName, container] of [
    ['skp', 'Out2026.skp', 'skp2026'],
    // Every stream release, newest to oldest: the picker's order.
    ...SKP_STREAM_FORMATS.map((f) => [f, `Out-${f}.skp`, f] as const),
  ] as const) {
    await page.evaluate((n) => { window.__skp!.saveName = n }, saveName)
    await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
    await page.getByText('Export…', { exact: true }).click()
    const dialog = page.getByRole('dialog', { name: 'Export' })
    await dialog.getByLabel('Format').selectOption(option)
    await dialog.getByRole('button', { name: 'Export' }).click()
    await expect(dialog).toHaveCount(0)
    await expect.poll(async () => (await written(page)).some((f) => f.name === saveName)).toBe(true)
    const file = (await written(page)).find((f) => f.name === saveName)!
    expect(isSkp(file.bytes)).toBe(true)
    expect(await containerOf(page, file.bytes)).toBe(container)
  }

  const calls = await page.evaluate(() => window.__skp!.saveCalls)
  expect(calls.map((c) => c.suggestedName)).toEqual(Array(1 + SKP_STREAM_FORMATS.length).fill('box.skp'))
  // An export is a copy: still bound to box.skp, still clean.
  await expect(page).toHaveTitle(/^box\.skp — Hew$/)
})

test('the one-time loss notice: shown once per document, "Save a .hew copy" does not rebind, later saves are silent', async ({ page }) => {
  await boot(page)
  await openViaMenu(page, 'box.skp', fixture(BOX_2026))

  // A true circle extruded into a cylinder: a SketchUp file holds only its facets.
  await page.evaluate(() => {
    const h = window.__hew_test!
    const { sketch, region } = h.drawCircle([12, 12, 0], 1)
    h.extrudeRegion(sketch, region, 2)
  })
  const notice = page.getByTestId('skp-loss-notice')
  await expect(notice).toHaveCount(0)

  await page.keyboard.press('ControlOrMeta+s')
  await expect(notice).toBeVisible()
  await expect(notice).toContainText(/curved surfaces/i)
  await expect(page).toHaveTitle(/^box\.skp — Hew$/)

  // "Save a .hew copy": the Save As picker in .hew format; the document stays bound.
  await page.evaluate(() => { window.__skp!.saveName = 'Copy.hew' })
  await notice.getByRole('button', { name: 'Save a .hew copy' }).click()
  await expect.poll(async () => (await written(page)).some((f) => f.name === 'Copy.hew')).toBe(true)
  const copy = (await written(page)).find((f) => f.name === 'Copy.hew')!
  expect(isHew(copy.bytes)).toBe(true)
  expect((await page.evaluate(() => window.__skp!.saveCalls)).at(-1)!.extensions).toEqual(['.hew'])
  await expect(page).toHaveTitle(/^box\.skp — Hew$/)

  // The notice closes once the copy is written. Later saves are silent:
  // edit, save again — it never comes back for this document.
  await expect(notice).toHaveCount(0)
  await page.evaluate(() => window.__hew_test!.drawBox([30, 30, 0], [31, 31, 0], 1))
  await page.keyboard.press('ControlOrMeta+s')
  await expect(page).toHaveTitle(/^box\.skp — Hew$/)
  await expect(notice).toHaveCount(0)

  // Safety: every write that reached the opened file's handle is SketchUp.
  for (const f of await written(page)) {
    if (f.handle.startsWith('open-')) expect(isHew(f.bytes)).toBe(false)
  }
})

test('Export of a model a SketchUp file cannot hold exactly lists what was simplified, without a .hew copy button', async ({ page }) => {
  await boot(page)
  await page.evaluate(() => {
    const h = window.__hew_test!
    const { sketch, region } = h.drawCircle([0, 0, 0], 1)
    h.extrudeRegion(sketch, region, 2)
  })
  await page.evaluate(() => { window.__skp!.saveName = 'Cyl.skp' })
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Export…', { exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'Export' })
  await dialog.getByLabel('Format').selectOption('skp')
  await dialog.getByRole('button', { name: 'Export' }).click()
  const notice = page.getByTestId('skp-loss-notice')
  await expect(notice).toBeVisible()
  await expect(notice.getByRole('button', { name: 'Save a .hew copy' })).toHaveCount(0)
  await notice.getByRole('button', { name: 'Dismiss' }).click()
  await expect(notice).toHaveCount(0)
})

test('a .skp open lands on the web recents shelf and reopens through the open-skp path', async ({ page }) => {
  test.setTimeout(90_000)
  await boot(page)
  await openViaMenu(page, 'box.skp', fixture(BOX_2017))
  // The shelf write is fire-and-forget; let it land before reloading.
  await page.waitForTimeout(500)

  await page.evaluate(() => localStorage.removeItem('hew.settings.showWelcome'))
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 20_000 })

  const welcome = page.getByRole('dialog', { name: /welcome to hew/i })
  await expect(welcome).toBeVisible()
  await welcome.getByText('box.skp').first().click()
  const report = page.getByRole('dialog', { name: /open report/i })
  await expect(report).toBeVisible({ timeout: 20_000 })
  await report.getByRole('button', { name: 'OK' }).click()

  // A document bound to the file (title, clean), not an import-shaped copy.
  await expect(page).toHaveTitle(/^box\.skp — Hew$/)
  expect(await objectCount(page)).toBeGreaterThan(0)
})
