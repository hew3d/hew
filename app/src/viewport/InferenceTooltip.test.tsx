import { act, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, it, expect, vi } from 'vitest'
import { HOVER_DETAIL_DWELL_MS, InferenceTooltip } from './InferenceTooltip'
import { setShowToolHints } from '../settings/view'

describe('InferenceTooltip hints setting', () => {
  afterEach(() => setShowToolHints(true))

  it('draws nothing when Show tool hints is off, and comes back when on', () => {
    setShowToolHints(false)
    const { container } = render(<InferenceTooltip info={{ kind: 'endpoint', screenX: 10, screenY: 20 }} />)
    expect(container.firstChild).toBeNull()
    act(() => setShowToolHints(true))
    expect(screen.getByText('Endpoint')).toBeInTheDocument()
  })
})

describe('InferenceTooltip', () => {
  it('renders nothing when info is null', () => {
    const { container } = render(<InferenceTooltip info={null} />)
    expect(container.firstChild).toBeNull()
  })

  it('shows the mapped label for a known kind', () => {
    render(<InferenceTooltip info={{ kind: 'endpoint', screenX: 10, screenY: 20 }} />)
    expect(screen.getByText('Endpoint')).toBeInTheDocument()
  })

  it('shows "On Edge" for the on-edge kind', () => {
    render(<InferenceTooltip info={{ kind: 'on-edge', screenX: 10, screenY: 20 }} />)
    expect(screen.getByText('On Edge')).toBeInTheDocument()
  })

  it('falls back to the raw kind string for an unmapped kind', () => {
    render(<InferenceTooltip info={{ kind: 'mystery-kind', screenX: 10, screenY: 20 }} />)
    expect(screen.getByText('mystery-kind')).toBeInTheDocument()
  })

  it('shows a tool\'s hover hint at once, as its own line, with no dwell', () => {
    render(
      <InferenceTooltip
        info={{ kind: 'on-face', screenX: 10, screenY: 20, hint: 'That face is part of a component — double-click it to edit the component' }}
      />,
    )
    expect(screen.getByText('On Face')).toBeInTheDocument()
    const hint = screen.getByTestId('inference-tip-hint')
    expect(hint).toHaveTextContent('double-click it to edit the component')
  })

  it('draws no hint line when the tool has nothing to explain', () => {
    render(<InferenceTooltip info={{ kind: 'on-face', screenX: 10, screenY: 20 }} />)
    expect(screen.queryByTestId('inference-tip-hint')).toBeNull()
  })

  it('shows an axis label when direction is axis-aligned and kind is not already on-axis', () => {
    render(<InferenceTooltip info={{ kind: 'endpoint', screenX: 10, screenY: 20, direction: [1, 0, 0] }} />)
    expect(screen.getByText('on red axis')).toBeInTheDocument()
  })

  it('does not show a redundant axis label for the on-axis kind itself', () => {
    render(<InferenceTooltip info={{ kind: 'on-axis', screenX: 10, screenY: 20, direction: [0, 1, 0] }} />)
    expect(screen.getByText('On Axis')).toBeInTheDocument()
    expect(screen.queryByText(/on green axis/)).not.toBeInTheDocument()
  })

  it('shows no axis label when direction is not axis-aligned', () => {
    render(<InferenceTooltip info={{ kind: 'endpoint', screenX: 10, screenY: 20, direction: [1, 1, 0] }} />)
    expect(screen.queryByText(/on .* axis/)).not.toBeInTheDocument()
  })

  it('positions the chip at screenX/screenY plus the fixed offset', () => {
    const { container } = render(<InferenceTooltip info={{ kind: 'endpoint', screenX: 100, screenY: 200 }} />)
    const chip = container.firstChild as HTMLElement
    expect(chip.style.left).toBe('116px')
    expect(chip.style.top).toBe('216px')
  })

  it('qualifies a projected snap so the chip stops claiming a point the commit will not use', () => {
    render(<InferenceTooltip info={{ kind: 'endpoint', screenX: 10, screenY: 20, projected: true }} />)
    // The kind still names what was inferred — the qualifier says the gesture
    // took only its x/y, which is what makes it an alignment aid rather than a
    // lie about where the geometry lands.
    expect(screen.getByText('Endpoint')).toBeInTheDocument()
    expect(screen.getByText('projected')).toBeInTheDocument()
  })

  it('says nothing extra when the snap was honoured as-is', () => {
    render(<InferenceTooltip info={{ kind: 'endpoint', screenX: 10, screenY: 20, projected: false }} />)
    expect(screen.queryByText('projected')).toBeNull()
  })

  it('omits the qualifier entirely when the field is absent', () => {
    render(<InferenceTooltip info={{ kind: 'endpoint', screenX: 10, screenY: 20 }} />)
    expect(screen.queryByText('projected')).toBeNull()
  })
})

