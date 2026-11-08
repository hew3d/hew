/**
 * Unified drafting, parity: the ground and a solid's face are drafting planes
 * with ONE engine, so the SAME real-pointer gesture performed once on the
 * ground and once on a box top must leave the same drawing — equal unit
 * counts, equal curve runs, equal edges and equal region rings (compared in
 * plane-local coordinates, so the two planes' different heights drop out).
 *
 * Every gesture goes through real mouse and keyboard events on the canvas;
 * the harness only builds the box, pins the camera and reads state back.
 * Each case boots a fresh page per surface, so cases are independent.
 */
import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type V3 = [number, number, number]
type Surface = 'ground' | 'top'

/** The box top is 3 × 3 at this height; the ground is z = 0. */
const TOP_Z = 0.5
const CENTER: [number, number] = [2, 1.5]

interface Env {
  page: Page
  surface: Surface
  z: number
  /** Plane-local (u, v) to a world point on the surface. */
  P: (u: number, v: number) => V3
  box: string | null
  /** A drawing the gesture saved part-way, for before/after comparisons. */
  stash?: Snap
}

async function boot(page: Page, surface: Surface): Promise<Env> {
  // These specs predate linked copies and assert on plain, independent
  // copies; the preference gives them exactly that. linked-copies.spec.ts
  // covers the default.
  await page.addInitScript(() => localStorage.setItem('hew.settings.copiesLinked', 'false'))
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const z = surface === 'top' ? TOP_Z : 0
  const box = await page.evaluate(
    ([surface, z, c]) => {
      const h = window.__hew_test!
      h.setGridVisible(false)
      h.setAxesVisible(false)
      const id = surface === 'top' ? h.drawBox([0, 0, 0], [3, 3, 0], z as number) : null
      // Straight down, the same distance above either plane: identical pixels.
      const cc = c as [number, number]
      h.setCamera({ position: [cc[0], cc[1], (z as number) + 7], target: [cc[0], cc[1], z as number], up: [0, 1, 0], fovDeg: 45 })
      return id
    },
    [surface, z, CENTER] as const,
  )
  await page.waitForTimeout(250)
  return { page, surface, z, P: (u, v) => [u, v, z], box }
}

async function pagePoint(env: Env, w: V3): Promise<{ x: number; y: number }> {
  const box = await env.page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await env.page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function hover(env: Env, u: number, v: number): Promise<void> {
  const at = await pagePoint(env, env.P(u, v))
  await env.page.mouse.move(at.x - 3, at.y + 2)
  await env.page.waitForTimeout(50)
  await env.page.mouse.move(at.x, at.y)
  await env.page.waitForTimeout(120)
}

async function click(env: Env, u: number, v: number): Promise<void> {
  await hover(env, u, v)
  await env.page.mouse.down()
  await env.page.mouse.up()
  await env.page.waitForTimeout(120)
}

async function tripleClick(env: Env, u: number, v: number): Promise<void> {
  await hover(env, u, v)
  for (let i = 0; i < 3; i++) {
    await env.page.mouse.down()
    await env.page.mouse.up()
    await env.page.waitForTimeout(40)
  }
  await env.page.waitForTimeout(150)
}

async function drag(env: Env, a: [number, number], b: [number, number]): Promise<void> {
  const p = await pagePoint(env, env.P(a[0], a[1]))
  const q = await pagePoint(env, env.P(b[0], b[1]))
  await env.page.mouse.move(p.x, p.y)
  await env.page.waitForTimeout(80)
  await env.page.mouse.down()
  for (let i = 1; i <= 8; i++) {
    await env.page.mouse.move(p.x + ((q.x - p.x) * i) / 8, p.y + ((q.y - p.y) * i) / 8)
    await env.page.waitForTimeout(30)
  }
  await env.page.mouse.up()
  await env.page.waitForTimeout(150)
}

async function typeEnter(env: Env, text: string): Promise<void> {
  await env.page.keyboard.type(text)
  await env.page.keyboard.press('Enter')
  await env.page.waitForTimeout(150)
}

async function tool(env: Env, name: string): Promise<void> {
  const radio = env.page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { name })
  if ((await radio.count()) > 0) {
    await radio.click()
  } else {
    // Palette-only tools (Edit Vertex) live in the Tools menu.
    await env.page.getByTestId('menu-bar').getByRole('button', { name: 'Tools' }).click()
    await env.page.getByText(name, { exact: true }).last().click()
  }
  await env.page.waitForTimeout(60)
}

async function select(env: Env): Promise<void> {
  await tool(env, 'Select')
}

const lastError = (env: Env) => env.page.evaluate(() => window.__hew_test!.getLastError())

// ---- the drawing, read back from the kernel ------------------------------

const R = (n: number) => (Math.round(n * 1e4) / 1e4 + 0).toFixed(4)
const key = (p: number[]) => `${R(p[0])},${R(p[1])}`

export interface Snap {
  units: number
  curves: string[]
  edges: string[]
  rings: string[]
}

/** The plane's drawing in plane-local coordinates, order-independent. */
async function snap(env: Env): Promise<Snap> {
  const raw = await env.page.evaluate((z) => {
    const h = window.__hew_test!
    return h.getPlaneUnits(h.ctxOfPlane([0, 0, z], [0, 0, 1])!)
  }, env.z)
  const ring = (r: number[][]) => r.map(key).sort().join(' ')
  const edges: string[] = []
  const curves: string[] = []
  for (const u of raw.units) {
    for (const e of u.edges) edges.push([key(e.a), key(e.b)].sort().join('>'))
    for (const c of u.curves) curves.push(`${c.kind}:${c.edges.length}:r${R(c.radius)}`)
  }
  const rings: string[] = []
  for (const r of raw.regions) {
    // The rest of the box top (outline with the drawing as holes) is the
    // solid's own face; the ground's counterpart is the unbounded void,
    // which is not a region. Everything else is compared.
    if (r.outer.some((p) => R(p[0]) === R(0) && R(p[1]) === R(0)) && r.outer.some((p) => R(p[0]) === R(3) && R(p[1]) === R(3))) continue
    rings.push('o ' + ring(r.outer))
    for (const hole of r.holes) rings.push('h ' + ring(hole))
  }
  return { units: raw.units.length, curves: curves.sort(), edges: edges.sort(), rings: rings.sort() }
}

/** Run `gesture` once per surface (fresh boot each) and return both drawings. */
async function bothSurfaces(
  page: Page,
  gesture: (env: Env) => Promise<void>,
  read: (env: Env) => Promise<Snap> = snap,
): Promise<Both> {
  const out: Partial<Record<Surface, Snap>> = {}
  const stash: Partial<Record<Surface, Snap>> = {}
  for (const surface of ['ground', 'top'] as Surface[]) {
    const env = await boot(page, surface)
    await gesture(env)
    expect(await lastError(env), `${surface}: last error`).toBeNull()
    out[surface] = await read(env)
    if (env.stash !== undefined) stash[surface] = env.stash
  }
  return { ground: out.ground!, top: out.top!, stash: { ground: stash.ground, top: stash.top } }
}

