/**
 * ViewsSheet tests — the seven standard-view rows (each acts AND closes),
 * the projection toggle (acts WITHOUT closing, reflects the current
 * projection), and the portrait/landscape container fork (mirrors
 * UnitPicker.test.tsx's own coverage shape for the identical fork).
 */
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ViewsSheet } from './ViewsSheet'

describe('ViewsSheet — closed', () => {
  it('renders nothing when closed', () => {
    const { container } = render(
      <ViewsSheet open={false} orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()} />,
    )
    expect(container).toBeEmptyDOMElement()
  })
})

describe('ViewsSheet — standard views', () => {
  it('lists all seven standard views', () => {
    render(
      <ViewsSheet open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()} />,
    )
    for (const label of ['ISO', 'Front', 'Back', 'Left', 'Right', 'Top', 'Bottom']) {
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument()
    }
  })

  it('tapping a view calls onSelectView with the right StandardView and closes the sheet', () => {
    const onSelectView = vi.fn()
    const onClose = vi.fn()
    render(
      <ViewsSheet open orientation="portrait" onClose={onClose} onSelectView={onSelectView} projection="perspective" onToggleProjection={vi.fn()} />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Top' }))
    expect(onSelectView).toHaveBeenCalledWith('top')
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('tapping ISO passes the "iso" view', () => {
    const onSelectView = vi.fn()
    render(
      <ViewsSheet open orientation="portrait" onClose={vi.fn()} onSelectView={onSelectView} projection="perspective" onToggleProjection={vi.fn()} />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'ISO' }))
    expect(onSelectView).toHaveBeenCalledWith('iso')
  })
})

describe('ViewsSheet — projection toggle', () => {
  it('reflects the current projection with an aria-pressed active segment', () => {
    render(
      <ViewsSheet open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="parallel" onToggleProjection={vi.fn()} />,
    )
    expect(screen.getByRole('button', { name: 'Parallel' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: 'Perspective' })).toHaveAttribute('aria-pressed', 'false')
  })

  it('tapping the INACTIVE segment toggles projection without closing the sheet', () => {
    const onToggleProjection = vi.fn()
    const onClose = vi.fn()
    render(
      <ViewsSheet open orientation="portrait" onClose={onClose} onSelectView={vi.fn()} projection="perspective" onToggleProjection={onToggleProjection} />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Parallel' }))
    expect(onToggleProjection).toHaveBeenCalledTimes(1)
    expect(onClose).not.toHaveBeenCalled()
  })

  it('tapping the already-ACTIVE segment is a no-op', () => {
    const onToggleProjection = vi.fn()
    render(
      <ViewsSheet open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={onToggleProjection} />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Perspective' }))
    expect(onToggleProjection).not.toHaveBeenCalled()
  })
})

describe('ViewsSheet — orientation fork', () => {
  it('renders a bottom sheet in portrait', () => {
    render(
      <ViewsSheet open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()} />,
    )
    const dialog = screen.getByRole('dialog', { name: 'Views' })
    expect((dialog as HTMLElement).style.left).toBe('0px')
    expect((dialog as HTMLElement).style.width).toBe('')
  })

  it('renders a centered 360px card in landscape', () => {
    render(
      <ViewsSheet open orientation="landscape" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()} />,
    )
    const dialog = screen.getByRole('dialog', { name: 'Views' })
    expect((dialog as HTMLElement).style.width).toBe('360px')
  })
})

describe('ViewsSheet — scrim', () => {
  it('closes on a scrim tap', () => {
    const onClose = vi.fn()
    render(
      <ViewsSheet open orientation="portrait" onClose={onClose} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()} />,
    )
    fireEvent.click(screen.getByTestId('shop-views-scrim'))
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
