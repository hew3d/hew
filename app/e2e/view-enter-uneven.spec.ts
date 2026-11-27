import { test, expect, type Page } from '@playwright/test'

/**
 * `hew.view.enter` on a top-level component instance scaled unevenly whose
 * definition holds one object: no session can take it, so the window enters
 * it in context. The call must leave the window inside it and select the
 * target, whether it is named by `id` or by a path ending in the member.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type Reply = { result?: unknown; error?: { code: number; message: string; data?: { refusal?: string } } }

async function ready(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

const dispatch = (page: Page, method: string, params: unknown = {}): Promise<Reply> =>
  page.evaluate(([m, p]) => window.__hew_test!.apiDispatch({ method: m as string, params: p }), [method, params]) as Promise<Reply>

type Occ = { id: string; depth: number; members: { objects: string[]; groups: string[]; instances: string[] } }

async function setup(
  page: Page,
  factors: [number, number, number] = [1, 1, 1.5],
): Promise<{ ins: string; obj: string }> {
  await ready(page)
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.makeComponent([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
  })
  const occ = ((await dispatch(page, 'hew.query.occurrences', { kinds: ['instance'] })).result as { occurrences: Occ[] })
    .occurrences.find((r) => r.depth === 0)!
  const scaled = await dispatch(page, 'hew.entity.scale', { ids: [occ.id], anchor: [0, 0, 0], factors })
  expect(scaled.error).toBeUndefined()
  return { ins: occ.id, obj: occ.members.objects[0] }
}

async function state(page: Page) {
  // Let the window render and run its effects, as a socket client's next
  // call always arrives after they have.
  await page.waitForTimeout(400)
  const ctx = (await dispatch(page, 'hew.query.context')).result
  const sel = (await dispatch(page, 'hew.selection.get')).result
  return { ctx, sel }
}

test('hew.view.enter inside a one-command transaction enters and selects', async ({ page }) => {
  const { ins, obj } = await setup(page)
  const r = await dispatch(page, 'hew.doc.transact', { commands: [{ method: 'hew.view.enter', params: { path: [ins, obj] } }] })
  const s = await state(page)
  expect(r.error).toBeUndefined()
  expect(JSON.stringify((s as { sel: unknown }).sel)).toContain(obj)
})

for (const form of ['id', 'path'] as const) {
  test(`hew.view.enter on an unevenly scaled instance by ${form} enters in context and selects`, async ({ page }) => {
    const { ins, obj } = await setup(page)
    const params = form === 'id' ? { id: ins } : { path: [ins, obj] }
    const r = await dispatch(page, 'hew.view.enter', params)
    const s = await state(page)
    expect(r.error).toBeUndefined()
    const { ctx, sel } = s as { ctx: { stack: unknown[]; fallback: unknown[] }; sel: { entries?: unknown[]; ids?: string[] } }
    expect(ctx.fallback.length).toBeGreaterThan(0)
    expect(JSON.stringify(sel)).toContain(form === 'id' ? ins : obj)
  })
}

/** The top-level instance of the definition named `name`. */
async function instanceOf(page: Page, name: string): Promise<string> {
  const rows = (
    (await dispatch(page, 'hew.query.occurrences', { kinds: ['instance'] })).result as {
      occurrences: (Occ & { definition_name: string })[]
    }
  ).occurrences
  return rows.find((r) => r.depth === 0 && r.definition_name === name)!.id
}

test('a one-command transaction around hew.view.enter answers the refusal the window gives', async ({ page }) => {
  await ready(page)
  // A part, and an assembly nesting a placement of it: open the assembly,
  // place the part, close it again.
  const part = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.makeComponent([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
  })
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.makeComponent([h.drawBox([3, 0, 0], [4, 1, 0], 1)])
  })
  const assemblyId = await instanceOf(page, 'Component 2')
  expect((await dispatch(page, 'hew.view.enter', { id: assemblyId })).error).toBeUndefined()
  await page.evaluate((c) => window.__hew_test!.placeInstance(c, 3, 3, 0), part.component)
  expect((await dispatch(page, 'hew.view.exit_all')).error).toBeUndefined()
  const scaled = await dispatch(page, 'hew.entity.scale', { ids: [assemblyId], anchor: [3.5, 0.5, 0], factors: [1, 2, 1] })
  expect(scaled.error).toBeUndefined()

  const r = await dispatch(page, 'hew.doc.transact', {
    commands: [{ method: 'hew.view.enter', params: { id: assemblyId } }],
  })
  expect(r.result).toBeUndefined()
  expect(r.error).toMatchObject({ code: -32000, data: { refusal: 'nested_component_in_context', failed_method: 'hew.view.enter' } })
  const ctx = (await dispatch(page, 'hew.query.context')).result
  expect(ctx).toMatchObject({ stack: [], fallback: [] })
})

