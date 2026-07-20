import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * STL import E2E — the real pipeline in a real browser with the real WASM
 * kernel, driven through `window.__hew_test` (harness `exportStl`/`importStl`).
 *
 * Covers the flagship concern behind the "silent geometry loss" blocker:
 *   1. Round-trip through Hew's OWN STL export (author → export → import back).
 *   2. No silent geometry loss on messy input — a multi-part plate, an open
 *      (leaky) mesh, a non-manifold part, and a hollow/nested part — asserting
 *      nothing is ever dropped without a visible warning.
 *
 * The import UI (units chooser, overlay, report) is covered by App.test.tsx in
 * jsdom against the same code; here we exercise the real geometry pipeline.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const STL_DIR = 'e2e/fixtures/stl'
const fixture = (name: string): number[] =>
  Array.from(readFileSync(resolve(process.cwd(), `${STL_DIR}/${name}`)))

/** Committed crate-level fixtures (the reviewer-mandated cavity fixtures live
 * with the Rust tests; read them straight from there — no duplication). */
const crateFixture = (name: string): number[] =>
  Array.from(
    readFileSync(resolve(process.cwd(), `../crates/stl-import/tests/fixtures/${name}`)),
  )

/** Signed volume of a binary STL's triangle soup (divergence-theorem sum;
 * cavity walls wound inward subtract). For a genuine hollow this is
 * outer − cavity, well below the solid-block volume. */
function stlSignedVolume(bytes: number[]): number {
  const u8 = Uint8Array.from(bytes)
  const dv = new DataView(u8.buffer)
  const n = dv.getUint32(80, true)
  let v6 = 0
  for (let t = 0; t < n; t++) {
    const rec = 84 + t * 50
    const p = (k: number): [number, number, number] => {
      const o = rec + 12 + k * 12
      return [dv.getFloat32(o, true), dv.getFloat32(o + 4, true), dv.getFloat32(o + 8, true)]
    }
    const [ax, ay, az] = p(0)
    const [bx, by, bz] = p(1)
    const [cx, cy, cz] = p(2)
    // a · (b × c)
    v6 += ax * (by * cz - bz * cy) + ay * (bz * cx - bx * cz) + az * (bx * cy - by * cx)
  }
  return v6 / 6
}

/** Axis-aligned bounding box + extent of a binary STL's vertices (all f32 LE;
 * 80-byte header, u32 count at [80..84], then 50-byte triangle records with
 * three vertices starting 12 bytes in). Used to check round-trip dimensions. */
function stlExtent(bytes: number[]): [number, number, number] {
  const u8 = Uint8Array.from(bytes)
  const dv = new DataView(u8.buffer)
  const n = dv.getUint32(80, true)
  let minX = Infinity, minY = Infinity, minZ = Infinity
  let maxX = -Infinity, maxY = -Infinity, maxZ = -Infinity
  for (let t = 0; t < n; t++) {
    const rec = 84 + t * 50
    for (let v = 0; v < 3; v++) {
      const o = rec + 12 + v * 12
      const x = dv.getFloat32(o, true)
      const y = dv.getFloat32(o + 4, true)
      const z = dv.getFloat32(o + 8, true)
      minX = Math.min(minX, x); maxX = Math.max(maxX, x)
      minY = Math.min(minY, y); maxY = Math.max(maxY, y)
      minZ = Math.min(minZ, z); maxZ = Math.max(maxZ, z)
    }
  }
  return [maxX - minX, maxY - minY, maxZ - minZ]
}

