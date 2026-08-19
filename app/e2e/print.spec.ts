import { test, expect } from '@playwright/test'

/**
 * File ▸ Print… (docs/design/printing.md §12) — the dialog end to end,
 * stopping short of the OS print dialog: the harness installs a recording
 * `PrintHost`, so "Print…" renders the real pages through the real WebGL
 * print pass and hands them to the recorder instead of the system.
 *
 * The scale assertion is the point of the feature: a 100 mm cube printed
 * 1:1 in Top view must span 100 / 25.4 × 300 = 1181 px in the page bitmap.
 * That is checked on the ACTUAL rendered pixels (dark-edge bounding box),
 * not on layout math.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  // Record instead of opening the real system print / save dialogs.
  await page.evaluate(() => window.__hew_test!.print.arm())
})

async function openPrintDialog(page: import('@playwright/test').Page) {
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Print…', { exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'Print Layout' })
  await expect(dialog).toBeVisible()
  return dialog
}

/** Click a segmented-control chip by its group testid + visible label
 * (Orientation/Margins/Extent/Style/Pages all render as real `<button
 * aria-pressed>`s inside `data-testid="print-<group>"`, not `<select>`s). */
async function clickChip(dialog: import('@playwright/test').Locator, testId: string, label: string) {
  await dialog.getByTestId(testId).getByRole('button', { name: label }).click()
}

test('File ▸ Print… opens ONE dialog; Standard mode is one page; Escape closes', async ({ page }) => {
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [0.1, 0.1, 0], 0.1))
  const dialog = await openPrintDialog(page)
  await expect(page.getByRole('dialog', { name: 'Print Layout' })).toHaveCount(1)
  await expect(dialog).toHaveAttribute('data-summary', /1 page/)
  await expect(dialog.getByTestId('print-preview-page')).toHaveCount(1)
  await page.keyboard.press('Escape')
  await expect(dialog).toBeHidden()
})

test('Standard Zoom: Fit fills the page with the model (the dark bounds span most of the image)', async ({ page }) => {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([0, 0, 0], [0.1, 0.05, 0], 0.05)
    // Zoomed well out: the box is a small thing in the viewport.
    h.setCamera({ position: [1.5, -1.5, 1.2], target: [0.05, 0.025, 0.025] })
  })
  const dialog = await openPrintDialog(page)
  await clickChip(dialog, 'print-zoom', 'Fit')
  await dialog.getByTestId('print-confirm').click()
  await expect(dialog).toHaveAttribute('data-status', 'sent', { timeout: 30_000 })
  const bounds = await page.evaluate(() => window.__hew_test!.print.pageDarkBounds(0, 128))
  expect(bounds).not.toBeNull()
  // Fit: the model spans ≥ 85 % of the image along its longer axis (a 4 %
  // margin each side); Current would leave it a small thing in the middle.
  const span = Math.max(bounds!.w / bounds!.imageW, bounds!.h / bounds!.imageH)
  expect(span).toBeGreaterThan(0.85)
})

