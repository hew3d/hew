/**
 * LibraryMark — the marker beside a part that no longer matches its Library
 * item (v1.5 OW-2), on its own and on a Components panel row.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { LibraryMark } from './LibraryMark'
import { ComponentsPanel } from './ComponentsPanel'
import type { LibraryLink, LinkState } from '../library/libraryUpdates'
import type { Scene as WasmScene } from '../wasm/loader'

function link(state: LinkState): LibraryLink {
  return { definition: '1', item: 'a', state, copies: 1, locked: false, mine: 'p1:a' }
}

describe('LibraryMark', () => {
  it('shows nothing for a part in step, one without a link, or an item the Library lost', () => {
    for (const l of [link('in_step'), link('item_missing'), undefined]) {
      const { container } = render(<LibraryMark link={l} />)
      expect(container).toBeEmptyDOMElement()
    }
  })

  it('says in its tooltip what the state is', () => {
    const { rerender } = render(<LibraryMark link={link('library_newer')} />)
    expect(screen.getByTestId('library-mark')).toHaveAttribute('title', 'A newer version of this part is in the Library.')
    rerender(<LibraryMark link={link('edited_here')} />)
    expect(screen.getByTestId('library-mark')).toHaveAttribute('title', 'You changed this part here. The Library still has the older version.')
    rerender(<LibraryMark link={link('both_changed')} />)
    expect(screen.getByTestId('library-mark')).toHaveAttribute('title', 'This part changed here and in the Library.')
    rerender(<LibraryMark link={link('different')} />)
    expect(screen.getByTestId('library-mark')).toHaveAttribute('title', 'This part differs from the Library, and Hew can’t tell which one is newer.')
    expect(screen.getByTestId('library-mark')).toHaveTextContent('≠')
  })

  it('a part that only reads different opens Review on click', () => {
    const onReview = vi.fn()
    render(<LibraryMark link={link('different')} onReview={onReview} />)
    fireEvent.click(screen.getByRole('button'))
    expect(onReview).toHaveBeenCalledTimes(1)
  })

  it('a newer part opens Review on click; a part only changed here is just a marker', () => {
    const onReview = vi.fn()
    const { rerender } = render(<LibraryMark link={link('library_newer')} onReview={onReview} />)
    fireEvent.click(screen.getByRole('button'))
    expect(onReview).toHaveBeenCalledTimes(1)
    rerender(<LibraryMark link={link('edited_here')} onReview={onReview} />)
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })
})

describe('ComponentsPanel markers', () => {
  const scene = {
    component_ids: () => new BigUint64Array([1n, 2n]),
    component_name: (id: bigint) => (id === 1n ? 'Oak leg' : 'Door'),
    definition_usage: () => 1,
    instances_of: () => new BigUint64Array(),
    render_definition_thumbnail: () => undefined,
  } as unknown as WasmScene
  const props = {
    scene,
    docRev: 0,
    onSelectNodes: vi.fn(),
    onRenameComponent: vi.fn(() => null),
    onDeleteComponent: vi.fn(),
  }

  it('marks only the rows whose part differs from the Library', () => {
    const links = new Map<string, LibraryLink>([['1', link('library_newer')], ['2', link('in_step')]])
    render(<ComponentsPanel {...props} libraryLinks={links} />)
    const marks = screen.getAllByTestId('library-mark')
    expect(marks).toHaveLength(1)
    expect(marks[0]).toHaveAttribute('data-state', 'library_newer')
  })

  it('shows no marker without links', () => {
    render(<ComponentsPanel {...props} />)
    expect(screen.queryByTestId('library-mark')).not.toBeInTheDocument()
  })
})
