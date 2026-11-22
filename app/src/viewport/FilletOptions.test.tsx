import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ToolOptionsCard } from './ToolOptionsCard'
import { FilletOptions, cornerChoices, profileLabel } from './FilletOptions'
import {
  getFilletSettings,
  profileSizeLabel,
  setFilletSegmentsOverride,
  resetFilletSettingsForTest,
  setFilletCornerOnly,
  setFilletProfile,
  setFilletThreeWayCorners,
  subscribeFilletSettings,
} from '../tools/filletSettings'

beforeEach(() => {
  resetFilletSettingsForTest()
  setFilletThreeWayCorners(0)
})
afterEach(() => {
  setFilletThreeWayCorners(0)
})

describe('FilletOptions', () => {
  it('offers four quick choices without duplicating them in More shapes', () => {
    render(<FilletOptions />)
    expect(screen.getByRole('button', { name: /^More shapes/ })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /^More shapes/ }))
    for (const name of ['Round', 'Chamfer', 'Cove', 'Bullnose']) {
      expect(screen.getByRole('button', { name })).toBeTruthy()
      expect(screen.queryByRole('menuitemradio', { name })).toBeNull()
    }
  })

  it('has Round and Chamfer pills that set the profile in one click', () => {
    render(<FilletOptions />)
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
    // All four basic choices remain reachable directly while the additional menu is open.
    fireEvent.click(screen.getByRole('button', { name: /^More shapes/ }))
    expect(screen.getByRole('group', { name: 'Quick edge shape' }).querySelectorAll('button')).toHaveLength(4)
  })

  it('the pills stay enabled over a drawn corner, and neither is pressed for Cove', () => {
    render(<FilletOptions />)
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
      <FilletOptions
        sections={[
          { id: 'builtin', items: [{ id: 'ogee', label: 'Ogee', profile: ogee }] },
          { id: 'actions', items: [{ id: 'save', label: 'Save profile…', onSelect: onSave }] },
        ]}
      />,
    )
    act(() => setFilletCornerOnly(true))
    fireEvent.click(screen.getByRole('button', { name: /^More shapes/ }))
    for (const name of ['Cove', 'Bullnose', 'Ogee']) {
      const row = screen.getByRole(name === 'Ogee' ? 'menuitemradio' : 'button', { name }) as HTMLButtonElement
      expect(row.disabled).toBe(true)
      expect(row.title).toBe('Drawn corners can be rounded or chamfered.')
    }
    expect((screen.getByRole('menuitem', { name: 'Save profile…' }) as HTMLButtonElement).disabled).toBe(false)
    fireEvent.click(screen.getByRole('button', { name: 'Cove' }))
    expect(getFilletSettings().profile.kind).toBe('round')
    fireEvent.click(screen.getByRole('button', { name: 'Chamfer' }))
    expect(getFilletSettings().profile.kind).toBe('chamfer')
    act(() => setFilletCornerOnly(false))
    fireEvent.click(screen.getByRole('button', { name: /^More shapes/ }))
    expect((screen.getByRole('button', { name: 'Cove' }) as HTMLButtonElement).disabled).toBe(false)
  })

  it('picking a kind sets the session profile and closes the menu', () => {
    render(<FilletOptions />)
    fireEvent.click(screen.getByRole('button', { name: /^More shapes/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Cove' }))
    expect(getFilletSettings().profile.kind).toBe('cove')
    expect(screen.queryByRole('menu')).toBeNull()
    expect(screen.getByRole('button', { name: /^More shapes/ })).toBeTruthy()
  })

  it('shows extra sections with badges, action rows and disabled rows', () => {
    const onSave = vi.fn()
    render(
      <FilletOptions
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
    fireEvent.click(screen.getByRole('button', { name: /^More shapes/ }))
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
      <FilletOptions
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
    fireEvent.click(screen.getByRole('button', { name: /^More shapes/ }))
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

describe('FilletOptions corners', () => {
  it('is hidden until the armed pick has a three-way corner', () => {
    render(<FilletOptions />)
    expect(screen.queryByRole('radiogroup', { name: 'Corners' })).toBeNull()
    act(() => setFilletThreeWayCorners(2))
    expect(screen.getByRole('radiogroup', { name: 'Corners' })).toBeTruthy()
  })

  it('offers Rolled | Mitered for a round and Mitered | Flat for a chamfer', () => {
    expect(cornerChoices(false).map((c) => c.label)).toEqual(['Rolled', 'Mitered'])
    expect(cornerChoices(true).map((c) => c.label)).toEqual(['Mitered', 'Flat'])
    render(<FilletOptions />)
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

describe('FilletOptions rows and field editing', () => {
  const facets = () => screen.getByRole('textbox', { name: 'Facets' }) as HTMLInputElement

  it('shows Facets for every curved shape and hides it for Chamfer; external changes swap rows', () => {
    render(<FilletOptions />)
    for (const profile of [{ kind: 'round' as const }, { kind: 'cove' as const }, { kind: 'bullnose' as const },
      { kind: 'shape' as const, id: 'x', name: 'X', shape: {} }]) {
      act(() => setFilletProfile(profile))
      expect(facets()).toBeVisible()
    }
    act(() => setFilletProfile({ kind: 'chamfer' }))
    expect(screen.queryByRole('textbox', { name: 'Facets' })).toBeNull()
    act(() => setFilletProfile({ kind: 'round' }))
    expect(facets()).toHaveAttribute('placeholder', 'Auto')
  })

  it('applies valid values live, finalizes on blur/Enter, and clearing means Auto', () => {
    render(<FilletOptions />)
    const field = facets()
    fireEvent.focus(field)
    fireEvent.change(field, { target: { value: '12' } })
    expect(getFilletSettings().segmentsOverride).toBe(12)
    fireEvent.blur(field)
    expect(field.value).toBe('12')
    // Use real focus so Enter's blur also fires React's onBlur handler.
    act(() => field.focus())
    fireEvent.change(field, { target: { value: '' } })
    expect(getFilletSettings().segmentsOverride).toBeNull()
    fireEvent.keyDown(field, { key: 'Enter' })
    expect(field.value).toBe('')
    expect(document.activeElement).not.toBe(field)
    act(() => setFilletSegmentsOverride(24))
    expect(field.value).toBe('24')
  })

  it.each([
    [false, '1', 2, 'blur'], [false, '999', 96, 'blur'],
    [true, '0', 1, 'blur'], [true, '9999', 1024, 'blur'],
    [false, '1', 2, 'Enter'], [false, '999', 96, 'Enter'],
    [true, '0', 1, 'Enter'], [true, '9999', 1024, 'Enter'],
  ])('defers clamping until commit (corner=%s, input=%s, expected=%s, finish=%s)', (corner, text, expected, commit) => {
    setFilletCornerOnly(corner)
    setFilletSegmentsOverride(8)
    render(<FilletOptions />)
    act(() => facets().focus())
    fireEvent.change(facets(), { target: { value: text } })
    expect(getFilletSettings().segmentsOverride).toBe(8)
    expect(facets().value).toBe(text)
    if (commit === 'Enter') fireEvent.keyDown(facets(), { key: 'Enter' })
    else fireEvent.blur(facets())
    expect(getFilletSettings().segmentsOverride).toBe(expected)
    expect(facets().value).toBe(String(expected))
  })

  it('keeps intermediate 1 out of the preview, then applies 12 without waiting for blur', () => {
    setFilletSegmentsOverride(8)
    render(<FilletOptions />)
    act(() => facets().focus())
    const update = vi.fn()
    const unsubscribe = subscribeFilletSettings(update)
    try {
      fireEvent.change(facets(), { target: { value: '1' } })
      expect(facets().value).toBe('1')
      expect(getFilletSettings().segmentsOverride).toBe(8)
      expect(update).not.toHaveBeenCalled()
      fireEvent.change(facets(), { target: { value: '12' } })
      expect(getFilletSettings().segmentsOverride).toBe(12)
      expect(update).toHaveBeenCalledTimes(1)
      // A later out-of-range or incomplete draft holds the last valid preview.
      fireEvent.change(facets(), { target: { value: '120' } })
      fireEvent.change(facets(), { target: { value: '-' } })
      expect(getFilletSettings().segmentsOverride).toBe(12)
      expect(update).toHaveBeenCalledTimes(1)
    } finally {
      unsubscribe()
    }
  })

  it.each([[false, 2], [false, 96], [true, 1], [true, 1024]])('previews an in-range boundary (corner=%s, count=%s)', (corner, count) => {
    setFilletCornerOnly(corner)
    render(<FilletOptions />)
    fireEvent.focus(facets())
    fireEvent.change(facets(), { target: { value: String(count) } })
    expect(getFilletSettings().segmentsOverride).toBe(count)
  })

  it('clearing selects Auto immediately and an out-of-range Escape restores Auto', () => {
    render(<FilletOptions />)
    act(() => facets().focus())
    fireEvent.change(facets(), { target: { value: '999' } })
    expect(getFilletSettings().segmentsOverride).toBeNull()
    fireEvent.keyDown(facets(), { key: 'Escape' })
    expect(facets().value).toBe('')
    expect(getFilletSettings().segmentsOverride).toBeNull()
    act(() => setFilletSegmentsOverride(8))
    act(() => facets().focus())
    fireEvent.change(facets(), { target: { value: '' } })
    expect(getFilletSettings().segmentsOverride).toBeNull()
    fireEvent.keyDown(facets(), { key: 'Escape' })
    expect(getFilletSettings().segmentsOverride).toBe(8)
  })

  it.each(['-', '3.5', '12s', 'Auto', '1e2'])('reverts invalid/incomplete input %s on blur', (text) => {
    setFilletSegmentsOverride(8)
    render(<FilletOptions />)
    fireEvent.focus(facets())
    fireEvent.change(facets(), { target: { value: '16' } })
    expect(getFilletSettings().segmentsOverride).toBe(16)
    fireEvent.change(facets(), { target: { value: text } })
    fireEvent.blur(facets())
    expect(getFilletSettings().segmentsOverride).toBe(8)
    expect(facets().value).toBe('8')
  })

  it('reverts an incomplete value on Enter instead of committing the last live preview', () => {
    setFilletSegmentsOverride(8)
    render(<FilletOptions />)
    act(() => facets().focus())
    fireEvent.change(facets(), { target: { value: '12' } })
    expect(getFilletSettings().segmentsOverride).toBe(12)
    fireEvent.change(facets(), { target: { value: '+' } })
    fireEvent.keyDown(facets(), { key: 'Enter' })
    expect(getFilletSettings().segmentsOverride).toBe(8)
    expect(facets().value).toBe('8')
  })

  it('uses the approved Facets help text verbatim', () => {
    render(<FilletOptions />)
    fireEvent.click(screen.getByRole('button', { name: 'About Facets' }))
    expect(screen.getByRole('note').textContent).toBe('Choose how many flat faces make the curve, or leave it on Auto and Hew picks.')
  })

  it('Escape restores the setting from focus, stops propagation, and releases focus', () => {
    setFilletSegmentsOverride(8)
    const outer = vi.fn()
    render(<div onKeyDown={outer}><FilletOptions /></div>)
    const field = facets()
    act(() => field.focus())
    fireEvent.change(field, { target: { value: '18' } })
    fireEvent.keyDown(field, { key: 'Escape' })
    expect(getFilletSettings().segmentsOverride).toBe(8)
    expect(field.value).toBe('8')
    expect(document.activeElement).not.toBe(field)
    expect(outer).not.toHaveBeenCalled()
  })

  it('does not accept an IME-confirming Enter as a command', () => {
    render(<FilletOptions />)
    const field = facets()
    act(() => field.focus())
    fireEvent.keyDown(field, { key: 'Enter', keyCode: 229 })
    expect(document.activeElement).toBe(field)
  })

  it('offers all four quick shapes and restores menu dismissal on outside press and Escape', () => {
    render(<FilletOptions />)
    for (const name of ['Round', 'Chamfer', 'Cove', 'Bullnose']) {
      fireEvent.click(screen.getByRole('button', { name }))
      expect(screen.getByRole('button', { name })).toHaveAttribute('aria-pressed', 'true')
    }
    const more = screen.getByRole('button', { name: /^More shapes/ })
    fireEvent.click(more)
    fireEvent.pointerDown(document.body)
    expect(screen.queryByRole('menu')).toBeNull()
    fireEvent.click(more)
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('keeps Library thumbnails, selected identity, and one-sentence row help', () => {
    const profile = { kind: 'shape' as const, id: 'mine', name: 'My curve', shape: {} }
    setFilletProfile(profile)
    render(<FilletOptions sections={[{ id: 'library', heading: 'My profiles', items: [
      { id: 'mine', label: 'My curve', profile, thumbnail: 'data:image/png;base64,AA==' },
    ] }]} />)
    fireEvent.click(screen.getByRole('button', { name: /^More shapes/ }))
    const row = screen.getByRole('menuitemradio', { name: 'My curve' })
    expect(row).toHaveAttribute('aria-checked', 'true')
    expect(row.querySelector('img')).toHaveAttribute('width', '32')
    fireEvent.click(screen.getByRole('button', { name: 'About Shape' }))
    expect(screen.getByRole('note')).toHaveTextContent('Choose the shape of the edge.')
  })
})

it('Facets Escape owns its rollback even if More Shapes was opened from the keyboard', () => {
  setFilletSegmentsOverride(8)
  render(<FilletOptions />)
  fireEvent.click(screen.getByRole('button', { name: /^More shapes/ }))
  const field = screen.getByRole('textbox', { name: 'Facets' })
  act(() => field.focus())
  fireEvent.change(field, { target: { value: '16' } })
  fireEvent.keyDown(field, { key: 'Escape' })
  expect(getFilletSettings().segmentsOverride).toBe(8)
  expect(field).toHaveValue('8')
  expect(document.activeElement).not.toBe(field)
})


it('closing the card also closes its menu, including keyboard-only collapse', () => {
  render(<ToolOptionsCard tool="Menu collapse" info="Help."><FilletOptions /></ToolOptionsCard>)
  fireEvent.click(screen.getByRole('button', { name: /^More shapes/ }))
  expect(screen.getByRole('menu')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Collapse Menu collapse options' }))
  fireEvent.click(screen.getByRole('button', { name: 'Expand Menu collapse options' }))
  expect(screen.queryByRole('menu')).toBeNull()
})

it('navigates additional profiles and actions, skipping disabled items and restoring trigger focus', () => {
  const profile = { kind: 'shape' as const, id: 'curve', name: 'Curve', shape: {} }
  const save = vi.fn()
  render(<FilletOptions sections={[{ id: 'library', items: [
    { id: 'curve', label: 'Curve', profile },
    { id: 'off', label: 'Unavailable', profile, disabled: true },
    { id: 'save', label: 'Save profile…', onSelect: save },
  ] }]} />)
  const trigger = screen.getByRole('button', { name: 'More shapes' })
  fireEvent.keyDown(trigger, { key: 'ArrowDown' })
  const curve = screen.getByRole('menuitemradio', { name: 'Curve' })
  const action = screen.getByRole('menuitem', { name: 'Save profile…' })
  expect(curve).toHaveFocus()
  fireEvent.keyDown(curve, { key: 'ArrowDown' })
  expect(action).toHaveFocus()
  fireEvent.keyDown(action, { key: 'Home' })
  expect(curve).toHaveFocus()
  fireEvent.keyDown(curve, { key: 'End' })
  expect(action).toHaveFocus()
  fireEvent.keyDown(action, { key: 'Escape' })
  expect(screen.queryByRole('menu')).toBeNull()
  expect(trigger).toHaveFocus()
  fireEvent.keyDown(trigger, { key: 'ArrowUp' })
  expect(screen.getByRole('menuitem', { name: 'Save profile…' })).toHaveFocus()
  fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Save profile…' }), { key: 'Home' })
  fireEvent.click(screen.getByRole('menuitemradio', { name: 'Curve' }), { detail: 0 })
  expect(getFilletSettings().profile).toEqual(profile)
  expect(trigger).toHaveFocus()
  expect(screen.queryByRole('menu')).toBeNull()
  expect(save).not.toHaveBeenCalled()
})

it('keeps closing content mounted but inaccessible, and can reopen without losing profile identity', () => {
  const profile = { kind: 'shape' as const, id: 'saved', name: 'A long saved profile name', shape: {} }
  const { container } = render(<FilletOptions sections={[{ id: 'saved', heading: 'My profiles', items: [
    { id: 'saved', label: profile.name, profile },
  ] }]} />)
  const trigger = screen.getByRole('button', { name: 'More shapes' })
  const expansion = container.querySelector('.fillet-profile-expansion')!
  expect(expansion).toHaveAttribute('inert')
  fireEvent.keyDown(trigger, { key: 'ArrowDown' })
  const row = screen.getByRole('menuitemradio', { name: profile.name })
  expect(row).toHaveFocus()
  expect(trigger.getAttribute('aria-controls')).toBe(screen.getByRole('menu').id)
  fireEvent.click(row, { detail: 0 })
  expect(expansion).toHaveAttribute('data-open', 'false')
  expect(expansion).toHaveAttribute('aria-hidden', 'true')
  expect(expansion).toHaveAttribute('inert')
  expect(container.querySelector('[role="menuitemradio"]')).toBe(row)
  expect(screen.queryByRole('menu')).toBeNull()
  expect(trigger).toHaveFocus()
  fireEvent.keyDown(trigger, { key: 'ArrowDown' })
  expect(screen.getByRole('menuitemradio', { name: profile.name })).toBe(row)
  expect(row).toHaveAttribute('aria-checked', 'true')
  fireEvent.keyDown(row, { key: 'Tab' })
  expect(screen.queryByRole('menu')).toBeNull()
})

it('unmounting an open menu removes its Escape listener', () => {
  const { unmount } = render(<FilletOptions />)
  fireEvent.click(screen.getByRole('button', { name: 'More shapes' }))
  unmount()
  const event = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true, bubbles: true })
  window.dispatchEvent(event)
  expect(event.defaultPrevented).toBe(false)
})
