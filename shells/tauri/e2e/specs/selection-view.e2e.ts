/**
 * Desktop: selection and view state over the bus (docs/agents/HEW_API.md
 * §7, "Selection and view state"). The harness dispatches on its own live
 * API connection through the bridge's own path, directives applied, and
 * the spec reads the app's selection and hidden state back after each call.
 * Wiring proof for the real WebKitGTK window: the command rules and the
 * bridge's directive arms have their own suites.
 */

export {}

type Vec3 = [number, number, number]

interface Reply {
  result?: Record<string, unknown>
  error?: { code: number; message: string; data?: Record<string, unknown> }
}

interface HarnessSlice {
  isReady(): boolean
  setCamera(pose: { position: Vec3; target: Vec3; up?: Vec3; fovDeg?: number }): void
  drawBox(p0: Vec3, p1: Vec3, height: number): string
  getSelection(): { kind: string; id: string }[]
  isNodeHidden(node: { kind: string; id: string }): boolean
  apiDispatch(request: { method: string; params?: unknown }): Reply
}

// A local window type rather than a global augmentation: each spec keeps its
// own slice, and two augmentations of `Window.__hew_test` would conflict.
type HarnessWindow = Window & { __hew_test?: HarnessSlice }

/** One bus call. The reply crosses the driver as JSON text: WebKitWebDriver
 * fails to hand back some replies as objects (a `null` member, nesting). */
async function bus(method: string, params: unknown = {}): Promise<Reply> {
  const text = await browser.execute(
    (m, p) => JSON.stringify((window as HarnessWindow).__hew_test!.apiDispatch({ method: m, params: p })),
    method,
    params,
  )
  return JSON.parse(text) as Reply
}

describe('Hew desktop: selection and view state over the bus', () => {
  it('selects, hides, shows, and isolates through the live bridge', async () => {
    await browser.waitUntil(
      async () => (await browser.execute(() => (window as HarnessWindow).__hew_test?.isReady() === true)) === true,
      { timeout: 30_000, timeoutMsg: 'harness/kernel never became ready' },
    )
    await browser.execute(() => {
      const h = (window as HarnessWindow).__hew_test!
      h.setCamera({ position: [10, 8, 8], target: [2, 1, 0.5], up: [0, 0, 1], fovDeg: 45 })
      h.drawBox([0, 0, 0], [1, 1, 0], 1)
      h.drawBox([3, 0, 0], [4, 1, 0], 1)
    })
    const scene = await bus('hew.query.scene')
    const tree = (scene.result?.tree ?? []) as { id: string }[]
    expect(tree.length).toBe(2)
    const [a, b] = tree.map((n) => n.id)

    // Replace, then add: the app's own selection follows.
    expect((await bus('hew.selection.set', { ids: [a] })).result?.ids).toEqual([a])
    await browser.waitUntil(async () => (await browser.execute(() => (window as HarnessWindow).__hew_test!.getSelection().length)) === 1)
    const [nodeA] = await browser.execute(() => (window as HarnessWindow).__hew_test!.getSelection())
    expect(nodeA.kind).toBe('object')
    expect((await bus('hew.selection.set', { ids: [b], mode: 'add' })).result?.ids).toEqual([a, b])
    await browser.waitUntil(async () => (await browser.execute(() => (window as HarnessWindow).__hew_test!.getSelection().length)) === 2)
    expect((await bus('hew.selection.get')).result?.ids).toEqual([a, b])

    // Hide: the Outliner's hidden set follows, and a hidden target refuses.
    expect((await bus('hew.view.hide', { ids: [a] })).result).toEqual({ hidden: [a] })
    await browser.waitUntil(
      async () => (await browser.execute((n) => (window as HarnessWindow).__hew_test!.isNodeHidden(n), nodeA)) === true,
      { timeoutMsg: 'the app never saw the hide' },
    )
    const refused = await bus('hew.selection.set', { ids: [a] })
    expect(refused.error?.data?.refusal).toBe('selection_target_not_visible')

    // Show everything again.
    expect((await bus('hew.view.show_all')).result).toEqual({ hidden: [] })
    await browser.waitUntil(
      async () => (await browser.execute((n) => (window as HarnessWindow).__hew_test!.isNodeHidden(n), nodeA)) === false,
      { timeoutMsg: 'the app never saw the show' },
    )

    // Isolate b: hidden reports it; clearing ends it.
    expect((await bus('hew.view.isolate', { ids: [b] })).result).toEqual({ isolated: [b] })
    expect((await bus('hew.view.hidden')).result?.isolated).toEqual([b])
    await bus('hew.view.clear_isolation')
    expect((await bus('hew.view.hidden')).result?.isolated).toBeNull()

    // Framing answers the box before padding.
    expect((await bus('hew.view.zoom_to', { ids: [a], padding: 0.5 })).result).toEqual({
      min: [0, 0, 0],
      max: [1, 1, 1],
    })

    // Clear empties the app's selection.
    await bus('hew.selection.clear')
    await browser.waitUntil(async () => (await browser.execute(() => (window as HarnessWindow).__hew_test!.getSelection().length)) === 0)
  })
})