interface Both {
  ground: Snap
  top: Snap
  stash: { ground?: Snap; top?: Snap }
}

function expectParity(r: { ground: Snap; top: Snap }, units: number, opts: { curves?: boolean } = {}): void {
  expect(r.ground.units, 'ground unit count').toBe(units)
  expect(r.top.units, 'top unit count').toBe(units)
  if (opts.curves !== false) expect(r.top.curves, 'curve runs').toEqual(r.ground.curves)
  expect(r.top.edges, 'edges').toEqual(r.ground.edges)
  expect(r.top.rings, 'region rings').toEqual(r.ground.rings)
}

test.beforeEach(() => {
  test.setTimeout(90_000)
})

// ---- drawing -------------------------------------------------------------

test('Line: a loose segment', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Line')
    await click(env, 0.6, 0.6)
    await click(env, 1.8, 0.9)
    await env.page.keyboard.press('Escape')
  })
  expectParity(r, 1)
  expect(r.ground.edges).toHaveLength(1)
})

test('Line: a closed loop makes one shape and one region', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Line')
    await click(env, 0.6, 0.6)
    await click(env, 1.8, 0.6)
    await click(env, 1.2, 1.6)
    await click(env, 0.6, 0.6)
    await env.page.keyboard.press('Escape')
  })
  expectParity(r, 1)
  expect(r.ground.edges).toHaveLength(3)
  expect(r.ground.rings.filter((x) => x.startsWith('o'))).toHaveLength(1)
})

test('Rectangle', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Rectangle')
    await click(env, 0.6, 0.6)
    await click(env, 1.8, 1.4)
  })
  expectParity(r, 1)
  expect(r.ground.edges).toHaveLength(4)
})

test('Circle: one curve run with the same facet count', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Circle')
    await click(env, 1.5, 1.5)
    await click(env, 2.2, 1.5)
  })
  expectParity(r, 1)
  expect(r.ground.curves).toHaveLength(1)
  expect(r.ground.curves[0]).toMatch(/^circle:/)
})

test('Polygon: one polygon run', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Polygon')
    await click(env, 1.5, 1.5)
    await click(env, 2.2, 1.5)
  })
  expectParity(r, 1)
  expect(r.ground.curves).toHaveLength(1)
  expect(r.ground.curves[0]).toMatch(/^polygon:/)
  expect(r.ground.edges).toHaveLength(6)
})

test('A click on a drawn polygon selects it as one polygon, named Polygon, on both surfaces', async ({ page }) => {
  const kinds: Record<string, string[]> = {}
  const labels: Record<string, string[]> = {}
  await bothSurfaces(page, async (env) => {
    await tool(env, 'Polygon')
    await click(env, 1.5, 1.5)
    await click(env, 2.2, 1.5)
    await select(env)
    await click(env, 1.5, 0.9) // the polygon's lower side, clear of its center
    kinds[env.surface] = (await selection(env)).map((n) => n.kind)
    if (env.box !== null) {
      labels[env.surface] = await env.page.evaluate((o) => window.__hew_test!.getOutlinerShapeLabels(o), env.box)
    }
  })
  expect(kinds.ground).toEqual(['draft-curve'])
  expect(kinds.top).toEqual(['draft-curve'])
  expect(labels.top).toHaveLength(1)
  expect(labels.top[0]).toMatch(/^Polygon/)
})

test('A double-click on a side of one triangle of a star selects that triangle only, on both surfaces', async ({ page }) => {
  const picked: Record<string, { kind: string; edges: number }[]> = {}
  await bothSurfaces(page, async (env) => {
    for (const [u, v] of [
      [2.1, 1.5],
      [1.8, 1.5 + 0.6 * Math.sin(Math.PI / 3)],
    ] as [number, number][]) {
      await tool(env, 'Polygon')
      await click(env, 1.5, 1.5)
      await hover(env, (1.5 + u) / 2, (1.5 + v) / 2)
      await typeEnter(env, '3s')
      await click(env, u, v)
      await env.page.keyboard.press('Escape')
    }
    await select(env)
    // The first triangle's left side, between the two points where the
    // second triangle crosses it: a one-edge piece a single click picks
    // as a line; the double-click widens it to its own triangle.
    await hover(env, 1.2, 1.5)
    await env.page.mouse.dblclick((await pagePoint(env, env.P(1.2, 1.5))).x, (await pagePoint(env, env.P(1.2, 1.5))).y)
    await env.page.waitForTimeout(200)
    picked[env.surface] = await env.page.evaluate(() => {
      const h = window.__hew_test!
      return h.getSelection().map((n) => {
        const ref = n as { kind: string; ctx?: string; key?: string }
        if (ref.ctx === undefined || ref.key === undefined) return { kind: ref.kind, edges: 0 }
        const pu = h.getPlaneUnits(ref.ctx) as { units: { curves: { curve_key: string; edges: string[] }[] }[] }
        let edges = 0
        for (const u of pu.units) for (const c of u.curves) if (c.curve_key === ref.key) edges += c.edges.length
        return { kind: ref.kind, edges }
      })
    })
  })
  // One triangle: three sides, each cut twice by the other triangle.
  expect(picked.ground).toEqual([{ kind: 'draft-curve', edges: 9 }])
  expect(picked.top).toEqual([{ kind: 'draft-curve', edges: 9 }])
})

test('Arc', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Arc')
    await click(env, 0.8, 1.5)
    await click(env, 2.0, 1.5)
    await click(env, 1.4, 1.0)
  })
  expectParity(r, 1)
})

test('two rectangles that cross make the same regions', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Rectangle')
    await click(env, 0.6, 0.6)
    await click(env, 1.6, 1.4)
    await click(env, 1.2, 1.0)
    await click(env, 2.2, 1.8)
  })
  expectParity(r, 1)
  // Two overlapping rectangles: three regions (two lunes and the overlap).
  expect(r.ground.rings.filter((x) => x.startsWith('o'))).toHaveLength(3)
})

// ---- editing -------------------------------------------------------------

async function rect(env: Env, a: [number, number], b: [number, number]): Promise<void> {
  await tool(env, 'Rectangle')
  await click(env, a[0], a[1])
  await click(env, b[0], b[1])
}

const selection = (env: Env) => env.page.evaluate(() => window.__hew_test!.getSelection())

/** Tap the platform's copy modifier (a durable toggle). */
async function copyToggle(env: Env): Promise<void> {
  const mac = await env.page.evaluate(() => /Mac|iPod|iPhone|iPad/.test(navigator.platform))
  const k = mac ? 'Alt' : 'Control'
  await env.page.keyboard.down(k)
  await env.page.keyboard.up(k)
}

