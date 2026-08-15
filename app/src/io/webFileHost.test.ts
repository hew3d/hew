// @vitest-environment jsdom
//
// This suite needs a real `window`/`document` (hidden <input> creation,
// `window.matchMedia`) — `vitest.config.ts` only routes `.test.tsx` files to
// jsdom by default, so this `.test.ts` file opts in explicitly (the existing
// precedent is api/liveBridge.test.ts and text/fontSources.test.ts).

/**
 * webFileHost.test.ts — covers the fallback `<input type=file>`'s `accept`
 * attribute (docs/design/shop-mode.md §1: iOS Files greys out any file whose
 * extension has no registered UTI, `.hew` among them, so the fallback must
 * drop `accept` entirely on coarse-pointer devices or users can see but
 * never select their own files).
 *
 * jsdom has no File System Access API (no `window.showSaveFilePicker`), so
 * `hasFSAA()` is false here and every open path below exercises the hidden
 * `<input>` fallback without further mocking. jsdom also has no
 * `window.matchMedia` at all by default (verified: calling it throws
 * "not a function"), which is exactly the "matchMedia absent" environment
 * `isCoarsePointer()` treats as fine-pointer — so the "no mock" tests below
 * double as that guard's regression test.
 *
 * These tests never resolve `open()`/`openForImport()`/`openAny()` — no
 * file is ever picked — they only inspect the `<input>` synchronously
 * appended to `document.body` by the Promise executor before it awaits the
 * 'change' event.
 */

import { afterEach, describe, expect, it, vi } from 'vitest'
import { WebFileHost, anchorDownload } from './webFileHost'

// jsdom doesn't implement the Blob URL APIs — stub them so anchorDownload's
// tests below can run without a real browser, mirroring LibraryDialog.test.tsx's
// identical guard.
if (typeof URL.createObjectURL !== 'function') {
  URL.createObjectURL = () => 'blob:mock'
}
if (typeof URL.revokeObjectURL !== 'function') {
  URL.revokeObjectURL = () => { /* no-op */ }
}

/** Mock `window.matchMedia` to report `(pointer: coarse)` as matching (or not). */
function mockPointer(coarse: boolean): void {
  window.matchMedia = ((query: string) => ({
    matches: coarse && query === '(pointer: coarse)',
  })) as typeof window.matchMedia
}

afterEach(() => {
  // Each test appends a hidden <input> to document.body and never resolves
  // its promise (no file is picked) — clear it so inputs don't accumulate
  // across tests in this file.
  document.body.innerHTML = ''
  // @ts-expect-error — restore jsdom's default (unimplemented) matchMedia.
  delete window.matchMedia
})

describe('WebFileHost fallback <input> accept attribute', () => {
  it('open(): omits accept on a coarse pointer', () => {
    mockPointer(true)
    void new WebFileHost().open()
    const input = document.querySelector('input[type=file]') as HTMLInputElement
    expect(input.accept).toBe('')
  })

  it('open(): keeps the .hew filter on a fine pointer', () => {
    mockPointer(false)
    void new WebFileHost().open()
    const input = document.querySelector('input[type=file]') as HTMLInputElement
    expect(input.accept).toBe('.hew')
  })

  it('open(): keeps the .hew filter when matchMedia is absent (jsdom default)', () => {
    void new WebFileHost().open()
    const input = document.querySelector('input[type=file]') as HTMLInputElement
    expect(input.accept).toBe('.hew')
  })

  it('openForImport(): omits accept on a coarse pointer', () => {
    mockPointer(true)
    void new WebFileHost().openForImport()
    const input = document.querySelector('input[type=file]') as HTMLInputElement
    expect(input.accept).toBe('')
  })

  it('openForImport(): keeps the import-format filter on a fine pointer', () => {
    mockPointer(false)
    void new WebFileHost().openForImport()
    const input = document.querySelector('input[type=file]') as HTMLInputElement
    expect(input.accept).toBe('.dae,.skp,.glb,.gltf,.stl')
  })

  it('openAny(): omits accept on a coarse pointer', () => {
    mockPointer(true)
    void new WebFileHost().openAny()
    const input = document.querySelector('input[type=file]') as HTMLInputElement
    expect(input.accept).toBe('')
  })

  it('openAny(): keeps the combined .hew + import filter on a fine pointer', () => {
    mockPointer(false)
    void new WebFileHost().openAny()
    const input = document.querySelector('input[type=file]') as HTMLInputElement
    expect(input.accept).toBe('.hew,.dae,.skp,.glb,.gltf,.stl')
  })
})

// anchorDownload — exported for Shop Mode's "Save a copy (.hew)" overflow-
// menu entry (docs/design/shop-mode.md §4), which wants a plain download
// unconditionally, never the FSAA save-picker `saveAs` would reach for.
describe('anchorDownload', () => {
  /** Captures the <a> anchorDownload creates by spying on its `.click()` —
   *  the anchor is never actually attached to the DOM (module code builds
   *  it, clicks it, and lets it be garbage-collected), so this patches
   *  `HTMLAnchorElement.prototype.click` rather than querying the document. */
  function captureAnchorClick(): { href: string; download: string }[] {
    const captured: { href: string; download: string }[] = []
    const original = HTMLAnchorElement.prototype.click
    HTMLAnchorElement.prototype.click = function (this: HTMLAnchorElement) {
      captured.push({ href: this.href, download: this.download })
    }
    return captured
  }

  afterEach(() => {
    // Restore the prototype patch between tests — captureAnchorClick reruns
    // it fresh each time it's called.
    delete (HTMLAnchorElement.prototype as { click?: unknown }).click
  })

  it('appends .hew when the name lacks it', () => {
    const captured = captureAnchorClick()
    anchorDownload(new Uint8Array([1, 2, 3]), 'Bench')
    expect(captured).toHaveLength(1)
    expect(captured[0].download).toBe('Bench.hew')
  })

  it('does not double the suffix when the name already ends in .hew', () => {
    const captured = captureAnchorClick()
    anchorDownload(new Uint8Array([1, 2, 3]), 'Bench.hew')
    expect(captured[0].download).toBe('Bench.hew')
  })

  it('builds the anchor from a blob: URL', () => {
    const captured = captureAnchorClick()
    anchorDownload(new Uint8Array([1, 2, 3]), 'Bench')
    expect(captured[0].href).toMatch(/^blob:/)
  })

  it('revokes the object URL after clicking (no leaked blob: URLs)', () => {
    captureAnchorClick()
    const revoke = vi.spyOn(URL, 'revokeObjectURL')
    anchorDownload(new Uint8Array([1, 2, 3]), 'Bench')
    expect(revoke).toHaveBeenCalledOnce()
    revoke.mockRestore()
  })
})
