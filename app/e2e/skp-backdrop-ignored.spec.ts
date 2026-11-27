import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/** A SketchUp file's background, sky and ground colors are not used: a loud
 * style under either theme paints the theme's own backdrop, scenes included,
 * while the kernel keeps and saves the style untouched. Scenes apply their
 * style through the same viewport gate (`documentBackdropColors`). */
const LOUD = { background: [255, 0, 255], sky: [0, 255, 0], ground: [255, 255, 0], axes: true }
const CLEAR = { dark: 0x15181d, light: 0xf2f5f9 }

async function setup(page: Page, theme: 'dark' | 'light'): Promise<void> {
  await page.addInitScript((t) => localStorage.setItem('hew.settings.theme', t), theme)
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

for (const theme of ['dark', 'light'] as const) {
  test(`loud document style paints the ${theme} theme backdrop`, async ({ page }, testInfo) => {
    await setup(page, theme)
    const before = await page.evaluate(() => window.__hew_test!.getBackdropInfo())
    expect(before.clearColor).toBe(CLEAR[theme])

    await page.evaluate((s) => {
      const h = window.__hew_test!
      h.drawBox([0, 0, 0], [2, 2, 0], 1)
      h.setViewStyleJson(JSON.stringify(s))
      // Real Open path: save then load applies the style as a file open does.
      h.load(h.save())
    }, LOUD)

    // The kernel kept the file's style (save/reload untouched).
    const stored = await page.evaluate(() => JSON.parse(window.__hew_test!.getViewStyleJson()!))
    expect(stored).toMatchObject(LOUD)

    const after = await page.evaluate(() => window.__hew_test!.getBackdropInfo())
    expect(after.clearColor).toBe(CLEAR[theme])
    expect(after.groundColor).toBe(before.groundColor)
    expect(after.groundFilled).toBe(before.groundFilled)

    await page.screenshot({ path: `${process.env.HEW_SHOT_DIR ?? testInfo.outputDir}/backdrop-${theme}.png` })
  })
}