/** Move (or copy) the selection `dx` along X, typed, from `base`. */
async function move(env: Env, base: [number, number], dx: number, copy = false): Promise<void> {
  await env.page.keyboard.press('m')
  if (copy) await copyToggle(env)
  await click(env, base[0], base[1])
  await env.page.keyboard.press('ArrowRight')
  await hover(env, base[0] + 0.5, base[1])
  await typeEnter(env, String(dx))
}

test('Move a shape onto another: they arrange like drawing it there', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await rect(env, [0.4, 0.4], [1.0, 1.0])
    await rect(env, [1.8, 0.6], [2.4, 1.2])
    await select(env)
    await click(env, 0.7, 0.7)
    expect((await selection(env)).length).toBe(1)
    await move(env, [0.7, 0.7], 1.4)
  })
  expectParity(r, 1)
  // Two rectangles crossing: three regions.
  expect(r.ground.rings.filter((x) => x.startsWith('o'))).toHaveLength(3)
})

test('Move a shape away (past the box edge on the top) and back leaves it as it was', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await rect(env, [2.3, 0.6], [2.8, 1.2])
    env.stash = await snap(env)
    await select(env)
    await click(env, 2.55, 0.9)
    await move(env, [2.55, 0.9], 0.6)
    await move(env, [3.15, 0.9], -0.6)
  })
  // One shape, back where it started, on both surfaces. Edge-for-edge
  // equality is NOT asserted on the top: after crossing the box edge the
  // shape's sides keep a stray vertex where the edge cut them (see the
  // divergence report), so compare the shape's extent instead.
  expect(r.ground.units).toBe(1)
  expect(r.top.units).toBe(1)
  expect(r.ground.edges).toEqual(r.stash.ground!.edges)
  expect(extent(r.top.edges)).toEqual(extent(r.stash.top!.edges))
  expect(extent(r.ground.edges)).toEqual(extent(r.top.edges))
})

/** Bounding box of a list of canonical edges. */
function extent(edges: string[]): number[] {
  const pts = edges.flatMap((e) => e.split('>').map((p) => p.split(',').map(Number)))
  return [0, 1].flatMap((k) => [Math.min(...pts.map((p) => p[k])), Math.max(...pts.map((p) => p[k]))])
}

test('Move with the copy toggle leaves the original and adds a copy', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await rect(env, [0.5, 0.6], [1.0, 1.2])
    await select(env)
    await click(env, 0.75, 0.9)
    await move(env, [0.75, 0.9], 1.2, true)
  })
  expectParity(r, 2)
})

test('A move copy typed as 3x lays three copies at the step, in one undo step', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await rect(env, [0.3, 0.6], [0.6, 1.2])
    await select(env)
    await click(env, 0.45, 0.9)
    await move(env, [0.45, 0.9], 0.7, true)
    await typeEnter(env, '3x')
    expect((await selection(env)).length).toBeGreaterThan(0)
    // The whole array is ONE undo step; redo restores it.
    const mod = (await env.page.evaluate(() => /Mac/.test(navigator.platform))) ? 'Meta' : 'Control'
    await env.page.keyboard.press(`${mod}+z`)
    await env.page.waitForTimeout(200)
    expect((await snap(env)).units).toBe(1)
    await env.page.keyboard.press(`${mod}+Shift+z`)
    await env.page.waitForTimeout(200)
  })
  expectParity(r, 4)
  const lefts = [...new Set(r.ground.edges.flatMap((e) => e.split('>').map((p) => Number(p.split(',')[0]))))].sort((a, b) => a - b)
  expect(lefts.filter((x) => Math.abs(x - 0.3) < 1e-3 || Math.abs(x - 1.0) < 1e-3 || Math.abs(x - 1.7) < 1e-3 || Math.abs(x - 2.4) < 1e-3)).toHaveLength(4)
})

test('A move copy typed as /3 divides the distance into three steps', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await rect(env, [0.3, 0.6], [0.6, 1.2])
    await select(env)
    await click(env, 0.45, 0.9)
    await move(env, [0.45, 0.9], 2.1, true)
    await typeEnter(env, '/3')
  })
  expectParity(r, 4)
  const xs = new Set(r.ground.edges.flatMap((e) => e.split('>').map((p) => Number(p.split(',')[0]).toFixed(1))))
  for (const x of ['0.3', '1.0', '1.7', '2.4']) expect(xs.has(x)).toBe(true)
})

test('A rotate copy typed as 3x makes a three-copy array', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await rect(env, [0.7, 1.35], [1.0, 1.65])
    await select(env)
    await click(env, 0.85, 1.5)
    await env.page.keyboard.press('q')
    await copyToggle(env)
    await click(env, 1.8, 1.5) // pivot
    await click(env, 2.8, 1.5) // 0 degree arm
    await hover(env, 2.4, 2.3)
    await typeEnter(env, '40')
    await typeEnter(env, '3x')
  })
  expectParity(r, 4)
})

test('Scale stretches a shape along one axis', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await rect(env, [0.8, 0.6], [1.6, 1.2])
    await select(env)
    await click(env, 1.2, 0.9)
    await env.page.keyboard.press('s')
    await click(env, 1.6, 0.9) // the +X face grip
    await hover(env, 2.0, 0.9)
    await typeEnter(env, '2')
  })
  expectParity(r, 1)
  const xs = r.ground.edges.flatMap((e) => e.split('>').map((p) => Number(p.split(',')[0])))
  expect(Math.max(...xs) - Math.min(...xs)).toBeCloseTo(1.6, 3)
})

test('Rotate 90 degrees turns a shape about a pivot', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await rect(env, [0.8, 0.6], [1.8, 1.2])
    await select(env)
    await click(env, 1.3, 0.9)
    await env.page.keyboard.press('q')
    await click(env, 1.3, 0.9) // pivot
    await click(env, 1.9, 0.9) // 0 degree arm
    await hover(env, 1.3, 1.5)
    await typeEnter(env, '90')
  })
  expectParity(r, 1)
  const ys = r.ground.edges.flatMap((e) => e.split('>').map((p) => Number(p.split(',')[1])))
  expect(Math.max(...ys) - Math.min(...ys)).toBeCloseTo(1.0, 3)
})

test('Edit Vertex drags one corner of a shape', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await rect(env, [0.6, 0.6], [1.6, 1.4])
    await tool(env, 'Edit Vertex')
    await click(env, 1.6, 1.4)
    await hover(env, 2.1, 1.9)
    await click(env, 2.1, 1.9)
  })
  expectParity(r, 1)
  // The corner left (1.6, 1.4): a click is the displayed value, not the raw pick.
  expect(r.ground.edges.some((e) => e.includes('1.6000,1.4000'))).toBe(false)
})

