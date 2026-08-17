/**
 * DocumentMenu tests — the document half of the maintainer-approved "Idea 2"
 * split of the old combined `OverflowMenu.tsx`. `ShopApp.test.tsx`'s own
 * "document menu" describe block covers the real pill click opening this
 * (an integration concern — the pill itself lives in `ShopApp.tsx`, not
 * here); this file drives `DocumentMenu` directly via its `open` prop and
 * covers the component's own contract: header/row content, empty-state
 * degradation, row dispatch, scrim close, and corner anchoring in both
 * orientations.
 */
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { DocumentMenu } from './DocumentMenu'

function renderMenu(overrides: Partial<Parameters<typeof DocumentMenu>[0]> = {}) {
  const props = {
    open: true,
    docName: 'Wall Clock.hew' as string | null,
    orientation: 'portrait' as const,
    onClose: vi.fn(),
    onOpen: vi.fn(),
    onOpenScanner: vi.fn(),
    onOpenRecents: vi.fn(),
    onSaveCopy: vi.fn(),
    // Off by default — most of this file's existing tests don't care about
    // "View in AR…" at all, so they'd otherwise need to know about it just
    // to keep asserting an absence.
    showViewInAr: false,
    arBusy: false,
    onViewInAr: vi.fn(),
    onUseFullEditor: vi.fn(),
    ...overrides,
  }
  render(<DocumentMenu {...props} />)
  return props
}

describe('DocumentMenu', () => {
  it('renders nothing while closed', () => {
    renderMenu({ open: false })
    expect(screen.queryByRole('button', { name: /^open…$/i })).not.toBeInTheDocument()
    expect(screen.queryByTestId('shop-document-scrim')).not.toBeInTheDocument()
  })

  it('shows the document name as its header once loaded', () => {
    renderMenu({ docName: 'Wall Clock.hew' })
    expect(screen.getByText('Wall Clock.hew')).toBeInTheDocument()
  })

  it('degrades to a "Shop Mode" header and hides the document-only row with nothing loaded', () => {
    renderMenu({ docName: null })
    expect(screen.getByText('Shop Mode')).toBeInTheDocument()
    // Open…/Open from desktop…/Use full editor stay reachable — this panel
    // is the ONLY seam onto "Use full editor" before a document exists.
    expect(screen.getByRole('button', { name: /^open…$/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^open from desktop…$/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^use full editor$/i })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^save a copy/i })).not.toBeInTheDocument()
  })

  it('shows "Save a copy (.hew)" once a document is loaded', () => {
    renderMenu({ docName: 'Wall Clock.hew' })
    expect(screen.getByRole('button', { name: /^save a copy \(\.hew\)$/i })).toBeInTheDocument()
  })

  it('dispatches each row to its own handler', () => {
    const props = renderMenu({ docName: 'Wall Clock.hew' })

    fireEvent.click(screen.getByRole('button', { name: /^open…$/i }))
    expect(props.onOpen).toHaveBeenCalledTimes(1)

    fireEvent.click(screen.getByRole('button', { name: /^open from desktop…$/i }))
    expect(props.onOpenScanner).toHaveBeenCalledTimes(1)

    fireEvent.click(screen.getByRole('button', { name: /^save a copy \(\.hew\)$/i }))
    expect(props.onSaveCopy).toHaveBeenCalledTimes(1)

    fireEvent.click(screen.getByRole('button', { name: /^use full editor$/i }))
    expect(props.onUseFullEditor).toHaveBeenCalledTimes(1)
  })

  it('closes on a scrim tap', () => {
    const props = renderMenu()
    fireEvent.click(screen.getByTestId('shop-document-scrim'))
    expect(props.onClose).toHaveBeenCalledTimes(1)
  })

  it('anchors its top-left corner at the portrait offset (left:12px)', () => {
    renderMenu({ orientation: 'portrait' })
    const panel = screen.getByTestId('shop-document-panel')
    expect(panel.style.left).toBe('12px')
    expect(panel.style.right).toBe('')
  })

  it('anchors its top-left corner at the landscape safe-area offset', () => {
    renderMenu({ orientation: 'landscape' })
    const panel = screen.getByTestId('shop-document-panel')
    // jsdom's CSSOM re-serializes `calc()` (operand order, `env()`
    // fallback-comma spacing) — assert on content, not jsdom's exact
    // formatting of the literal `LANDSCAPE_LEFT_OFFSET_CSS` string.
    expect(panel.style.left).toContain('safe-area-inset-left')
    expect(panel.style.left).toContain('16px')
    expect(panel.style.right).toBe('')
  })

  // "View in AR…" (task 3) — moved here from the now-removed dock/rail
  // button once the toolbar reshuffle (task 2) dropped it from both bars.
  describe('View in AR…', () => {
    it('is absent when showViewInAr is false, even with a document loaded', () => {
      renderMenu({ docName: 'Wall Clock.hew', showViewInAr: false })
      expect(screen.queryByRole('button', { name: /view in ar/i })).not.toBeInTheDocument()
    })

    // Unlike "Save a copy" (gated on `docName` directly, right here in this
    // component), `showViewInAr` is a single externally-computed flag —
    // `ShopApp.tsx` already bundles its own document-loaded check together
    // with `isArQuickLookCandidate()` before passing it down (that
    // component's own doc comment), so this component trusts it as-is
    // rather than re-deriving a second gate from `docName`.
    // `ShopApp.test.tsx`'s own "View in AR" describe block covers the
    // bundled, document-loaded-aware contract end to end.
    it('renders even with nothing loaded, trusting showViewInAr as the single gate', () => {
      renderMenu({ docName: null, showViewInAr: true })
      expect(screen.getByRole('button', { name: /view in ar/i })).toBeInTheDocument()
    })

    it('renders and dispatches to onViewInAr when showViewInAr is true', () => {
      const props = renderMenu({ docName: 'Wall Clock.hew', showViewInAr: true })
      const row = screen.getByRole('button', { name: /^view in ar…$/i })
      expect(row).toBeInTheDocument()
      fireEvent.click(row)
      expect(props.onViewInAr).toHaveBeenCalledTimes(1)
    })

    it('relabels to "Preparing…" while arBusy, without hiding the row', () => {
      renderMenu({ docName: 'Wall Clock.hew', showViewInAr: true, arBusy: true })
      expect(screen.queryByRole('button', { name: /^view in ar…$/i })).not.toBeInTheDocument()
      expect(screen.getByRole('button', { name: /^preparing…$/i })).toBeInTheDocument()
    })
  })
})

describe('DocumentMenu — Recent models', () => {
  it('offers "Recent models…" whether or not a document is open, and it calls onOpenRecents', () => {
    const props = renderMenu({ docName: null })
    fireEvent.click(screen.getByRole('button', { name: /^recent models…$/i }))
    expect(props.onOpenRecents).toHaveBeenCalledTimes(1)
  })
})
