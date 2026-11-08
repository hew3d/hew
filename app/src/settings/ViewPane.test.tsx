import { render, screen, fireEvent, act } from '@testing-library/react'
import { describe, expect, it, beforeEach } from 'vitest'
import { ViewPane } from './ViewPane'
import { SettingsWindow } from './SettingsWindow'
import {
  getZoomWheelStop, setZoomWheelStop, getSmoothCamera, setSmoothCamera,
  getShowToolHints, setShowToolHints, getShowToolPointer, setShowToolPointer,
  getMeasurementsPosition, setMeasurementsPosition,
} from './view'

beforeEach(() => {
  setZoomWheelStop(2)
  setSmoothCamera(true)
  setShowToolHints(true)
  setShowToolPointer(true)
  setMeasurementsPosition('top-right')
})

describe('ViewPane', () => {
  it('sits right after Units in the Settings window', () => {
    render(<SettingsWindow />)
    expect(screen.getAllByRole('tab').map((t) => t.textContent).slice(0, 4)).toEqual(['Units', 'View', 'Editing', 'Theme'])
    fireEvent.click(screen.getByRole('tab', { name: 'View' }))
    expect(screen.getByText('Zoom wheel speed')).toBeInTheDocument()
  })

  it('shows today\'s behavior by default', () => {
    render(<ViewPane />)
    expect(screen.getByRole('slider')).toHaveValue('2')
    expect(screen.getByRole('checkbox', { name: 'Smooth camera moves' })).toBeChecked()
    expect(screen.getByRole('checkbox', { name: 'Show tool hints' })).toBeChecked()
    expect(screen.getByRole('checkbox', { name: 'Show tool pointer' })).toBeChecked()
    expect(screen.getByRole('radio', { name: 'Top right' })).toBeChecked()
  })

  it('the slider sets the wheel-speed stop', () => {
    render(<ViewPane />)
    fireEvent.change(screen.getByRole('slider'), { target: { value: '4' } })
    expect(getZoomWheelStop()).toBe(4)
    expect(localStorage.getItem('hew.settings.zoomWheelSpeed')).toBe('4')
  })

  it('the switches set their settings', () => {
    render(<ViewPane />)
    fireEvent.click(screen.getByRole('checkbox', { name: 'Smooth camera moves' }))
    fireEvent.click(screen.getByRole('checkbox', { name: 'Show tool hints' }))
    fireEvent.click(screen.getByRole('checkbox', { name: 'Show tool pointer' }))
    expect(getSmoothCamera()).toBe(false)
    expect(getShowToolHints()).toBe(false)
    expect(getShowToolPointer()).toBe(false)
  })

  it('the radio sets the Measurements box position', () => {
    render(<ViewPane />)
    fireEvent.click(screen.getByRole('radio', { name: 'Bottom right' }))
    expect(getMeasurementsPosition()).toBe('bottom-right')
    expect(screen.getByRole('radio', { name: 'Bottom right' })).toBeChecked()
  })

  it('follows a change made in another window', () => {
    render(<ViewPane />)
    act(() => {
      window.dispatchEvent(new StorageEvent('storage', { key: 'hew.settings.showToolHints', newValue: 'false' }))
    })
    expect(screen.getByRole('checkbox', { name: 'Show tool hints' })).not.toBeChecked()
  })
})
