/**
 * LibraryUpdateReview — the Review list (v1.5 OW-2): states in plain words,
 * Library-changed rows checked, risky rows unchecked with a warning, locked
 * rows disabled with the reason, and the skipped parts named afterwards.
 */
import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { LibraryUpdateReview } from './LibraryUpdateReview'
import type { LibraryLink, UpdateOutcome } from '../library/libraryUpdates'

function link(over: Partial<LibraryLink> & { definition: string; item: string }): LibraryLink {
  return { state: 'library_newer', copies: 1, locked: false, mine: 'p1:a', theirs: 'p1:b', ...over }
}

const LINKS: LibraryLink[] = [
  link({ definition: '1', item: 'a', name: 'Oak leg', copies: 4 }),
  link({ definition: '2', item: 'b', name: 'Shelf', state: 'both_changed' }),
  link({ definition: '3', item: 'c', name: 'Door', state: 'different' }),
  link({ definition: '4', item: 'd', name: 'Lamp', locked: true }),
  link({ definition: '5', item: 'e', name: 'Plain', state: 'in_step' }),
]

function renderReview(over: Partial<React.ComponentProps<typeof LibraryUpdateReview>> = {}) {
  const props = {
    links: LINKS,
    outcome: null,
    outcomeLinks: [],
    busy: false,
    onUpdateSelected: vi.fn(),
    onUpdateAllNewer: vi.fn(),
    onClose: vi.fn(),
    ...over,
  }
  render(<LibraryUpdateReview {...props} />)
  return props
}

const rowFor = (name: string): HTMLElement => screen.getByText(name).closest('[data-testid="library-review-row"]') as HTMLElement

describe('LibraryUpdateReview', () => {
  it('lists the parts that differ, in plain words, and leaves the matching one out', () => {
    renderReview()
    expect(screen.getAllByTestId('library-review-row')).toHaveLength(4)
    expect(screen.queryByText('Plain')).not.toBeInTheDocument()
    expect(within(rowFor('Oak leg')).getByText(/Newer in the Library · 4 copies in this model/)).toBeInTheDocument()
    expect(within(rowFor('Shelf')).getByText(/Changed here and in the Library/)).toBeInTheDocument()
    expect(within(rowFor('Door')).getByText(/Different from the Library/)).toBeInTheDocument()
  })

  it('starts newer rows checked, and the warned ones unchecked with their warning', () => {
    renderReview()
    expect(within(rowFor('Oak leg')).getByRole('checkbox')).toBeChecked()
    expect(within(rowFor('Shelf')).getByRole('checkbox')).not.toBeChecked()
    expect(within(rowFor('Shelf')).getByText('Your changes here would be replaced.')).toBeInTheDocument()
    expect(within(rowFor('Door')).getByRole('checkbox')).not.toBeChecked()
  })

  it('shows a locked row disabled, with the reason', () => {
    renderReview()
    const lamp = rowFor('Lamp')
    expect(within(lamp).getByRole('checkbox')).toBeDisabled()
    expect(within(lamp).getByRole('checkbox')).not.toBeChecked()
    expect(within(lamp).getByText('Locked. Unlock it first to update it.')).toBeInTheDocument()
  })

  it('Update Selected sends exactly the checked rows', () => {
    const props = renderReview()
    fireEvent.click(within(rowFor('Shelf')).getByRole('checkbox'))
    fireEvent.click(screen.getByRole('button', { name: 'Update Selected' }))
    expect(props.onUpdateSelected).toHaveBeenCalledWith(['1', '2'])
  })

  it('unchecking everything disables Update Selected', () => {
    renderReview()
    fireEvent.click(within(rowFor('Oak leg')).getByRole('checkbox'))
    expect(screen.getByRole('button', { name: 'Update Selected' })).toBeDisabled()
  })

  it('"Update all that changed in the Library" is its own action', () => {
    const props = renderReview()
    fireEvent.click(screen.getByRole('button', { name: 'Update all that changed in the Library' }))
    expect(props.onUpdateAllNewer).toHaveBeenCalledTimes(1)
  })

  it('has nothing to update-all when no row is newer', () => {
    renderReview({ links: [LINKS[1]!] })
    expect(screen.getByRole('button', { name: 'Update all that changed in the Library' })).toBeDisabled()
  })

  it('names what an update left alone, in plain English', () => {
    const outcome: UpdateOutcome = {
      updated: ['1'],
      alsoUpdated: [],
      unchanged: [],
      kept: [],
      skipped: [
        { definition: '4', name: 'Lamp', code: 'LockedNode' },
        { definition: '2', name: 'Shelf', code: 'ExplodeSessionScope' },
      ],
      definitionsAdded: 0,
      materialsAdded: 0,
    }
    renderReview({ outcome, outcomeLinks: LINKS })
    const box = screen.getByTestId('library-update-outcome')
    expect(within(box).getByText('Updated 1 part from the Library. 2 parts were not updated.')).toBeInTheDocument()
    const lines = within(box).getAllByTestId('library-update-skipped').map((n) => n.textContent)
    expect(lines).toEqual([
      '“Lamp” is locked and was not updated. Unlock it first.',
      '“Shelf” is open for editing and was not updated. Close the open group or component first.',
    ])
    expect(screen.getByRole('button', { name: 'Done' })).toBeInTheDocument()
  })

  it('says which part was kept and that the updated one got its own copy', () => {
    const links = [
      { definition: '1', item: 'table', name: 'Table', state: 'in_step', copies: 1, locked: false, mine: 'p1:a' },
      { definition: '2', item: 'table', name: 'Leg', state: 'in_step', copies: 1, locked: false, mine: 'p1:b' },
    ] as const
    const outcome: UpdateOutcome = { updated: ['1'], alsoUpdated: [], unchanged: [], kept: ['2'], skipped: [], definitionsAdded: 1, materialsAdded: 0 }
    renderReview({ outcome, outcomeLinks: links })
    expect(screen.getByTestId('library-update-kept')).toHaveTextContent(
      '“Leg” was left as it is because it was changed here or is locked; the updated “Table” got its own copy of the new “Leg”.',
    )
  })

  it('names a row by the Library\'s name for the item when it is known', () => {
    renderReview({ itemName: (item) => (item === 'a' ? 'Leg' : undefined) })
    expect(screen.getByText('Leg')).toBeInTheDocument()
    expect(screen.queryByText('Oak leg')).not.toBeInTheDocument()
    expect(screen.getByText('Shelf')).toBeInTheDocument()
  })

  it('closes on Escape without reaching the window', () => {
    const props = renderReview()
    const windowKeyDown = vi.fn()
    window.addEventListener('keydown', windowKeyDown)
    try {
      fireEvent.keyDown(document, { key: 'Escape' })
      expect(windowKeyDown).not.toHaveBeenCalled()
    } finally {
      window.removeEventListener('keydown', windowKeyDown)
    }
    expect(props.onClose).toHaveBeenCalledTimes(1)
  })
})
