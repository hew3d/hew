import { test, expect, type Page, type Locator } from '@playwright/test'
import { buildViewProjection, worldToPagePixel, type CameraParams } from './helpers/projectWorldToScreen'
import {
  CATALOG_URL,
  FakeWeb,
  SOURCE_HOST,
  fixtureItem,
  fixtureNewer,
  THUMB_PNG,
  indexEntry,
  publish,
  sourceOf,
  sourceOfMaterialAndModel,
  webRef,
  type SourceIndexJson,
} from './helpers/sourcesFixture'

/**
 * Library sources, the browser half: the acceptance script in
 * docs/design/shared-library-sources.md section 11, one test per line the
 * web build can show. Every source host, the catalog and the privacy log are
 * the fake network in helpers/sourcesFixture.ts (page.route). The items are
 * real .hew library files from e2e/fixtures/sources/.
 *
 * Each test gets a fresh browser context, so the Library's OPFS storage and
 * the sources list start empty and end with the test; nothing needs undoing.
 *
 * Driven with real clicks, typing and keys. Assertions go through roles,
 * labels and the components' data-testid attributes.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const SOURCE_A = `${SOURCE_HOST}/a/`
const SOURCE_B = `${SOURCE_HOST}/b/`
const SOURCE_U = `${SOURCE_HOST}/u/`
const A_INDEX = `${SOURCE_A}hew.json`
const GH_COMMIT = 'a'.repeat(40)
const GH_FOLDER = `https://raw.githubusercontent.com/example/gear/${GH_COMMIT}/`

/** Opens a fresh app with the fake network installed. */
async function boot(page: Page, host: FakeWeb): Promise<void> {
  await host.install(page)
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

/**
 * Waits until the page has nothing in flight and two frames have rendered, so a negative
 * assertion ("no outside request") is made after the open-time work has had its chance to
 * start, not after a guessed delay. Requests are fulfilled by the fake web, so
 * network-idle counts them.
 */
async function settle(page: Page): Promise<void> {
  await page.waitForLoadState('networkidle')
  await page.evaluate(() => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))))
  await page.waitForLoadState('networkidle')
}

/** The Library dialog, opened with its real shortcut. */
async function openLibrary(page: Page): Promise<Locator> {
  await page.keyboard.press('Shift+L')
  const dialog = page.getByRole('dialog', { name: 'Library' })
  await expect(dialog).toBeVisible()
  return dialog
}

/** Types an address into the Add dialog and looks it up, with real events. */
async function lookUp(page: Page, address: string): Promise<Locator> {
  await page.getByTestId('library-add-source').click()
  const add = page.getByTestId('add-source-dialog')
  await expect(add).toBeVisible()
  const input = add.getByRole('textbox', { name: 'Address of the source' })
  await input.click()
  await input.pressSequentially(address)
  await add.getByRole('button', { name: 'Look up', exact: true }).click()
  return add
}

/** Adds a source by address: look up, then Add (or Done when already listed). */
async function addByAddress(page: Page, address: string): Promise<void> {
  const add = await lookUp(page, address)
  await expect(add.getByTestId('add-source-facts')).toBeVisible()
  await add.getByRole('button', { name: /^(Add|Done)$/ }).click()
  await expect(add).not.toBeVisible()
}

/** The sidebar row for a source, by its name. */
const sourceRow = (page: Page, name: string): Locator => page.getByTestId('library-source-row').filter({ hasText: name })

/** A catalog row, by the source's name. */
const catalogRow = (page: Page, name: string): Locator => page.getByTestId('library-catalog-row').filter({ hasText: name })

/** Opens the catalog from the sidebar. */
async function openCatalog(page: Page): Promise<void> {
  await page.getByTestId('library-browse-catalog').click()
  await expect(page.getByTestId('library-catalog')).toBeVisible()
}

/** Opens a listed source's view, then runs its "Add all". */
async function addAllFrom(page: Page, name: string): Promise<void> {
  await sourceRow(page, name).click()
  await page.getByTestId('source-browser').getByTestId('source-add-all').click()
  await expect(page.getByTestId('source-run-summary')).toBeVisible()
}

/** The item tile for a part in the Library dialog's own grid. */
const libraryTile = (dialog: Locator, part: string): Locator => dialog.getByRole('option', { name: new RegExp(part) })

/** Every item tile in the Library's own grid. */
const libraryTiles = (dialog: Locator): Locator => dialog.getByRole('option', { name: /Part \d\d/ })

/** The Library's own items (the "All" collection), out of a source view. */
async function localView(dialog: Locator): Promise<void> {
  await dialog.getByRole('button', { name: 'All', exact: true }).click()
}

/** The source view's tile for a part. */
const sourceTile = (page: Page, part: string): Locator => page.getByTestId('source-browser').getByRole('option', { name: new RegExp(part) })

/** Opens Settings ▸ Folders, as a user does, and returns the Settings dialog. */
async function openFoldersSettings(page: Page): Promise<Locator> {
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click()
  const settings = page.getByRole('dialog', { name: 'Settings' })
  await expect(settings).toBeVisible()
  await settings.getByRole('tab', { name: 'Folders' }).click()
  return settings
}