async function boot(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

test.beforeEach(async ({ page }) => {
  await boot(page)
})

// ── 1. Round-trip through Hew's own STL export ───────────────────────────────

test('round-trip: author solids incl. a curved wall, export STL, import it back', async ({
  page,
}) => {
  // Author three disjoint solids, one with a curved wall (a cylinder).
  const authored = await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([0, 0, 0], [2, 2, 0], 2) // 2×2×2 m box at the origin
    h.drawBox([5, 0, 0], [6, 1, 0], 1) // 1×1×1 m box, disjoint
    h.drawCircle([0, 8, 0], 1) // r=1 m circle …
    const s = h.getObjectIds() // (drawCircle leaves a sketch; extrude it)
    return { count: h.getObjectCount(), ids: s }
  })
  // Extrude the cylinder's region: drawCircle returns {sketch, region}.
  await page.evaluate(() => {
    const h = window.__hew_test!
    const c = h.drawCircle([0, 8, 0], 1)
    h.extrudeRegion(c.sketch, c.region, 2) // 2 m tall cylinder
  })

  const exported = await page.evaluate(() => {
    const h = window.__hew_test!
    const worldCount = h.getObjectCount()
    const out = h.exportStl(0) // 0 = stored facets
    return { worldCount, stl: out }
  })
  expect(exported.stl).not.toBeNull()
  const origExtent = stlExtent(exported.stl!.bytes)
  // Two boxes + one cylinder authored (the stray drawCircle above formed a
  // second cylinder region; both extruded → the exact world count is read live).
  expect(exported.worldCount).toBeGreaterThanOrEqual(3)

  // Fresh document, then import the STL Hew just wrote, at millimeter scale.
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })

  const reimported = await page.evaluate((bytes) => {
    const h = window.__hew_test!
    const report = h.importStl(bytes, 0.001) // mm → the export was in mm
    const reExport = h.exportStl(0)
    return { report, count: h.getObjectCount(), reExport }
  }, exported.stl!.bytes)

  // Every disjoint solid comes back as its own watertight Object; nothing
  // skipped, nothing fake-leaky. STL flattens instances, so the imported count
  // equals the number of disjoint solids in the export.
  expect(reimported.report.skipped).toHaveLength(0)
  expect(reimported.report.objects_created).toBe(exported.worldCount)
  expect(reimported.report.watertight).toBe(exported.worldCount)
  expect(reimported.report.leaky).toBe(0)

  // Dimensions survive the meter → mm → meter round-trip: the re-imported model
  // re-exported to mm must match the original export's mm extent (f32 slack).
  const rtExtent = stlExtent(reimported.reExport!.bytes)
  for (let i = 0; i < 3; i++) {
    expect(rtExtent[i]).toBeGreaterThan(0)
    expect(Math.abs(rtExtent[i] - origExtent[i])).toBeLessThan(0.05) // mm
  }
})

test('round-trip: a component instance flattens into the STL and re-imports as solids', async ({
  page,
}) => {
  const exported = await page.evaluate(() => {
    const h = window.__hew_test!
    const c = h.drawBox([0, 0, 0], [1, 1, 0], 1) // the master box
    const { component } = h.makeComponent([c]) // fold into a component + instance
    h.placeInstance(component, 4, 0, 0) // a second placement, disjoint
    return { stl: h.exportStl(0) }
  })
  expect(exported.stl).not.toBeNull()

  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })

  const report = await page.evaluate(
    (bytes) => window.__hew_test!.importStl(bytes, 0.001),
    exported.stl!.bytes,
  )
  // Two disjoint instance placements → two watertight Objects, none skipped.
  expect(report.skipped).toHaveLength(0)
  expect(report.objects_created).toBe(2)
  expect(report.watertight).toBe(2)
  expect(report.leaky).toBe(0)
})

// ── 2. No silent geometry loss on messy input ────────────────────────────────

test('multi-part plate imports as one Object per disjoint solid', async ({ page }) => {
  const report = await page.evaluate(
    (bytes) => window.__hew_test!.importStl(bytes, 0.001),
    fixture('plate_three_cubes.stl'),
  )
  expect(report.objects_created).toBe(3)
  expect(report.watertight).toBe(3)
  expect(report.leaky).toBe(0)
  expect(report.skipped).toHaveLength(0)
})

test('an open mesh imports as a leaky Object — flagged, not refused, not skipped', async ({
  page,
}) => {
  const report = await page.evaluate(
    (bytes) => window.__hew_test!.importStl(bytes, 0.001),
    fixture('cube_open.stl'),
  )
  expect(report.objects_created).toBe(1)
  expect(report.leaky).toBe(1) // honestly leaky (missing a facet)
  expect(report.watertight).toBe(0) // never fake-closed
  expect(report.skipped).toHaveLength(0) // never refused / dropped
})

