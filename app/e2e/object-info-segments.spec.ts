import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

async function ready(page: Page) {
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
}

type Rgb = { r: number; g: number; b: number }
const near = (a: Rgb, b: Rgb) =>
  Math.abs(a.r - b.r) + Math.abs(a.g - b.g) + Math.abs(a.b - b.b) < 24

test('Object Info: changing a circle\'s Segments redraws it on screen', async ({ page }) => {
  await page.goto('/')
  await ready(page)

  // A hexagon of radius 1, looked at straight down. Its flat sides sit at
  // 0.866 from the centre, so a point at 0.95 across one of them is outside
  // the hexagon but inside the 48-sided circle it becomes. All three sample
  // points sit on that one ray, clear of the axis lines.
  const at = (r: number): [number, number, number] => [r * Math.cos(Math.PI / 6), r * Math.sin(Math.PI / 6), 0]
  const { sketch } = await page.evaluate(() => {
    const t = window.__hew_test!
    const drawn = t.drawCircle([0, 0, 0], 1, 6)
    t.setCamera({ position: [0, 0, 6], target: [0, 0, 0], up: [0, 1, 0] })
    return drawn
  })
  const pixels = () =>
    page.evaluate(([inside, probe, outside]) => {
      const t = window.__hew_test!
      return { fill: t.pixelColorAt(inside)!, probe: t.pixelColorAt(probe)!, outside: t.pixelColorAt(outside)! }
    }, [at(0.5), at(0.95), at(1.1)])

  const before = await pixels()
  expect(near(before.fill, before.outside)).toBe(false)
  expect(near(before.probe, before.outside)).toBe(true)

  await page.evaluate((sk) => {
    const t = window.__hew_test!
    t.selectNodes([{ kind: 'sketch-curve', id: t.getSketchEdgeIds(sk)[0], sketch: sk }])
  }, sketch)
  const segments = page.getByLabel('Segments')
  await segments.fill('48')
  await segments.press('Enter')

  await expect.poll(async () => near((await pixels()).probe, before.fill)).toBe(true)
})