// The reporter's imported component: a very small, unevenly scaled instance.
test('a very small uneven scale enters in context like any other', async ({ page }) => {
  const { ins, obj } = await setup(page, [0.0053, 0.0053, 0.0058])
  const r = await dispatch(page, 'hew.doc.transact', { commands: [{ method: 'hew.view.enter', params: { path: [ins, obj] } }] })
  const s = (await state(page)) as { ctx: { stack: unknown[]; fallback: unknown[] }; sel: unknown }
  expect(r.error).toBeUndefined()
  expect(s.ctx.fallback).toHaveLength(2)
  expect(JSON.stringify(s.sel)).toContain(obj)
})

// What a caller hears when the window declines an entry because the thing is
// locked, bare and as the one command of a transaction.
for (const target of ['instance', 'member'] as const) {
  test(`a locked ${target} on an uneven instance declines the entry, bare and in a transaction`, async ({ page }) => {
    const { ins, obj } = await setup(page)
    const lock = await dispatch(page, 'hew.entity.lock', { targets: [target === 'instance' ? ins : obj] })
    expect(lock.error).toBeUndefined()
    const path = { path: [ins, obj] }
    const bare = await dispatch(page, 'hew.view.enter', path)
    const wrapped = await dispatch(page, 'hew.doc.transact', { commands: [{ method: 'hew.view.enter', params: path }] })
    const s = (await state(page)) as { ctx: { fallback: unknown[] }; sel: { ids: string[] } }
    // The window's own refusal for a locked thing is named `locked_node`
    // and says what the kernel's own locked refusal says; nothing is
    // entered or selected.
    for (const reply of [bare, wrapped]) {
      expect(reply.result).toBeUndefined()
      expect(reply.error).toMatchObject({ code: -32000, data: { refusal: 'locked_node', failed_method: 'hew.view.enter', explanation: 'Locked. Unlock it to change it.' } })
    }
    expect(s.ctx.fallback).toEqual([])
    expect(s.sel.ids).toEqual([])
  })
}

// A shape also used inside a locked group: the lock is not on the instance
// itself, and the explanation says to make it unique.
test('a shape shared with a locked group declines the entry with the shared-shape wording', async ({ page }) => {
  await ready(page)
  const ids = await page.evaluate(() => {
    const h = window.__hew_test!
    const part = h.makeComponent([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
    const copy = h.placeInstance(part.component, 3, 3, 0)
    h.groupNodes([{ kind: 'instance', id: copy }])
    return { free: part.instance }
  })
  void ids
  const groups = (
    (await dispatch(page, 'hew.query.occurrences', { kinds: ['group'] })).result as {
      occurrences: { id: string; members: { instances: string[] } }[]
    }
  ).occurrences
  expect((await dispatch(page, 'hew.entity.lock', { targets: [groups[0].id] })).error).toBeUndefined()
  const groupedId = groups[0].members.instances[0]
  const rows = (
    (await dispatch(page, 'hew.query.occurrences', { kinds: ['instance'] })).result as {
      occurrences: { id: string; depth: number; path: string[] }[]
    }
  ).occurrences
  const inGroup = rows.find((r) => r.id === groupedId)!
  const free = rows.find((r) => r.id !== groupedId)!
  const shared = await dispatch(page, 'hew.view.enter', { id: free.id })
  expect(shared.error).toMatchObject({
    data: {
      refusal: 'locked_node',
      detail: { indirect: true },
      explanation: 'This shape is also used inside a locked group. Make this copy unique to change it.',
    },
  })
  // A lock on a group above is the plain wording, not the shared-shape one.
  const above = await dispatch(page, 'hew.view.enter', { id: inGroup.id })
  expect(above.error).toMatchObject({ data: { refusal: 'locked_node', explanation: 'Locked. Unlock it to change it.' } })
  expect((above.error as { data: { detail: unknown } }).data.detail).toEqual({})
})
