import { test, expect } from '@playwright/test'

/**
 * Boot smoke. Proves the whole web pipeline end to end *now*, before
 * the semantic harness lands: the built app is served, React mounts, the
 * WASM kernel loads, and the WebGL2 viewport gets a live canvas — with no
 * console errors and no crash fallback. The richer draw → push/pull →
 * save/reload flow is.
 */

// Console noise that is not a real failure in a headless/preview context.
const BENIGN_CONSOLE = [
  /ServiceWorker/i,
  /service worker/i,
  /workbox/i,
  /Manifest/i,
  /favicon/i,
]

test('web build boots: app mounts, kernel loads, viewport canvas is live', async ({
  page,
}) => {
  const consoleErrors: string[] = []
  page.on('console', (msg) => {
    if (msg.type() !== 'error') return
    const text = msg.text()
    if (BENIGN_CONSOLE.some((re) => re.test(text))) return
    consoleErrors.push(text)
  })
  const pageErrors: string[] = []
  page.on('pageerror', (err) => pageErrors.push(err.message))

  await page.goto('/')

  // React mounted into #root.
  const root = page.locator('#root')
  await expect(root).toBeAttached()
  await expect(root).not.toBeEmpty()

  // The viewport's WebGL2 canvas exists, is visible, and has real size — this
  // only happens after renderer init succeeds (the fallback path appends a
  // plain <div>, not a <canvas>).
  const canvas = page.locator('canvas').first()
  await expect(canvas).toBeVisible()
  const box = await canvas.boundingBox()
  expect(box, 'viewport canvas should have a layout box').not.toBeNull()
  expect(box!.width).toBeGreaterThan(0)
  expect(box!.height).toBeGreaterThan(0)

  // The crash boundary (ErrorBoundary renders "Hew hit an error") is not shown.
  await expect(page.getByText('Hew hit an error')).toHaveCount(0)

  // No unexpected console errors or uncaught exceptions during boot.
  expect(pageErrors, `uncaught page errors:\n${pageErrors.join('\n')}`).toEqual([])
  expect(
    consoleErrors,
    `unexpected console errors:\n${consoleErrors.join('\n')}`,
  ).toEqual([])
})
