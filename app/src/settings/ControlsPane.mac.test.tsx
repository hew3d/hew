import { render, screen, fireEvent, within, act } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

// The macOS desktop: the native Edit items carry fixed accelerators.
vi.mock('../platform', async (orig) => ({ ...(await orig<typeof import('../platform')>()), isMac: true }))
vi.mock('../io/fileHost', async (orig) => ({ ...(await orig<typeof import('../io/fileHost')>()), isTauri: true }))

import { ControlsPane } from './ControlsPane'
import { chordsFor, commandForChord, resetAllChords, setChords } from '../keymap/keymap'

beforeEach(() => resetAllChords())

function row(name: string): HTMLElement {
  return screen.getByRole('listitem', { name })
}

function press(init: KeyboardEventInit): void {
  act(() => {
    window.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }))
  })
}

function listen(name: string): void {
  fireEvent.click(within(row(name)).getByRole('button', { name: `Add a shortcut to ${name}` }))
}

describe('ControlsPane on the macOS desktop', () => {
  it('refuses a pinned native Edit chord for another command', () => {
    render(<ControlsPane />)
    listen('Dimension')
    press({ key: 'z', code: 'KeyZ', metaKey: true })
    expect(within(row('Dimension')).getByText('⌘Z stays with Undo on this Mac and can’t be a shortcut.')).toBeInTheDocument()
    expect(chordsFor('tool-dimension')).toEqual([])
    expect(commandForChord('Meta+Z')).toBe('undo')
  })

  it('refuses a pinned Shift chord too', () => {
    render(<ControlsPane />)
    listen('Dimension')
    press({ key: 'a', code: 'KeyA', metaKey: true, shiftKey: true })
    expect(within(row('Dimension')).getByText('⇧⌘A stays with Select None on this Mac and can’t be a shortcut.')).toBeInTheDocument()
  })

  it('refuses Cmd+Q, Cmd+H and Cmd+M as macOS system chords', () => {
    render(<ControlsPane />)
    for (const [key, code] of [['q', 'KeyQ'], ['h', 'KeyH'], ['m', 'KeyM']]) {
      listen('Dimension')
      press({ key, code, metaKey: true })
      expect(
        within(row('Dimension')).getByText(`⌘${key.toUpperCase()} is reserved by macOS and can’t be a shortcut.`),
      ).toBeInTheDocument()
      press({ key: 'Escape', code: 'Escape' })
    }
    expect(chordsFor('tool-dimension')).toEqual([])
  })

  it('does not offer to remove the pinned chord, but removes the extras and keeps Reset', () => {
    render(<ControlsPane />)
    fireEvent.mouseEnter(row('Undo'))
    expect(within(row('Undo')).queryByRole('button', { name: 'Remove ⌘Z from Undo' })).toBeNull()
    expect(within(row('Undo')).getByText('⌘Z')).toBeInTheDocument()

    act(() => setChords('undo', ['Meta+Z', 'Meta+Shift+U']))
    fireEvent.mouseEnter(row('Undo'))
    fireEvent.click(within(row('Undo')).getByRole('button', { name: 'Remove ⇧⌘U from Undo' }))
    expect(chordsFor('undo')).toEqual(['Meta+Z'])

    act(() => setChords('undo', ['Meta+Z', 'Meta+Shift+U']))
    fireEvent.click(within(row('Undo')).getByRole('button', { name: 'Reset' }))
    expect(chordsFor('undo')).toEqual(['Meta+Z'])
  })

  it('still allows an unpinned Cmd chord', () => {
    render(<ControlsPane />)
    listen('Dimension')
    press({ key: 'u', code: 'KeyU', metaKey: true })
    expect(chordsFor('tool-dimension')).toEqual(['Meta+U'])
  })
})
