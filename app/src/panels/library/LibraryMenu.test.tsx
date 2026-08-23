/**
 * LibraryMenu — the tile's ⋯ popover. Pins the Download…/Reveal gating:
 * Download appears only on backends that report it (the web store) and
 * never for an errored item (there are no bytes to hand over), while
 * Reveal stays a desktop-only affair.
 */

import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { LibraryMenu, revealLabel } from './LibraryMenu'

function baseProps() {
  return {
    itemName: 'Theater Chair',
    anchor: { top: 10, left: 10, bottom: 20 },
    canReveal: false,
    canDownload: false,
    hasSourceInfo: false,
    onClose: vi.fn(),
    onOpenAsDocument: vi.fn(),
    onAddToCollection: vi.fn(),
    onRename: vi.fn(),
    onRerenderThumbnail: vi.fn(),
    onRemoveSourceInfo: vi.fn(),
    onReveal: vi.fn(),
    onDownload: vi.fn(),
    onDeleteRequest: vi.fn(),
  }
}

describe('LibraryMenu', () => {
  it('offers Download… when the backend reports canDownload, and runs it', () => {
    const props = baseProps()
    render(<LibraryMenu {...props} canDownload />)
    fireEvent.click(screen.getByText('Download…'))
    expect(props.onDownload).toHaveBeenCalledOnce()
    expect(screen.queryByText(revealLabel)).toBeNull()
  })

  it('hides Download… without canDownload (desktop shows Reveal instead)', () => {
    render(<LibraryMenu {...baseProps()} canReveal />)
    expect(screen.queryByText('Download…')).toBeNull()
    expect(screen.getByText(revealLabel)).toBeInTheDocument()
  })

  it('hides Download… for an errored item even when the backend supports it', () => {
    render(<LibraryMenu {...baseProps()} canDownload errored />)
    expect(screen.queryByText('Download…')).toBeNull()
    // Delete stays reachable — the one action an unreadable item still has
    // on a backend with no Reveal.
    expect(screen.getByText('Delete from library…')).toBeInTheDocument()
  })
})
