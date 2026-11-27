import { expect, test } from '@playwright/test'
import { requiresPreview } from './helpers/preview'
import { bootWithPlugins, call, examplePackage, installPackage, methodOf, observe, states, waitForState } from './helpers/pluginRealm'
import { hooksOf, variantPackage } from './helpers/pluginStage'

/**
 * A plugin that asks to be started by a change to the model: while it is
 * enabled and not running, the host listens for those changes for it; the
 * first one starts it and is handed to it, once; later ones reach it on its
 * own connection, once each. A plugin whose grant cannot receive the event
 * is not listened for.
 *
 * Idle unload (a realm with nothing to do for 10 minutes is unloaded) is
 * not here: the host's idle clock cannot be shortened from a page, and a
 * spec that waited out 10 minutes would prove nothing a unit test with a
 * fake clock does not.
 */

requiresPreview('plugins')
test.setTimeout(120_000)

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

interface Status {
  activation: string
  received: number
  generations: number[]
  stored: number | null
}

test('a change to the model starts the plugin that listens for it, once, and later changes reach it once each', async ({ page }, info) => {
  await bootWithPlugins(page)
  // A model with an object in it, before the plugins exist: the edits below are single commits.
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  const edit = (tag: string): Promise<void> => page.evaluate(([id, t]) => window.__hew_test!.addNodeTag('object', id, [t]), [box, tag] as const)
  const pkg = examplePackage('event-count')
  // Its grant (model.read) cannot receive the selection, so it asks for something it cannot hear.
  const deaf = variantPackage('event-count', (m) => {
    m.id = 'com.example.event-deaf'
    m.name = 'Event Deaf'
    m.contributes.events = ['selection_changed']
  })
  await installPackage(page, pkg)
  await installPackage(page, deaf)

  // Enabled, not started: a listener is open for the one that can hear, and for no other.
  await expect.poll(() => hooksOf<string[]>(page, 'editor', 'listening'), { timeout: 15_000 }).toEqual([pkg.id])
  expect((await page.evaluate(() => document.querySelectorAll('iframe[sandbox]').length)), 'nothing has started').toBe(0)

  await edit('one')
  await waitForState(page, pkg.id, 'activated', 30_000)
  const first = (await call(page, methodOf(pkg, 'status'))).result as unknown as Status
  observe(info, 'event-start', { listeningAfterStart: await hooksOf<string[]>(page, 'editor', 'listening'), first })
  expect(first.activation).toBe('event')
  expect(first.received, 'the event that started it reached it once').toBe(1)
  expect(new Set(first.generations).size, 'and was not repeated').toBe(first.generations.length)
  expect(first.stored).toBe(1)
  // Started: the listener has handed over and is gone.
  expect(await hooksOf<string[]>(page, 'editor', 'listening')).toEqual([])

  // A second change while it runs is delivered once.
  await edit('two')
  await expect
    .poll(async () => ((await call(page, methodOf(pkg, 'status'))).result as unknown as Status).received, { timeout: 15_000 })
    .toBe(2)
  const second = (await call(page, methodOf(pkg, 'status'))).result as unknown as Status
  observe(info, 'event-second', second)
  expect(second.generations).toHaveLength(2)
  expect(new Set(second.generations).size, 'two different changes').toBe(2)
  expect(second.generations[1]).toBeGreaterThan(second.generations[0])
  expect(second.stored).toBe(2)
  // A negative: a third delivery of either would be a duplicate. Nothing else is asked
  // of the plugin, so a short, bounded wait is what an absence needs.
  await expect.poll(async () => ((await call(page, methodOf(pkg, 'status'))).result as unknown as Status).received, { timeout: 1_500 }).toBe(2)

  // The plugin that cannot hear its event never started and is not listening.
  expect((await states(page))[deaf.id]).toBe('enabled')
  expect(await hooksOf<string[]>(page, 'editor', 'listening')).not.toContain(deaf.id)
})

test('changes made while the plugin is still starting are all handed to it, in order, once each', async ({ page }, info) => {
  await bootWithPlugins(page)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  const pkg = examplePackage('event-count')
  await installPackage(page, pkg)
  await expect.poll(() => hooksOf<string[]>(page, 'editor', 'listening'), { timeout: 15_000 }).toEqual([pkg.id])
  // Three commits in one turn of the page: the first starts the plugin, the others come while it starts.
  await page.evaluate(
    (id) => {
      for (const t of ['a', 'b', 'c']) window.__hew_test!.addNodeTag('object', id, [t])
    },
    box,
  )
  await waitForState(page, pkg.id, 'activated', 30_000)
  await expect.poll(async () => ((await call(page, methodOf(pkg, 'status'))).result as unknown as Status).received, { timeout: 15_000 }).toBe(3)
  const status = (await call(page, methodOf(pkg, 'status'))).result as unknown as Status
  observe(info, 'event-handover', status)
  expect(status.activation).toBe('event')
  expect(status.generations, 'in the order they happened, none twice').toEqual([...status.generations].sort((a, b) => a - b))
  expect(new Set(status.generations).size).toBe(3)
  // Nothing more arrives afterwards (a short, bounded wait: an absence cannot be waited for).
  await expect.poll(async () => ((await call(page, methodOf(pkg, 'status'))).result as unknown as Status).received, { timeout: 1_500 }).toBe(3)
})