test('a non-manifold part imports without any piece silently vanishing', async ({ page }) => {
  const report = await page.evaluate(
    (bytes) => window.__hew_test!.importStl(bytes, 0.001),
    fixture('cube_doubled_face.stl'),
  )
  // The non-manifold part decomposes; every piece must reach the kernel.
  expect(report.objects_created).toBeGreaterThanOrEqual(1)
  expect(report.skipped).toHaveLength(0)
  // A split is loud, never silent.
  expect(report.warnings.some((w) => w.includes('non-manifold'))).toBe(true)
})

test('a hollow/nested part reconstructs as ONE watertight Object with a cavity', async ({
  page,
}) => {
  const report = await page.evaluate(
    (bytes) => window.__hew_test!.importStl(bytes, 0.001),
    fixture('nested_hollow.stl'),
  )
  // Outer shell + enclosed inner shell → one hollow Object (a void in the
  // material), not two separate solids. Correct behavior, no warning.
  expect(report.objects_created).toBe(1)
  expect(report.watertight).toBe(1)
  expect(report.leaky).toBe(0)
  expect(report.skipped).toHaveLength(0)
})

test('the "solid"-prefixed binary header is detected as binary, not garbled ASCII', async ({
  page,
}) => {
  const report = await page.evaluate(
    (bytes) => window.__hew_test!.importStl(bytes, 0.001),
    fixture('solid_prefixed_binary.stl'),
  )
  expect(report.objects_created).toBe(1)
  expect(report.watertight).toBe(1)
  expect(report.skipped).toHaveLength(0)
})

// A universal no-silent-loss invariant across every messy fixture: if ANYTHING
// is skipped, there is a visible warning — nothing ever disappears in silence.
for (const name of [
  'plate_three_cubes.stl',
  'cube_open.stl',
  'cube_doubled_face.stl',
  'nested_hollow.stl',
  'cube_binary.stl',
  'cube_ascii.stl',
]) {
  test(`no silent loss: ${name} — any skip is accompanied by a warning`, async ({ page }) => {
    const report = await page.evaluate(
      (bytes) => window.__hew_test!.importStl(bytes, 0.001),
      fixture(name),
    )
    expect(report.objects_created).toBeGreaterThanOrEqual(1)
    if (report.skipped.length > 0) {
      expect(report.warnings.length).toBeGreaterThan(0)
    }
  })
}

// ── 3. The real File ▸ Import… UI: units chooser → overlay → report ───────────

/** Install a fake `showOpenFilePicker` that hands the app the given STL bytes,
 * then reload so the init script is live before the app boots. WebFileHost's
 * File System Access path (chromium has it) calls this; a `.stl` name routes to
 * the STL importer. Must be called from a test (page already `goto`'d once). */