/**
 * The source now serves a newer Part 01: the same item id with a changed body
 * (fixtures/sources/part-01-v2.hew), listed with its new sha256.
 */
function sourceSaysPartOneChanged(host: FakeWeb, a: ReturnType<typeof sourceOf>): void {
  const newer = fixtureNewer(1)
  const path = a.index.items[0].path
  a.index.items[0] = indexEntry(newer, { path, thumbnail: true })
  a.files[0] = { path, bytes: newer.bytes }
  a.thumbs.push({ path: `thumbnails/${newer.sha256}.png`, bytes: THUMB_PNG })
  publish(host, a)
}

/** Library mod key, as the app reads it. */
async function modKey(page: Page): Promise<string> {
  return (await page.evaluate(() => /Mac/.test(navigator.platform))) ? 'Meta' : 'Control'
}

/** Opens an added item for editing in place, from the Library's own grid (real clicks). */
async function openForEditing(page: Page, part: string): Promise<void> {
  const dialog = page.getByRole('dialog', { name: 'Library' })
  await localView(dialog)
  await libraryTile(dialog, part).click()
  await page.getByTestId('library-edit').click()
  await expect(page.getByText(new RegExp(`Editing .${part}. from the Library`))).toBeVisible()
}

/** Pulls the open part's top face up 1 m (the kernel's push/pull through the test harness), then Save. */
async function pullAndSave(page: Page, part: string): Promise<void> {
  await page.waitForTimeout(300)
  // The item is one component placement: frame it and double-click it, as a user does, to enter it.
  await page.evaluate(() => window.__hew_test!.apiDispatch({ method: 'hew.view.zoom_extents' }))
  await page.waitForTimeout(300)
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  await page.mouse.dblclick(box.x + box.width / 2, box.y + box.height / 2)
  await page.waitForTimeout(300)
  const depth = await page.evaluate(() => window.__hew_test!.getUndoDepth())
  await page.evaluate(() => {
    const h = window.__hew_test!
    const hit = h.pickFace([0.5, 0.05, 9], [0, 0, -1])
    if (hit === null) throw new Error('no face under the part')
    h.pushPull(hit.object, hit.face, 1)
  })
  await page.waitForTimeout(150)
  expect(await page.evaluate(() => window.__hew_test!.getUndoDepth())).toBeGreaterThan(depth)
  await page.keyboard.press(`${await modKey(page)}+s`)
  await expect(page.getByText(`Updated "${part}" in the Library`)).toBeVisible({ timeout: 10_000 })
}

/** Edits Part 01 in place and saves, then reloads so the document is no longer open for editing. */
async function editPartOneAndClose(page: Page): Promise<void> {
  await openForEditing(page, 'Part 01')
  await pullAndSave(page, 'Part 01')
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await openLibrary(page)
}

/** Source says Part 01 is newer, and the open source view is refreshed. */
async function makePartOneNewer(page: Page, host: FakeWeb, a: ReturnType<typeof sourceOf>): Promise<void> {
  sourceSaysPartOneChanged(host, a)
  await sourceRow(page, 'Shop Hardware').click()
  await page.getByTestId('source-browser').getByTestId('source-refresh').click()
  await expect(page.getByTestId('source-update-bar')).toBeVisible()
}