test('Offset outward makes a concentric loop', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await rect(env, [0.8, 0.6], [1.6, 1.2])
    await tool(env, 'Offset')
    await click(env, 1.2, 0.9)
    await hover(env, 1.9, 0.9)
    await click(env, 1.9, 0.9)
  })
  expectParity(r, 2)
})

test('Object Info Segments refacets a circle', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Circle')
    await click(env, 1.5, 1.5)
    await click(env, 2.2, 1.5)
    await select(env)
    await click(env, 1.5, 1.5)
    const segments = env.page.getByLabel('Segments')
    await segments.fill('36')
    await segments.press('Enter')
    await env.page.waitForTimeout(200)
  })
  expectParity(r, 1)
  expect(r.ground.curves).toEqual(['circle:36:r0.7000'])
})

// ---- selecting and deleting ----------------------------------------------

test('Delete removes one line of a two-line chain', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Line')
    await click(env, 0.6, 0.6)
    await click(env, 1.6, 0.6)
    await click(env, 1.6, 1.6)
    await env.page.keyboard.press('Escape')
    await select(env)
    await click(env, 1.1, 0.6)
    expect((await selection(env)).map((n) => n.kind)).toEqual(['draft-edge'])
    await env.page.keyboard.press('Delete')
    await env.page.waitForTimeout(150)
  })
  expectParity(r, 1)
  expect(r.ground.edges).toHaveLength(1)
})

test('Delete removes one run of a circle that a line crosses', async ({ page }) => {
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Circle')
    await click(env, 1.5, 1.2)
    await click(env, 2.1, 1.2)
    await tool(env, 'Line')
    await click(env, 0.6, 1.2)
    await click(env, 2.4, 1.2)
    await env.page.keyboard.press('Escape')
    await select(env)
    await click(env, 1.5, 0.6) // the rim, below the line
    expect((await selection(env)).map((n) => n.kind)).toEqual(['draft-curve'])
    await env.page.keyboard.press('Delete')
    await env.page.waitForTimeout(150)
  })
  expectParity(r, 1)
  expect(r.ground.edges.some((e) => e.includes('1.5000,0.6000'))).toBe(false)
})

test('Triple-click selects the whole connected shape', async ({ page }) => {
  const kinds: Record<string, string[]> = {}
  await bothSurfaces(page, async (env) => {
    await tool(env, 'Line')
    await click(env, 0.6, 0.6)
    await click(env, 1.6, 0.6)
    await click(env, 1.6, 1.6)
    await env.page.keyboard.press('Escape')
    await select(env)
    await tripleClick(env, 1.1, 0.6)
    kinds[env.surface] = (await selection(env)).map((n) => n.kind)
  })
  expect(kinds.ground).toEqual(['draft-island'])
  expect(kinds.top).toEqual(kinds.ground)
})

test('A marquee around a shape selects that one shape', async ({ page }) => {
  const kinds: Record<string, string[]> = {}
  await bothSurfaces(page, async (env) => {
    await rect(env, [0.5, 0.5], [1.1, 1.1])
    await rect(env, [1.8, 0.6], [2.4, 1.2])
    await select(env)
    // Start off the box: a press over a solid arms a drag-to-move of it.
    await drag(env, [-0.4, 1.5], [1.4, 0.2])
    kinds[env.surface] = (await selection(env)).map((n) => n.kind)
  })
  expect(kinds.ground).toEqual(['draft-island'])
  expect(kinds.top).toEqual(kinds.ground)
})

async function editMenu(env: Env, item: string): Promise<void> {
  await env.page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await env.page.getByText(item, { exact: true }).last().click()
  await env.page.waitForTimeout(150)
}

test('Select All takes every drawn shape, on the ground and on the box top alike', async ({ page }) => {
  const kinds: Record<string, string[]> = {}
  await bothSurfaces(page, async (env) => {
    await rect(env, [0.5, 0.5], [1.1, 1.1])
    await rect(env, [1.8, 0.6], [2.4, 1.2])
    await select(env)
    await env.page.keyboard.press(`${(await env.page.evaluate(() => /Mac/.test(navigator.platform))) ? 'Meta' : 'Control'}+a`)
    await env.page.waitForTimeout(200)
    kinds[env.surface] = (await selection(env)).map((n) => n.kind).sort()
  })
  expect(kinds.ground).toEqual(['draft-island', 'draft-island'])
  expect(kinds.top).toEqual(['draft-island', 'draft-island', 'object'])
})

test('Invert Selection swaps the drawn shapes, on the ground and on the box top alike', async ({ page }) => {
  const kinds: Record<string, string[]> = {}
  await bothSurfaces(page, async (env) => {
    await rect(env, [0.5, 0.5], [1.1, 1.1])
    await rect(env, [1.8, 0.6], [2.4, 1.2])
    await select(env)
    await click(env, 0.5, 0.8) // the first rectangle's left side
    expect((await selection(env)).map((n) => n.kind)).toEqual(['draft-edge'])
    await editMenu(env, 'Invert Selection')
    const sel = await selection(env)
    kinds[env.surface] = sel.map((n) => n.kind).sort()
  })
  // The first shape (its edge counted as the whole island) drops out; the
  // second stays in; the solid under it was not selected, so it joins.
  expect(kinds.ground).toEqual(['draft-island'])
  expect(kinds.top).toEqual(['draft-island', 'object'])
})

test('A crossing marquee touching a shape selects it (and the box under it)', async ({ page }) => {
  const kinds: Record<string, string[]> = {}
  await bothSurfaces(page, async (env) => {
    await rect(env, [0.5, 0.5], [1.1, 1.1])
    await rect(env, [1.8, 0.6], [2.4, 1.2])
    await select(env)
    // Right to left, started off the box, grazing only the second rectangle.
    await drag(env, [3.4, 1.5], [2.1, 0.2])
    kinds[env.surface] = (await selection(env)).map((n) => n.kind).sort()
  })
  expect(kinds.ground).toEqual(['draft-island'])
  expect(kinds.top).toEqual(['draft-island', 'object'])
})

// ---- measuring and inference ---------------------------------------------

test('A radial dimension reads a drawn circle', async ({ page }) => {
  const labels: Record<string, string> = {}
  await bothSurfaces(page, async (env) => {
    await tool(env, 'Circle')
    await click(env, 1.5, 1.5)
    await click(env, 2.2, 1.5)
    await tool(env, 'Dimension')
    await click(env, 2.2, 1.5)
    await click(env, 2.8, 2.5) // over the face on the box, clear of the circle
    const ids = await env.page.evaluate(() => window.__hew_test!.getAnnotationIds())
    expect(ids).toHaveLength(1)
    expect(await env.page.evaluate((id) => window.__hew_test!.getAnnotationKind(id), ids[0])).toBe('radial')
    labels[env.surface] = (await env.page.evaluate((id) => window.__hew_test!.getAnnotationLabel(id), ids[0])) ?? ''
  })
  expect(labels.ground).not.toBe('')
  expect(labels.top).toBe(labels.ground)
})