async function armFilePicker(page: Page, name: string, bytes: number[]): Promise<void> {
  await page.addInitScript(
    ([n, b]) => {
      const buf = new Uint8Array(b as number[])
      // @ts-expect-error test override of the File System Access API
      window.showOpenFilePicker = async () => [
        {
          getFile: async () => ({
            name: n as string,
            arrayBuffer: async () => buf.buffer,
          }),
        },
      ]
    },
    [name, bytes] as [string, number[]],
  )
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

for (const variant of [
  { label: 'binary', file: 'cube_binary.stl' },
  { label: 'ASCII', file: 'cube_ascii.stl' },
]) {
  test(`import UI (${variant.label}): units chooser defaults to mm, then the report shows the counts`, async ({
    page,
  }) => {
    await armFilePicker(page, variant.file, fixture(variant.file))

    // File ▸ Import…
    await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
    await page.getByText('Import…', { exact: true }).click()

    // The units chooser appears with Millimeters selected by default.
    const chooser = page.getByRole('dialog', { name: /stl import units/i })
    await expect(chooser).toBeVisible()
    await expect(chooser.getByRole('radio', { name: /millimeters/i })).toBeChecked()

    // Import at the mm default → the Import Complete report shows the real
    // counts (a single watertight cube).
    await chooser.getByRole('button', { name: /^import$/i }).click()

    const report = page.getByRole('dialog', { name: /import report/i })
    await expect(report).toBeVisible()
    await expect(report).toContainText('Import Complete')
    await expect(report).toContainText('1')
    await expect(report).toContainText(/solid/i)
    await expect(report).not.toContainText(/leaky/i) // a clean cube isn't leaky
  })
}

test('import UI: a leaky STL reports the leaky count honestly', async ({ page }) => {
  await armFilePicker(page, 'cube_open.stl', fixture('cube_open.stl'))
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Import…', { exact: true }).click()

  const chooser = page.getByRole('dialog', { name: /stl import units/i })
  await expect(chooser).toBeVisible()
  await chooser.getByRole('button', { name: /^import$/i }).click()

  const report = page.getByRole('dialog', { name: /import report/i })
  await expect(report).toBeVisible()
  await expect(report).toContainText(/leaky/i) // the open cube is flagged leaky
})

// ── 4. Re-entrancy guard in the real browser ─────────────────────────────────

test('import UI: a second import while the units chooser is open is refused (no hang)', async ({
  page,
}) => {
  await armFilePicker(page, 'cube_binary.stl', fixture('cube_binary.stl'))

  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Import…', { exact: true }).click()
  const chooser = page.getByRole('dialog', { name: /stl import units/i })
  await expect(chooser).toBeVisible()

  // Fire a SECOND import while the chooser is open, via the exact path the guard
  // was written for: the command palette (Ctrl+K ▸ "import" ▸ Enter behind the
  // modal). The re-entrancy guard must refuse it — still exactly one chooser.
  // (The File menu itself is unreachable behind the modal overlay, a second,
  // UI-level guard; the palette is the reachable re-entry.)
  await page.keyboard.press('Control+k')
  const palette = page.getByRole('dialog', { name: 'Command palette' })
  await expect(palette).toBeVisible()
  await palette.getByRole('textbox', { name: 'Search' }).fill('import')
  await expect(palette.getByRole('option', { name: /Import/ }).first()).toBeVisible()
  await page.keyboard.press('Enter')

  // The guard refused: the original chooser is still the one and only, not
  // clobbered, and no second chooser opened.
  await expect(page.getByRole('dialog', { name: /stl import units/i })).toHaveCount(1)

  // Completing the first chooser still drives its import to a report — proof the
  // first call was never orphaned by the refused second (no hang, no silent drop).
  await chooser.getByRole('button', { name: /^import$/i }).click()
  await expect(page.getByRole('dialog', { name: /import report/i })).toBeVisible()
})

// ── 5. Deferral A — a hollow round-trips as ONE Object with a real cavity ─────

test('hollow round-trip: box minus an interior box, export STL, import back as one hollow Object', async ({
  page,
}) => {
  // Build a genuine hollow in the real app: a 4×4×4 m outer box, minus a
  // 2×2×2 m box fully inside it (a Boolean subtract → an enclosed void).
  // drawBox always bases on the ground plane, so the inner box is drawn on the
  // ground then lifted to z∈[1,3] so it touches no face of the outer.
  const exported = await page.evaluate(() => {
    const h = window.__hew_test!
    const outer = h.drawBox([0, 0, 0], [4, 4, 0], 4) // [0,4]³
    const inner = h.drawBox([1, 1, 0], [3, 3, 0], 2) // [1,3]×[1,3]×[0,2]
    h.moveObject(inner, 0, 0, 1) // → [1,3]³, fully interior (touches no face)
    h.boolean(1, outer, inner) // op 1 = subtract → hollow solid with a cavity
    return { count: h.getObjectCount(), stl: h.exportStl(0) }
  })
  expect(exported.stl).not.toBeNull()
  expect(exported.count).toBe(1) // the subtract yields one hollow Object

  // The exported STL is genuinely hollow: signed volume = outer − cavity =
  // 4³ − 2³ = 56 m³ = 5.6e10 mm³ (export is mm scale), NOT the solid 64 m³.
  const origVol = stlSignedVolume(exported.stl!.bytes)
  expect(origVol).toBeGreaterThan(5.5e10)
  expect(origVol).toBeLessThan(5.7e10)

  // Fresh document; import the STL back at mm scale.
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })

  const reimported = await page.evaluate((bytes) => {
    const h = window.__hew_test!
    const report = h.importStl(bytes, 0.001)
    return { report, count: h.getObjectCount(), reExport: h.exportStl(0) }
  }, exported.stl!.bytes)

  // ONE watertight Object (not two nested solids), nothing skipped.
  expect(reimported.report.objects_created).toBe(1)
  expect(reimported.report.watertight).toBe(1)
  expect(reimported.report.leaky).toBe(0)
  expect(reimported.report.skipped).toHaveLength(0)

  // The cavity is REAL: the re-imported, re-exported model still has the
  // hollow signed volume (outer − cavity), not the solid block. A "two nested
  // solids" bug would give outer + cavity (a bigger number), and a "filled"
  // bug would give the solid 64 m³.
  const rtVol = stlSignedVolume(reimported.reExport!.bytes)
  expect(Math.abs(rtVol - origVol)).toBeLessThan(origVol * 1e-4)
})

