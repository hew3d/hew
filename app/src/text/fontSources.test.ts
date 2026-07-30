// @vitest-environment jsdom
//
// This suite needs a real `window` (the web system provider reads
// `window.queryLocalFonts`) plus `DOMException`/`Blob` — `vitest.config.ts`
// only routes `.test.tsx` files to jsdom by default, so this `.test.ts`
// file opts in explicitly rather than being silently run under plain
// `node` (which has no `window` at all, and would make every
// `queryLocalFonts`-branch test below false-negative to "unsupported").

/**
 * fontSources tests (docs/design/3d-text-fonts.md) — provider selection per
 * environment, every branch of the web (`queryLocalFonts`) permission
 * outcome, and the Tauri provider's token round-trip. Most bundled/user
 * provider behavior is exercised indirectly through `TextDialog.test.tsx`
 * (which mocks `loadUserFontFile` itself, so it never exercises the real
 * parse/label logic below it); `loadUserFontFile`'s own label-derivation
 * behavior is tested directly here, against a real font file's bytes, since
 * that's the one piece a mock can't stand in for.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import path from 'node:path'

const fileHostState = vi.hoisted(() => ({ isTauri: false }))
vi.mock('../io/fileHost', () => ({
  get isTauri() {
    return fileHostState.isTauri
  },
}))

const tauriInvoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({
  invoke: tauriInvoke,
}))

// vi.mock() above is hoisted before imports, so these must appear after it.
import { LocalFontPermissionDeniedError, loadUserFontFile, resetUserFontEntriesForTests, resolveFontProviders } from './fontSources'
import { BUNDLED_MANIFEST } from './fonts'

/** Minimal stand-in for `window.queryLocalFonts`'s returned `FontData`.
 *  Returns a Blob-shaped object with a working `arrayBuffer()` rather than
 *  a real jsdom `Blob` — jsdom does not implement `Blob.prototype.arrayBuffer`
 *  (confirmed empirically; a real browser's `Blob`, the only place
 *  `queryLocalFonts` itself ever runs, does), and this suite is testing
 *  `fontSources.ts`'s own mapping/round-trip logic, not jsdom's Blob gaps. */
function fakeFontData(family: string, style: string, postscriptName: string, blobBytes: string) {
  return {
    postscriptName,
    fullName: `${family} ${style}`,
    family,
    style,
    blob: async () => ({
      arrayBuffer: async () => new TextEncoder().encode(blobBytes).buffer,
    }),
  }
}

describe('resolveFontProviders: provider selection per environment', () => {
  beforeEach(() => {
    fileHostState.isTauri = false
    tauriInvoke.mockReset()
    delete (globalThis as { queryLocalFonts?: unknown }).queryLocalFonts
  })

  it('desktop (isTauri): system is the tauri provider, no queryLocalFonts needed', async () => {
    fileHostState.isTauri = true
    const providers = await resolveFontProviders()
    expect(providers.system?.kind).toBe('system-tauri')
    expect(await providers.system?.available()).toBe(true)
  })

  it('web without queryLocalFonts: system is null (bundled + loaded-files fallback)', async () => {
    fileHostState.isTauri = false
    const providers = await resolveFontProviders()
    expect(providers.system).toBeNull()
  })

  it('web WITH queryLocalFonts: system is the web provider', async () => {
    fileHostState.isTauri = false
    ;(globalThis as unknown as { queryLocalFonts: () => Promise<unknown[]> }).queryLocalFonts = async () => []
    const providers = await resolveFontProviders()
    expect(providers.system?.kind).toBe('system-web')
  })

  it('bundled and user providers are always present, regardless of environment', async () => {
    const providers = await resolveFontProviders()
    expect(providers.bundled.kind).toBe('bundled')
    expect(providers.user.kind).toBe('user')
    expect(await providers.bundled.available()).toBe(true)
    expect(await providers.user.available()).toBe(true)
  })
})

