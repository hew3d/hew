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
})
