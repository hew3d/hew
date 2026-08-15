import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { MeasurementBox } from './MeasurementBox'

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
      expect(root.style.top).toBe('16px')
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
})