test('Standard Line art inks the silhouette of a cylinder (curved walls have no hard edges) in perspective', async ({ page }) => {
  await page.evaluate(() => {
    const h = window.__hew_test!
    const c = h.drawCircle([0, 0, 0], 0.05, 32)
    h.extrudeRegion(c.sketch, c.region, 0.2)
    // A low, oblique view: the cylinder's sides are silhouettes, its ends ellipses.
    h.setCamera({ position: [0.5, -0.6, 0.15], target: [0, 0, 0.1] })
  })
  const dialog = await openPrintDialog(page)
  await clickChip(dialog, 'print-zoom', 'Fit')
  await clickChip(dialog, 'print-style', 'Line art')
  await dialog.getByTestId('print-confirm').click()
  await expect(dialog).toHaveAttribute('data-status', 'sent', { timeout: 30_000 })
  const bounds = await page.evaluate(() => window.__hew_test!.print.pageDarkBounds(0, 128))
  expect(bounds).not.toBeNull()
  // With silhouettes the dark pixels span the whole cylinder (Fit → most of
  // the page); with only the two rim ellipses inked the vertical extent would
  // still span, but the WIDTH between the ellipses' side tangents would be
  // filled by the side lines — check a column in the middle height has ink
  // at both sides.
  const sides = await page.evaluate(async () => {
    const b = await window.__hew_test!.print.pageDarkBounds(0, 128)
    return b === null ? null : { w: b.w / b.imageW, h: b.h / b.imageH }
  })
  expect(sides).not.toBeNull()
  expect(Math.max(sides!.w, sides!.h)).toBeGreaterThan(0.85)
  // The decisive check: dark pixels exist at the cylinder's left/right side
  // at mid-height (rows between the two rims), which only silhouettes provide.
  const mid = await page.evaluate(() => window.__hew_test!.print.rowDarkSpan(0, 40, 0.5))
  expect(mid).not.toBeNull()
  expect(mid!.w).toBeGreaterThan(0.3 * mid!.imageW)
})

test('Ctrl+P opens the dialog', async ({ page }) => {
  await page.keyboard.press('Control+p')
  await expect(page.getByRole('dialog', { name: 'Print Layout' })).toBeVisible()
})

test('Scaled 1:1, Top view (As shown): a 100 × 50 mm block spans 1181 × 591 px (+ stroke) of the page bitmap, X across the page', async ({ page }) => {
  // 100 mm along world X, 50 mm along Y: a plan must put X across the page
  // (the shared print basis) — a quarter-turned raster would swap w and h.
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [0.1, 0.05, 0], 0.1))
  const dialog = await openPrintDialog(page)
  await dialog.getByRole('button', { name: 'Scaled' }).click()
  await dialog.locator('#print-view-select').selectOption('top')
  await dialog.locator('#print-paper-select').selectOption('letter')
  await clickChip(dialog, 'print-style', 'As shown')
  // Presets: value = ladder index; find "1:1" (metric ladder — the test build's
  // default unit is metres) by label.
  const scaleSelect = dialog.locator('#print-scale-select')
  const options = await scaleSelect.locator('option').allTextContents()
  const oneToOne = options.findIndex((t) => t.startsWith('1:1 ') || t === '1:1')
  expect(oneToOne).toBeGreaterThanOrEqual(0)
  await scaleSelect.selectOption({ index: oneToOne })
  await expect(dialog).toHaveAttribute('data-summary', /1 page/)
  await expect(dialog).toHaveAttribute('data-summary', /1:1/)

  await dialog.getByTestId('print-confirm').click()
  await expect(dialog).toHaveAttribute('data-status', 'sent', { timeout: 30_000 })

  const jobs = await page.evaluate(() => window.__hew_test!.print.jobs())
  expect(jobs).toHaveLength(1)
  expect(jobs[0].pageCount).toBe(1)
  expect(jobs[0].tiles).toEqual(['A1'])
  // Letter, portrait: 215.9 × 279.4 mm.
  expect(jobs[0].setup.paperWmm).toBeCloseTo(215.9, 1)
  expect(jobs[0].setup.paperHmm).toBeCloseTo(279.4, 1)
  expect(jobs[0].setup.landscape).toBe(false)
  // The print root holds the page for the OS dialog.
  expect(await page.evaluate(() => window.__hew_test!.print.printRootPages())).toBe(1)

  // The cube's dark edges (As shown: 0x1a1a1a 0.2 mm ≈ 2.4 px strokes,
  // centred on the edge) span 100 mm = 1181 px edge-to-edge, so the dark
  // bounding box is 1181 + one stroke width (+ a pixel of AA each side).
  const bounds = await page.evaluate(() => window.__hew_test!.print.pageDarkBounds(0, 128))
  expect(bounds).not.toBeNull()
  const expectedPx = (100 / 25.4) * 300
  const strokePx = (0.2 / 25.4) * 300
  expect(Math.abs(bounds!.w - (expectedPx + strokePx))).toBeLessThanOrEqual(3)
  expect(Math.abs(bounds!.h - (expectedPx / 2 + strokePx))).toBeLessThanOrEqual(3)
  // Centred on the drawing area (within a few px).
  expect(Math.abs(bounds!.x + bounds!.w / 2 - bounds!.imageW / 2)).toBeLessThanOrEqual(6)
})

