/**
 * ScenePill tests (SPEC.md §2 "Active-Scene pill") — hidden when no Scene
 * is active, name/dot/ring rendering, chevron wrap (delegated — the
 * component just calls its own `onPrevious`/`onNext`, `ShopApp.test.tsx`'s
 * own "Scenes" describe block covers the real `neighborScene` wrap-around
 * math), and tapping the name opening the Views sheet.
 */
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ScenePill } from './ScenePill'
import type { SceneEntry } from '../scenes/scenesModel'

const ENTRY: SceneEntry = { sid: 1, name: 'Cut layout', description: '', props: 31 }

function renderPill(overrides: Partial<Parameters<typeof ScenePill>[0]> = {}) {
  const props = {
    entry: ENTRY,
    drifted: false,
    onPrevious: vi.fn(),
    onNext: vi.fn(),
    onOpenName: vi.fn(),
    placement: { kind: 'bottom' as const, bottomPx: 100 },
    ...overrides,
  }
  render(<ScenePill {...props} />)
  return props
}

describe('ScenePill — hidden when no Scene is active', () => {
  it('renders nothing when entry is null', () => {
    const { container } = render(
      <ScenePill entry={null} drifted={false} onPrevious={vi.fn()} onNext={vi.fn()} onOpenName={vi.fn()} placement={{ kind: 'bottom', bottomPx: 100 }} />,
    )
    expect(container).toBeEmptyDOMElement()
  })
})

describe('ScenePill — active Scene', () => {
  it('shows the Scene name', () => {
    renderPill()
    expect(screen.getByText('Cut layout')).toBeInTheDocument()
  })

  it('has Previous/Next Scene chevrons with the SPEC aria-labels', () => {
    renderPill()
    expect(screen.getByRole('button', { name: 'Previous Scene' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Next Scene' })).toBeInTheDocument()
  })

  it('tapping Previous Scene calls onPrevious', () => {
    const onPrevious = vi.fn()
    renderPill({ onPrevious })
    fireEvent.click(screen.getByRole('button', { name: 'Previous Scene' }))
    expect(onPrevious).toHaveBeenCalledTimes(1)
  })

  it('tapping Next Scene calls onNext', () => {
    const onNext = vi.fn()
    renderPill({ onNext })
    fireEvent.click(screen.getByRole('button', { name: 'Next Scene' }))
    expect(onNext).toHaveBeenCalledTimes(1)
  })

  it('tapping the name opens the Views sheet (onOpenName)', () => {
    const onOpenName = vi.fn()
    renderPill({ onOpenName })
    fireEvent.click(screen.getByText('Cut layout'))
    expect(onOpenName).toHaveBeenCalledTimes(1)
  })

  it('long names truncate — the visible text is still the full name (ellipsis is CSS-only)', () => {
    renderPill({ entry: { sid: 2, name: 'Section through the leg joint, close-up', description: '', props: 31 } })
    expect(screen.getByText('Section through the leg joint, close-up')).toBeInTheDocument()
  })
})

describe('ScenePill — drift (SPEC.md §2 "Drift & Show all")', () => {
  it('not drifted: renders a filled dot (a circle with a fill, no stroke ring)', () => {
    renderPill({ drifted: false })
    const filled = document.querySelector('circle[fill="var(--shop-accent)"]')
    expect(filled).not.toBeNull()
    expect(document.querySelector('circle[stroke="var(--shop-accent)"]')).toBeNull()
  })

  it('drifted: renders a ring (stroked, transparent fill) instead of a filled dot', () => {
    renderPill({ drifted: true })
    const ring = document.querySelector('circle[stroke="var(--shop-accent)"]')
    expect(ring).not.toBeNull()
    expect(ring).toHaveAttribute('fill', 'none')
    expect(document.querySelector('circle[fill="var(--shop-accent)"]')).toBeNull()
  })

  it('drifted: the name drops to 80% opacity', () => {
    renderPill({ drifted: true })
    const name = screen.getByText('Cut layout')
    expect(name).toHaveStyle({ opacity: '0.8' })
  })

  it('not drifted: the name is fully opaque', () => {
    renderPill({ drifted: false })
    const name = screen.getByText('Cut layout')
    expect(name).toHaveStyle({ opacity: '1' })
  })
})

describe('ScenePill — placement (playtest round 1)', () => {
  it('portrait sits 12px above the dock/sheet stack it is given', () => {
    renderPill({ placement: { kind: 'bottom', bottomPx: 150 } })
    const wrap = screen.getByTestId('scene-pill')
    expect(wrap).toHaveAttribute('data-placement', 'bottom')
    expect(wrap.style.bottom).toBe('162px')
    expect(wrap.style.top).toBe('')
  })
  it('landscape sits on the top strip row', () => {
    renderPill({ placement: { kind: 'top', topCss: '20px' } })
    const wrap = screen.getByTestId('scene-pill')
    expect(wrap).toHaveAttribute('data-placement', 'top')
    expect(wrap.style.top).toBe('20px')
    expect(wrap.style.bottom).toBe('')
  })
})
