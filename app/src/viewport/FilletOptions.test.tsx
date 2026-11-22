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
    expect((screen.getByRole('button', { name: 'Save profile…' }) as HTMLButtonElement).disabled).toBe(false)
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
    fireEvent.click(screen.getByRole('button', { name: 'Save profile…' }))
    expect(onSave).toHaveBeenCalledTimes(1)
    fireEvent.click(screen.getByRole('button', { name: /Ogee/ }))
    expect((screen.getByRole('button', { name: 'Nope' }) as HTMLButtonElement).disabled).toBe(true)
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
    const save = screen.getByRole('button', { name: 'Save profile…' }) as HTMLButtonElement
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
    expect(row.querySelector('img')).toHaveAttribute('width', '24')
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

it('navigates built-in profiles, skipping disabled items and restoring trigger focus', () => {
  const curve = { kind: 'shape' as const, id: 'builtin:curve', name: 'Curve', shape: {} }
  const other = { ...curve, id: 'builtin:other', name: 'Other' }
  const save = vi.fn()
  render(<FilletOptions sections={[
    { id: 'builtin', items: [
      { id: 'curve', label: 'Curve', profile: curve },
      { id: 'off', label: 'Unavailable', profile: curve, disabled: true },
      { id: 'other', label: 'Other', profile: other },
    ] },
    { id: 'actions', items: [{ id: 'save-profile', label: 'Save profile…', onSelect: save }] },
  ]} />)
  const trigger = screen.getByRole('button', { name: 'More shapes' })
  fireEvent.keyDown(trigger, { key: 'ArrowDown' })
  const first = screen.getByRole('menuitemradio', { name: 'Curve' })
  const last = screen.getByRole('menuitemradio', { name: 'Other' })
  expect(first).toHaveFocus()
  fireEvent.keyDown(first, { key: 'ArrowDown' })
  expect(last).toHaveFocus()
  fireEvent.keyDown(last, { key: 'Home' })
  expect(first).toHaveFocus()
  fireEvent.keyDown(first, { key: 'End' })
  expect(last).toHaveFocus()
  fireEvent.keyDown(last, { key: 'Escape' })
  expect(screen.queryByRole('menu')).toBeNull()
  expect(trigger).toHaveFocus()
  fireEvent.keyDown(trigger, { key: 'ArrowUp' })
  expect(screen.getByRole('menuitemradio', { name: 'Other' })).toHaveFocus()
  fireEvent.click(screen.getByRole('menuitemradio', { name: 'Curve' }), { detail: 0 })
  expect(getFilletSettings().profile).toEqual(curve)
  expect(trigger).toHaveFocus()
  expect(screen.queryByRole('menu')).toBeNull()
  expect(save).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: 'Save profile…' }))
  expect(save).toHaveBeenCalledOnce()
})

