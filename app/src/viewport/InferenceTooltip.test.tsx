import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { InferenceTooltip } from './InferenceTooltip'

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
})