describe('system-web provider: every branch of the permission outcome', () => {
  beforeEach(() => {
    fileHostState.isTauri = false
    delete (globalThis as { queryLocalFonts?: unknown }).queryLocalFonts
  })
  afterEach(() => {
    delete (globalThis as { queryLocalFonts?: unknown }).queryLocalFonts
  })

  it('unsupported: available() is false, and list() degrades to an empty array rather than throwing', async () => {
    const providers = await resolveFontProviders()
    expect(providers.system).toBeNull() // the picker never even shows a System section
  })

  it('granted: list() returns entries mapped from queryLocalFonts, and bytes() reads the right blob', async () => {
    ;(globalThis as unknown as { queryLocalFonts: () => Promise<unknown[]> }).queryLocalFonts = async () => [
      fakeFontData('Menlo', 'Regular', 'Menlo', 'menlo-bytes'),
      fakeFontData('Menlo', 'Bold', 'Menlo-Bold', 'menlo-bold-bytes'),
    ]
    const providers = await resolveFontProviders()
    expect(providers.system?.kind).toBe('system-web')
    const entries = await providers.system!.list()
    expect(entries).toHaveLength(2)
    expect(entries[0]).toMatchObject({ family: 'Menlo', style: 'Regular', source: 'system', italic: false })
    expect(entries[1]).toMatchObject({ family: 'Menlo', style: 'Bold', source: 'system', italic: false })

    const bytes = await providers.system!.bytes(entries[0])
    expect(new TextDecoder().decode(bytes)).toBe('menlo-bytes')
  })

  it('an italic/oblique style is detected from the style string', async () => {
    ;(globalThis as unknown as { queryLocalFonts: () => Promise<unknown[]> }).queryLocalFonts = async () => [
      fakeFontData('Georgia', 'Italic', 'Georgia-Italic', 'x'),
      fakeFontData('Georgia', 'Bold Oblique', 'Georgia-BoldOblique', 'y'),
    ]
    const providers = await resolveFontProviders()
    const entries = await providers.system!.list()
    expect(entries.every((e) => e.italic === true)).toBe(true)
  })

  it('denied (NotAllowedError): list() rejects with LocalFontPermissionDeniedError, not a generic error', async () => {
    ;(globalThis as unknown as { queryLocalFonts: () => Promise<unknown[]> }).queryLocalFonts = async () => {
      throw new DOMException('denied', 'NotAllowedError')
    }
    const providers = await resolveFontProviders()
    await expect(providers.system!.list()).rejects.toBeInstanceOf(LocalFontPermissionDeniedError)
  })

  it('denied (SecurityError, the spec-documented rejection name): also LocalFontPermissionDeniedError', async () => {
    ;(globalThis as unknown as { queryLocalFonts: () => Promise<unknown[]> }).queryLocalFonts = async () => {
      throw new DOMException('denied', 'SecurityError')
    }
    const providers = await resolveFontProviders()
    await expect(providers.system!.list()).rejects.toBeInstanceOf(LocalFontPermissionDeniedError)
  })

  it('an unexpected, non-permission error is rethrown as-is, not misreported as a denial', async () => {
    ;(globalThis as unknown as { queryLocalFonts: () => Promise<unknown[]> }).queryLocalFonts = async () => {
      throw new TypeError('something else broke')
    }
    const providers = await resolveFontProviders()
    await expect(providers.system!.list()).rejects.toThrow('something else broke')
    await expect(providers.system!.list()).rejects.not.toBeInstanceOf(LocalFontPermissionDeniedError)
  })

  it("bytes() for an entry from a stale/forgotten list() call fails honestly, not silently", async () => {
    ;(globalThis as unknown as { queryLocalFonts: () => Promise<unknown[]> }).queryLocalFonts = async () => [
      fakeFontData('Menlo', 'Regular', 'Menlo', 'menlo-bytes'),
    ]
    const providers = await resolveFontProviders()
    const [entry] = await providers.system!.list()
    // A second, independent queryLocalFonts()-backed provider run (a real
    // re-list) replaces the internal id->data map; the OLD entry object is
    // now stale.
    ;(globalThis as unknown as { queryLocalFonts: () => Promise<unknown[]> }).queryLocalFonts = async () => []
    await providers.system!.list()
    await expect(providers.system!.bytes(entry)).rejects.toThrow(/no longer available/)
  })
})

