/**
 * LibraryMenu — the tile's ⋯ popover. Pins the Download…/Reveal gating:
 * Download appears only on backends that report it (the web store) and
 * never for an errored item (there are no bytes to hand over), while
 * Reveal stays a desktop-only affair.
 */

import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { i18n } from '@lingui/core'
import { LibraryMenu, revealLabel } from './LibraryMenu'

function baseProps() {
  return {
    itemName: 'Theater Chair',
    anchor: { top: 10, left: 10, bottom: 20 },
    canReveal: false,
    canDownload: false,
    hasSourceInfo: false,
    canEdit: true,
    onClose: vi.fn(),
    onEdit: vi.fn(),
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
    expect(screen.queryByText(i18n._(revealLabel))).toBeNull()
  })

  it('hides Download… without canDownload (desktop shows Reveal instead)', () => {
    render(<LibraryMenu {...baseProps()} canReveal />)
    expect(screen.queryByText('Download…')).toBeNull()
    expect(screen.getByText(i18n._(revealLabel))).toBeInTheDocument()
  })

  it('hides Download… for an errored item even when the backend supports it', () => {
    render(<LibraryMenu {...baseProps()} canDownload errored />)
    expect(screen.queryByText('Download…')).toBeNull()
    // Delete stays reachable — the one action an unreadable item still has
    // on a backend with no Reveal.
    expect(screen.getByText('Delete from library…')).toBeInTheDocument()
  })

  it('offers Edit first, runs it and closes the menu', () => {
    const props = baseProps()
    render(<LibraryMenu {...props} />)
    const rows = screen.getAllByRole('button').map((b) => b.textContent)
    expect(rows.indexOf('Edit')).toBeLessThan(rows.indexOf('Open as document'))
    fireEvent.click(screen.getByText('Edit'))
    expect(props.onEdit).toHaveBeenCalledOnce()
    expect(props.onClose).toHaveBeenCalledOnce()
  })

  it('hides Edit when the item cannot be edited, and for an errored item', () => {
    const { unmount } = render(<LibraryMenu {...baseProps()} canEdit={false} />)
    expect(screen.queryByText('Edit')).toBeNull()
    unmount()
    render(<LibraryMenu {...baseProps()} errored />)
    expect(screen.queryByText('Edit')).toBeNull()
  })

  it('an item from a source offers "Update from source" (when newer) and "Unlink from <source>"', () => {
    const props = baseProps()
    const onUpdateFromSource = vi.fn()
    const onUnlinkFromSource = vi.fn()
    render(<LibraryMenu {...props} fromSource={{ name: 'Hinges', canUpdate: true }} onUpdateFromSource={onUpdateFromSource} onUnlinkFromSource={onUnlinkFromSource} />)
    fireEvent.click(screen.getByText('Update from source'))
    expect(onUpdateFromSource).toHaveBeenCalledOnce()
    expect(props.onClose).toHaveBeenCalledOnce()
    fireEvent.click(screen.getByText('Unlink from Hinges'))
    expect(onUnlinkFromSource).toHaveBeenCalledOnce()
  })

  it('an up-to-date item from a source offers Unlink only', () => {
    render(<LibraryMenu {...baseProps()} fromSource={{ name: 'Hinges', canUpdate: false }} onUpdateFromSource={vi.fn()} onUnlinkFromSource={vi.fn()} />)
    expect(screen.queryByText('Update from source')).toBeNull()
    expect(screen.getByText('Unlink from Hinges')).toBeInTheDocument()
  })

  it('an item with no note shows neither row', () => {
    render(<LibraryMenu {...baseProps()} onUpdateFromSource={vi.fn()} onUnlinkFromSource={vi.fn()} />)
    expect(screen.queryByText('Update from source')).toBeNull()
    expect(screen.queryByText(/^Unlink from/)).toBeNull()
  })

  it('an errored item shows no source rows', () => {
    render(<LibraryMenu {...baseProps()} errored fromSource={{ name: 'Hinges', canUpdate: true }} onUpdateFromSource={vi.fn()} onUnlinkFromSource={vi.fn()} />)
    expect(screen.queryByText('Update from source')).toBeNull()
    expect(screen.queryByText(/^Unlink from/)).toBeNull()
  })
})