describe('InferenceTooltip hover detail line', () => {
  beforeEach(() => { vi.useFakeTimers() })
  afterEach(() => { vi.useRealTimers() })

  const at = (object: bigint | undefined, instance?: bigint) =>
    ({ kind: 'on-face', screenX: 10, screenY: 20, object, instance })
  const advance = (ms: number) => act(() => { vi.advanceTimersByTime(ms) })

  it('shows the first line at once and the detail only after the dwell', () => {
    render(<InferenceTooltip info={at(7n)} detail="Door · in Cabinet 2" />)
    expect(screen.getByText('On Face')).toBeInTheDocument()
    expect(screen.queryByTestId('inference-tip-detail')).toBeNull()
    advance(HOVER_DETAIL_DWELL_MS - 1)
    expect(screen.queryByTestId('inference-tip-detail')).toBeNull()
    advance(1)
    expect(screen.getByTestId('inference-tip-detail')).toHaveTextContent('Door · in Cabinet 2')
  })

  it('restarts the dwell when the cursor moves onto a different object', () => {
    const { rerender } = render(<InferenceTooltip info={at(7n)} detail="Door · in Cabinet 2" />)
    advance(300)
    rerender(<InferenceTooltip info={at(8n)} detail="Carcass · in Cabinet 2" />)
    advance(300)
    expect(screen.queryByTestId('inference-tip-detail')).toBeNull()
    advance(100)
    expect(screen.getByTestId('inference-tip-detail')).toHaveTextContent('Carcass · in Cabinet 2')
  })

  it('keeps the line while the cursor stays on the same object and moves', () => {
    const { rerender } = render(<InferenceTooltip info={at(7n)} detail="Door" />)
    advance(HOVER_DETAIL_DWELL_MS)
    rerender(<InferenceTooltip info={{ ...at(7n), screenX: 40 }} detail="Door" />)
    expect(screen.getByTestId('inference-tip-detail')).toBeInTheDocument()
  })

  it('hides the line and restarts when the cursor leaves geometry and returns', () => {
    const { rerender } = render(<InferenceTooltip info={at(7n)} detail="Door" />)
    advance(HOVER_DETAIL_DWELL_MS)
    rerender(<InferenceTooltip info={null} detail={null} />)
    rerender(<InferenceTooltip info={at(7n)} detail="Door" />)
    expect(screen.queryByTestId('inference-tip-detail')).toBeNull()
    advance(HOVER_DETAIL_DWELL_MS)
    expect(screen.getByTestId('inference-tip-detail')).toBeInTheDocument()
  })

  it('shows no line when the snap names no object or no detail was resolved', () => {
    const { rerender } = render(<InferenceTooltip info={at(undefined)} detail="Ghost" />)
    advance(HOVER_DETAIL_DWELL_MS * 2)
    expect(screen.queryByTestId('inference-tip-detail')).toBeNull()
    rerender(<InferenceTooltip info={at(7n)} detail={null} />)
    advance(HOVER_DETAIL_DWELL_MS * 2)
    expect(screen.queryByTestId('inference-tip-detail')).toBeNull()
  })
})
