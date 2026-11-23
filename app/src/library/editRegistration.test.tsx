import { renderHook, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { libraryStore } from '../io/libraryStore'
import { syncEditRegistration, useLibraryEditRegistration } from './editRegistration'
import { itemFileName } from './fileNaming'
import { SRC, makeEnv, uuid } from './sourcesTestKit'

vi.mock('../io/libraryStore', () => ({ libraryStore: vi.fn() }))

const A = 'Components/a-000001.hew'
const B = 'Components/b-000002.hew'

function registry() {
  const calls: string[] = []
  return {
    calls,
    registerEdit: vi.fn(async (n: string) => void calls.push(`register ${n}`)),
    unregisterEdit: vi.fn(async (n: string) => void calls.push(`unregister ${n}`)),
  }
}

describe('syncEditRegistration', () => {
  it('registers the item a window starts editing', async () => {
    const r = registry()
    await syncEditRegistration(r, null, A)
    expect(r.calls).toEqual([`register ${A}`])
  })

  it('unregisters when the binding is cleared', async () => {
    const r = registry()
    await syncEditRegistration(r, A, null)
    expect(r.calls).toEqual([`unregister ${A}`])
  })

  it('swaps one item for another: drop, then register', async () => {
    const r = registry()
    await syncEditRegistration(r, A, B)
    expect(r.calls).toEqual([`unregister ${A}`, `register ${B}`])
  })

  it('does nothing when the item did not change (a rename keeps the file)', async () => {
    const r = registry()
    await syncEditRegistration(r, A, A)
    await syncEditRegistration(r, null, null)
    expect(r.calls).toEqual([])
  })

  it('is a no-op on a store without the commands (the web) and swallows a failing shell', async () => {
    await expect(syncEditRegistration({}, null, A)).resolves.toBeUndefined()
    const bad = { registerEdit: vi.fn().mockRejectedValue(new Error('no such command')) }
    await expect(syncEditRegistration(bad, null, A)).resolves.toBeUndefined()
  })

  it('the registry feeds what the sources module reads: an update of an open item reports open_for_editing', async () => {
    const env = makeEnv()
    const id = uuid(1)
    const file = itemFileName('Part 1', id, 'component')
    const pub = await env.addSource(SRC, { name: 'Shop Hardware', items: [{ id, name: 'Part 1' }] })
    env.store.seed(file, { id, name: 'Part 1', remote: { source: SRC, name: 'Part 1', path: 'Components/part-1.hew', sha256: pub.sha[id], commit: 'c1', license: null, fetchedAt: env.clock.iso(), edited: false } as never })
    env.server.publish(SRC, { name: 'Shop Hardware', commit: 'c2', items: [{ id, name: 'Part 1', body: 'v2' }] })
    await env.sources.check(SRC)
    const open = new Set<string>()
    const reg = { registerEdit: async (n: string) => void open.add(n), unregisterEdit: async (n: string) => void open.delete(n) }
    await syncEditRegistration(reg, null, file)
    env.store.editing = [...open] // the shell's `library_edit_list`
    expect(await env.sources.updateItem(SRC, id)).toMatchObject({ ok: false, code: 'open_for_editing' })
    await syncEditRegistration(reg, file, null)
    env.store.editing = [...open]
    expect(await env.sources.updateItem(SRC, id)).toMatchObject({ ok: true })
  })
})

describe('useLibraryEditRegistration', () => {
  it('registers when an item opens for editing and unregisters when it closes or the window unmounts', async () => {
    const r = registry()
    vi.mocked(libraryStore).mockReturnValue(r as never)
    const { rerender, unmount } = renderHook(({ f }: { f: string | null }) => useLibraryEditRegistration(f), { initialProps: { f: null as string | null } })
    await Promise.resolve()
    expect(r.calls).toEqual([])
    rerender({ f: A })
    await waitFor(() => expect(r.calls).toEqual([`register ${A}`]))
    rerender({ f: B })
    await waitFor(() => expect(r.calls).toEqual([`register ${A}`, `unregister ${A}`, `register ${B}`]))
    rerender({ f: null })
    await waitFor(() => expect(r.calls.at(-1)).toBe(`unregister ${B}`))
    rerender({ f: A })
    await waitFor(() => expect(r.calls.at(-1)).toBe(`register ${A}`))
    unmount()
    await waitFor(() => expect(r.calls.at(-1)).toBe(`unregister ${A}`))
  })
})
