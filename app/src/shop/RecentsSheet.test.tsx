import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { RecentsSheet } from './RecentsSheet'
import type { RecentEntry } from '../io/recents'

const ENTRIES: RecentEntry[] = [
  { id: 'a', name: 'Table.hew', timestamp: Date.now() - 60_000, bytes: new Uint8Array([1]), contentHash: 'h1', partCount: 9 } as RecentEntry,
  { id: 'b', name: 'Bench.hew', timestamp: Date.now() - 3_600_000, bytes: new Uint8Array([2]), contentHash: 'h2' } as RecentEntry,
]

describe('RecentsSheet', () => {
  it('renders nothing while closed', () => {
    const { container } = render(<RecentsSheet open={false} orientation="portrait" entries={ENTRIES} onClose={vi.fn()} onOpen={vi.fn()} />)
    expect(container).toBeEmptyDOMElement()
  })

  it('lists every recent; a row opens it and closes the sheet; the scrim closes', () => {
    const onOpen = vi.fn()
    const onClose = vi.fn()
    render(<RecentsSheet open orientation="portrait" entries={ENTRIES} onClose={onClose} onOpen={onOpen} />)
    expect(screen.getByRole('dialog', { name: 'Recent models' })).toBeInTheDocument()
    expect(screen.getByText('Table.hew')).toBeInTheDocument()
    expect(screen.getByText('Bench.hew')).toBeInTheDocument()
    fireEvent.click(screen.getByText('Bench.hew'))
    expect(onOpen).toHaveBeenCalledWith(ENTRIES[1])
    expect(onClose).toHaveBeenCalledTimes(1)
    fireEvent.click(screen.getByTestId('shop-recents-scrim'))
    expect(onClose).toHaveBeenCalledTimes(2)
  })

  it('explains the empty case', () => {
    render(<RecentsSheet open orientation="landscape" entries={[]} onClose={vi.fn()} onOpen={vi.fn()} />)
    expect(screen.getByText(/no recent models yet/i)).toBeInTheDocument()
  })
})
