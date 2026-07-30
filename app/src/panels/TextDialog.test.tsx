/**
 * TextDialog — font-picker component tests (playtest finding 3, and the
 * system-font-picker follow-up in docs/design/3d-text-fonts.md).
 *
 * `../text/fontSources`, `../text/fonts`, and `../text/outlineValidation`
 * are all mocked so these tests exercise the DIALOG's own wiring (section
 * composition, filtering, style selection, session memory, the OK handoff)
 * without touching real fetch/opentype.js parsing or Tauri/`queryLocalFonts`
 * — those are covered by `fontSources.test.ts` and `outlineValidation.test.ts`
 * respectively.
 */

import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import type { FontFaceEntry, FontProvider, FontProviderSet } from '../text/fontSources'
import type { LoadedFont } from '../text/fonts'

function entry(
  id: string,
  family: string,
  style: string,
  source: FontFaceEntry['source'],
  extra: Partial<FontFaceEntry> = {},
): FontFaceEntry {
  return { id, family, style, source, locator: {}, ...extra }
}

/** A stand-in `LoadedFont` whose `font` is just a marker object, not a real
 *  opentype.js `Font` — these tests exercise the DIALOG's wiring, never the
 *  actual font's contents, so satisfying the full `Font` interface would
 *  only add noise. Cast at the boundary rather than trying to structurally
 *  extend `LoadedFont` (its `font: Font` is a large real interface). */
function loadedFrom(e: FontFaceEntry): LoadedFont {
  return {
    id: e.id,
    label: e.style === 'Regular' ? e.family : `${e.family} ${e.style}`,
    source: e.source,
    font: { marker: e.id } as unknown as LoadedFont['font'],
  }
}

const state = vi.hoisted(() => ({
  bundled: [] as FontFaceEntry[],
  systemKind: null as null | 'system-tauri' | 'system-web',
  systemEntries: [] as FontFaceEntry[],
  systemListImpl: null as null | (() => Promise<FontFaceEntry[]>),
  userEntries: [] as FontFaceEntry[],
  lastSelected: null as string | null,
  failNextUserLoad: false,
}))

vi.mock('../text/fontSources', async () => {
  const actual = await vi.importActual<typeof import('../text/fontSources')>('../text/fontSources')
  return {
    LocalFontPermissionDeniedError: actual.LocalFontPermissionDeniedError,
    resolveFontProviders: async (): Promise<FontProviderSet> => ({
      bundled: {
        kind: 'bundled',
        available: async () => true,
        list: async () => state.bundled,
        bytes: async () => new ArrayBuffer(0),
      },
      system:
        state.systemKind !== null
          ? ({
              kind: state.systemKind,
              available: async () => true,
              list: async () => (state.systemListImpl ? state.systemListImpl() : state.systemEntries),
              bytes: async () => new ArrayBuffer(0),
            } satisfies FontProvider)
          : null,
      user: {
        kind: 'user',
        available: async () => true,
        list: async () => state.userEntries,
        bytes: async () => new ArrayBuffer(0),
      },
    }),
    loadUserFontFile: async (file: File) => {
      if (state.failNextUserLoad) {
        state.failNextUserLoad = false
        throw new Error('not a font')
      }
      const fallback = file.name.replace(/\.(ttf|otf)$/i, '')
      const e = entry(`user-${state.userEntries.length}`, fallback, 'Regular', 'user')
      state.userEntries.push(e)
      return { entry: e, loaded: loadedFrom(e) }
    },
  }
})

vi.mock('../text/fonts', async () => {
  const actual = await vi.importActual<typeof import('../text/fonts')>('../text/fonts')
  return {
    ...actual,
    loadFontEntry: async (e: FontFaceEntry) => loadedFrom(e),
    lastSelectedFont: () => state.lastSelected,
    rememberSelectedFont: (id: string) => {
      state.lastSelected = id
    },
  }
})

vi.mock('../text/outlineValidation', () => ({
  validateGlyphOutlines: vi.fn(() => []),
}))

// FontFace isn't implemented in jsdom; previews are best-effort and
// swallow failures (see `registerPreviewFontFace`'s doc comment), so a
// minimal stub is enough for these tests to run without it throwing.
class StubFontFace {
  constructor(
    public family: string,
    public source: unknown,
  ) {}
  async load() {
    return this
  }
}
vi.stubGlobal('FontFace', StubFontFace)
if (!('fonts' in document)) {
  Object.defineProperty(document, 'fonts', { value: { add: () => {} }, configurable: true })
}

// vi.mock() above is hoisted before imports, so this must appear after it.
import { TextDialog } from './TextDialog'
import { validateGlyphOutlines } from '../text/outlineValidation'

function makeFontFile(name: string): File {
  return new File(['fake-font-bytes'], name, { type: 'font/ttf' })
}

