/**
 * LibraryUpdateBar — the notice under the menu bar (v1.5 OW-2): silent with
 * nothing newer, counts in plain words, and offers Update All / Review… /
 * Later.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { LibraryUpdateBar } from './LibraryUpdateBar'

const noop = (): void => {}

describe('LibraryUpdateBar', () => {
  it('renders nothing when nothing is newer', () => {
    render(<LibraryUpdateBar count={0} onUpdateAll={noop} onReview={noop} onLater={noop} />)
    expect(screen.queryByTestId('library-update-bar')).not.toBeInTheDocument()
  })

  it('says one part, or N parts, has a newer version', () => {
    const { rerender } = render(<LibraryUpdateBar count={1} onUpdateAll={noop} onReview={noop} onLater={noop} />)
    expect(screen.getByText('1 part has a newer version in the Library.')).toBeInTheDocument()
    rerender(<LibraryUpdateBar count={3} onUpdateAll={noop} onReview={noop} onLater={noop} />)
    expect(screen.getByText('3 parts have newer versions in the Library.')).toBeInTheDocument()
  })

  it('wires the three buttons', () => {
    const onUpdateAll = vi.fn()
    const onReview = vi.fn()
    const onLater = vi.fn()
    render(<LibraryUpdateBar count={2} onUpdateAll={onUpdateAll} onReview={onReview} onLater={onLater} />)
    fireEvent.click(screen.getByRole('button', { name: 'Update All' }))
    fireEvent.click(screen.getByRole('button', { name: 'Review…' }))
    fireEvent.click(screen.getByRole('button', { name: 'Later' }))
    expect(onUpdateAll).toHaveBeenCalledTimes(1)
    expect(onReview).toHaveBeenCalledTimes(1)
    expect(onLater).toHaveBeenCalledTimes(1)
  })

  it('rests Update All and Review while an update runs, but Later still works', () => {
    render(<LibraryUpdateBar count={2} busy onUpdateAll={noop} onReview={noop} onLater={noop} />)
    expect(screen.getByRole('button', { name: 'Update All' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Review…' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Later' })).toBeEnabled()
  })
})
