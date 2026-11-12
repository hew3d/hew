import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { MEASUREMENT_BOX_TOP_PX, MeasurementBox, measurementBoxBottomPx } from './MeasurementBox'

describe('MeasurementBox', () => {
  it('renders nothing when value is empty', () => {
    const { container } = render(<MeasurementBox toolName="Move" value="" />)
    expect(container.firstChild).toBeNull()
  })

  it('shows the value when non-empty', () => {
    render(<MeasurementBox toolName="Move" value={"8' 0\""} />)
    expect(screen.getByText(/8' 0"/)).toBeInTheDocument()
  })

  it('labels "Distance" for Move', () => {
    render(<MeasurementBox toolName="Move" value="1m" />)
    expect(screen.getByText('Distance')).toBeInTheDocument()
  })

  it('labels "Push depth" for Push/Pull', () => {
    render(<MeasurementBox toolName="Push/Pull" value="1m" />)
    expect(screen.getByText('Push depth')).toBeInTheDocument()
  })

  it('labels "Angle" for Rotate and Protractor', () => {
    render(<MeasurementBox toolName="Rotate" value="45°" />)
    expect(screen.getByText('Angle')).toBeInTheDocument()
  })

  it('falls back to "Value" for an unmapped tool', () => {
    render(<MeasurementBox toolName="Select" value="something" />)
    expect(screen.getByText('Value')).toBeInTheDocument()
  })

  // tape-measure-rework part 1: the caret's presence means "a typed buffer
  // is live and Enter will act on it"; its absence on a non-empty value
  // means "a finished reading, kept on screen for reference".
  it('shows the blinking caret by default (frozen omitted/false)', () => {
    const { container } = render(<MeasurementBox toolName="Tape Measure" value="1m" />)
    expect(container.querySelector('.hew-vcb-caret')).not.toBeNull()
  })

  it('hides the caret when frozen — a finished reading, not a live buffer', () => {
    const { container } = render(<MeasurementBox toolName="Tape Measure" value="1m" frozen />)
    expect(container.querySelector('.hew-vcb-caret')).toBeNull()
    expect(screen.getByText('1m')).toBeInTheDocument()
  })

  // Shop-mode playtest finding 4: the editor's top-right docking sits
  // directly under Shop Mode's own ⋯ menu button in both orientations.
  describe('variant="shop" (finding 4)', () => {
    it('defaults to the editor placement/style when omitted — byte-identical to before this prop existed', () => {
      const { container } = render(<MeasurementBox toolName="Move" value="1m" />)
      const root = container.firstChild as HTMLElement
      expect(root.style.bottom).toBe('16px')
      expect(root.style.right).toBe('16px')
      expect(root.style.left).toBe('')
    })

    // Playtest fix 5 (maintainer's own words): "default to the lower right
    // in portrait mode, and in the same horizontal space at the top along
    // with the two menus in landscape mode" — the prior unconditional
    // top-center spot could hide behind the centered magnifier loupe.
    it('defaults to a lower-right dock in portrait, clear of the safe-area insets', () => {
      const { container } = render(<MeasurementBox toolName="Tape Measure" value="1m" variant="shop" />)
      const root = container.firstChild as HTMLElement
      // Docked lower-right, not top-center.
      expect(root.style.left).toBe('')
      expect(root.style.top).toBe('')
      // jsdom's CSSOM re-serializes `env(x, fallback)` with its own
      // (slightly mangled but content-preserving) spacing/punctuation, so
      // this checks for the safe-area token surviving rather than the exact
      // `env(safe-area-inset-right` substring a real browser would keep.
      expect(root.style.right).toContain('safe-area-inset-right')
      expect(root.style.bottom).toContain('safe-area-inset-bottom')
      // Shop chrome's charcoal pill family, not the editor's control surface.
      expect(root.style.background).toBe('var(--shop-dock)')
    })

    it('clears the dock/Parts-sheet height in portrait via bottomOffsetPx', () => {
      const { container: containerA } = render(<MeasurementBox toolName="Tape Measure" value="1m" variant="shop" bottomOffsetPx={0} />)
      const { container: containerB } = render(<MeasurementBox toolName="Tape Measure" value="1m" variant="shop" bottomOffsetPx={130} />)
      const rootA = containerA.firstChild as HTMLElement
      const rootB = containerB.firstChild as HTMLElement
      // A relative comparison rather than asserting the exact generated
      // `calc()` string — jsdom's CSSOM algebraically folds the constant
      // terms in a `calc()` expression during serialization, so the literal
      // "130" substring isn't guaranteed to survive verbatim the way it
      // would in a real browser's computed style.
      expect(rootB.style.bottom).not.toBe(rootA.style.bottom)
    })

    // Task 5: the prior top-center spot blocked measurements taken above
    // the screen centerline — moved to lower-LEFT instead, mirroring
    // portrait's own lower-right corner onto the opposite edge so it clears
    // the right rail's tools.
    it('moves to a lower-LEFT position in landscape, clear of the right rail', () => {
      const { container } = render(<MeasurementBox toolName="Tape Measure" value="1m" variant="shop" orientation="landscape" />)
      const root = container.firstChild as HTMLElement
      // Docked lower-left, not top-center or right/bottom-docked.
      expect(root.style.top).toBe('')
      expect(root.style.right).toBe('')
      expect(root.style.transform).toBe('')
      // jsdom's CSSOM re-serializes `env(x, fallback)` with its own
      // (slightly mangled but content-preserving) spacing/punctuation, so
      // this checks for the safe-area token surviving rather than the exact
      // `env(safe-area-inset-left` substring a real browser would keep.
      expect(root.style.left).toContain('safe-area-inset-left')
      expect(root.style.bottom).toContain('safe-area-inset-bottom')
      // Shop chrome's charcoal pill family, not the editor's control surface.
      expect(root.style.background).toBe('var(--shop-dock)')
    })

    it('still shows the value and respects frozen in the shop variant', () => {
      const value = `3' 2"`
      const { container } = render(<MeasurementBox toolName="Tape Measure" value={value} frozen variant="shop" />)
      expect(screen.getByText(/3' 2"/)).toBeInTheDocument()
      expect(container.querySelector('.hew-vcb-caret')).toBeNull()
    })
  })

  it('docks bottom-right by default and under the die stack when top-right is asked', () => {
    const { rerender } = render(<MeasurementBox toolName="Move" value="1m" />)
    const bottom = screen.getByTestId('measurement-box')
    expect(bottom.style.bottom).toBe('16px')
    expect(bottom.style.top).toBe('')
    expect(bottom.style.right).toBe('16px')
    rerender(<MeasurementBox toolName="Move" value="1m" position="top-right" />)
    const top = screen.getByTestId('measurement-box')
    // Below the die (16 + 76), its 6 px gap and 16 px pips, then a 10 px gap.
    expect(top.style.top).toBe(`${MEASUREMENT_BOX_TOP_PX}px`)
    expect(MEASUREMENT_BOX_TOP_PX).toBe(124)
    expect(top.style.bottom).toBe('')
    expect(top.style.right).toBe('16px')
  })

  describe('measurementBoxBottomPx (lifts above the contextual dock)', () => {
    const container = { left: 0, right: 1000, top: 0, bottom: 700 }
    // A 200 x 30 box at its normal spot, bottom-right.
    const box = { left: 784, right: 984, top: 654, bottom: 684 }
    it('stays at 16 px with no dock', () => {
      expect(measurementBoxBottomPx(box, null, container)).toBe(16)
    })
    it('stays at 16 px when 12 px or more clear of the dock', () => {
      const dock = { left: 300, right: 772, top: 630, bottom: 682 }
      expect(measurementBoxBottomPx(box, dock, container)).toBe(16)
    })
    it('lifts to 12 px above the dock when the gap is under 12 px', () => {
      const dock = { left: 300, right: 780, top: 630, bottom: 682 }
      expect(measurementBoxBottomPx(box, dock, container)).toBe(700 - 630 + 12)
    })
    it('lifts when the dock overlaps the box sideways', () => {
      const dock = { left: 400, right: 900, top: 630, bottom: 682 }
      expect(measurementBoxBottomPx(box, dock, container)).toBe(82)
    })
  })

  it('applies the lift measured against a real dock element', () => {
    const rect = (r: { left: number; right: number; top: number; bottom: number }) => () =>
      ({ ...r, x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top, toJSON: () => ({}) }) as DOMRect
    const host = document.createElement('div')
    document.body.appendChild(host)
    host.style.position = 'relative'
    host.getBoundingClientRect = rect({ left: 0, right: 1000, top: 0, bottom: 700 })
    const dock = document.createElement('div')
    dock.setAttribute('data-testid', 'contextual-dock')
    dock.getBoundingClientRect = rect({ left: 300, right: 790, top: 630, bottom: 682 })
    host.appendChild(dock)
    const slot = document.createElement('div')
    host.appendChild(slot)
    const proto = HTMLElement.prototype
    const orig = Object.getOwnPropertyDescriptor(proto, 'offsetParent')
    Object.defineProperty(proto, 'offsetParent', { configurable: true, get: () => host })
    const origRect = Element.prototype.getBoundingClientRect
    Element.prototype.getBoundingClientRect = function (this: Element) {
      if (this.getAttribute('data-testid') !== 'measurement-box') return origRect.call(this)
      // Like a real layout, the box moves up as its `bottom` grows.
      const lifted = parseFloat((this as HTMLElement).style.bottom || '16') - 16
      return rect({ left: 784, right: 984, top: 654 - lifted, bottom: 684 - lifted })()
    }
    try {
      render(<MeasurementBox toolName="Move" value="1m" />, { container: slot })
      expect(screen.getByTestId('measurement-box').style.bottom).toBe('82px')
    } finally {
      Element.prototype.getBoundingClientRect = origRect
      if (orig) Object.defineProperty(proto, 'offsetParent', orig)
      else delete (proto as unknown as Record<string, unknown>).offsetParent
      host.remove()
    }
  })
})