test('a 500 × 300 mm slab at 1:1 tiles 3 × 2 on Letter (auto → portrait) with A1…B3 ids', async ({ page }) => {
  // The overlap band is reserved INSIDE the printable area (SPEC.md §2
  // decision #12), so a 500 × 300 mm slab at 1:1 on Letter tiles 3 × 2 in
  // either orientation; Auto keeps its portrait tie-break.
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [0.5, 0.3, 0], 0.02))
  const dialog = await openPrintDialog(page)
  await dialog.getByRole('button', { name: 'Scaled' }).click()
  await dialog.locator('#print-view-select').selectOption('top')
  await dialog.locator('#print-paper-select').selectOption('letter')
  await clickChip(dialog, 'print-orientation', 'Auto')
  const scaleSelect = dialog.locator('#print-scale-select')
  const options = await scaleSelect.locator('option').allTextContents()
  await scaleSelect.selectOption({ index: options.findIndex((t) => t.startsWith('1:1 ') || t === '1:1') })
  await expect(dialog).toHaveAttribute('data-summary', /6 pages \(3 × 2\)/)
  await expect(dialog).toHaveAttribute('data-summary', /portrait/)
  const tiles = await dialog.getByTestId('print-preview-page').evaluateAll((els) => els.map((e) => e.getAttribute('data-tile')))
  expect(tiles).toEqual(['A1', 'A2', 'A3', 'B1', 'B2', 'B3'])
})

test('Fit picks the exact scale that fills one A4 page to the margins', async ({ page }) => {
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2.4, 1.0, 0], 0.05))
  const dialog = await openPrintDialog(page)
  await dialog.getByRole('button', { name: 'Scaled' }).click()
  await dialog.locator('#print-view-select').selectOption('top')
  await dialog.locator('#print-paper-select').selectOption('a4')
  await dialog.getByRole('button', { name: 'Fit' }).click()
  // 2.4 m across an A4 landscape drawing area (297 − 25.4 = 271.6 mm) →
  // exactly 1:8.84, one page, right up to the margins.
  await expect(dialog).toHaveAttribute('data-summary', /1 page/)
  await expect(dialog).toHaveAttribute('data-summary', /1:8\.8/)
  await expect(dialog.locator('#print-scale-select option:checked')).toHaveText(/\(fit\)/)
})

test('Selection extent omits everything not selected', async ({ page }) => {
  const ids = await page.evaluate(() => {
    const a = window.__hew_test!.drawBox([0, 0, 0], [0.1, 0.1, 0], 0.1)
    const b = window.__hew_test!.drawBox([0.5, 0, 0], [0.6, 0.1, 0], 0.1)
    window.__hew_test!.selectObjects([a])
    return { a, b }
  })
  expect(ids.a).not.toBe(ids.b)
  const dialog = await openPrintDialog(page)
  await dialog.getByRole('button', { name: 'Scaled' }).click()
  await dialog.locator('#print-view-select').selectOption('top')
  await dialog.locator('#print-paper-select').selectOption('letter')
  await clickChip(dialog, 'print-style', 'As shown')
  const scaleSelect = dialog.locator('#print-scale-select')
  const options = await scaleSelect.locator('option').allTextContents()
  await scaleSelect.selectOption({ index: options.findIndex((t) => t.startsWith('1:1 ') || t === '1:1') })
  await clickChip(dialog, 'print-extent', 'Selection')
  await expect(dialog).toHaveAttribute('data-summary', /1 page/)
  await dialog.getByTestId('print-confirm').click()
  await expect(dialog).toHaveAttribute('data-status', 'sent', { timeout: 30_000 })
  // Only the selected 100 mm cube is on the page: the dark span is one cube,
  // not the 600 mm two-cube spread (which would not even fit one page).
  const bounds = await page.evaluate(() => window.__hew_test!.print.pageDarkBounds(0, 128))
  const expectedPx = (100 / 25.4) * 300 + (0.2 / 25.4) * 300
  expect(Math.abs(bounds!.w - expectedPx)).toBeLessThanOrEqual(3)
})