async function waitForPickerReady() {
  await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument())
}

describe('TextDialog font picker', () => {
  beforeEach(() => {
    state.bundled = [entry('bundled-a', 'Alpha', 'Regular', 'bundled'), entry('bundled-b', 'Beta', 'Regular', 'bundled')]
    state.systemKind = null
    state.systemEntries = []
    state.systemListImpl = null
    state.userEntries = []
    state.lastSelected = null
    state.failNextUserLoad = false
    vi.mocked(validateGlyphOutlines).mockReturnValue([])
  })

  it('lists the bundled fonts as family rows under a Bundled heading, selecting the first by default', async () => {
    render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitForPickerReady()
    expect(screen.getByText('Bundled')).toBeInTheDocument()
    expect(screen.getByText('Alpha')).toBeInTheDocument()
    expect(screen.getByText('Beta')).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('option', { name: 'Alpha' })).toHaveAttribute('aria-selected', 'true'))
  })

  it('offers "Load font file…" as a secondary path', async () => {
    render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitForPickerReady()
    expect(screen.getByRole('button', { name: 'Load font file…' })).toBeInTheDocument()
  })

  it('the filter box narrows the visible family rows, case- and diacritic-insensitively', async () => {
    render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitForPickerReady()
    fireEvent.change(screen.getByPlaceholderText('Filter fonts…'), { target: { value: 'bEt' } })
    expect(screen.queryByText('Alpha')).not.toBeInTheDocument()
    expect(screen.getByText('Beta')).toBeInTheDocument()
  })

  it('clicking a different bundled family selects it', async () => {
    render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitForPickerReady()
    fireEvent.click(screen.getByRole('option', { name: 'Beta' }))
    await waitFor(() => expect(screen.getByRole('option', { name: 'Beta' })).toHaveAttribute('aria-selected', 'true'))
    expect(screen.getByRole('option', { name: 'Alpha' })).toHaveAttribute('aria-selected', 'false')
  })

  it('a family with more than one face offers a style selector, and picking a style updates the selection', async () => {
    state.bundled = [
      entry('fam-reg', 'Gamma', 'Regular', 'bundled'),
      entry('fam-bold', 'Gamma', 'Bold', 'bundled', { weight: 700 }),
    ]
    render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitFor(() => expect(screen.getByText('Gamma')).toBeInTheDocument())
    const styleSelect = screen.getByLabelText('Gamma style') as HTMLSelectElement
    expect(Array.from(styleSelect.options).map((o) => o.textContent)).toEqual(['Regular', 'Bold'])
    fireEvent.change(styleSelect, { target: { value: 'fam-bold' } })
    await waitFor(() => expect(styleSelect.value).toBe('fam-bold'))
    expect(screen.getByRole('option', { name: /Gamma/ })).toHaveAttribute('aria-selected', 'true')
  })

  it('loading a font file adds it under "Loaded this session" and selects it', async () => {
    render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitForPickerReady()
    const file = makeFontFile('MyFont.ttf')
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    fireEvent.change(input, { target: { files: [file] } })

    await waitFor(() => expect(screen.getByText('Loaded this session')).toBeInTheDocument())
    expect(screen.getByText('MyFont')).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('option', { name: 'MyFont' })).toHaveAttribute('aria-selected', 'true'))
  })

  it('surfaces a font that fails to load as an error, not a silent failure', async () => {
    state.failNextUserLoad = true
    render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitForPickerReady()
    const file = makeFontFile('Broken.ttf')
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    fireEvent.change(input, { target: { files: [file] } })

    await waitFor(() => expect(screen.getByText(/Couldn't load "Broken.ttf"/)).toBeInTheDocument())
    expect(screen.queryByText('Loaded this session')).not.toBeInTheDocument()
  })

  it('remembers the chosen bundled font across a dialog close/reopen within the session', async () => {
    const { unmount } = render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitForPickerReady()
    fireEvent.click(screen.getByRole('option', { name: 'Beta' }))
    await waitFor(() => expect(state.lastSelected).toBe('bundled-b'))
    unmount()

    render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitForPickerReady()
    await waitFor(() => expect(screen.getByRole('option', { name: 'Beta' })).toHaveAttribute('aria-selected', 'true'))
  })

  it('remembers a loaded user font, not just a bundled reselection, across reopen', async () => {
    const { unmount } = render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitForPickerReady()
    const file = makeFontFile('MyFont.ttf')
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    fireEvent.change(input, { target: { files: [file] } })
    await waitFor(() => expect(screen.getByText('MyFont')).toBeInTheDocument())
    unmount()

    render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitForPickerReady()
    await waitFor(() => expect(screen.getByText('MyFont')).toBeInTheDocument())
    await waitFor(() => expect(screen.getByRole('option', { name: 'MyFont' })).toHaveAttribute('aria-selected', 'true'))
  })

  it('OK hands the resolved font to onPlace', async () => {
    const onPlace = vi.fn()
    render(<TextDialog onPlace={onPlace} onCancel={vi.fn()} />)
    await waitForPickerReady()
    fireEvent.change(screen.getByLabelText('Text'), { target: { value: 'Hi' } })
    await waitFor(() => expect(screen.getByRole('button', { name: /^OK$/ })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: /^OK$/ }))
    await waitFor(() => expect(onPlace).toHaveBeenCalledTimes(1))
    const result = onPlace.mock.calls[0][0]
    expect(result.text).toBe('Hi')
    expect(result.font.id).toBe('bundled-a')
  })

  it('a self-intersecting-outline warning is shown but never blocks OK', async () => {
    vi.mocked(validateGlyphOutlines).mockReturnValue([{ char: 'X' }])
    render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
    await waitForPickerReady()
    await waitFor(() => expect(screen.getByText(/overlap themselves/)).toBeInTheDocument())
    expect(screen.getByRole('button', { name: /^OK$/ })).toBeEnabled()
  })

  describe('system fonts: no provider available', () => {
    it('renders no System section at all', async () => {
      render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
      await waitForPickerReady()
      expect(screen.queryByText('System')).not.toBeInTheDocument()
    })
  })

  describe('system fonts: desktop (tauri) provider', () => {
    it('lists system fonts automatically — no separate gesture required', async () => {
      state.systemKind = 'system-tauri'
      state.systemEntries = [entry('sys-menlo', 'Menlo', 'Regular', 'system')]
      render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
      await waitForPickerReady()
      await waitFor(() => expect(screen.getByText('Menlo')).toBeInTheDocument())
      expect(screen.queryByRole('button', { name: 'Use my system fonts…' })).not.toBeInTheDocument()
    })

    it("an enumeration that SUCCEEDS but returns nothing shows an honest note, not a blank heading (the queryLocalFonts-resolves-empty gap)", async () => {
      state.systemKind = 'system-tauri'
      state.systemEntries = [] // list() resolves fine, just with zero entries
      render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
      await waitForPickerReady()
      expect(await screen.findByText('No system fonts were found on this system.')).toBeInTheDocument()
      // Still an honest section, not a silently blank one: no leftover
      // "loading" note, and no unrelated error/permission wording.
      expect(screen.queryByText('Loading system fonts…')).not.toBeInTheDocument()
      expect(screen.queryByText('Permission was denied.')).not.toBeInTheDocument()
    })
  })

  describe('system fonts: web (queryLocalFonts) provider', () => {
    it('shows a permission affordance and only lists after the explicit gesture', async () => {
      state.systemKind = 'system-web'
      state.systemEntries = [entry('sysweb-menlo', 'Menlo', 'Regular', 'system')]
      render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
      await waitForPickerReady()
      const useButton = await screen.findByRole('button', { name: 'Use my system fonts…' })
      expect(screen.queryByText('Menlo')).not.toBeInTheDocument()
      fireEvent.click(useButton)
      await waitFor(() => expect(screen.getByText('Menlo')).toBeInTheDocument())
    })

    it('a denied permission shows an honest note with a way to ask again, not an error toast', async () => {
      state.systemKind = 'system-web'
      state.systemListImpl = async () => {
        const { LocalFontPermissionDeniedError } = await import('../text/fontSources')
        throw new LocalFontPermissionDeniedError()
      }
      render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
      await waitForPickerReady()
      fireEvent.click(await screen.findByRole('button', { name: 'Use my system fonts…' }))
      await waitFor(() => expect(screen.getByText('Permission was denied.')).toBeInTheDocument())
      expect(screen.getByRole('button', { name: 'Ask again…' })).toBeInTheDocument()
    })

    it("a successful-but-empty resolve (this build's queryLocalFonts behavior with no permission granted) shows an honest note and a way to try again, not a blank heading", async () => {
      state.systemKind = 'system-web'
      state.systemListImpl = async () => [] // resolves, not rejects — the documented gap
      render(<TextDialog onPlace={vi.fn()} onCancel={vi.fn()} />)
      await waitForPickerReady()
      fireEvent.click(await screen.findByRole('button', { name: 'Use my system fonts…' }))
      expect(
        await screen.findByText('No system fonts were found — your browser may have blocked access without asking.'),
      ).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Ask again…' })).toBeInTheDocument()
      // Not misreported as denied or a generic error.
      expect(screen.queryByText('Permission was denied.')).not.toBeInTheDocument()
      expect(screen.queryByText(/Couldn't list system fonts/)).not.toBeInTheDocument()
    })
  })
})
