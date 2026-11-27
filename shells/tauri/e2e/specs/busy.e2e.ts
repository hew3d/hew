/**
 * Desktop: a plugin command holding the document (docs/agents/PLUGINS.md
 * "Compounds"). The harness opens a compound as a plugin command would;
 * the window then ignores the person's input, shows the busy notice after
 * a quarter second, and Cancel puts everything back. Wiring proof for the
 * real WebKitGTK window: the kernel and bridge halves have their own
 * suites.
 */

export {}

type Vec3 = [number, number, number]

interface HarnessSlice {
  isReady(): boolean
  setCamera(pose: { position: Vec3; target: Vec3; up?: Vec3; fovDeg?: number }): void
  drawBox(p0: Vec3, p1: Vec3, height: number): string
  getObjectCount(): number
  getStateHash(): string
  beginCompound(label: string): number
  isBusy(): boolean
}

// A local window type rather than a global augmentation: each spec keeps its
// own slice, and two augmentations of `Window.__hew_test` would conflict.
type HarnessWindow = Window & { __hew_test?: HarnessSlice }

describe('Hew desktop: a plugin command holds the document', () => {
  it('ignores input while busy, shows the notice, and Cancel frees the document', async () => {
    // The welcome screen is read once at startup and would sit over the
    // viewport, so turn it off and reload before the first step.
    await browser.execute(() => localStorage.setItem('hew.settings.showWelcome', 'false'))
    await browser.refresh()
    await browser.waitUntil(
      async () => (await browser.execute(() => (window as HarnessWindow).__hew_test?.isReady() === true)) === true,
      { timeout: 30_000, timeoutMsg: 'harness/kernel never became ready' },
    )
    const before = await browser.execute(() => {
      const h = (window as HarnessWindow).__hew_test!
      h.setCamera({ position: [8, 6, 8], target: [1, 1, 1], up: [0, 0, 1], fovDeg: 45 })
      h.drawBox([0, 0, 0], [2, 2, 0], 2)
      return h.getStateHash()
    })

    await browser.execute(() => (window as HarnessWindow).__hew_test!.beginCompound('test'))
    expect(await browser.execute(() => (window as HarnessWindow).__hew_test!.isBusy())).toBe(true)

    // Undo by keyboard, a tool key, and a click in the viewport: none acts.
    await browser.keys(['Control', 'z'])
    await browser.keys('r')
    const canvas = await $('canvas')
    await canvas.click()
    expect(await browser.execute(() => (window as HarnessWindow).__hew_test!.getStateHash())).toBe(before)

    // The notice comes up after a quarter second, naming the command.
    const notice = await $('[data-testid="busy-notice"]')
    await notice.waitForDisplayed({ timeout: 5_000 })
    expect(await notice.getText()).toContain('test is working')

    // Cancel: the notice goes, the document is free, and drawing works.
    await (await notice.$('button')).click()
    await notice.waitForDisplayed({ reverse: true, timeout: 5_000 })
    const after = await browser.execute(() => {
      const h = (window as HarnessWindow).__hew_test!
      const busy = h.isBusy()
      h.drawBox([4, 0, 0], [5, 1, 0], 1)
      return { busy, objects: h.getObjectCount() }
    })
    expect(after.busy).toBe(false)
    expect(after.objects).toBe(2)
  })
})