it('keeps closing content mounted but inaccessible, and can reopen without losing profile identity', () => {
  const profile = { kind: 'shape' as const, id: 'builtin:curve', name: 'A long built-in profile name', shape: {} }
  const { container } = render(<FilletOptions sections={[{ id: 'builtin', heading: 'Built-in', items: [
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

it('keeps Built-in and My profiles in More Shapes while saving stays below the main settings', () => {
  const profile = { kind: 'shape' as const, id: 'saved', name: 'Saved profile', shape: {} }
  const builtIn = { ...profile, id: 'builtin:ogee', name: 'Ogee' }
  const save = vi.fn()
  setFilletThreeWayCorners(1)
  const { container } = render(<FilletOptions sections={[
    { id: 'builtin', heading: 'Built-in', items: [{ id: 'ogee', label: 'Ogee', profile: builtIn }] },
    { id: 'library', heading: 'My profiles', items: [{ id: 'saved', label: profile.name, profile }] },
    { id: 'actions', items: [{ id: 'save-profile', label: 'Save profile…', onSelect: save }] },
  ]} />)
  const more = screen.getByRole('button', { name: /^More shapes/ })
  const before = (a: Element, b: Element) => expect(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  for (const open of [false, true]) {
    if (open) fireEvent.click(more)
    const primary = screen.getByRole('group', { name: 'Quick edge shape' })
    const facets = screen.getByRole('textbox', { name: 'Facets' })
    const corners = screen.getByRole('radiogroup', { name: 'Corners' })
    const saveButton = screen.getByRole('button', { name: 'Save profile…' })
    before(more, primary); before(primary, facets); before(facets, corners); before(corners, saveButton)
    expect(more.nextElementSibling).toBe(container.querySelector('.fillet-profile-expansion'))
    expect(more).toHaveAttribute('aria-expanded', String(open))
    expect(screen.getAllByRole('button', { name: 'Save profile…' })).toHaveLength(1)
    if (open) {
      const menu = screen.getByRole('menu')
      const builtinGroup = screen.getByRole('group', { name: 'Built-in' })
      const savedGroup = screen.getByRole('group', { name: 'My profiles' })
      before(more, menu); before(menu, primary); before(builtinGroup, savedGroup)
      expect(menu.contains(savedGroup)).toBe(true)
      expect(menu.contains(saveButton)).toBe(false)
      expect(screen.getAllByRole('menuitemradio', { name: 'Saved profile' })).toHaveLength(1)
    } else {
      expect(screen.queryByRole('menu')).toBeNull()
      expect(screen.queryByRole('group', { name: 'My profiles' })).toBeNull()
    }
    for (const name of ['Round', 'Chamfer', 'Cove', 'Bullnose']) {
      expect(screen.getByRole('button', { name }).querySelector('svg')).toHaveAttribute('data-profile-kind', name.toLowerCase())
    }
    expect(screen.getByRole('radio', { name: 'Rolled' })).toBeTruthy()
    expect(screen.getByRole('radio', { name: 'Mitered' })).toBeTruthy()
  }
  const saved = screen.getByRole('menuitemradio', { name: 'Saved profile' })
  fireEvent.keyDown(screen.getByRole('menuitemradio', { name: 'Ogee' }), { key: 'ArrowDown' })
  expect(saved).toHaveFocus()
  fireEvent.click(saved, { detail: 0 })
  expect(getFilletSettings().profile).toEqual(profile)
  expect(more).toHaveFocus()
  expect(screen.queryByRole('menu')).toBeNull()
  expect(screen.queryByRole('group', { name: 'My profiles' })).toBeNull()
  fireEvent.click(more)
  expect(screen.getByRole('menuitemradio', { name: 'Saved profile' })).toHaveAttribute('aria-checked', 'true')
  fireEvent.click(screen.getByRole('button', { name: 'Chamfer' }))
  expect(screen.getByRole('radio', { name: 'Flat' })).toBeTruthy()
  expect(screen.queryByRole('radio', { name: 'Rolled' })).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: 'Save profile…' }))
  expect(save).toHaveBeenCalledOnce()
})

it('displays the canonical built-in name without renaming a saved profile', () => {
  expect(profileLabel({ kind: 'shape', id: 'builtin:roman-ogee', name: 'Roman ogee', shape: {} })).toBe('Roman Ogee')
  expect(profileLabel({ kind: 'shape', id: 'saved:mine', name: 'Roman ogee', shape: {} })).toBe('Roman ogee')
})


it('dismisses the menu when interacting with the main settings above it', () => {
  render(<FilletOptions />)
  fireEvent.click(screen.getByRole('button', { name: /^More shapes/ }))
  const field = screen.getByRole('textbox', { name: 'Facets' })
  fireEvent.pointerDown(field)
  expect(screen.queryByRole('menu')).toBeNull()
  fireEvent.focus(field)
  fireEvent.change(field, { target: { value: '12' } })
  expect(getFilletSettings().segmentsOverride).toBe(12)
})
