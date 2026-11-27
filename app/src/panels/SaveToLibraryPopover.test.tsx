import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { SaveToLibraryPopover, dockPopoverPlacement } from './SaveToLibraryPopover'

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

  describe('Update in Library', () => {
    it('leads with the update, keeps Save as a new item beneath, and says other models are offered it', () => {
      const onUpdate = vi.fn()
      render(
        <SaveToLibraryPopover
          defaultName="Oak leg"
          updateInLibrary={{ name: 'Oak leg', confirm: false }}
          onUpdateInLibrary={onUpdate}
          onSave={vi.fn()}
          onClose={vi.fn()}
        />,
      )
      expect(screen.getByRole('button', { name: 'Update “Oak leg” in the Library' })).toBeInTheDocument()
      expect(screen.getByText('Other models that use it will be offered the update.')).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Save as a new item' })).toBeInTheDocument()
      fireEvent.click(screen.getByTestId('update-in-library'))
      expect(onUpdate).toHaveBeenCalledTimes(1)
    })

    it('Save as a new item still saves under the typed name', () => {
      const onSave = vi.fn()
      render(
        <SaveToLibraryPopover
          defaultName="Oak leg"
          updateInLibrary={{ name: 'Oak leg', confirm: false }}
          onUpdateInLibrary={vi.fn()}
          onSave={onSave}
          onClose={vi.fn()}
        />,
      )
      fireEvent.change(screen.getByLabelText('Item name'), { target: { value: 'Oak leg 2' } })
      fireEvent.click(screen.getByRole('button', { name: 'Save as a new item' }))
      expect(onSave).toHaveBeenCalledWith('Oak leg 2')
    })

    it('asks first when the Library has also changed, and only then replaces', () => {
      const onUpdate = vi.fn()
      render(
        <SaveToLibraryPopover
          defaultName="Oak leg"
          updateInLibrary={{ name: 'Oak leg', confirm: true }}
          onUpdateInLibrary={onUpdate}
          onSave={vi.fn()}
          onClose={vi.fn()}
        />,
      )
      fireEvent.click(screen.getByTestId('update-in-library'))
      expect(onUpdate).not.toHaveBeenCalled()
      expect(screen.getByTestId('update-in-library-confirm')).toHaveTextContent(/Library has also changed since you inserted/)
      fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
      expect(onUpdate).not.toHaveBeenCalled()
      fireEvent.click(screen.getByTestId('update-in-library'))
      fireEvent.click(screen.getByTestId('update-in-library-replace'))
      expect(onUpdate).toHaveBeenCalledTimes(1)
    })

    it('asks first, in its own words, when Hew cannot tell which side is newer', () => {
      const onUpdate = vi.fn()
      render(
        <SaveToLibraryPopover
          defaultName="Oak leg"
          updateInLibrary={{ name: 'Oak leg', confirm: true, cannotCompare: true }}
          onUpdateInLibrary={onUpdate}
          onSave={vi.fn()}
          onClose={vi.fn()}
        />,
      )
      fireEvent.click(screen.getByTestId('update-in-library'))
      expect(onUpdate).not.toHaveBeenCalled()
      expect(screen.getByTestId('update-in-library-confirm')).toHaveTextContent(
        'The Library copy and this part have both changed or can’t be compared. Replacing it will lose the Library’s version.',
      )
      fireEvent.click(screen.getByTestId('update-in-library-replace'))
      expect(onUpdate).toHaveBeenCalledTimes(1)
    })

    it('says why a linked copy cannot update its item, and offers no update', () => {
      render(
        <SaveToLibraryPopover defaultName="Oak leg" updateInLibraryBlocked={{ name: 'Oak leg' }} onSave={vi.fn()} onClose={vi.fn()} />,
      )
      expect(screen.queryByTestId('update-in-library')).not.toBeInTheDocument()
      expect(screen.getByTestId('update-in-library-blocked')).toHaveTextContent(
        'This part is a linked copy, so it can’t update “Oak leg” in the Library. Make it a component or unlink it first.',
      )
    })

    it('offers nothing extra for an ordinary save', () => {
      render(<SaveToLibraryPopover defaultName="Box" onSave={vi.fn()} onClose={vi.fn()} />)
      expect(screen.queryByTestId('update-in-library')).not.toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Save' })).toBeInTheDocument()
    })
  })
})

describe('dockPopoverPlacement', () => {
  const win = { width: 1280, height: 720 }

  it('centers the dialog on the anchor and sits a gap above the dock', () => {
    const p = dockPopoverPlacement({ left: 600, width: 80 }, 640, win, 310)
    expect(p.left + 155).toBe(640)
    expect(p.bottom).toBe(720 - 640 + 16)
  })

  it('shifts to keep an 8 px margin at the left and right window edges', () => {
    expect(dockPopoverPlacement({ left: 10, width: 60 }, 640, win).left).toBe(8)
    expect(dockPopoverPlacement({ left: 1200, width: 60 }, 640, win).left).toBe(1280 - 310 - 8)
  })

  it('stays at the left margin when the window is narrower than the dialog', () => {
    expect(dockPopoverPlacement({ left: 100, width: 60 }, 640, { width: 200, height: 720 }).left).toBe(8)
  })
})

describe('SaveToLibraryPopover placement over a dock button', () => {
  function mountDock(left: number): () => void {
    const dock = document.createElement('div')
    dock.setAttribute('data-testid', 'contextual-dock')
    const button = document.createElement('button')
    button.setAttribute('data-dock-verb', 'save-to-library')
    dock.append(button)
    document.body.append(dock)
    dock.getBoundingClientRect = () => ({ top: 600, left: 0, width: 400, height: 54, right: 400, bottom: 654, x: 0, y: 600, toJSON: () => ({}) })
    button.getBoundingClientRect = () => ({ top: 608, left, width: 60, height: 38, right: left + 60, bottom: 646, x: left, y: 608, toJSON: () => ({}) })
    return () => dock.remove()
  }

  it('anchors the dock variant to its button', () => {
    const cleanup = mountDock(500)
    try {
      render(<SaveToLibraryPopover defaultName="Box" anchorVerb="save-to-library" onSave={vi.fn()} onClose={vi.fn()} />)
      const dialog = screen.getByRole('dialog')
      expect(dialog.style.position).toBe('absolute')
      expect(dialog.style.left).toBe(`${530 - 155}px`)
      expect(dialog.style.bottom).toBe(`${window.innerHeight - 600 + 16}px`)
    } finally {
      cleanup()
    }
  })

  it('leaves the modal variant and an unanchored dock variant as they were', () => {
    const cleanup = mountDock(500)
    try {
      const { unmount } = render(<SaveToLibraryPopover defaultName="Box" variant="modal" anchorVerb="save-to-library" onSave={vi.fn()} onClose={vi.fn()} />)
      expect(screen.getByRole('dialog').style.position).toBe('')
      unmount()
      render(<SaveToLibraryPopover defaultName="Box" onSave={vi.fn()} onClose={vi.fn()} />)
      expect(screen.getByRole('dialog').style.position).toBe('')
    } finally {
      cleanup()
    }
  })

  it('still closes on a backdrop click and Escape when anchored', () => {
    const cleanup = mountDock(500)
    try {
      const onClose = vi.fn()
      const { container } = render(<SaveToLibraryPopover defaultName="Box" anchorVerb="save-to-library" onSave={vi.fn()} onClose={onClose} />)
      fireEvent.click(container.firstChild as HTMLElement)
      fireEvent.keyDown(document, { key: 'Escape' })
      expect(onClose).toHaveBeenCalledTimes(2)
    } finally {
      cleanup()
    }
  })
})
