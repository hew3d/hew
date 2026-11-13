// @vitest-environment jsdom
/**
 * modelClipboard — `isTauri` is flipped on by defining
 * `window.__TAURI_INTERNALS__` BEFORE the module under test first evaluates
 * (the same isolation trick `settings/server.test.ts` uses), which is why
 * the module is imported dynamically inside each test.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mockInvoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))

async function loadTauriModule() {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
  vi.resetModules()
  return import('./modelClipboard')
}

async function loadWebModule() {
  // @ts-expect-error — jsdom has no such property unless a test set it.
  delete window.__TAURI_INTERNALS__
  vi.resetModules()
  return import('./modelClipboard')
}

describe('modelClipboard — web build (no shell)', () => {
  afterEach(() => {
    // @ts-expect-error — see loadWebModule.
    delete window.__TAURI_INTERNALS__
  })

  it('starts empty', async () => {
    const m = await loadWebModule()
    expect(m.getClipboard()).toBeNull()
    expect(m.hasClipboardContent()).toBe(false)
  })

  it('setClipboardBytes stores content locally, readable via getClipboard/hasClipboardContent', async () => {
    const m = await loadWebModule()
    const bytes = new Uint8Array([1, 2, 3, 4])
    const affine = [1, 0, 0, 1, 0, 1, 0, 2, 0, 0, 1, 0]
    const content = await m.setClipboardBytes(bytes, 2, ['object'], 'doc-1', affine)
    expect(m.hasClipboardContent()).toBe(true)
    expect(m.getClipboard()).toBe(content)
    expect(content.count).toBe(2)
    expect(content.kinds).toEqual(['object'])
    expect(content.bytes).toBe(bytes)
    expect(content.placementAffine).toEqual(affine)
    // sourceId is derived from the content hash, deterministic for the same bytes.
    expect(content.sourceId).toBe(`clipboard:${content.contentHash}`)
    expect(content.contentHash).toMatch(/^[0-9a-f]{64}$/)
  })

  it('copy_records_the_link_source_with_document_id_and_nonce', async () => {
    const m = await loadWebModule()
    const source = { documentId: 'doc-uuid', nonce: 'win-1', generation: 0, nodes: [{ kind: 'object' as const, id: 7n }] }
    const content = await m.setClipboardBytes(new Uint8Array([1]), 1, ['object'], 'win-1', null, source)
    expect(content.linkSource).toEqual(source)
    expect(m.getClipboard()?.linkSource).toEqual(source)
    // Without one (a cross-window reconstruction), nothing links.
    const plain = await m.setClipboardBytes(new Uint8Array([2]), 1, ['object'], 'win-1', null)
    expect(plain.linkSource).toBeNull()
  })

  it('repointLinkSource records the placement on the clipboard the paste came from, beside the originals', async () => {
    const m = await loadWebModule()
    const source = { documentId: 'doc-uuid', nonce: 'win-1', generation: 0, nodes: [{ kind: 'object' as const, id: 7n }] }
    const content = await m.setClipboardBytes(new Uint8Array([1]), 1, ['object'], 'win-1', null, source)
    // A later copy replaces the current clipboard; the earlier one is still repointed.
    await m.setClipboardBytes(new Uint8Array([2]), 1, ['object'], 'win-1', null, null)
    m.repointLinkSource(content, [{ kind: 'instance', id: 9n }])
    expect(content.linkSource?.repointed).toEqual([{ kind: 'instance', id: 9n }])
    expect(content.linkSource?.nodes).toEqual(source.nodes)
    expect(m.getClipboard()?.linkSource).toBeNull()
  })

  it('clearClipboard empties it', async () => {
    const m = await loadWebModule()
    await m.setClipboardBytes(new Uint8Array([1]), 1, ['object'], 'doc-1', null)
    m.clearClipboard()
    expect(m.getClipboard()).toBeNull()
  })

  it('refreshClipboardFromShell is a no-op — never touches invoke', async () => {
    const m = await loadWebModule()
    await m.setClipboardBytes(new Uint8Array([9]), 1, ['object'], 'doc-1', null)
    await m.refreshClipboardFromShell()
    expect(mockInvoke).not.toHaveBeenCalled()
    expect(m.getClipboard()?.count).toBe(1)
  })

  it('setClipboardBytes never invokes clipboard_set on the web build', async () => {
    const m = await loadWebModule()
    await m.setClipboardBytes(new Uint8Array([1]), 1, ['object'], 'doc-1', null)
    expect(mockInvoke).not.toHaveBeenCalled()
  })
})

describe('modelClipboard — Tauri build (shared shell state)', () => {
  beforeEach(() => {
    mockInvoke.mockReset()
  })
  afterEach(() => {
    // @ts-expect-error — see loadWebModule.
    delete window.__TAURI_INTERNALS__
  })

  it('setClipboardBytes pushes the raw bytes to clipboard_set', async () => {
    const m = await loadTauriModule()
    mockInvoke.mockResolvedValue(undefined)
    await m.setClipboardBytes(new Uint8Array([5, 6, 7]), 1, ['object'], 'doc-1', null)
    expect(mockInvoke).toHaveBeenCalledWith('clipboard_set', { bytes: [5, 6, 7] })
  })

  it('refreshClipboardFromShell adopts the shell content when nothing is held locally', async () => {
    const m = await loadTauriModule()
    mockInvoke.mockResolvedValue([9, 9, 9])
    await m.refreshClipboardFromShell()
    expect(mockInvoke).toHaveBeenCalledWith('clipboard_get')
    const content = m.getClipboard()
    expect(content).not.toBeNull()
    expect(Array.from(content!.bytes)).toEqual([9, 9, 9])
    // No local record of the original selection — count/kinds/placementAffine are unknown.
    expect(content!.count).toBe(-1)
    expect(content!.placementAffine).toBeNull()
  })

  it('refreshClipboardFromShell clears local content when the shell reports empty', async () => {
    const m = await loadTauriModule()
    mockInvoke.mockResolvedValueOnce(undefined) // clipboard_set (from the copy below)
    await m.setClipboardBytes(new Uint8Array([1]), 1, ['object'], 'doc-1', null)
    mockInvoke.mockResolvedValueOnce(null) // clipboard_get
    await m.refreshClipboardFromShell()
    expect(m.getClipboard()).toBeNull()
  })

  it('refreshClipboardFromShell leaves the richer local copy alone when shell bytes match', async () => {
    const m = await loadTauriModule()
    mockInvoke.mockResolvedValueOnce(undefined) // clipboard_set
    const original = await m.setClipboardBytes(new Uint8Array([3, 3, 3]), 4, ['object', 'group'], 'doc-1', null)
    mockInvoke.mockResolvedValueOnce([3, 3, 3]) // clipboard_get — same bytes
    await m.refreshClipboardFromShell()
    // Still the SAME rich object (count/kinds preserved), not a -1 reconstruction.
    expect(m.getClipboard()).toBe(original)
    expect(m.getClipboard()?.count).toBe(4)
  })

  it('a shell without the command fails softly and leaves the local clipboard intact', async () => {
    const m = await loadTauriModule()
    mockInvoke.mockRejectedValue(new Error('no such command'))
    await m.setClipboardBytes(new Uint8Array([1]), 1, ['object'], 'doc-1', null)
    await expect(m.refreshClipboardFromShell()).resolves.toBeUndefined()
    expect(m.getClipboard()?.count).toBe(1)
  })
})
