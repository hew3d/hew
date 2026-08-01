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
})