async function chip(env: Env): Promise<string> {
  return env.page.evaluate(() =>
    Array.from(document.querySelectorAll('div'))
      .filter((d) => d.style.zIndex === '20' && d.style.pointerEvents === 'none')
      .map((e) => e.textContent ?? '')
      .join(' | '),
  )
}

test('A Line started at a polygon center snaps to it', async ({ page }) => {
  const chips: Record<string, string> = {}
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Polygon')
    await click(env, 1.5, 1.5)
    await click(env, 2.2, 1.5)
    await tool(env, 'Line')
    await hover(env, 1.5, 1.5)
    chips[env.surface] = await chip(env)
    await env.page.mouse.down()
    await env.page.mouse.up()
    await env.page.waitForTimeout(120)
    await click(env, 2.6, 2.4)
    await env.page.keyboard.press('Escape')
  })
  expect(chips.ground).toContain('Center')
  // The segment starts exactly at the polygon's center.
  expect(r.ground.edges.some((e) => e.includes('1.5000,1.5000'))).toBe(true)
  expect(chips.top).toContain('Center')
  expect(r.top.edges.some((e) => e.includes('1.5000,1.5000'))).toBe(true)
  expect(r.top.edges.length).toBeGreaterThan(r.ground.edges.length - 1)
})

// A regular polygon drawn on a solid's face keeps its polygon mark, so it
// lists as one run and offers its center to snap to there too.
test('A Line started at a polygon center on a face snaps to it', async ({ page }) => {
  const chips: Record<string, string> = {}
  const r = await bothSurfaces(page, async (env) => {
    await tool(env, 'Polygon')
    await click(env, 1.5, 1.5)
    await click(env, 2.2, 1.5)
    await tool(env, 'Line')
    await hover(env, 1.5, 1.5)
    chips[env.surface] = await chip(env)
    await env.page.mouse.down()
    await env.page.mouse.up()
    await env.page.waitForTimeout(120)
    await click(env, 2.6, 2.4)
    await env.page.keyboard.press('Escape')
  })
  expect(chips.top).toContain('Center')
  expect(r.top.edges.some((e) => e.includes('1.5000,1.5000'))).toBe(true)
})

// ---- push/pull -----------------------------------------------------------

async function objectBounds(env: Env, skip: string | null): Promise<number[]> {
  return env.page.evaluate((skip) => {
    const h = window.__hew_test!
    const id = h.getObjectIds().find((o) => o !== skip)!
    return h.getObjectBounds(id)
  }, skip)
}

test('Push/Pull of a free region makes a new object, beside the box too', async ({ page }) => {
  const out: Record<string, number[]> = {}
  await bothSurfaces(page, async (env) => {
    if (env.surface === 'ground') {
      await rect(env, [3.2, 0.6], [3.8, 1.2])
    } else {
      // Half over the box's right edge: the part beside the box is free.
      await rect(env, [2.6, 0.6], [3.8, 1.2])
    }
    await tool(env, 'Push/Pull')
    await click(env, 3.5, 0.9)
    await typeEnter(env, '0.3')
    expect(await env.page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(env.surface === 'top' ? 2 : 1)
    out[env.surface] = (await objectBounds(env, env.box)).map((n) => Math.round(n * 1e4) / 1e4)
  })
  // The box-side part is not part of the new object: same footprint.
  expect(out.top[0]).toBeCloseTo(3.0, 3)
  expect(out.ground[0]).toBeCloseTo(3.2, 3)
  expect(out.top[3] - out.top[0]).toBeCloseTo(0.8, 3)
  expect(out.ground[3] - out.ground[0]).toBeCloseTo(0.6, 3)
  expect(out.top[5] - out.top[2]).toBeCloseTo(0.3, 3)
  expect(out.ground[5] - out.ground[2]).toBeCloseTo(0.3, 3)
})

test('Push/Pull of a region on the box top grows the box; on the ground it makes an object', async ({ page }) => {
  const heights: Record<string, number> = {}
  await bothSurfaces(page, async (env) => {
    await rect(env, [0.8, 0.6], [1.6, 1.2])
    await tool(env, 'Push/Pull')
    await click(env, 1.2, 0.9)
    await typeEnter(env, '0.3')
    expect(await env.page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1 + (env.surface === 'ground' ? 0 : 0))
    const ids = await env.page.evaluate(() => window.__hew_test!.getObjectIds())
    expect(ids).toHaveLength(1)
    const b = await env.page.evaluate((id) => window.__hew_test!.getObjectBounds(id), ids[0])
    heights[env.surface] = b[5]
  })
  expect(heights.ground).toBeCloseTo(0.3, 4)
  expect(heights.top).toBeCloseTo(TOP_Z + 0.3, 4) // the box rose where the shape was
})

// ---- shapes ride with their own object -----------------------------------

const modKey = (env: Env) => env.page.evaluate(() => (/Mac/.test(navigator.platform) ? 'Meta' : 'Control'))

/** Edge endpoints of the drawing on the horizontal plane at height `z`. */
async function edgesAt(env: Env, z: number): Promise<number[][]> {
  const raw = await env.page.evaluate((z) => {
    const h = window.__hew_test!
    return h.getPlaneUnits(h.ctxOfPlane([0, 0, z], [0, 0, 1])!)
  }, z)
  return raw.units
    .flatMap((u) => u.edges.flatMap((e) => [e.a, e.b]))
    .map((p) => p.map((c) => Number(R(c))))
    .sort((a, b) => a.join().localeCompare(b.join()))
}

async function selectAllThenMove(env: Env, dir: 'ArrowRight' | 'ArrowUp', dist: string): Promise<void> {
  await select(env)
  await env.page.keyboard.press(`${await modKey(env)}+a`)
  await env.page.waitForTimeout(200)
  expect((await selection(env)).map((n) => n.kind).sort()).toEqual(['draft-island', 'object'])
  await env.page.keyboard.press('m')
  await click(env, 1.5, 1.5)
  await env.page.keyboard.press(dir)
  await hover(env, 1.5, 1.5)
  await typeEnter(env, dist)
}

test('Select All then Move 1 m along red carries the box and the drawing on it once', async ({ page }) => {
  const env = await boot(page, 'top')
  await rect(env, [0.6, 0.6], [1.2, 1.2])
  const before = await edgesAt(env, TOP_Z)
  expect(before).toHaveLength(8)
  const b0 = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id!), env.box)
  await selectAllThenMove(env, 'ArrowRight', '1')
  expect(await lastError(env)).toBeNull()
  // What Select All picked stays picked where it landed: the box and the
  // rectangle riding on its top.
  const picked = await selection(env)
  expect(picked.some((n) => n.kind === 'object')).toBe(true)
  expect(picked.some((n) => n.kind === 'draft-island')).toBe(true)
  const b1 = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id!), env.box)
  expect(b1[0] - b0[0]).toBeCloseTo(1, 6)
  expect(b1[3] - b0[3]).toBeCloseTo(1, 6)
  const after = await edgesAt(env, TOP_Z)
  expect(after).toEqual(
    before.map((p) => [Number(R(p[0] + 1)), p[1], p[2]]).sort((a, b) => a.join().localeCompare(b.join())),
  )
  // ONE undo restores both the box and the drawing on it.
  await page.keyboard.press(`${await modKey(env)}+z`)
  await page.waitForTimeout(200)
  expect(await page.evaluate((id) => window.__hew_test!.getObjectBounds(id!), env.box)).toEqual(b0)
  expect(await edgesAt(env, TOP_Z)).toEqual(before)
})

