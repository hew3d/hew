/**
 * InspectCard tests — the two shapes (design_handoff_shop_mode/README.md
 * §3): PART (name + tag chip + L/W/H mono dims line, axis-colored letters)
 * and EDGE (owning part's name + "edge" chip, never bare "Edge" + the
 * measurement). Renders through real DOM (jsdom), asserting on text content
 * and the axis-letter colors rather than inline-style pixel values —
 * `ShopApp.test.tsx` already covers the wiring (tap → card → dismiss); this
 * file is purely about what the two shapes render.
 */
import { act, render, screen } from '@testing-library/react'
import { describe, expect, it, beforeEach } from 'vitest'
import { InspectCard } from './InspectCard'
import { setLengthUnit } from '../settings/units'
import type { InspectResult } from './inspect'
import type { NodeRef } from '../panels/treeModel'

const OBJECT_NODE: NodeRef = { kind: 'object', id: 1n }

beforeEach(() => {
  setLengthUnit('m')
})

function renderCard(result: InspectResult, leaving = false) {
  return render(
    <InspectCard
      result={result}
      leaving={leaving}
      screenX={100}
      screenY={200}
      containerWidth={390}
      containerHeight={844}
    />,
  )
}

describe('InspectCard — PART shape', () => {
  const PART_RESULT: InspectResult = {
    kind: 'node',
    node: OBJECT_NODE,
    label: 'Tabletop',
    tagLabel: 'Structure',
    extentsM: [0.6, 0.4, 0.02],
  }

  it('renders the part name, tag chip, and the L/W/H dims line with formatted values', () => {
    renderCard(PART_RESULT)
    expect(screen.getByText('Tabletop')).toBeInTheDocument()
    expect(screen.getByText('Structure')).toBeInTheDocument()
    expect(screen.getByText('L')).toBeInTheDocument()
    expect(screen.getByText('W')).toBeInTheDocument()
    expect(screen.getByText('H')).toBeInTheDocument()
    // The dims line mixes span-wrapped letters with bare formatted-value text
    // nodes (mirrors PartsSheet.tsx's own imperial stacked-row markup) — read
    // the line's own combined text rather than querying each value as its
    // own element.
    const dimsLine = screen.getByText('L').parentElement as HTMLElement
    expect(dimsLine.textContent).toBe('L 0.6 m W 0.4 m H 0.02 m')
  })

  it('gives the L/W/H letters distinct axis colors (design: "colored letters")', () => {
    renderCard(PART_RESULT)
    const l = screen.getByText('L')
    const w = screen.getByText('W')
    const h = screen.getByText('H')
    const lColor = l.style.color
    const wColor = w.style.color
    const hColor = h.style.color
    expect(lColor).toBe('var(--shop-axis-l)')
    expect(wColor).toBe('var(--shop-axis-w)')
    expect(hColor).toBe('var(--shop-axis-h)')
    expect(new Set([lColor, wColor, hColor]).size).toBe(3)
  })

  it('omits the tag chip when the part has no tag', () => {
    renderCard({ ...PART_RESULT, tagLabel: null })
    expect(screen.queryByText('Structure')).not.toBeInTheDocument()
  })

  it('re-renders dims live when the length unit changes after mount', () => {
    renderCard(PART_RESULT)
    const dimsLine = () => screen.getByText('L').parentElement as HTMLElement
    expect(dimsLine().textContent).toBe('L 0.6 m W 0.4 m H 0.02 m')
    act(() => setLengthUnit('cm'))
    expect(dimsLine().textContent).toBe('L 60 cm W 40 cm H 2 cm')
  })
})

describe('InspectCard — EDGE shape', () => {
  const EDGE_RESULT: InspectResult = { kind: 'edge', lengthM: 0.09, partLabel: 'Pen Cup' }

  it('titles the card with the OWNING PART\'s name plus a small "edge" chip — never bare "Edge"', () => {
    renderCard(EDGE_RESULT)
    expect(screen.getByText('Pen Cup')).toBeInTheDocument()
    expect(screen.getByText('edge')).toBeInTheDocument()
    expect(screen.queryByText('Edge')).not.toBeInTheDocument()
  })

  it('renders the formatted length as the big measurement', () => {
    renderCard(EDGE_RESULT)
    expect(screen.getByText('0.09 m')).toBeInTheDocument()
  })

  it('never renders L/W/H dims for an edge result', () => {
    renderCard(EDGE_RESULT)
    expect(screen.queryByText('L')).not.toBeInTheDocument()
    expect(screen.queryByText('W')).not.toBeInTheDocument()
    expect(screen.queryByText('H')).not.toBeInTheDocument()
  })
})

describe('InspectCard — motion (leaving prop)', () => {
  it('applies the entrance class by default and the exit class while leaving', () => {
    const { container, rerender } = renderCard({ kind: 'edge', lengthM: 1, partLabel: 'Leg' })
    expect(container.querySelector('.shop-inspect-in')).not.toBeNull()
    expect(container.querySelector('.shop-inspect-out')).toBeNull()

    rerender(
      <InspectCard result={{ kind: 'edge', lengthM: 1, partLabel: 'Leg' }} leaving screenX={100} screenY={200} containerWidth={390} containerHeight={844} />,
    )
    expect(container.querySelector('.shop-inspect-out')).not.toBeNull()
  })
})