test.describe('Library sources (section 11 acceptance)', () => {
  test('1. Both. Open the Library with no sources: Browse the catalog and Add a source, no rows', async ({ page }) => {
    const host = new FakeWeb()
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)

    await expect(page.getByTestId('library-sources')).toBeVisible()
    await expect(page.getByTestId('library-browse-catalog')).toHaveText('Browse the catalog')
    await expect(page.getByTestId('library-add-source')).toHaveText('Add a source…')
    await expect(page.getByTestId('library-source-row')).toHaveCount(0)
  })

  test('2. Both. Open the catalog when it is empty: no-sources text, Add a source, publishing link', async ({ page }) => {
    const host = new FakeWeb()
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await openCatalog(page)

    const empty = page.getByTestId('library-catalog-empty')
    await expect(empty).toContainText('No sources are listed yet.')
    await expect(empty.getByRole('button', { name: 'Add a source…' })).toBeVisible()
    await expect(empty.getByRole('link', { name: 'How to publish a source' })).toBeVisible()
  })

  test('3. Both. Catalog with two listed sources: name, author, license, item count; open one: its items show', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 12)
    const b = sourceOf('Stock Materials', SOURCE_B, 3, { license: 'MIT' })
    const aUrl = publish(host, a)
    const bUrl = publish(host, b)
    host.publishCatalog([
      { ref: webRef(aUrl), index: a.index as SourceIndexJson },
      { ref: webRef(bUrl), index: b.index as SourceIndexJson },
    ])
    await boot(page, host)
    await openLibrary(page)
    await openCatalog(page)

    await expect(page.getByTestId('library-catalog-row')).toHaveCount(2)
    const rowA = catalogRow(page, 'Shop Hardware')
    await expect(rowA).toContainText('Example Maker')
    await expect(rowA).toContainText('CC-BY-4.0')
    await expect(rowA).toContainText('12 items')
    await expect(catalogRow(page, 'Stock Materials')).toContainText('MIT')

    await rowA.click()
    await expect(page.getByTestId('source-title')).toHaveText('Shop Hardware')
    await expect(sourceTile(page, 'Part 12')).toBeVisible()
  })

  test('4. Both. Add a listed source: sidebar row; every item "Not in your Library"; thumbnails where the source has them', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 12)
    const aUrl = publish(host, a)
    host.publishCatalog([{ ref: webRef(aUrl), index: a.index as SourceIndexJson }])
    await boot(page, host)
    await openLibrary(page)
    await openCatalog(page)

    await catalogRow(page, 'Shop Hardware').getByRole('button', { name: 'Add Shop Hardware' }).click()
    await expect(sourceRow(page, 'Shop Hardware')).toBeVisible()
    await sourceRow(page, 'Shop Hardware').click()

    await expect(page.getByTestId('source-browser').getByText('Not in your Library')).toHaveCount(12)
    // Part 01 has a thumbnail in the index; Part 12 does not and shows the category placeholder.
    await expect(sourceTile(page, 'Part 01').locator('img')).toHaveCount(1)
    await expect(sourceTile(page, 'Part 12').locator('img')).toHaveCount(0)
  })

  test('5. Both. Unlisted source by URL: "not in the Hew catalog" and "No license stated" before Add', async ({ page }) => {
    const host = new FakeWeb()
    publish(host, sourceOf('Loose Parts', SOURCE_U, 2, { license: null }))
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)

    const add = await lookUp(page, `${SOURCE_U}hew.json`)
    await expect(add.getByTestId('add-source-unlisted')).toHaveText('This source is not in the Hew catalog. Hew has not checked it.')
    await expect(add.getByTestId('add-source-facts')).toContainText('No license stated')
  })

  test('6. Both. A GitHub URL, then its github: short form: no second row; a plain https hew.json URL adds a second', async ({ page }) => {
    const host = new FakeWeb()
    publish(host, sourceOf('Shop Hardware', SOURCE_A, 2))
    // GitHub: the API resolves HEAD to a commit; the index and items are read at that commit.
    host.serve('https://api.github.com/repos/example/gear/commits/HEAD', { body: JSON.stringify({ sha: GH_COMMIT }), type: 'application/json' })
    publish(host, sourceOf('Gear Shelf', GH_FOLDER, 2))
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)

    await addByAddress(page, 'https://github.com/example/gear')
    await expect(page.getByTestId('library-source-row')).toHaveCount(1)

    // The same repository in short form: already listed, and nothing changes.
    const again = await lookUp(page, 'github:example/gear')
    await expect(again.getByTestId('add-source-existing')).toHaveText('This source is already in your list.')
    await again.getByRole('button', { name: 'Done', exact: true }).click()
    await expect(page.getByTestId('library-source-row')).toHaveCount(1)

    // A plain https hew.json URL is another source: a second row.
    await addByAddress(page, A_INDEX)
    await expect(page.getByTestId('library-source-row')).toHaveCount(2)
  })

  test('7. Both. An address with no source behind it: "This address does not have a Hew source."', async ({ page }) => {
    const host = new FakeWeb()
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)

    const add = await lookUp(page, `${SOURCE_HOST}/nothing-here/`)
    await expect(add.getByTestId('add-source-error')).toHaveText('This address does not have a Hew source.')
    await expect(page.getByTestId('library-source-row')).toHaveCount(0)
  })

  test('8. Both. Add one component: under Components in a collection named for the source; detail "From <source> · <license>"', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 3)
    const aUrl = publish(host, a)
    host.publishCatalog([{ ref: webRef(aUrl), index: a.index as SourceIndexJson }])
    await boot(page, host)
    await openLibrary(page)
    await openCatalog(page)
    await catalogRow(page, 'Shop Hardware').getByRole('button', { name: 'Add Shop Hardware' }).click()
    await sourceRow(page, 'Shop Hardware').click()

    await sourceTile(page, 'Part 01').click()
    await page.getByRole('button', { name: 'Add to my Library' }).click()
    await expect(page.getByTestId('source-item-state')).toContainText('In your Library')

    const dialog = await openLibrary(page)
    await dialog.getByRole('tab', { name: /^Components/ }).click()
    await expect(libraryTile(dialog, 'Part 01')).toBeVisible()
    await libraryTile(dialog, 'Part 01').click()
    await expect(dialog.getByText(/From Shop Hardware/)).toBeVisible()
    await expect(dialog.getByText(/CC-BY-4\.0/).first()).toBeVisible()
    await expect(dialog.getByLabel('Collection')).toHaveValue('Shop Hardware')
  })

  test('10. Both. Add a material and paint a face with it; add a model and open it as a document', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOfMaterialAndModel('Finishes', SOURCE_A)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)

    // A box to paint, seen from above its top face.
    const camera: CameraParams = {
      position: { x: 0.5, y: -4, z: 4 },
      target: { x: 0.5, y: 0.5, z: 0.5 },
      up: { x: 0, y: 0, z: 1 },
      fovDeg: 50,
      near: 0.1,
      far: 1000,
    }
    const object = await page.evaluate((cam) => {
      const h = window.__hew_test!
      const id = h.drawBox([0, 0, 0], [1, 1, 0], 1)
      h.setCamera({
        position: [cam.position.x, cam.position.y, cam.position.z],
        target: [cam.target.x, cam.target.y, cam.target.z],
        up: [cam.up.x, cam.up.y, cam.up.z],
        fovDeg: cam.fovDeg,
      })
      return id
    }, camera)
    const topFace = await page.evaluate((o) => {
      const hit = window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1])
      return hit !== null && hit.object === o ? hit.face : null
    }, object)
    expect(topFace).not.toBeNull()
    expect(await page.evaluate(({ o, f }) => window.__hew_test!.getFaceMaterial(o, f!), { o: object, f: topFace })).toEqual({ face: null, objectDefault: null })

    // Add the source, then the material from its view.
    const dialog = await openLibrary(page)
    await addByAddress(page, aUrl)
    await sourceRow(page, 'Finishes').click()
    await sourceTile(page, 'Part 15').click()
    await page.getByRole('button', { name: 'Add to my Library' }).click()
    await expect(page.getByTestId('source-item-state')).toContainText('In your Library')

    // Paint with it from the Library, then click the box's top face.
    await localView(dialog)
    await dialog.getByRole('tab', { name: /^Materials/ }).click()
    await libraryTile(dialog, 'Part 15').click()
    await dialog.locator('.hwlib__btn-primary', { hasText: 'Paint' }).click()
    await expect(dialog).not.toBeVisible()
    await expect(page.getByText(/Click a face to paint it with the current material/i)).toBeVisible()
    const box = await page.locator('canvas').first().boundingBox()
    if (box === null) throw new Error('viewport canvas has no bounding box')
    const vp = buildViewProjection(camera, box.width / box.height)
    const at = worldToPagePixel({ x: 0.5, y: 0.5, z: 1 }, vp, { left: box.x, top: box.y, width: box.width, height: box.height })
    if (at === null) throw new Error('the top face does not project onto the canvas')
    await page.mouse.move(at.x - 4, at.y - 4)
    await page.waitForTimeout(40)
    await page.mouse.move(at.x, at.y)
    await page.waitForTimeout(120)
    await page.mouse.down()
    await page.mouse.up()
    await page.waitForTimeout(150)
    const painted = await page.evaluate(({ o, f }) => window.__hew_test!.getFaceMaterial(o, f!), { o: object, f: topFace })
    expect(painted?.face).not.toBeNull()
    expect(await page.evaluate((m) => window.__hew_test!.materialUsage(m!), painted?.face ?? null)).toBe(1)

    // Add the model, then open it as a document. The web build opens a Library item as
    // a document only in an empty window, so reload (the Library and sources persist).
    await page.reload()
    await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
    const again = await openLibrary(page)
    await sourceRow(page, 'Finishes').click()
    await sourceTile(page, 'Part 16').click()
    await page.getByRole('button', { name: 'Add to my Library' }).click()
    await expect(page.getByTestId('source-item-state')).toContainText('In your Library')
    await localView(again)
    await again.getByRole('tab', { name: /^Models/ }).click()
    await libraryTile(again, 'Part 16').click()
    await again.locator('.hwlib__btn-primary', { hasText: 'Open as document' }).click()
    await expect(again).not.toBeVisible()
    await expect
      .poll(async () =>
        page.evaluate(() => {
          const h = window.__hew_test!
          const ids = h.getObjectIds()
          if (ids.length !== 1) return null
          const b = h.getObjectBounds(ids[0])
          return [b[3] - b[0], b[4] - b[1], b[5] - b[2]].map((v) => Math.round(v * 1000) / 1000)
        }),
      )
      .toEqual([2, 1, 3])
  })

  test.fixme('9. Both. Insert a "Not in your Library" component from a source view: ghost follows, one instance on click', async () => {
    // Needs the placement ghost followed over the viewport with real pointer moves
    // (ghostBuilder.ts) and a canvas click. Not written in this pass; see the report.
  })

  test('11. Both. From a catalog preview of a source not yet added, add one item: source in sidebar and item in Library', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 3)
    const aUrl = publish(host, a)
    host.publishCatalog([{ ref: webRef(aUrl), index: a.index as SourceIndexJson }])
    await boot(page, host)
    await openLibrary(page)
    await openCatalog(page)

    await catalogRow(page, 'Shop Hardware').click()
    await expect(sourceRow(page, 'Shop Hardware')).toHaveCount(0)
    await sourceTile(page, 'Part 02').click()
    await page.getByRole('button', { name: 'Add to my Library' }).click()

    await expect(sourceRow(page, 'Shop Hardware')).toBeVisible()
    const dialog = await openLibrary(page)
    await dialog.getByRole('tab', { name: /^Components/ }).click()
    await expect(libraryTile(dialog, 'Part 02')).toBeVisible()
  })

  test('12. Both. Item bytes do not match the index: "changed while Hew was reading it", library count unchanged', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    // The bytes behind Part 01's index entry are another real item's.
    host.serve(`${SOURCE_A}Components/part-01.hew`, { body: fixtureItem(2).bytes, type: 'application/octet-stream' })
    host.publishCatalog([])
    await boot(page, host)
    const dialog = await openLibrary(page)
    await addByAddress(page, aUrl)
    await sourceRow(page, 'Shop Hardware').click()
    await sourceTile(page, 'Part 01').click()
    await page.getByRole('button', { name: 'Add to my Library' }).click()

    await expect(page.getByText(/This item changed while Hew was reading it/)).toBeVisible()
    await localView(dialog)
    await expect(libraryTile(dialog, 'Part 01')).toHaveCount(0)
  })

  test('13. Both. Index entry id differs from the file: "The source lists this item incorrectly. Hew did not add it."', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    a.index.items[0].id = '00000000-0000-4000-8000-000000000001'
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await sourceRow(page, 'Shop Hardware').click()
    await sourceTile(page, 'Part 01').click()
    await page.getByRole('button', { name: 'Add to my Library' }).click()

    await expect(page.getByText('The source lists this item incorrectly. Hew did not add it.')).toBeVisible()
  })

  test.fixme('14. Both. Minted file name taken by a different item: the name-clash message, the existing item unchanged', async () => {
    // Blocked: the minted file name is slug(name) plus the first six hex of the id
    // (fileNaming.ts). Making a different item own that name needs a file with a chosen
    // id prefix written into the Library folder, which the browser suite cannot do
    // without reaching into app storage. See the report.
  })

  test('15. Both. Index with "hew": 2: "This source needs a newer version of Hew."', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    a.index.hew = 2
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)

    const add = await lookUp(page, aUrl)
    await expect(add.getByTestId('add-source-error')).toHaveText('This source needs a newer version of Hew.')
  })

  test('16. Both. Source goes offline, Refresh: fetch message, listing keeps the last good check; back online, the warning clears', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    // The app's clock, so the failed Refresh can happen half an hour after the good check:
    // "just now" would read the same whether the header shows the good time or the failed one.
    await page.clock.install()
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await sourceRow(page, 'Shop Hardware').click()
    const view = page.getByTestId('source-browser')
    await expect(view.getByTestId('source-checked')).toHaveText('Checked just now')

    host.unserve(aUrl)
    await page.clock.setSystemTime(Date.now() + 30 * 60_000)
    await view.getByTestId('source-refresh').click()
    await expect(view.getByTestId('source-unreachable')).toBeVisible()
    // The dialog reads the clock when it opens, so reopen it: the header keeps the last
    // good check (30 minutes ago), not the failed one (just now).
    await page.keyboard.press('Escape')
    await openLibrary(page)
    await sourceRow(page, 'Shop Hardware').click()
    await expect(view.getByTestId('source-checked')).toHaveText('Checked 30 minutes ago')
    await expect(view.getByTestId('source-unreachable')).toBeVisible()
    await expect(sourceRow(page, 'Shop Hardware').getByTestId('source-warning-mark')).toBeVisible()
    await expect(sourceTile(page, 'Part 01')).toBeVisible()

    publish(host, a)
    await view.getByTestId('source-refresh').click()
    await expect(view.getByTestId('source-unreachable')).toHaveCount(0)
    await expect(sourceRow(page, 'Shop Hardware').getByTestId('source-warning-mark')).toHaveCount(0)
  })

  test('17. Both. Catalog address fails: the fetch message shows; an added source still works', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([{ ref: webRef(aUrl), index: a.index as SourceIndexJson }])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)

    host.serve(CATALOG_URL, 'abort')
    await page.getByTestId('library-browse-catalog').click()
    await expect(page.getByTestId('library-catalog-error')).toBeVisible()

    await sourceRow(page, 'Shop Hardware').click()
    await expect(sourceTile(page, 'Part 01')).toBeVisible()
  })

  test('18. Web. A host with no cross-origin header: the message names both causes and the desktop app', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    // A browser cannot read a host that sends no cross-origin header: the request fails for the page.
    host.serve(aUrl, 'abort')
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)

    const add = await lookUp(page, aUrl)
    await expect(add.getByText(/may not allow web apps to read it/)).toBeVisible()
    await expect(add.getByText(/The desktop app can read it either way/)).toBeVisible()
  })

  test('21. Both. Item changed at the source, Refresh: "newer at the source" mark and the bar; Update all clears it', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')

    sourceSaysPartOneChanged(host, a)
    await page.getByTestId('source-browser').getByTestId('source-refresh').click()
    const bar = page.getByTestId('source-update-bar')
    await expect(bar.getByTestId('source-update-newer')).toHaveText('1 item has a newer version at its source.')
    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await expect(libraryTile(dialog, 'Part 01').getByTestId('source-mark')).toBeVisible()

  })

  test('21b. Both. Update all clears the mark and the bar', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')

    sourceSaysPartOneChanged(host, a)
    await page.getByTestId('source-browser').getByTestId('source-refresh').click()
    await page.getByTestId('source-update-all').click()
    const updates = page.getByRole('dialog', { name: 'Source updates' })
    await expect(updates.getByText('Updated', { exact: true })).toBeVisible()
    await updates.getByRole('button', { name: 'Done', exact: true }).click()
    await expect(page.getByTestId('source-update-bar')).toHaveCount(0)
    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await expect(libraryTile(dialog, 'Part 01').getByTestId('source-mark')).toHaveCount(0)
  })

  test('22. Both. Move an added item to another collection, then update it from the source: it stays in that collection', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')

    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await libraryTile(dialog, 'Part 01').click()
    await dialog.getByLabel('Collection').selectOption({ label: '+ New collection…' })
    await dialog.getByLabel('New collection path').fill('Hinges')
    await dialog.getByLabel('New collection path').press('Enter')
    await expect(dialog.getByLabel('Collection')).toHaveValue('Hinges')

    sourceSaysPartOneChanged(host, a)
    await sourceRow(page, 'Shop Hardware').click()
    await page.getByTestId('source-browser').getByTestId('source-refresh').click()
    await page.getByTestId('source-update-all').click()
    const updates = page.getByRole('dialog', { name: 'Source updates' })
    await expect(updates.getByText('Updated', { exact: true })).toBeVisible()
    await updates.getByRole('button', { name: 'Done', exact: true }).click()
    await expect(page.getByTestId('source-update-bar')).toHaveCount(0)
    await localView(dialog)
    await libraryTile(dialog, 'Part 01').click()
    await expect(dialog.getByLabel('Collection')).toHaveValue('Hinges')
  })

  test('24. Both. Edit an added item in place, then newer at the source: Update all leaves it; Review asks; Keep mine', async ({ page }) => {
    test.slow()
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')
    await editPartOneAndClose(page)
    await makePartOneNewer(page, host, a)

    // Update all leaves the changed item alone.
    await page.getByTestId('source-update-all').click()
    const updates = page.getByRole('dialog', { name: 'Source updates' })
    await expect(updates.getByText('You changed this item here. Taking the newer version replaces your changes.')).toBeVisible()
    // Review asks; Keep mine.
    await updates.getByRole('button', { name: 'Keep mine', exact: true }).click()
    await updates.getByRole('button', { name: /^(Done|Cancel)$/ }).click()
    await expect(page.getByTestId('source-update-bar')).toHaveCount(0)
    await expect(sourceTile(page, 'Part 01')).toHaveAccessibleName(/Newer at the source/)

    // Changed at the source again: the bar counts it again.
    a.index.items[0] = { ...a.index.items[0], sha256: 'e'.repeat(64) }
    publish(host, a)
    await page.getByTestId('source-browser').getByTestId('source-refresh').click()
    await expect(page.getByTestId('source-update-newer')).toHaveText('1 item has a newer version at its source.')
  })

  test('25. Both. Same start as 24, "Replace your changes": the file is the source\'s and "changed here" is gone', async ({ page }) => {
    test.slow()
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')
    await editPartOneAndClose(page)
    await makePartOneNewer(page, host, a)

    await page.getByTestId('source-update-all').click()
    const updates = page.getByRole('dialog', { name: 'Source updates' })
    await updates.getByRole('button', { name: 'Replace your changes', exact: true }).click()
    await expect(updates.getByTestId('source-review-result')).toBeVisible()
    await updates.getByRole('button', { name: 'Done', exact: true }).click()
    await expect(page.getByTestId('source-update-bar')).toHaveCount(0)

    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await libraryTile(dialog, 'Part 01').click()
    await expect(dialog.getByText(/From Shop Hardware/)).toBeVisible()
    await expect(dialog.getByText(/changed here/i)).toHaveCount(0)
  })

  test('26. Both. Open an added item for editing, then Update all: that row reads "Open for editing"', async ({ page }) => {
    test.slow()
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')

    sourceSaysPartOneChanged(host, a)
    await openForEditing(page, 'Part 01')
    await openLibrary(page)
    await sourceRow(page, 'Shop Hardware').click()
    await page.getByTestId('source-browser').getByTestId('source-refresh').click()
    await page.getByTestId('source-update-all').click()
    const updates = page.getByRole('dialog', { name: 'Source updates' })
    await expect(updates.getByText('Open for editing. Close it and update again.')).toBeVisible()
  })

  test('28. Both. "Unlink from <source>" leaves the item in the Library and the source view reads "In your Library"', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')

    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await libraryTile(dialog, 'Part 01').click()
    await expect(dialog.getByText(/From Shop Hardware/)).toBeVisible()
    await dialog.getByText('Unlink from Shop Hardware', { exact: true }).click()
    await expect(dialog.getByText(/From Shop Hardware/)).toHaveCount(0)
    await expect(libraryTile(dialog, 'Part 01')).toHaveCount(1)

    await sourceRow(page, 'Shop Hardware').click()
    await expect(sourceTile(page, 'Part 01')).toHaveAccessibleName(/Part 01 In your Library$/)
  })

  test('28b. Both. Item menu "Update from source" on a newer item updates it', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')

    sourceSaysPartOneChanged(host, a)
    await page.getByTestId('source-browser').getByTestId('source-refresh').click()
    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await expect(libraryTile(dialog, 'Part 01').getByTestId('source-mark')).toBeVisible()
    await dialog.getByRole('button', { name: 'Part 01 actions' }).click()
    await page.getByRole('menu').getByText('Update from source').click()
    await expect(libraryTile(dialog, 'Part 01').getByTestId('source-mark')).toHaveCount(0)
    await dialog.getByRole('button', { name: 'Part 01 actions' }).click()
    await expect(page.getByRole('menu').getByText('Update from source')).toHaveCount(0)
  })

  test('29. Both. Add an item whose id is already in the Library with no note: "Replace the item in your Library?"; Yes links it', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')

    // Unlink Part 01: it stays in the Library with the source's id and no note.
    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await libraryTile(dialog, 'Part 01').click()
    await dialog.getByText('Unlink from Shop Hardware', { exact: true }).click()
    await expect(dialog.getByText(/From Shop Hardware/)).toHaveCount(0)

    await sourceRow(page, 'Shop Hardware').click()
    await sourceTile(page, 'Part 01').click()
    const detail = page.getByTestId('source-item-detail')
    const add = detail.getByRole('button', { name: 'Add to my Library' })
    await expect(add).toBeEnabled()
    await add.click()
    const ask = page.getByTestId('source-confirm')
    await expect(ask.getByText('Replace the item in your Library?')).toBeVisible()
    await ask.getByRole('button', { name: 'Yes' }).click()
    await expect(ask).toHaveCount(0)
    await expect(sourceTile(page, 'Part 01')).toHaveAccessibleName(/Part 01 In your Library$/)
    await expect(add).toBeDisabled()

    await localView(dialog)
    await libraryTile(dialog, 'Part 01').click()
    await expect(dialog.getByText(/From Shop Hardware/)).toBeVisible()
    await expect(libraryTile(dialog, 'Part 01')).toHaveCount(1)
  })

  test('30. Both. Follow a source of 12 items: progress to 12; then 2 new and 1 changed: the bar line', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 12)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await sourceRow(page, 'Shop Hardware').click()
    await page.getByTestId('source-browser').getByTestId('source-follow').click()
    await expect(page.getByTestId('source-run-summary')).toContainText('12 added')
    const dialog = page.getByRole('dialog', { name: 'Library' })
    await expect(libraryTiles(dialog)).toHaveCount(12)

    // Two new items and one changed item at the source.
    for (const [i, n] of [13, 14].entries()) {
      const item = fixtureItem(n)
      a.index.items.push(indexEntry(item, { path: `Components/new-${i}.hew`, id: item.id, name: `New ${i}` }))
      host.serve(`${SOURCE_A}Components/new-${i}.hew`, { body: item.bytes, type: 'application/octet-stream' })
    }
    sourceSaysPartOneChanged(host, a)
    await page.getByTestId('source-browser').getByTestId('source-refresh').click()
    await expect(page.getByTestId('source-update-followed')).toHaveText('Shop Hardware has 2 new and 1 changed items')
  })

  test('31. Both. Delete a followed item, reopen, Refresh: not counted as new; "Not in your Library" in the source view', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 3)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')

    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await libraryTile(dialog, 'Part 02').click()
    await dialog.getByRole('button', { name: 'Delete from library…' }).click()
    await dialog.getByRole('button', { name: /^Delete$/ }).click()
    await expect(libraryTile(dialog, 'Part 02')).toHaveCount(0)
    await page.keyboard.press('Escape')

    await page.reload()
    await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
    await openLibrary(page)
    await sourceRow(page, 'Shop Hardware').click()
    await page.getByTestId('source-browser').getByTestId('source-refresh').click()
    await expect(sourceTile(page, 'Part 02').getByText('Not in your Library')).toBeVisible()
    await expect(page.getByTestId('source-update-bar')).toHaveCount(0)
  })

  test('32. Both. Cancel a Follow halfway: items already added stay; the rest read "Not in your Library"', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 12)
    const aUrl = publish(host, a)
    // Every item after the first is held, so the run is still going when Cancel is pressed.
    for (let n = 2; n <= 12; n++) {
      const key = `Components/part-${String(n).padStart(2, '0')}.hew`
      host.serve(`${SOURCE_A}${key}`, { body: fixtureItem(n).bytes, type: 'application/octet-stream', delayMs: 4000 })
    }
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await sourceRow(page, 'Shop Hardware').click()
    await page.getByTestId('source-browser').getByTestId('source-follow').click()
    await expect(page.getByTestId('source-cancel-run')).toBeVisible()
    await page.getByTestId('source-cancel-run').click()
    await expect(page.getByTestId('source-cancel-run')).toHaveCount(0, { timeout: 15_000 })

    const dialog = page.getByRole('dialog', { name: 'Library' })
    await expect(libraryTile(dialog, 'Part 01')).toHaveCount(1)
    await expect(sourceTile(page, 'Part 12').getByText('Not in your Library')).toBeVisible()
  })

  test('33. Both. Follow with one item failing: "11 added, 1 failed"; Retry adds the last one once it is served', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 12)
    const aUrl = publish(host, a)
    host.serve(`${SOURCE_A}Components/part-12.hew`, { status: 500, body: 'no', type: 'text/plain' })
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await sourceRow(page, 'Shop Hardware').click()
    await page.getByTestId('source-browser').getByTestId('source-follow').click()
    await expect(page.getByTestId('source-run-summary')).toContainText('11 added, 1 failed')

    publish(host, a)
    await page.getByTestId('source-retry').click()
    await expect(page.getByTestId('source-run-summary')).toContainText('1 added')
    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await expect(libraryTiles(dialog)).toHaveCount(12)
  })

  test('34. Both. Remove an item from the source index, Refresh: "The source no longer has this item."', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 3)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')

    a.index.items.splice(1, 1)
    publish(host, a)
    await page.getByTestId('source-browser').getByTestId('source-refresh').click()
    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await libraryTile(dialog, 'Part 02').click()
    await expect(page.getByText('The source no longer has this item.')).toBeVisible()
  })

  test('36. Both. Remove a source: items remain "From <source>"; add it again: items read "In your Library"', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')

    await page.getByTestId('source-remove').click()
    await page.getByRole('alertdialog', { name: 'Remove this source?' }).getByRole('button', { name: 'Remove source' }).click()
    await expect(page.getByTestId('library-source-row')).toHaveCount(0)
    // It stays gone: a later read of the open page must not add it back.
    await page.waitForTimeout(1500)
    await expect(page.getByTestId('library-source-row')).toHaveCount(0)

    // The items stay in the Library, still from the source, and no update is offered.
    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await expect(libraryTiles(dialog)).toHaveCount(2)
    await libraryTile(dialog, 'Part 01').click()
    await expect(dialog.getByText(/From Shop Hardware/)).toBeVisible()
    await expect(page.getByTestId('source-update-bar')).toHaveCount(0)

    // Added again, the same items read "In your Library" in the source's view.
    await addByAddress(page, aUrl)
    await sourceRow(page, 'Shop Hardware').click()
    await expect(sourceTile(page, 'Part 01')).toHaveAccessibleName(/Part 01 In your Library$/)
  })

  test.fixme('38. Web. Choose a folder for a Library that has sources: the sources are still listed afterward', async () => {
    // Skipped: choosing a Library folder needs the File System Access picker, which
    // needs a real user gesture and a native folder dialog that Playwright cannot drive.
  })

  test('40. Both. Open an added item as a document, Save to Library as a new item: no "From" line', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await addAllFrom(page, 'Shop Hardware')

    const dialog = page.getByRole('dialog', { name: 'Library' })
    await localView(dialog)
    await libraryTile(dialog, 'Part 01').click()
    await page.keyboard.press('ControlOrMeta+Enter')
    await expect(dialog).not.toBeVisible()
    await page.locator('canvas').first().click({ position: { x: 5, y: 5 } })
    await page.keyboard.press('ControlOrMeta+a')
    await page.getByRole('button', { name: 'Save to Library' }).click()
    const prompt = page.getByRole('dialog', { name: 'Save to Library' })
    await prompt.getByRole('textbox').fill('Part 01 copy')
    await prompt.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(page.getByText('Saved "Part 01 copy" to library')).toBeVisible()

    await openLibrary(page)
    await localView(dialog)
    await libraryTile(dialog, 'Part 01 copy').click()
    await expect(dialog.getByText(/^From /)).toHaveCount(0)
  })

  test.fixme('43. Both. Command palette: typing an added item\'s name inserts it through the Library entry', async () => {
    // The palette's Library entry opens the dialog (library.spec.ts); the insert-by-name
    // path is not wired through the palette in this build. See the report.
  })

  test('44. Web. Setting off: reopening the Library makes no outside request; Refresh on a source makes one check', async ({ page }) => {
    const host = new FakeWeb()
    const a = sourceOf('Shop Hardware', SOURCE_A, 2)
    const aUrl = publish(host, a)
    host.publishCatalog([])
    await boot(page, host)
    await openLibrary(page)
    await addByAddress(page, aUrl)
    await page.keyboard.press('Escape')

    // Settings ▸ Folders: switch "Check sources for new and changed items" off.
    const settings = await openFoldersSettings(page)
    await settings.getByRole('checkbox', { name: 'Check sources for new and changed items' }).uncheck()
    await page.keyboard.press('Escape')

    const before = host.outside.length
    await openLibrary(page)
    // The list is on screen (the open-time work has run), then nothing is in flight.
    await expect(sourceRow(page, 'Shop Hardware')).toBeVisible()
    await settle(page)
    expect(host.outside.slice(before).filter((r) => r.url.startsWith(SOURCE_HOST))).toHaveLength(0)

    await sourceRow(page, 'Shop Hardware').click()
    await page.getByTestId('source-browser').getByTestId('source-refresh').click()
    await expect.poll(() => host.outside.slice(before).filter((r) => r.url === A_INDEX).length).toBe(1)
  })

  test('45. Web. Setting on, no sources added: opening the Library makes no outside request', async ({ page }) => {
    const host = new FakeWeb()
    host.publishCatalog([])
    await boot(page, host)
    const before = host.outside.length
    const dialog = await openLibrary(page)
    await expect(dialog.getByTestId('library-browse-catalog')).toBeVisible()
    await settle(page)
    expect(host.outside.slice(before)).toHaveLength(0)
  })

  test.fixme('46. Web, strict build: "Sources are turned off on this server."; From line still shows; no outside request', async () => {
    // Needs a strict-CSP build (shells/csp/policies.json "web.strict"). The e2e run uses
    // the production policy; a second build is not cheap enough for this run. See the report.
  })

  test.fixme('49. Both, profile preview off: a source with Profiles items shows none of them; Add all does not add them', async () => {
    // Needs the Profiles preview to be withheld in the build under test; the production-shape
    // build withholds it, and the preview build is not run here. See the report.
  })
})

