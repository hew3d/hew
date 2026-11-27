/**
 * SourceUpdateBar — the notice at the top of the Library dialog (design 7.5):
 * silent with nothing to say, counts in plain words, a line for a followed
 * source, and Update all / Review… / Later.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { SourceUpdateBar } from './SourceUpdateBar'

const noop = (): void => {}

describe('SourceUpdateBar', () => {
  it('renders nothing when nothing is newer or new', () => {
    render(<SourceUpdateBar model={{ newer: 0, followed: [] }} onUpdateAll={noop} onReview={noop} onLater={noop} />)
    expect(screen.queryByTestId('source-update-bar')).not.toBeInTheDocument()
  })

  it('says one item, or N items, have newer versions at their sources', () => {
    const { rerender } = render(<SourceUpdateBar model={{ newer: 1, followed: [] }} onUpdateAll={noop} onReview={noop} onLater={noop} />)
    expect(screen.getByText('1 item has a newer version at its source.')).toBeInTheDocument()
    rerender(<SourceUpdateBar model={{ newer: 3, followed: [] }} onUpdateAll={noop} onReview={noop} onLater={noop} />)
    expect(screen.getByText('3 items have newer versions at their sources.')).toBeInTheDocument()
  })

  it('gives each followed source a line of its own', () => {
    render(
      <SourceUpdateBar
        model={{ newer: 2, followed: [{ source: 'github:a/b', name: 'Hinges', fresh: 2, changed: 1 }, { source: 'github:a/c', name: 'Slides', fresh: 0, changed: 4 }] }}
        onUpdateAll={noop}
        onReview={noop}
        onLater={noop}
      />,
    )
    expect(screen.getByText('2 items have newer versions at their sources.')).toBeInTheDocument()
    expect(screen.getByText('Hinges has 2 new and 1 changed items')).toBeInTheDocument()
    expect(screen.getByText('Slides has 4 changed items')).toBeInTheDocument()
  })

  it('wires the three buttons, and rests the first two while busy', () => {
    const onUpdateAll = vi.fn()
    const onReview = vi.fn()
    const onLater = vi.fn()
    const { rerender } = render(<SourceUpdateBar model={{ newer: 2, followed: [] }} onUpdateAll={onUpdateAll} onReview={onReview} onLater={onLater} />)
    fireEvent.click(screen.getByRole('button', { name: 'Update all' }))
    fireEvent.click(screen.getByRole('button', { name: 'Review…' }))
    fireEvent.click(screen.getByRole('button', { name: 'Later' }))
    expect(onUpdateAll).toHaveBeenCalledOnce()
    expect(onReview).toHaveBeenCalledOnce()
    expect(onLater).toHaveBeenCalledOnce()
    rerender(<SourceUpdateBar model={{ newer: 2, followed: [] }} onUpdateAll={onUpdateAll} onReview={onReview} onLater={onLater} busy />)
    expect(screen.getByRole('button', { name: 'Update all' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Review…' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Later' })).toBeEnabled()
  })
})