test('Line art is vector: the page holds an SVG whose hard-edge path spans exactly the 100 mm cube; hidden lines dashed on request', async ({ page }) => {
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [0.1, 0.1, 0], 0.1))
  const dialog = await openPrintDialog(page)
  await dialog.getByRole('button', { name: 'Scaled' }).click()
  await dialog.locator('#print-view-select').selectOption('iso')
  await dialog.locator('#print-paper-select').selectOption('letter')
  await clickChip(dialog, 'print-style', 'Line art')
  const scaleSelect = dialog.locator('#print-scale-select')
  const options = await scaleSelect.locator('option').allTextContents()
  await scaleSelect.selectOption({ index: options.findIndex((t) => t.startsWith('1:1 ') || t === '1:1') })
  // Preview is vector too.
  await expect(dialog.locator('[data-testid=print-preview-page] .hew-print-drawing[data-kind=vector] svg').first()).toBeVisible()
  await dialog.getByTestId('print-confirm').click()
  await expect(dialog).toHaveAttribute('data-status', 'sent', { timeout: 30_000 })
  const info = await page.evaluate(() => {
    const svg = document.querySelector('#hew-print-root .hew-print-drawing[data-kind=vector] svg')
    if (!svg) return null
    const hard = svg.querySelector('path.hard')?.getAttribute('d') ?? ''
    const hidden = svg.querySelector('path.hidden')?.getAttribute('d') ?? ''
    // Bounding box of the hard path in user units (metres).
    const nums = hard.match(/-?\d+(?:\.\d+)?/g)?.map(Number) ?? []
    const xs = nums.filter((_, i) => i % 2 === 0)
    const ys = nums.filter((_, i) => i % 2 === 1)
    return { hardCount: (hard.match(/M/g) ?? []).length, hiddenCount: (hidden.match(/M/g) ?? []).length, w: Math.max(...xs) - Math.min(...xs), h: Math.max(...ys) - Math.min(...ys), width: svg.getAttribute('width') }
  })
  expect(info).not.toBeNull()
  // Iso of a cube: 9 visible edges, no hidden ones unless asked.
  expect(info!.hardCount).toBe(9)
  expect(info!.hiddenCount).toBe(0)
  // A unit-free check of scale: from the [1,-1,1] eye the projected cube
  // spans √2·a wide (right = (1,1,0)/√2) and 4/√6·a tall (up = (-1,1,2)/√6).
  expect(Math.abs(info!.w - Math.SQRT2 * 0.1)).toBeLessThan(1e-4)
  expect(Math.abs(info!.h - (4 / Math.sqrt(6)) * 0.1)).toBeLessThan(1e-4)
  // The SVG's physical width is the drawing area (mm).
  expect(info!.width).toMatch(/mm$/)
  // Hidden dashed adds the three hidden edges.
  await dialog.getByLabel('Hidden lines dashed').check()
  await expect
    .poll(async () => page.evaluate(() => (document.querySelector('[data-testid=print-preview-page] svg path.hidden')?.getAttribute('d')?.match(/M/g) ?? []).length))
    .toBe(3)
})

