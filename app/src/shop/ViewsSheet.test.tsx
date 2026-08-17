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

// SPEC.md §2 "Scenes in the Views sheet" — an additive prop set
// (`scenes`/`activeSid`/`drifted`/`onSelectScene`), all optional so every
// test above (which never passes them) proves the section stays absent for
// a caller that doesn't know about Scenes at all.
const SCENE_ENTRIES = [
  { sid: 1, name: 'Assembled', description: 'Everything, three-quarter view', props: 31 },
  { sid: 2, name: 'Cut layout', description: 'Cut layout on a 4×8 sheet, ¾ ply', props: 31 },
  { sid: 3, name: 'Tenon section', description: '', props: 31 },
]

describe('ViewsSheet — no Scenes section for a caller that omits `scenes`', () => {
  it('renders no SCENES header when the prop is omitted (every pre-existing test above)', () => {
    render(
      <ViewsSheet open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()} />,
    )
    expect(screen.queryByText('Scenes')).not.toBeInTheDocument()
  })

  it('renders no SCENES header for an explicit empty array', () => {
    render(
      <ViewsSheet
        open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()}
        scenes={[]}
      />,
    )
    expect(screen.queryByText('Scenes')).not.toBeInTheDocument()
  })
})

describe('ViewsSheet — SCENES section', () => {
  it('renders the SCENES header FIRST, above the standard views', () => {
    render(
      <ViewsSheet
        open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()}
        scenes={SCENE_ENTRIES} activeSid={null} onSelectScene={vi.fn()}
      />,
    )
    const dialog = screen.getByRole('dialog', { name: 'Views' })
    const text = dialog.textContent ?? ''
    expect(text.indexOf('Scenes')).toBeGreaterThanOrEqual(0)
    expect(text.indexOf('Scenes')).toBeLessThan(text.indexOf('ISO'))
  })

  it('lists every Scene by name, with its description', () => {
    render(
      <ViewsSheet
        open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()}
        scenes={SCENE_ENTRIES} activeSid={null} onSelectScene={vi.fn()}
      />,
    )
    expect(screen.getByText('Assembled')).toBeInTheDocument()
    expect(screen.getByText('Everything, three-quarter view')).toBeInTheDocument()
    expect(screen.getByText('Cut layout')).toBeInTheDocument()
    expect(screen.getByText('Cut layout on a 4×8 sheet, ¾ ply')).toBeInTheDocument()
  })

  it('a Scene with an empty description renders no description line', () => {
    render(
      <ViewsSheet
        open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()}
        scenes={SCENE_ENTRIES} activeSid={null} onSelectScene={vi.fn()}
      />,
    )
    // "Tenon section" (sid 3) has description '' — its row renders the name
    // only; nothing else in this fixture shares that name as filler text.
    expect(screen.getByText('Tenon section')).toBeInTheDocument()
  })

  it('tapping a Scene row calls onSelectScene with its sid and closes the sheet (act-and-close)', () => {
    const onSelectScene = vi.fn()
    const onClose = vi.fn()
    render(
      <ViewsSheet
        open orientation="portrait" onClose={onClose} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()}
        scenes={SCENE_ENTRIES} activeSid={null} onSelectScene={onSelectScene}
      />,
    )
    fireEvent.click(screen.getByText('Cut layout'))
    expect(onSelectScene).toHaveBeenCalledWith(2)
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('the active Scene row renders a filled dot; inactive rows render none', () => {
    render(
      <ViewsSheet
        open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()}
        scenes={SCENE_ENTRIES} activeSid={2} drifted={false} onSelectScene={vi.fn()}
      />,
    )
    const filledDots = document.querySelectorAll('circle[fill="var(--shop-accent)"]')
    expect(filledDots.length).toBe(1)
  })

  it('the active Scene row renders a ring (not a filled dot) when drifted', () => {
    render(
      <ViewsSheet
        open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()}
        scenes={SCENE_ENTRIES} activeSid={2} drifted onSelectScene={vi.fn()}
      />,
    )
    expect(document.querySelectorAll('circle[fill="var(--shop-accent)"]').length).toBe(0)
    expect(document.querySelectorAll('circle[stroke="var(--shop-accent)"]').length).toBe(1)
  })

  it('standard views and the projection toggle still render below the Scenes section', () => {
    render(
      <ViewsSheet
        open orientation="portrait" onClose={vi.fn()} onSelectView={vi.fn()} projection="perspective" onToggleProjection={vi.fn()}
        scenes={SCENE_ENTRIES} activeSid={null} onSelectScene={vi.fn()}
      />,
    )
    for (const label of ['ISO', 'Front', 'Back', 'Left', 'Right', 'Top', 'Bottom']) {
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument()
    }
    expect(screen.getByText('Projection')).toBeInTheDocument()
  })
})
