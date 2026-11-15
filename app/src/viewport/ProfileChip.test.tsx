import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CornersChip, cornerChoices } from './CornersChip'
import { ProfileChip, profileLabel, profileSizeLabel } from './ProfileChip'
import {
  getFilletSettings,
  resetFilletSettingsForTest,
  setFilletCornerOnly,
  setFilletProfile,
  setFilletThreeWayCorners,
} from '../tools/filletSettings'

beforeEach(() => {
  resetFilletSettingsForTest()
  setFilletThreeWayCorners(0)
})
afterEach(() => {
  setFilletThreeWayCorners(0)
})

describe('ProfileChip', () => {
  it('names the current profile and lists the four basic kinds', () => {
    render(<ProfileChip />)
    expect(screen.getByRole('button', { name: /^Edge/ })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /^Edge/ }))
    for (const name of ['Round', 'Chamfer', 'Cove', 'Bullnose']) {
      expect(screen.getByRole('menuitemradio', { name })).toBeTruthy()
    }
  })

  it('has Round and Chamfer pills that set the profile in one click', () => {
    render(<ProfileChip />)
    const round = screen.getByRole('button', { name: 'Round' }) as HTMLButtonElement
    const chamfer = screen.getByRole('button', { name: 'Chamfer' }) as HTMLButtonElement
    expect(round.getAttribute('aria-pressed')).toBe('true')
    expect(chamfer.getAttribute('aria-pressed')).toBe('false')
    fireEvent.click(chamfer)
    expect(getFilletSettings().profile.kind).toBe('chamfer')
    expect(chamfer.getAttribute('aria-pressed')).toBe('true')
    expect(round.getAttribute('aria-pressed')).toBe('false')
    fireEvent.click(round)
    expect(getFilletSettings().profile.kind).toBe('round')
    // The menu keeps every entry.
    fireEvent.click(screen.getByRole('button', { name: /^Edge/ }))
    expect(screen.getAllByRole('menuitemradio')).toHaveLength(4)
  })

  it('the pills stay enabled over a drawn corner, and neither is pressed for Cove', () => {
    render(<ProfileChip />)
    act(() => setFilletCornerOnly(true))
    expect((screen.getByRole('button', { name: 'Round' }) as HTMLButtonElement).disabled).toBe(false)
    expect((screen.getByRole('button', { name: 'Chamfer' }) as HTMLButtonElement).disabled).toBe(false)
    act(() => {
      setFilletCornerOnly(false)
      setFilletProfile({ kind: 'cove' })
    })
    expect(screen.getByRole('button', { name: 'Round' }).getAttribute('aria-pressed')).toBe('false')
    expect(screen.getByRole('button', { name: 'Chamfer' }).getAttribute('aria-pressed')).toBe('false')
  })

  it('over a drawn corner, only Round and Chamfer can be picked', () => {
    const ogee = { kind: 'shape' as const, id: 'builtin:ogee', name: 'Ogee', shape: { b: 1, segs: [] } }
    const onSave = vi.fn()
    render(
      <ProfileChip
        sections={[
          { id: 'builtin', items: [{ id: 'ogee', label: 'Ogee', profile: ogee }] },
          { id: 'actions', items: [{ id: 'save', label: 'Save profile…', onSelect: onSave }] },
        ]}
      />,
    )
    act(() => setFilletCornerOnly(true))
    fireEvent.click(screen.getByRole('button', { name: /^Edge/ }))
    for (const name of ['Cove', 'Bullnose', 'Ogee']) {
      const row = screen.getByRole('menuitemradio', { name }) as HTMLButtonElement
      expect(row.disabled).toBe(true)
      expect(row.title).toBe('Drawn corners can be rounded or chamfered.')
    }
    expect((screen.getByRole('menuitem', { name: 'Save profile…' }) as HTMLButtonElement).disabled).toBe(false)
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Cove' }))
    expect(getFilletSettings().profile.kind).toBe('round')
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Chamfer' }))
    expect(getFilletSettings().profile.kind).toBe('chamfer')
    act(() => setFilletCornerOnly(false))
    fireEvent.click(screen.getByRole('button', { name: /^Edge/ }))
    expect((screen.getByRole('menuitemradio', { name: 'Cove' }) as HTMLButtonElement).disabled).toBe(false)
  })

  it('picking a kind sets the session profile and closes the menu', () => {
    render(<ProfileChip />)
    fireEvent.click(screen.getByRole('button', { name: /^Edge/ }))
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Cove' }))
    expect(getFilletSettings().profile.kind).toBe('cove')
    expect(screen.queryByRole('menu')).toBeNull()
    expect(screen.getByRole('button', { name: /Cove/ })).toBeTruthy()
  })

  it('shows extra sections with badges, action rows and disabled rows', () => {
    const onSave = vi.fn()
    render(
      <ProfileChip
        sections={[
          {
            id: 'builtin',
            heading: 'Built-in',
            items: [
              {
                id: 'ogee',
                label: 'Ogee',
                badge: 'Built-in',
                profile: { kind: 'shape', id: 'builtin:ogee', name: 'Ogee', shape: { b: 1, segs: [] } },
              },
            ],
          },
          { id: 'actions', items: [{ id: 'save', label: 'Save profile…', onSelect: onSave }, { id: 'off', label: 'Nope', disabled: true, onSelect: vi.fn() }] },
        ]}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /^Edge/ }))
    fireEvent.click(screen.getByRole('menuitemradio', { name: /Ogee/ }))
    expect(getFilletSettings().profile).toMatchObject({ kind: 'shape', id: 'builtin:ogee' })
    fireEvent.click(screen.getByRole('button', { name: /Ogee/ }))
    fireEvent.click(screen.getByRole('menuitem', { name: 'Save profile…' }))
    expect(onSave).toHaveBeenCalledTimes(1)
    fireEvent.click(screen.getByRole('button', { name: /Ogee/ }))
    expect((screen.getByRole('menuitem', { name: 'Nope' }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('a disabled row carries its reason, and an empty section shows its hint', () => {
    render(
      <ProfileChip
        sections={[
          { id: 'library', heading: 'My profiles', items: [], emptyHint: 'No saved profiles yet.' },
          { id: 'hidden', heading: 'Nothing', items: [] },
          {
            id: 'actions',
            items: [{ id: 'save', label: 'Save profile…', disabled: true, title: 'Select a chain first.', onSelect: vi.fn() }],
          },
        ]}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /^Edge/ }))
    expect(screen.getByText('No saved profiles yet.')).toBeTruthy()
    expect(screen.queryByText('Nothing')).toBeNull()
    const save = screen.getByRole('menuitem', { name: 'Save profile…' }) as HTMLButtonElement
    expect(save.disabled).toBe(true)
    expect(save.title).toBe('Select a chain first.')
  })

  it('labels the size by profile family', () => {
    expect(profileLabel({ kind: 'bullnose' })).toBe('Bullnose')
    expect(profileSizeLabel({ kind: 'round' })).toBe('Radius')
    expect(profileSizeLabel({ kind: 'cove' })).toBe('Radius')
    expect(profileSizeLabel({ kind: 'chamfer' })).toBe('Distance')
    expect(profileSizeLabel({ kind: 'shape', id: 'x', name: 'X', shape: {} })).toBe('Width')
  })
})

describe('CornersChip', () => {
  it('is hidden until the armed pick has a three-way corner', () => {
    render(<CornersChip />)
    expect(screen.queryByTestId('fillet-corners-chip')).toBeNull()
    act(() => setFilletThreeWayCorners(2))
    expect(screen.getByTestId('fillet-corners-chip')).toBeTruthy()
  })

  it('offers Rolled | Mitered for a round and Mitered | Flat for a chamfer', () => {
    expect(cornerChoices(false).map((c) => c.label)).toEqual(['Rolled', 'Mitered'])
    expect(cornerChoices(true).map((c) => c.label)).toEqual(['Mitered', 'Flat'])
    render(<CornersChip />)
    act(() => setFilletThreeWayCorners(1))
    fireEvent.click(screen.getByRole('radio', { name: 'Mitered' }))
    expect(getFilletSettings().corners).toBe('mitered')
    act(() => setFilletProfile({ kind: 'chamfer' }))
    // Switching family resets the corner style; the chamfer pair shows.
    expect(getFilletSettings().corners).toBe('default')
    fireEvent.click(screen.getByRole('radio', { name: 'Flat' }))
    expect(getFilletSettings().corners).toBe('flat')
  })
})