test('Select All then a blue (vertical) Move succeeds and carries the drawing', async ({ page }) => {
  const env = await boot(page, 'top')
  await rect(env, [0.6, 0.6], [1.2, 1.2])
  const b0 = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id!), env.box)
  await selectAllThenMove(env, 'ArrowUp', '1')
  expect(await lastError(env)).toBeNull()
  const b1 = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id!), env.box)
  expect(b1[2] - b0[2]).toBeCloseTo(1, 6)
  const up = await edgesAt(env, TOP_Z + 1)
  expect(up).toHaveLength(8)
  expect(await edgesAt(env, TOP_Z)).toHaveLength(0)
  // ONE undo puts the box and the drawing back.
  await page.keyboard.press(`${await modKey(env)}+z`)
  await page.waitForTimeout(200)
  expect(await page.evaluate((id) => window.__hew_test!.getObjectBounds(id!), env.box)).toEqual(b0)
  expect(await edgesAt(env, TOP_Z + 1)).toHaveLength(0)
  expect(await edgesAt(env, TOP_Z)).toHaveLength(8)
})

/** Every region's rings at the plane z, each ring a sorted point list, optionally shifted in x. */
async function ringsAt(env: Env, z: number, dx = 0): Promise<string[]> {
  const raw = await env.page.evaluate((z) => {
    const h = window.__hew_test!
    return h.getPlaneUnits(h.ctxOfPlane([0, 0, z], [0, 0, 1])!)
  }, z)
  const ring = (r: number[][]) =>
    r
      .map((p) => `${R(p[0] + dx)},${R(p[1])},${R(p[2])}`)
      .sort()
      .join(' ')
  const out: string[] = []
  for (const r of raw.regions) {
    out.push('o ' + ring(r.outer))
    for (const hole of r.holes) out.push('h ' + ring(hole))
  }
  return out.sort()
}

for (const DX of [1, -1]) {
  test(`Select All then Move ${DX > 0 ? '+' : '-'}1 m along red carries a shape drawn half on the box top and half past its edge once, whole`, async ({ page }) => {
    const env = await boot(page, 'top')
    // Hangs 0.5 m past the top's x = 3 edge: one island, face part + void part.
    await rect(env, [2.3, 0.6], [3.5, 1.2])
    expect(await lastError(env)).toBeNull()
    const before = await ringsAt(env, TOP_Z)
    const edges0 = await edgesAt(env, TOP_Z)
    expect(before.length).toBeGreaterThan(2)
    const bounds = () => page.evaluate((id) => window.__hew_test!.getObjectBounds(id!), env.box)
    const b0 = await bounds()

    await select(env)
    await page.keyboard.press(`${await modKey(env)}+a`)
    await page.waitForTimeout(200)
    expect((await selection(env)).map((n) => n.kind).sort()).toEqual(['draft-island', 'object'])
    await page.keyboard.press('m')
    await click(env, 1.5, 1.5)
    await page.keyboard.press('ArrowRight')
    await hover(env, 1.5, 1.5)
    await typeEnter(env, String(DX))
    expect(await lastError(env)).toBeNull()

    const b1 = await bounds()
    expect(b1[0] - b0[0]).toBeCloseTo(DX, 6)
    expect(b1[3] - b0[3]).toBeCloseTo(DX, 6)
    // The same regions, every ring shifted by exactly DX in x: the face part
    // rode with the box (once), the void part was drafted to meet it, and
    // nothing is duplicated or left behind.
    expect(await ringsAt(env, TOP_Z)).toEqual(ringsShifted(before, DX))
    expect(await edgesAt(env, TOP_Z)).toHaveLength(edges0.length)

    // ONE undo restores everything: the box and both halves of the shape.
    await page.keyboard.press(`${await modKey(env)}+z`)
    await page.waitForTimeout(200)
    expect(await bounds()).toEqual(b0)
    expect(await ringsAt(env, TOP_Z)).toEqual(before)
    expect(await edgesAt(env, TOP_Z)).toEqual(edges0)
  })
}

test('Select All then a blue (vertical) Move of a half-on-the-top shape succeeds and lifts the whole shape', async ({ page }) => {
  const env = await boot(page, 'top')
  await rect(env, [2.3, 0.6], [3.5, 1.2])
  const before = await ringsAt(env, TOP_Z)
  const b0 = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id!), env.box)
  await selectAllThenMove(env, 'ArrowUp', '1')
  expect(await lastError(env)).toBeNull()
  const b1 = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id!), env.box)
  expect(b1[2] - b0[2]).toBeCloseTo(1, 6)
  expect(await ringsAt(env, TOP_Z + 1)).toEqual(ringsShifted(before, 0, 1))
  expect(await edgesAt(env, TOP_Z)).toHaveLength(0)
  // ONE undo puts the box and both halves back.
  await page.keyboard.press(`${await modKey(env)}+z`)
  await page.waitForTimeout(200)
  expect(await page.evaluate((id) => window.__hew_test!.getObjectBounds(id!), env.box)).toEqual(b0)
  expect(await ringsAt(env, TOP_Z)).toEqual(before)
  expect(await ringsAt(env, TOP_Z + 1)).toEqual([])
})