test('Save PDF… writes a PDF with one page per tile (vector Line art and raster As shown)', async ({ page }) => {
  await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [0.5, 0.3, 0], 0.02))
  const dialog = await openPrintDialog(page)
  await dialog.getByRole('button', { name: 'Scaled' }).click()
  await dialog.locator('#print-view-select').selectOption('top')
  await dialog.locator('#print-paper-select').selectOption('letter')
  await clickChip(dialog, 'print-style', 'Line art')
  const scaleSelect = dialog.locator('#print-scale-select')
  const options = await scaleSelect.locator('option').allTextContents()
  await scaleSelect.selectOption({ index: options.findIndex((t) => t.startsWith('1:1 ') || t === '1:1') })
  await expect(dialog).toHaveAttribute('data-summary', /6 pages \(3 × 2\)/)
  await dialog.getByTestId('print-save-pdf').click()
  await expect(dialog).toHaveAttribute('data-status', 'saved', { timeout: 30_000 })
  const pdf = await page.evaluate(() => window.__hew_test!.print.lastPdf())
  expect(pdf).not.toBeNull()
  expect(pdf!.head.startsWith('%PDF-1.4')).toBe(true)
  expect(pdf!.pages).toBe(6)
  expect(pdf!.name).toContain('1-1') // ':' is not a filename character
  // Vector pages are small: six Letter pages of a slab well under 100 KB.
  expect(pdf!.bytes).toBeLessThan(100_000)
  // Raster route too.
  await clickChip(dialog, 'print-style', 'As shown')
  await dialog.getByTestId('print-save-pdf').click()
  await expect(dialog).toHaveAttribute('data-status', 'saved', { timeout: 60_000 })
  const pdf2 = await page.evaluate(() => window.__hew_test!.print.lastPdf())
  expect(pdf2!.pages).toBe(6)
  expect(pdf2!.bytes).toBeGreaterThan(pdf!.bytes)
})

test('Pages: Each Scene prints one page per Scene with global numbering; the cut-list page appends', async ({ page }) => {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([0, 0, 0], [0.3, 0.2, 0], 0.1)
    h.setCamera({ position: [2, -2, 1.5], target: [0.15, 0.1, 0.05] })
    h.addScene('Iso view')
    h.setCamera({ position: [0.15, -3, 0.05], target: [0.15, 0.1, 0.05] })
    h.addScene('Front view')
  })
  const dialog = await openPrintDialog(page)
  const pages = dialog.locator('#print-pages-select')
  await expect(pages).toBeVisible()
  await pages.selectOption('each_scene')
  await expect(dialog).toHaveAttribute('data-summary', /2 pages · 2 Scenes/)
  await expect(dialog.getByTestId('print-preview-page')).toHaveCount(2)
  await expect(dialog.getByTestId('print-preview-page').nth(0)).toHaveAttribute('data-scene', 'Iso view')
  await expect(dialog.getByTestId('print-preview-page').nth(1)).toHaveAttribute('data-scene', 'Front view')
  // Cut list adds a page.
  await dialog.getByLabel('Cut list page').check()
  await expect(dialog).toHaveAttribute('data-summary', /\+ 1 cut list/)
  await expect(dialog.getByTestId('print-preview-page')).toHaveCount(3)
  await dialog.getByTestId('print-save-pdf').click()
  await expect(dialog).toHaveAttribute('data-status', 'saved', { timeout: 60_000 })
  const pdf = await page.evaluate(() => window.__hew_test!.print.lastPdf())
  expect(pdf!.pages).toBe(3)
  // And the print root gets all three with the second Scene's name in its title block.
  await dialog.getByTestId('print-confirm').click()
  await expect(dialog).toHaveAttribute('data-status', 'sent', { timeout: 60_000 })
  expect(await page.evaluate(() => window.__hew_test!.print.printRootPages())).toBe(3)
  const titles = await page.evaluate(() => Array.from(document.querySelectorAll('#hew-print-root .hew-print-page text[data-role=title-view]')).map((t) => t.textContent))
  expect(titles[0]).toContain('Iso view')
  expect(titles[1]).toContain('Front view')
  const pageTexts = await page.evaluate(() => Array.from(document.querySelectorAll('#hew-print-root .hew-print-page text[data-role=title-page]')).map((t) => t.textContent))
  expect(pageTexts[0]).toContain('Page 1 of 3')
  expect(pageTexts[1]).toContain('Page 2 of 3')
  expect(pageTexts[2]).toContain('Page 3 of 3')
})
