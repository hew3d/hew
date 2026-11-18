import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { FilletDimensionControl } from './FilletDimensionControl'
import { connectFilletDimension, disconnectFilletDimension, publishFilletDimension, type DimensionSnapshot } from '../tools/fillet/dimensionStore'
import { setLengthUnit } from '../settings/units'

function connect(overrides: Partial<DimensionSnapshot> = {}) {
  let snapshot: DimensionSnapshot = { identity: 'operation:radius', grip: { kind: 'radius' }, profile: { kind: 'round' }, label: 'Radius', value: 0.02, requested: 0.02, limit: null, reason: null, ...overrides }
  const gesture = { update: vi.fn(), finish: vi.fn(), cancel: vi.fn() }
  const source = { snapshot: () => snapshot, begin: vi.fn(() => gesture) }
  connectFilletDimension(source)
  return { gesture, source, set: (next: Partial<DimensionSnapshot>) => { snapshot = { ...snapshot, ...next }; publishFilletDimension(source) } }
}
afterEach(() => { setLengthUnit('mm'); vi.restoreAllMocks() })

it('shows actual numeric state, parses precise units, and releases without committing', () => {
  setLengthUnit('mm')
  const h = connect()
  const { unmount } = render(<FilletDimensionControl sections={[]} />)
  const field = screen.getByRole('textbox', { name: 'Radius value' })
  expect(field).toHaveValue('20 mm')
  act(() => field.focus())
  fireEvent.change(field, { target: { value: '1 in' } })
  expect(h.gesture.update).toHaveBeenLastCalledWith(0.0254, '1 in')
  fireEvent.keyDown(field, { key: 'Enter' })
  expect(h.gesture.finish).toHaveBeenCalledTimes(1)
  act(() => h.set({ value: 0.03 }))
  expect(field).toHaveValue('30 mm')
  act(() => h.set({ draft: '45 mm' }))
  expect(field).toHaveValue('45 mm')
  unmount()
  disconnectFilletDimension(h.source)
})

it('Escape cancels only the input gesture and invalid input also rolls back', () => {
  const h = connect()
  const { unmount } = render(<FilletDimensionControl sections={[]} />)
  const field = screen.getByRole('textbox', { name: 'Radius value' })
  act(() => field.focus())
  fireEvent.change(field, { target: { value: '40 mm' } })
  fireEvent.keyDown(field, { key: 'Escape' })
  expect(h.gesture.cancel).toHaveBeenCalledTimes(1)
  expect(h.gesture.finish).not.toHaveBeenCalled()
  act(() => field.focus())
  fireEvent.change(field, { target: { value: '-' } })
  fireEvent.blur(field)
  expect(h.gesture.cancel).toHaveBeenCalledTimes(2)
  unmount()
  disconnectFilletDimension(h.source)
})

it('recentered travel can repeatedly grow the value without a geometry maximum', () => {
  const h = connect()
  const { unmount } = render(<FilletDimensionControl sections={[]} />)
  const slider = screen.getByRole('slider')
  fireEvent.change(slider, { target: { value: '1' } })
  expect(h.gesture.update).toHaveBeenLastCalledWith(0.04)
  fireEvent.keyUp(slider, { key: 'ArrowRight' })
  act(() => h.set({ value: 0.04 }))
  fireEvent.change(slider, { target: { value: '1' } })
  expect(h.gesture.update).toHaveBeenLastCalledWith(0.08)
  fireEvent.keyDown(slider, { key: 'Escape' })
  expect(h.gesture.cancel).toHaveBeenCalledTimes(1)
  expect(screen.getByRole('img', { name: 'Smaller radius' })).toBeVisible()
  expect(screen.getByRole('img', { name: 'Larger radius' })).toBeVisible()
  unmount()
  disconnectFilletDimension(h.source)
})

it('uses the selected decorative thumbnail and identifies active grip adjustments', () => {
  const profile = { kind: 'shape' as const, id: 'ogee', name: 'Ogee', shape: {} }
  const h = connect({ profile, label: 'Radius', grip: { kind: 'handleRadius', index: 1 } })
  const { unmount } = render(<FilletDimensionControl sections={[{ id: 'built-in', items: [{ id: 'ogee', label: 'Ogee', profile, thumbnail: 'data:image/png;base64,AA==' }] }]} />)
  const smaller = screen.getByRole('img', { name: 'Smaller radius at active grip' })
  expect(smaller.querySelector('img')).toHaveAttribute('src', 'data:image/png;base64,AA==')
  expect(screen.getByRole('slider')).toHaveAccessibleName('Adjust radius at active grip')
  unmount()
  disconnectFilletDimension(h.source)
})

it('omits the slider and both indicators together when unavailable', () => {
  const h = connect({ reason: 'Bullnose fits automatically.', value: null })
  const { unmount } = render(<FilletDimensionControl sections={[]} />)
  expect(screen.queryByRole('slider')).toBeNull()
  expect(screen.queryByRole('img')).toBeNull()
  expect(screen.getByText('Bullnose fits automatically.')).toBeVisible()
  unmount()
  disconnectFilletDimension(h.source)
})


it('represents stop length and point position rather than a change to every radius', () => {
  const h = connect({ label: 'Stop', value: 0, grip: { kind: 'stop', end: 'start' } })
  const { unmount } = render(<FilletDimensionControl sections={[]} />)
  expect(screen.getByRole('img', { name: 'Shorter stop at active grip' })).toBeVisible()
  expect(screen.getByRole('img', { name: 'Longer stop at active grip' })).toBeVisible()
  act(() => h.set({ label: 'Position', grip: { kind: 'handlePos', index: 0 } }))
  expect(screen.getByRole('img', { name: 'Earlier radius point at active grip' })).toBeVisible()
  expect(screen.getByRole('img', { name: 'Later radius point at active grip' })).toBeVisible()
  unmount()
  disconnectFilletDimension(h.source)
})


it('cancels a gesture when the operation identity changes, and on unmount', () => {
  const h = connect()
  const { unmount } = render(<FilletDimensionControl sections={[]} />)
  const field = screen.getByRole('textbox', { name: 'Radius value' })
  act(() => field.focus())
  fireEvent.change(field, { target: { value: '35 mm' } })
  act(() => h.set({ identity: 'next-operation:radius', value: 0.01 }))
  expect(h.gesture.cancel).toHaveBeenCalledTimes(1)
  expect(field).toHaveValue('10 mm')
  act(() => (field as HTMLInputElement).blur())
  act(() => field.focus())
  unmount()
  expect(h.gesture.cancel).toHaveBeenCalledTimes(2)
  disconnectFilletDimension(h.source)
})
