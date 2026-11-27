import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { CutScopeStrip } from './CutScopeStrip'

describe('CutScopeStrip', () => {
  it('offers Cut all N copies and Only this one, with the first as the default', () => {
    render(<CutScopeStrip copies={4} onChoose={vi.fn()} onCancel={vi.fn()} />)
    const all = screen.getByRole('button', { name: 'Cut all 4 copies' })
    expect(screen.getByRole('button', { name: 'Only this one' })).toBeInTheDocument()
    expect(document.activeElement).toBe(all)
  })

  it('Enter chooses all copies, the buttons choose their scope, Escape cancels', () => {
    const onChoose = vi.fn()
    const onCancel = vi.fn()
    render(<CutScopeStrip copies={3} onChoose={onChoose} onCancel={onCancel} />)
    fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Enter' })
    expect(onChoose).toHaveBeenLastCalledWith('all')
    fireEvent.click(screen.getByRole('button', { name: 'Only this one' }))
    expect(onChoose).toHaveBeenLastCalledWith('one')
    fireEvent.keyDown(document.body, { key: 'Escape' })
    expect(onCancel).toHaveBeenCalledTimes(1)
  })

  it('Enter and Escape typed into a field or dialog are not about the strip', () => {
    const onChoose = vi.fn()
    const onCancel = vi.fn()
    render(
      <div>
        <CutScopeStrip copies={3} onChoose={onChoose} onCancel={onCancel} />
        <input aria-label="name" />
        <div role="dialog"><button type="button">inside</button></div>
      </div>,
    )
    fireEvent.keyDown(screen.getByLabelText('name'), { key: 'Enter' })
    fireEvent.keyDown(screen.getByLabelText('name'), { key: 'Escape' })
    fireEvent.keyDown(screen.getByRole('button', { name: 'inside' }), { key: 'Enter' })
    expect(onChoose).not.toHaveBeenCalled()
    expect(onCancel).not.toHaveBeenCalled()
  })

  it('Enter on the other button presses that button instead of the default', () => {
    const onChoose = vi.fn()
    render(<CutScopeStrip copies={3} onChoose={onChoose} onCancel={vi.fn()} />)
    const only = screen.getByRole('button', { name: 'Only this one' })
    only.focus()
    fireEvent.keyDown(only, { key: 'Enter' })
    expect(onChoose).not.toHaveBeenCalled()
  })
})
