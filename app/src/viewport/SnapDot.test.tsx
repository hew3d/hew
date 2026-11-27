import { render } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { PLANE_RING_HELD_PX, PLANE_RING_PX, SnapDot, snapDotShadow } from './SnapDot'

describe('SnapDot — the plane ring (view-die §4)', () => {
  it('a 2 px ring in the plane\'s axis color while the plane is inferred', () => {
    expect(PLANE_RING_PX).toBe(2)
    expect(snapDotShadow({ kind: 'plane', screenX: 0, screenY: 0, plane: { pip: 'green', hold: 'inferred' } }, '#888'))
      .toBe('0 0 0 2px var(--axis-green)')
  })

  it('3.5 px when the plane is held: locked, pinned or anchored', () => {
    expect(PLANE_RING_HELD_PX).toBe(3.5)
    for (const hold of ['locked', 'pinned', 'anchored'] as const) {
      expect(snapDotShadow({ kind: 'endpoint', screenX: 0, screenY: 0, plane: { pip: 'blue', hold } }, '#888'))
        .toBe('0 0 0 3.5px var(--axis-blue)')
    }
  })

  it('the custom plane rings in the violet token', () => {
    expect(snapDotShadow({ kind: 'on-face', screenX: 0, screenY: 0, plane: { pip: 'custom', hold: 'inferred' } }, '#888'))
      .toContain('--plane-custom')
  })

  it('full alpha: the token itself, never a color-mix fade', () => {
    expect(snapDotShadow({ kind: 'plane', screenX: 0, screenY: 0, plane: { pip: 'red', hold: 'inferred' } }, '#888')).not.toContain('color-mix')
  })

  it('without a drawing plane, the soft halo in the snap\'s own color, as before', () => {
    expect(snapDotShadow({ kind: 'endpoint', screenX: 0, screenY: 0 }, '#00cc44'))
      .toBe('0 0 0 3px color-mix(in srgb, #00cc44 22%, transparent)')
  })

  it('renders the ring on the dot', () => {
    const { container } = render(<SnapDot info={{ kind: 'plane', screenX: 5, screenY: 6, plane: { pip: 'red', hold: 'locked' } }} />)
    const dot = container.firstChild as HTMLElement
    expect(dot.style.boxShadow).toContain('3.5px')
  })
})
