import { render, screen, fireEvent, within, act } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { ControlsPane } from './ControlsPane'
import { SettingsWindow } from './SettingsWindow'
import { chordsFor, commandForChord, overrides, resetAllChords, setChords, isRecordingChord } from '../keymap/keymap'

beforeEach(() => resetAllChords())

function row(name: string): HTMLElement {
  return screen.getByRole('listitem', { name })
}

function press(init: KeyboardEventInit): void {
  act(() => {
    window.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }))
  })
}

describe('ControlsPane', () => {
  it('is the Settings window tab after View', () => {
    render(<SettingsWindow />)
    fireEvent.click(screen.getByRole('tab', { name: 'Controls' }))
    expect(screen.getByRole('list', { name: 'Keyboard shortcuts' })).toBeInTheDocument()
  })

  it('lists tools and actions with their keys', () => {
    render(<ControlsPane />)
    expect(within(row('Rectangle')).getByText('R')).toBeInTheDocument()
    expect(within(row('Rectangle')).getByText('2')).toBeInTheDocument()
    expect(within(row('Save')).getByText('Ctrl+S')).toBeInTheDocument()
    expect(row('Zoom Extents')).toBeInTheDocument()
    // Keyboard-only commands the palette does not list.
    expect(row('Command Palette…')).toBeInTheDocument()
  })

  it('filters with the palette\'s search, synonyms included', () => {
    render(<ControlsPane />)
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search commands' }), { target: { value: 'extrude' } })
    expect(row('Push/Pull')).toBeInTheDocument()
    expect(screen.queryByRole('listitem', { name: 'Rectangle' })).toBeNull()
  })

  it('records a new key for a command', () => {
    render(<ControlsPane />)
    fireEvent.click(within(row('Dimension')).getByRole('button', { name: 'Add a shortcut to Dimension' }))
    expect(within(row('Dimension')).getByText('Press a key…')).toBeInTheDocument()
    expect(isRecordingChord()).toBe(true)
    press({ key: 'Shift', code: 'ShiftLeft', shiftKey: true }) // a modifier alone keeps listening
    press({ key: 'd', code: 'KeyD' })
    expect(chordsFor('tool-dimension')).toEqual(['D'])
    expect(within(row('Dimension')).getByText('D')).toBeInTheDocument()
    expect(within(row('Dimension')).getByLabelText('Changed from the default')).toBeInTheDocument()
    expect(isRecordingChord()).toBe(false)
  })

  it('keeps the recorded key away from every other listener', () => {
    let reached = false
    const spy = () => { reached = true }
    window.addEventListener('keydown', spy)
    render(<ControlsPane />)
    fireEvent.click(within(row('Dimension')).getByRole('button', { name: 'Add a shortcut to Dimension' }))
    press({ key: 'd', code: 'KeyD' })
    window.removeEventListener('keydown', spy)
    expect(reached).toBe(false)
  })

  it('Esc cancels the recording', () => {
    render(<ControlsPane />)
    fireEvent.click(within(row('Dimension')).getByRole('button', { name: 'Add a shortcut to Dimension' }))
    press({ key: 'Escape', code: 'Escape' })
    expect(chordsFor('tool-dimension')).toEqual([])
    expect(within(row('Dimension')).queryByText('Press a key…')).toBeNull()
  })

  it('asks before taking a key another command uses', () => {
    render(<ControlsPane />)
    fireEvent.click(within(row('Dimension')).getByRole('button', { name: 'Add a shortcut to Dimension' }))
    press({ key: 'o', code: 'KeyO' })
    expect(within(row('Dimension')).getByText('O is used by Orbit.')).toBeInTheDocument()
    expect(commandForChord('O')).toBe('tool-orbit')
    fireEvent.click(within(row('Dimension')).getByRole('button', { name: 'Reassign' }))
    expect(commandForChord('O')).toBe('tool-dimension')
    expect(chordsFor('tool-orbit')).toEqual([])
  })

  it('Cancel on a conflict changes nothing', () => {
    render(<ControlsPane />)
    fireEvent.click(within(row('Dimension')).getByRole('button', { name: 'Add a shortcut to Dimension' }))
    press({ key: 'o', code: 'KeyO' })
    fireEvent.click(within(row('Dimension')).getByRole('button', { name: 'Cancel' }))
    expect(commandForChord('O')).toBe('tool-orbit')
    expect(overrides()).toEqual({})
  })

  it('refuses the tools\' gesture keys', () => {
    render(<ControlsPane />)
    fireEvent.click(within(row('Dimension')).getByRole('button', { name: 'Add a shortcut to Dimension' }))
    press({ key: 'ArrowUp', code: 'ArrowUp' })
    expect(within(row('Dimension')).getByText('↑ belongs to the tools and can’t be a shortcut.')).toBeInTheDocument()
    expect(chordsFor('tool-dimension')).toEqual([])
  })

  it('removes a key, resets a command, resets everything', () => {
    render(<ControlsPane />)
    // A chip's × shows on hover (or focus inside the row).
    expect(within(row('Rectangle')).queryByRole('button', { name: 'Remove 2 from Rectangle' })).toBeNull()
    fireEvent.mouseEnter(row('Rectangle'))
    fireEvent.click(within(row('Rectangle')).getByRole('button', { name: 'Remove 2 from Rectangle' }))
    expect(chordsFor('tool-rectangle')).toEqual(['R'])
    fireEvent.click(within(row('Rectangle')).getByRole('button', { name: 'Reset' }))
    expect(chordsFor('tool-rectangle')).toEqual(['R', '2'])

    act(() => setChords('tool-orbit', ['Ctrl+H']))
    const resetAll = screen.getByRole('button', { name: 'Reset All' })
    expect(resetAll).toBeEnabled()
    fireEvent.click(resetAll)
    expect(overrides()).toEqual({})
    expect(screen.getByRole('button', { name: 'Reset All' })).toBeDisabled()
  })
})