// ── 6. Deferral B — imported Objects are named from the file stem ─────────────

test('name from file: importing bunny.stl names the Object "bunny", not "Imported"', async ({
  page,
}) => {
  // Drive the REAL File ▸ Import path (which computes the stem from the picked
  // filename) with a file the picker reports as "bunny.stl".
  await armFilePicker(page, 'bunny.stl', fixture('cube_binary.stl'))

  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Import…', { exact: true }).click()
  const chooser = page.getByRole('dialog', { name: /stl import units/i })
  await expect(chooser).toBeVisible()
  await chooser.getByRole('button', { name: /^import$/i }).click()
  await expect(page.getByRole('dialog', { name: /import report/i })).toBeVisible()

  const names = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.getObjectIds().map((id) => h.getNodeName('object', id))
  })
  expect(names).toEqual(['bunny']) // a single-solid file → the bare stem
})

test('name from file: a multi-part bunny.stl names parts "bunny", "bunny (2)"', async ({
  page,
}) => {
  await armFilePicker(page, 'bunny.stl', fixture('plate_three_cubes.stl'))

  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Import…', { exact: true }).click()
  const chooser = page.getByRole('dialog', { name: /stl import units/i })
  await expect(chooser).toBeVisible()
  await chooser.getByRole('button', { name: /^import$/i }).click()
  await expect(page.getByRole('dialog', { name: /import report/i })).toBeVisible()

  const names = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.getObjectIds().map((id) => h.getNodeName('object', id))
  })
  expect(names.sort()).toEqual(['bunny', 'bunny (2)', 'bunny (3)'])
})

// ── 7. Regression honesty: the two reviewer-mandated cavity-gate fixtures ─────

test('an OPEN shell nested in a solid stays its own leaky Object (never merged as a cavity)', async ({
  page,
}) => {
  const report = await page.evaluate(
    (bytes) => window.__hew_test!.importStl(bytes, 0.001),
    crateFixture('open_nested_in_solid.stl'),
  )
  // Closed outer + nested OPEN inner → TWO Objects: the outer stays a
  // watertight solid, the inner is its own honestly-leaky Object. NOT one
  // merged leaky blob that corrupts the good outer.
  expect(report.objects_created).toBe(2)
  expect(report.watertight).toBe(1)
  expect(report.leaky).toBe(1)
  expect(report.skipped).toHaveLength(0)
})

test('a straddling shell in a curved container is not fabricated into a hollow', async ({
  page,
}) => {
  const report = await page.evaluate(
    (bytes) => window.__hew_test!.importStl(bytes, 0.001),
    crateFixture('octahedron_straddle.stl'),
  )
  // Octahedron container + a cube whose bbox fits but whose geometry pokes out
  // through a face → TWO separate watertight Objects, never a self-intersecting
  // "watertight" merge.
  expect(report.objects_created).toBe(2)
  expect(report.watertight).toBe(2)
  expect(report.skipped).toHaveLength(0)
})