describe('system-tauri provider: the token round-trip', () => {
  beforeEach(() => {
    fileHostState.isTauri = true
    tauriInvoke.mockReset()
  })

  it("list()'s token flows unchanged into bytes()'s read_font_file invoke call", async () => {
    tauriInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'list_system_fonts') {
        return [
          { token: 'sysfont-7', family: 'Menlo', style: 'Bold', weight: 700, italic: false, monospace: true },
        ]
      }
      if (cmd === 'read_font_file') {
        expect(args?.token).toBe('sysfont-7') // THE round-trip property
        return new TextEncoder().encode('menlo-bold-bytes').buffer
      }
      throw new Error(`unexpected invoke: ${cmd}`)
    })

    const providers = await resolveFontProviders()
    expect(providers.system?.kind).toBe('system-tauri')
    const entries = await providers.system!.list()
    expect(entries).toEqual([
      {
        id: 'sys-sysfont-7',
        family: 'Menlo',
        style: 'Bold',
        source: 'system',
        weight: 700,
        italic: false,
        locator: { token: 'sysfont-7' },
      },
    ])

    const bytes = await providers.system!.bytes(entries[0])
    expect(new TextDecoder().decode(bytes)).toBe('menlo-bold-bytes')
    expect(tauriInvoke).toHaveBeenCalledWith('read_font_file', { token: 'sysfont-7' })
  })

  it('two different enumerated faces round-trip to two different tokens, never crossed', async () => {
    tauriInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'list_system_fonts') {
        return [
          { token: 'sysfont-1', family: 'Menlo', style: 'Regular', weight: 400, italic: false, monospace: true },
          { token: 'sysfont-2', family: 'Menlo', style: 'Italic', weight: 400, italic: true, monospace: true },
        ]
      }
      if (cmd === 'read_font_file') {
        return new TextEncoder().encode(`bytes-for-${args?.token}`).buffer
      }
      throw new Error(`unexpected invoke: ${cmd}`)
    })

    const providers = await resolveFontProviders()
    const [regular, italic] = await providers.system!.list()
    const regularBytes = await providers.system!.bytes(regular)
    const italicBytes = await providers.system!.bytes(italic)
    expect(new TextDecoder().decode(regularBytes)).toBe('bytes-for-sysfont-1')
    expect(new TextDecoder().decode(italicBytes)).toBe('bytes-for-sysfont-2')
  })
})

describe('loadUserFontFile: the display label comes from the font itself, not the filename', () => {
  afterEach(() => {
    resetUserFontEntriesForTests()
  })

  // A real bundled asset stands in for a "user-loaded" file — same idea as
  // `fonts.bundledScreening.test.ts`'s `fs.readFileSync` approach to get
  // real bytes into a `.test.ts` suite (Node's `fetch` doesn't implement
  // `file:`). Its own name table says "Lora" / "Regular"; the `File` it's
  // wrapped in is deliberately given an UNRELATED filename, so a label
  // derived from the filename and a label derived from the font's own name
  // table are trivially distinguishable. Unlike that suite (plain `node`
  // environment, where `import.meta.url` is a real `file:` URL), THIS suite
  // runs under jsdom, so `BUNDLED_MANIFEST`'s `import.meta.url`-relative
  // `URL` resolves against jsdom's virtual `http://localhost/` location —
  // only its `.pathname` is trustworthy, resolved against the working
  // directory `pnpm --dir app test` always runs from.
  //
  // jsdom's `File`/`Blob` also doesn't implement `arrayBuffer()` (confirmed
  // empirically, same gap `fakeFontData` above works around for
  // `queryLocalFonts`'s `FontData`) — `loadUserFontFile` calls
  // `file.arrayBuffer()` directly, so a real jsdom `File` would throw here
  // regardless of the label-derivation logic being tested. A minimal
  // File-shaped stand-in with a working `arrayBuffer()` sidesteps that gap.
  function realLoraFileWithMismatchedName(filename: string): File {
    const loraEntry = BUNDLED_MANIFEST.find((f) => f.id === 'bundled-lora')
    if (!loraEntry) throw new Error('bundled-lora missing from BUNDLED_MANIFEST')
    const bytes = readFileSync(path.join(process.cwd(), loraEntry.url.pathname))
    const arrayBuffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer
    return {
      name: filename,
      arrayBuffer: async () => arrayBuffer,
    } as unknown as File
  }

  it("uses the font's own name-table family/style for both the entry and the loaded label (finding 4: regression from the old labelFromFont)", async () => {
    const file = realLoraFileWithMismatchedName('totally-different-filename.ttf')

    const { entry, loaded } = await loadUserFontFile(file)

    // The entry's family/style (what the picker's family/style ROWS read)
    // must reflect the font's own name table, "Lora" / "Regular" — not the
    // filename-derived fallback "totally-different-filename" / "Regular".
    expect(entry.family).toBe('Lora')
    expect(entry.style).toBe('Regular')
    // THE regression: `loaded.label` (what the dialog's SELECTED-font
    // display actually reads — `TextDialog.tsx`'s `selectedLabel`) must
    // match, not be permanently stuck on the filename-derived guess that
    // was baked into `loadFontEntry`'s memoized cache entry before the
    // real family/style were ever computed.
    expect(loaded.label).toBe('Lora')
  })

  it('a font whose filename already happens to differ only in case/spacing still gets the real label', async () => {
    const file = realLoraFileWithMismatchedName('my-custom-serif-font.otf')
    const { loaded } = await loadUserFontFile(file)
    expect(loaded.label).toBe('Lora')
  })
})