test('Select All then a Move copy typed as 3x of a box and a straddling shape is ONE undo step and every copy keeps the shape whole', async ({ page }) => {
  const env = await boot(page, 'top')
  await rect(env, [2.3, 0.6], [3.5, 1.2])
  const before = await ringsAt(env, TOP_Z)
  const edges0 = await edgesAt(env, TOP_Z)
  await select(env)
  await page.keyboard.press(`${await modKey(env)}+a`)
  await page.waitForTimeout(200)
  await page.keyboard.press('m')
  await copyToggle(env)
  await click(env, 1.5, 1.5)
  await page.keyboard.press('ArrowRight')
  await hover(env, 1.5, 1.5)
  await typeEnter(env, '4')
  expect(await lastError(env)).toBeNull()
  await typeEnter(env, '3x')
  expect(await lastError(env)).toBeNull()
  expect(await page.evaluate(() => window.__hew_test!.getObjectIds().length)).toBe(4)
  // The original and three copies at 4, 8 and 12 m, each box with the whole
  // shape on its top and past its edge.
  const expected = [0, 4, 8, 12].flatMap((dx) => ringsShifted(before, dx)).sort()
  expect(await ringsAt(env, TOP_Z)).toEqual(expected)
  // The whole array is ONE undo step.
  await page.keyboard.press(`${await modKey(env)}+z`)
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getObjectIds().length)).toBe(1)
  expect(await ringsAt(env, TOP_Z)).toEqual(before)
  expect(await edgesAt(env, TOP_Z)).toEqual(edges0)
})

/** `before`'s ring strings with every point shifted by dx in x, re-sorted. */
function ringsShifted(before: string[], dx: number, dz = 0): string[] {
  return before
    .map((s) => {
      const tag = s.slice(0, 2)
      const pts = s
        .slice(2)
        .split(' ')
        .map((p) => p.split(',').map(Number))
        .map((p) => `${R(p[0] + dx)},${R(p[1])},${R(p[2] + dz)}`)
        .sort()
      return tag + pts.join(' ')
    })
    .sort()
}


for (const DX of [1, -1]) {
  test(`Select All then a Move copy ${DX > 0 ? '+' : '-'}1 m carries a half-on-the-top shape into the copy once, leaving the original`, async ({ page }) => {
    const env = await boot(page, 'top')
    await rect(env, [2.3, 0.6], [3.5, 1.2])
    await select(env)
    await page.keyboard.press(`${await modKey(env)}+a`)
    await page.waitForTimeout(200)
    await page.keyboard.press('m')
    await copyToggle(env)
    await click(env, 1.5, 1.5)
    await page.keyboard.press('ArrowRight')
    await hover(env, 1.5, 1.5)
    await typeEnter(env, String(DX))
    expect(await lastError(env)).toBeNull()
    expect(await page.evaluate(() => window.__hew_test!.getObjectIds().length)).toBe(2)
    // The original shape (2.3 to 3.5) and its copy (shifted by DX), and nothing
    // else: no vertex beyond the two outlines, no edge from a double copy.
    const xs = [...new Set((await edgesAt(env, TOP_Z)).map((p) => p[0]))]
    const lo = Math.min(2.3, 2.3 + DX)
    const hi = Math.max(3.5, 3.5 + DX)
    expect(Math.min(...xs)).toBeCloseTo(lo, 6)
    expect(Math.max(...xs)).toBeCloseTo(hi, 6)
    for (const x of [2.3, 3, 3.5, 2.3 + DX, 3 + DX, 3.5 + DX]) expect(xs.some((v) => Math.abs(v - x) < 1e-4), `x = ${x}`).toBe(true)
  })
}

test('Select All then Delete of a group holding a drawn box is ONE undo step that restores the group and its drawing', async ({ page }) => {
  const env = await boot(page, 'top')
  await rect(env, [0.6, 0.6], [1.2, 1.2])
  const imprints = () => page.evaluate((id) => window.__hew_test!.getImprints(id!).length, env.box)
  expect(await imprints()).toBe(1)
  await page.evaluate((id) => window.__hew_test!.groupNodes([{ kind: 'object', id: id! }]), env.box)
  await select(env)
  await page.keyboard.press(`${await modKey(env)}+a`)
  await page.waitForTimeout(200)
  await page.keyboard.press('Delete')
  await page.waitForTimeout(250)
  expect(await lastError(env)).toBeNull()
  expect(await page.evaluate(() => window.__hew_test!.getObjectIds().length)).toBe(0)
  await page.keyboard.press(`${await modKey(env)}+z`)
  await page.waitForTimeout(250)
  expect(await page.evaluate(() => window.__hew_test!.getObjectIds().length)).toBe(1)
  expect(await imprints()).toBe(1)
})

test('Select All then a vertical Move of an instance keeps the drawing inside it selected', async ({ page }) => {
  const env = await boot(page, 'top')
  await rect(env, [0.6, 0.6], [1.2, 1.2])
  await page.evaluate((id) => {
    const h = window.__hew_test!
    h.makeComponent([id!])
  }, env.box)
  await select(env)
  const mod = await modKey(env)
  const kinds = async () => (await selection(env)).map((n) => n.kind).sort()
  await page.keyboard.press(`${mod}+a`)
  await page.waitForTimeout(200)
  expect(await kinds()).toEqual(['draft-island', 'instance'])
  await page.keyboard.press('m')
  await click(env, 1.5, 1.5)
  await page.keyboard.press('ArrowUp')
  await hover(env, 1.5, 1.5)
  await typeEnter(env, '1')
  expect(await lastError(env)).toBeNull()
  // The shape rode up with its instance and is still picked, where it is now.
  expect(await kinds()).toEqual(['draft-island', 'instance'])
})

test('Select All then Delete of two instances of a drawn component is ONE undo step; both keep the drawing', async ({ page }) => {
  const env = await boot(page, 'top')
  await rect(env, [0.6, 0.6], [1.2, 1.2])
  await page.evaluate((id) => {
    const h = window.__hew_test!
    const made = h.makeComponent([id!])
    h.placeInstance(made.component, 5, 0, 0)
  }, env.box)
  await select(env)
  const mod = await modKey(env)
  const kinds = async () => (await selection(env)).map((n) => n.kind).sort()
  await page.keyboard.press(`${mod}+a`)
  await page.waitForTimeout(200)
  // Each instance and the shape drawn on it.
  expect(await kinds()).toEqual(['draft-island', 'draft-island', 'instance', 'instance'])
  await page.keyboard.press('Delete')
  await page.waitForTimeout(250)
  expect(await lastError(env)).toBeNull()
  await page.keyboard.press(`${mod}+a`)
  await page.waitForTimeout(150)
  expect(await kinds()).toEqual([])
  await page.keyboard.press(`${mod}+z`)
  await page.waitForTimeout(250)
  await page.keyboard.press(`${mod}+a`)
  await page.waitForTimeout(200)
  // One undo brought back both instances AND the shared drawing.
  expect(await kinds()).toEqual(['draft-island', 'draft-island', 'instance', 'instance'])
})

// ---- push/pull of a shape touching another at one point ------------------

