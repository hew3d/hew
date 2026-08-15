/**
 * HintOverlay tests — presentational coverage for the two ghost-affordance
 * hint variants (`hints.ts`'s `ActiveHint`), each drawn by a different
 * branch of this component. `ShopApp.test.tsx`'s own tests only ever
 * exercise 'tap' incidentally (via a live `HintEngine`, stubbed
 * `worldToScreen`); this file is the dedicated coverage for the component
 * itself, including the 'orbit' variant nothing else in the suite renders,
 * plus the null-dot case neither hint's own engine test reaches (that's
 * pure state-machine logic, not rendering).
 */
import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { HintOverlay } from './HintOverlay'

describe('HintOverlay — (a) tap', () => {
  it('renders the pulsing dot + "Tap a part for its size" tag at the given screen position', () => {
    const { container, getByText } = render(
      <HintOverlay
        hint={{ name: 'tap', targetPartId: '1' }}
        dotScreen={{ x: 120, y: 240 }}
        containerWidth={390}
        containerHeight={844}
      />,
    )
    expect(getByText('Tap a part for its size')).toBeInTheDocument()
    // The halo ring — the one animated piece; also the exact class
    // index.css's `prefers-reduced-motion` block turns static (that CSS
    // file's own module doc) — asserting the class name here is what
    // proves this component hooked into that mechanism rather than
    // animating some other, unguarded element.
    expect(container.querySelector('.shop-hint-tap-ring')).not.toBeNull()
  })

  it('renders nothing without a dot position (off-screen target or mid camera-drag)', () => {
    const { container } = render(
      <HintOverlay
        hint={{ name: 'tap', targetPartId: '1' }}
        dotScreen={null}
        containerWidth={390}
        containerHeight={844}
      />,
    )
    expect(container).toBeEmptyDOMElement()
  })
})

describe('HintOverlay — (b) orbit', () => {
  it('renders the dashed arc + ghost-finger + "Drag to look around" tag', () => {
    const { container, getByText } = render(
      <HintOverlay
        hint={{ name: 'orbit' }}
        dotScreen={null}
        containerWidth={390}
        containerHeight={844}
      />,
    )
    expect(getByText('Drag to look around')).toBeInTheDocument()
    // The traveling fingertip — the class the reduced-motion block pins
    // static mid-arc instead of animating (index.css's module doc).
    expect(container.querySelector('.shop-hint-orbit-finger')).not.toBeNull()
  })
})
