import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { SaveToLibraryPopover } from './SaveToLibraryPopover'

/** Every dialog's Escape handler must `stopPropagation()` alongside its
 * `preventDefault()` — see `dialogs.test.tsx`'s identical helper. */
function expectEscapeStopsPropagationToWindow(): void {
  const windowKeyDown = vi.fn()
  window.addEventListener('keydown', windowKeyDown)
  try {
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(windowKeyDown).not.toHaveBeenCalled()
  } finally {
    window.removeEventListener('keydown', windowKeyDown)
  }
}

describe('SaveToLibraryPopover', () => {
  it('renders the mono "Save to Library" label and the origin-point footnote', () => {
    render(<SaveToLibraryPopover defaultName="Box" onSave={vi.fn()} onClose={vi.fn()} />)
    expect(screen.getByText('Save to Library')).toBeInTheDocument()
    expect(screen.getByText(/inserts by its bottom center/i)).toBeInTheDocument()
  })

  it('has no keywords/collection hint (not implemented yet)', () => {
    render(<SaveToLibraryPopover defaultName="Box" onSave={vi.fn()} onClose={vi.fn()} />)
    expect(screen.queryByText(/keywords/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/collection/i)).not.toBeInTheDocument()
  })

  it('pre-fills the name input with defaultName', () => {
    render(<SaveToLibraryPopover defaultName="Patio Chair" onSave={vi.fn()} onClose={vi.fn()} />)
    expect(screen.getByLabelText('Item name')).toHaveValue('Patio Chair')
  })

  it('auto-focuses the name input', () => {
    render(<SaveToLibraryPopover defaultName="Box" onSave={vi.fn()} onClose={vi.fn()} />)
    expect(screen.getByLabelText('Item name')).toHaveFocus()
  })

  it('Enter in the name field submits the (possibly edited) name', () => {
    const onSave = vi.fn()
    render(<SaveToLibraryPopover defaultName="Box" onSave={onSave} onClose={vi.fn()} />)
    const input = screen.getByLabelText('Item name')
    fireEvent.change(input, { target: { value: 'Patio Chair' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onSave).toHaveBeenCalledWith('Patio Chair')
  })

  it('clicking Save submits the current name', () => {
    const onSave = vi.fn()
    render(<SaveToLibraryPopover defaultName="Box" onSave={onSave} onClose={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(onSave).toHaveBeenCalledWith('Box')
  })

  it('submits the default name when the field is cleared/blank', () => {
    const onSave = vi.fn()
    render(<SaveToLibraryPopover defaultName="Box" onSave={onSave} onClose={vi.fn()} />)
    const input = screen.getByLabelText('Item name')
    fireEvent.change(input, { target: { value: '   ' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(onSave).toHaveBeenCalledWith('Box')
  })

  it('Escape closes the popover and stops propagation to window', () => {
    const onClose = vi.fn()
    render(<SaveToLibraryPopover defaultName="Box" onSave={vi.fn()} onClose={onClose} />)
    expectEscapeStopsPropagationToWindow()
    expect(onClose).toHaveBeenCalled()
  })

  it('clicking the backdrop closes the popover', () => {
    const onClose = vi.fn()
    const { container } = render(
      <SaveToLibraryPopover defaultName="Box" onSave={vi.fn()} onClose={onClose} />,
    )
    fireEvent.click(container.firstChild as HTMLElement)
    expect(onClose).toHaveBeenCalled()
  })

  it('clicking inside the popover panel does not close it', () => {
    const onClose = vi.fn()
    render(<SaveToLibraryPopover defaultName="Box" onSave={vi.fn()} onClose={onClose} />)
    fireEvent.click(screen.getByRole('dialog'))
    expect(onClose).not.toHaveBeenCalled()
  })
})
