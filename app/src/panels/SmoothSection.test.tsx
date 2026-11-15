/**
 * SmoothSection: Object Info's Smooth row (Off / 1 / 2 / 3, Bake) and the
 * "Soften edges under [20]° [Apply]" row.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { SmoothSection } from './SmoothSection'
import type { Scene as WasmScene } from '../wasm/loader'
import { requiresPreview } from '../test/requiresPreview'

function setup(levels: Record<string, number>, objects: bigint[], opts: { throwOnSet?: boolean; softened?: number; dense?: bigint[] } = {}) {
  const free = vi.fn()
  const scene = {
    smooth_level: (id: bigint) => levels[String(id)] ?? 0,
    smooth_surface_available: (id: bigint) => !(opts.dense ?? []).includes(id),
    set_smooth_level: vi.fn((id: bigint, level: number) => {
      if (opts.throwOnSet === true) throw new Error('AmountOutOfRange: too dense')
      levels[String(id)] = level
      return { free }
    }),
    bake: vi.fn(() => ({ free })),
    soften_by_angle: vi.fn(() => opts.softened ?? 0),
    history_bracket_open: vi.fn(),
    history_bracket_commit: vi.fn(() => true),
    history_bracket_abort: vi.fn(),
  }
  const onGeometryChanged = vi.fn()
  const onToast = vi.fn()
  const utils = render(
    <SmoothSection scene={scene as unknown as WasmScene} docRev={0} objects={objects} onGeometryChanged={onGeometryChanged} onToast={onToast} />,
  )
  return { ...utils, scene, onGeometryChanged, onToast }
}

describe('SmoothSection', () => {
  it('says "too dense for this level" beside a level whose surface cannot be drawn', () => {
    setup({ '1': 2 }, [1n], { dense: [1n] })
    expect(screen.getByText('too dense for this level')).toBeInTheDocument()
  })

  it('shows no too-dense note when the surface draws, or when Smooth is Off', () => {
    const a = setup({ '1': 2 }, [1n])
    expect(screen.queryByText('too dense for this level')).not.toBeInTheDocument()
    a.unmount()
    setup({}, [1n], { dense: [1n] })
    expect(screen.queryByText('too dense for this level')).not.toBeInTheDocument()
  })

  it('shows Off for an object with no level and no Bake button', (ctx) => {
    requiresPreview(ctx, 'smooth', 'soften')
    setup({}, [1n])
    expect(screen.getByRole('combobox', { name: 'Smooth' })).toHaveValue('0')
    expect(screen.queryByRole('button', { name: 'Bake' })).not.toBeInTheDocument()
  })

  it('setting a level calls the kernel and refreshes', (ctx) => {
    requiresPreview(ctx, 'smooth', 'soften')
    const { scene, onGeometryChanged } = setup({}, [1n])
    fireEvent.change(screen.getByRole('combobox', { name: 'Smooth' }), { target: { value: '2' } })
    expect(scene.set_smooth_level).toHaveBeenCalledWith(1n, 2)
    expect(onGeometryChanged).toHaveBeenCalledOnce()
  })

  it('several objects are one bracket, and a mixed selection reads Mixed', () => {
    const { scene } = setup({ '1': 1, '2': 3 }, [1n, 2n])
    expect(screen.getByRole('combobox', { name: 'Smooth' })).toHaveValue('mixed')
    fireEvent.change(screen.getByRole('combobox', { name: 'Smooth' }), { target: { value: '0' } })
    expect(scene.history_bracket_open).toHaveBeenCalledOnce()
    expect(scene.set_smooth_level).toHaveBeenCalledTimes(2)
    expect(scene.history_bracket_commit).toHaveBeenCalledWith('Set Smooth')
  })

  it('a too-dense cage explains itself and changes nothing', (ctx) => {
    requiresPreview(ctx, 'smooth', 'soften')
    const { onToast, onGeometryChanged } = setup({}, [1n], { throwOnSet: true })
    fireEvent.change(screen.getByRole('combobox', { name: 'Smooth' }), { target: { value: '3' } })
    expect(onToast).toHaveBeenCalledWith('This cage is too dense for Smooth level 3.', 'AmountOutOfRange')
    expect(onGeometryChanged).not.toHaveBeenCalled()
  })

  it('Bake appears once a level is set and bakes', (ctx) => {
    requiresPreview(ctx, 'smooth', 'soften')
    const { scene, onGeometryChanged } = setup({ '1': 2 }, [1n])
    fireEvent.click(screen.getByRole('button', { name: 'Bake' }))
    expect(scene.bake).toHaveBeenCalledWith(1n)
    expect(onGeometryChanged).toHaveBeenCalledOnce()
  })

  it('Soften starts at 20 degrees and toasts how many edges it softened', (ctx) => {
    requiresPreview(ctx, 'smooth', 'soften')
    const { scene, onToast, onGeometryChanged } = setup({}, [1n], { softened: 12 })
    expect(screen.getByRole('textbox', { name: 'Soften angle in degrees' })).toHaveValue('20')
    fireEvent.click(screen.getByRole('button', { name: 'Apply' }))
    expect(scene.soften_by_angle).toHaveBeenCalledWith(1n, (20 * Math.PI) / 180)
    expect(onToast).toHaveBeenCalledWith('Softened 12 edges.')
    expect(onGeometryChanged).toHaveBeenCalledOnce()
  })

  it('Soften with nothing to soften says so and does not refresh', (ctx) => {
    requiresPreview(ctx, 'smooth', 'soften')
    const { onToast, onGeometryChanged } = setup({}, [1n], { softened: 0 })
    fireEvent.click(screen.getByRole('button', { name: 'Apply' }))
    expect(onToast).toHaveBeenCalledWith('No edges to soften.')
    expect(onGeometryChanged).not.toHaveBeenCalled()
  })

  it('Soften rejects a bad angle before calling the kernel', (ctx) => {
    requiresPreview(ctx, 'smooth', 'soften')
    const { scene, onToast } = setup({}, [1n])
    fireEvent.change(screen.getByRole('textbox', { name: 'Soften angle in degrees' }), { target: { value: '200' } })
    fireEvent.click(screen.getByRole('button', { name: 'Apply' }))
    expect(scene.soften_by_angle).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledWith('Type an angle between 1 and 179 degrees.')
  })

  it('renders nothing for an empty selection', () => {
    const { container } = setup({}, [])
    expect(container).toBeEmptyDOMElement()
  })
})