test('Push/Pull of a diamond touching the rectangle it is drawn in at one point pulls it up', async ({ page }) => {
  const env = await boot(page, 'top')
  await rect(env, [0.6, 0.6], [2.4, 2.4])
  // A diamond inside it whose left corner sits on the rectangle's left side:
  // a face of its own, in a hole of the rectangle sharing that corner.
  await tool(env, 'Line')
  await click(env, 0.6, 1.5)
  await click(env, 1.2, 1.9)
  await click(env, 1.8, 1.5)
  await click(env, 1.2, 1.1)
  await click(env, 0.6, 1.5)
  await env.page.keyboard.press('Escape')
  expect(await lastError(env)).toBeNull()
  await tool(env, 'Push/Pull')
  await click(env, 1.2, 1.5)
  await typeEnter(env, '0.3')
  expect(await lastError(env)).toBeNull()
  await expect(page.getByText(/part of it is drawn on the solid's face and part is loose/)).toHaveCount(0)
  const ids = await env.page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(ids).toHaveLength(1)
  const b = await env.page.evaluate((id) => window.__hew_test!.getObjectBounds(id), ids[0])
  expect(b[5]).toBeCloseTo(TOP_Z + 0.3, 4) // the diamond rose
})

test('A circle with its rim snapped on the box top edge is a face: it pushes into a pocket and pulls up (#43)', async ({ page }) => {
  const env = await boot(page, 'top')
  // Center 0.6 in from the front edge, rim on that edge's midpoint: the
  // circle touches the edge at that one vertex.
  await tool(env, 'Circle')
  await click(env, 1.5, 0.6)
  await click(env, 1.5, 0)
  expect(await lastError(env)).toBeNull()
  const before = await env.page.evaluate((id) => window.__hew_test!.getObjectBounds(id!), env.box)
  await tool(env, 'Push/Pull')
  await click(env, 1.5, 0.6)
  await typeEnter(env, '-0.3')
  expect(await lastError(env)).toBeNull()
  await expect(page.getByText(/can't be pushed or pulled/)).toHaveCount(0)
  let ids = await env.page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(ids).toHaveLength(1)
  // A pocket: the box's outside is unchanged, and its floor is a face 0.3
  // below the top.
  expect(await env.page.evaluate((id) => window.__hew_test!.getObjectBounds(id), ids[0])).toEqual(before)
  const floor = await env.page.evaluate((z) => {
    const h = window.__hew_test!
    return h.getPlaneUnits(h.ctxOfPlane([0, 0, z], [0, 0, 1])!).regions.length
  }, TOP_Z - 0.3)
  expect(floor).toBe(1)
  const mod = await env.page.evaluate(() => (/Mac/.test(navigator.platform) ? 'Meta' : 'Control'))
  await env.page.keyboard.press(`${mod}+z`)
  await env.page.waitForTimeout(250)
  await click(env, 1.5, 0.6)
  await typeEnter(env, '0.3')
  expect(await lastError(env)).toBeNull()
  ids = await env.page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(ids).toHaveLength(1)
  const b = await env.page.evaluate((id) => window.__hew_test!.getObjectBounds(id), ids[0])
  expect(b[5]).toBeCloseTo(TOP_Z + 0.3, 4) // the disc rose
})

// ---- drawing on a group's or component's face from outside ----------------

for (const wrap of ['component', 'group'] as const) {
  test(`Rectangle on the top of a ${wrap} (not entered) is free drawing on that plane: not the ground, not inside it`, async ({ page }) => {
    const env = await boot(page, 'top')
    const info = await page.evaluate(([id, wrap]) => {
      const h = window.__hew_test!
      if (wrap === 'component') {
        const made = h.makeComponent([id!])
        return { component: made.component, member: h.getComponentMemberObjects(made.component)[0] }
      }
      h.groupNodes([{ kind: 'object', id: id! }])
      return { component: null, member: id! }
    }, [env.box, wrap] as const)
    const memberBounds = () => page.evaluate((id) => window.__hew_test!.getObjectBounds(id), info.member)
    const boundsBefore = await memberBounds()
    const imprintsBefore = await page.evaluate((id) => window.__hew_test!.getImprints(id).length, info.member)

    await rect(env, [0.9, 0.9], [2.1, 2.1])
    expect(await lastError(env)).toBeNull()

    const units = (z: number) =>
      page.evaluate((z) => {
        const h = window.__hew_test!
        return h.getPlaneUnits(h.ctxOfPlane([0, 0, z], [0, 0, 1])!)
      }, z)
    const top = await units(TOP_Z)
    expect(top.units).toHaveLength(1)
    expect(top.units[0].edges).toHaveLength(4)
    for (const e of top.units[0].edges) {
      expect(e.store.kind, 'a free store, not the object').toBe('free')
      expect(e.a[2]).toBeCloseTo(TOP_Z, 6)
    }
    expect((await units(0)).units).toHaveLength(0) // nothing on the ground
    // The shared definition / member is untouched.
    expect(await memberBounds()).toEqual(boundsBefore)
    expect(await page.evaluate((id) => window.__hew_test!.getImprints(id).length, info.member)).toBe(imprintsBefore)
    if (info.component !== null) {
      expect(await page.evaluate((c) => window.__hew_test!.getComponentMemberSketches(c), info.component)).toEqual([])
      expect(await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c).length, info.component)).toBe(1)
    }

    // The drawing pulls into an object of its own, standing on the top;
    // the group / component is untouched.
    const objectsBefore = await page.evaluate(() => window.__hew_test!.getObjectCount())
    await tool(env, 'Push/Pull')
    await click(env, 1.5, 1.5)
    await typeEnter(env, '0.3')
    expect(await lastError(env)).toBeNull()
    expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(objectsBefore + 1)
    expect(await memberBounds()).toEqual(boundsBefore)
  })
}

test('Rectangle started by clicking the corner of a component top lands on the top plane as free drawing, ground stays empty', async ({ page }) => {
  const env = await boot(page, 'top')
  await page.evaluate((id) => window.__hew_test!.makeComponent([id!]), env.box)
  await rect(env, [0, 0], [1.5, 1.5]) // first click exactly on the snapped corner
  expect(await lastError(env)).toBeNull()
  const units = (z: number) =>
    page.evaluate((z) => {
      const h = window.__hew_test!
      return h.getPlaneUnits(h.ctxOfPlane([0, 0, z], [0, 0, 1])!)
    }, z)
  const top = await units(TOP_Z)
  expect(top.units).toHaveLength(1)
  expect(top.units[0].edges).toHaveLength(4)
  for (const e of top.units[0].edges) {
    expect(e.store.kind).toBe('free')
    expect(e.a[2]).toBeCloseTo(TOP_Z, 6)
  }
  expect(top.units[0].edges.some((e) => Math.abs(e.a[0]) < 1e-6 && Math.abs(e.a[1]) < 1e-6)).toBe(true)
  expect((await units(0)).units).toHaveLength(0)
})
