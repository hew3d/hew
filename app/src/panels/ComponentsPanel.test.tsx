/**
 * ComponentsPanel — filter box and thumbnail caching/invalidation.
 *
 * A plain JS object stands in for the wasm Scene (the panel imports the
 * type only, erased at runtime — the same convention scenePanels.test.tsx
 * uses for MaterialPalette et al.), so no wasm/loader mock is required.
 */

import { StrictMode } from 'react'
import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ComponentsPanel } from './ComponentsPanel'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from './treeModel'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function makeScene(overrides: Record<string, any> = {}): WasmScene {
  return {
    component_ids: () => new BigUint64Array(),
    component_name: (_id: bigint) => undefined as string | undefined,
    definition_usage: (_id: bigint) => 0,
    instances_of: (_id: bigint) => new BigUint64Array(),
    render_definition_thumbnail: (_id: bigint, _size: number) => undefined as Uint8Array | undefined,
    ...overrides,
  } as unknown as WasmScene
}

const baseProps = {
  docRev: 0,
  onSelectNodes: vi.fn() as (nodes: NodeRef[]) => void,
  onRenameComponent: vi.fn(() => null) as (id: bigint, name: string) => string | null,
  onDeleteComponent: vi.fn(),
}

beforeEach(() => {
  vi.clearAllMocks()
  if (typeof URL.createObjectURL !== 'function') {
    URL.createObjectURL = () => 'blob:mock'
  }
  if (typeof URL.revokeObjectURL !== 'function') {
    URL.revokeObjectURL = () => {}
  }
  vi.spyOn(URL, 'createObjectURL').mockImplementation((b: Blob | MediaSource) => `blob:${(b as Blob).size ?? 0}:${Math.random()}`)
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})
})

describe('ComponentsPanel filter', () => {
  it('shows every definition when the filter is empty', () => {
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n, 2n]),
      component_name: (id: bigint) => (id === 1n ? 'Door' : 'Window'),
    })
    render(<ComponentsPanel {...baseProps} scene={scene} />)
    expect(screen.getByText('Door')).toBeInTheDocument()
    expect(screen.getByText('Window')).toBeInTheDocument()
  })

  it('narrows the list by a case-insensitive substring match on name', () => {
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n, 2n]),
      component_name: (id: bigint) => (id === 1n ? 'Oak Door' : 'Sky Window'),
    })
    render(<ComponentsPanel {...baseProps} scene={scene} />)
    fireEvent.change(screen.getByLabelText('Filter components'), { target: { value: 'DOOR' } })
    expect(screen.getByText('Oak Door')).toBeInTheDocument()
    expect(screen.queryByText('Sky Window')).not.toBeInTheDocument()
  })

  it('shows "No components match" when nothing matches, and × clears it back to the full list', () => {
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n]),
      component_name: () => 'Door',
    })
    render(<ComponentsPanel {...baseProps} scene={scene} />)
    fireEvent.change(screen.getByLabelText('Filter components'), { target: { value: 'zzz' } })
    expect(screen.queryByText('Door')).not.toBeInTheDocument()
    expect(screen.getByText('No components match')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Clear filter' }))
    expect(screen.getByText('Door')).toBeInTheDocument()
    expect(screen.queryByText('No components match')).not.toBeInTheDocument()
  })

  it('typing a filter never calls onSelectNodes or touches any row action', () => {
    const onSelectNodes = vi.fn()
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n, 2n]),
      component_name: (id: bigint) => (id === 1n ? 'Door' : 'Window'),
    })
    render(<ComponentsPanel {...baseProps} scene={scene} onSelectNodes={onSelectNodes} />)
    fireEvent.change(screen.getByLabelText('Filter components'), { target: { value: 'door' } })
    fireEvent.change(screen.getByLabelText('Filter components'), { target: { value: '' } })
    expect(onSelectNodes).not.toHaveBeenCalled()
  })

  it('lists definitions by name, case-insensitive and numeric ("door 2" before "Door 10")', () => {
    const names: Record<string, string> = { '1': 'Window', '2': 'Door 10', '3': 'door 2', '4': 'bolt' }
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n, 2n, 3n, 4n]),
      component_name: (id: bigint) => names[id.toString()],
    })
    render(<ComponentsPanel {...baseProps} scene={scene} />)
    expect(screen.getAllByTestId('components-row-name').map((e) => e.textContent)).toEqual([
      'bolt', 'door 2', 'Door 10', 'Window',
    ])
  })

  it('keeps an unnamed definition\'s positional fallback numbering stable while filtering', () => {
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n, 2n]),
      component_name: (id: bigint) => (id === 1n ? 'Door' : undefined),
    })
    render(<ComponentsPanel {...baseProps} scene={scene} />)
    // Unnamed second definition falls back to a positional label built from
    // its ORIGINAL (unfiltered) index — filtering down to just it must not
    // renumber it.
    const unfilteredLabel = screen.getAllByTestId('components-row-name')[1].textContent
    fireEvent.change(screen.getByLabelText('Filter components'), { target: { value: 'Door' } })
    fireEvent.change(screen.getByLabelText('Filter components'), { target: { value: '' } })
    expect(screen.getAllByTestId('components-row-name')[1].textContent).toBe(unfilteredLabel)
  })
})

describe('ComponentsPanel thumbnails', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('renders lazily, one definition at a time, off the critical render path', () => {
    const calls: bigint[] = []
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n, 2n, 3n]),
      component_name: (id: bigint) => `Def ${id}`,
      render_definition_thumbnail: (id: bigint) => {
        calls.push(id)
        return new Uint8Array([1, 2, 3])
      },
    })
    render(<ComponentsPanel {...baseProps} scene={scene} />)
    // The first render happens synchronously in the mount effect; the rest
    // are deferred behind a macrotask so a long list never blocks a frame.
    expect(calls).toEqual([1n])

    act(() => {
      vi.advanceTimersByTime(0)
    })
    expect(calls).toEqual([1n, 2n])

    act(() => {
      vi.runAllTimers()
    })
    expect(calls).toEqual([1n, 2n, 3n])
  })

  it('shows a placeholder before a thumbnail is ready, and an <img> with a src once it lands', () => {
    // Two definitions: the first resolves synchronously in the mount
    // effect (see the "one at a time" test above), so the SECOND is the
    // one still showing a placeholder until the deferred queue reaches it.
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n, 2n]),
      component_name: (id: bigint) => (id === 1n ? 'Door' : 'Window'),
      render_definition_thumbnail: () => new Uint8Array([1, 2, 3, 4]),
    })
    render(<ComponentsPanel {...baseProps} scene={scene} />)
    const placeholders = screen.getAllByTestId('components-row-thumb-placeholder')
    expect(placeholders).toHaveLength(1)
    expect(screen.getAllByTestId('components-row-thumb')).toHaveLength(1)

    act(() => {
      vi.runAllTimers()
    })
    const imgs = screen.getAllByTestId('components-row-thumb') as HTMLImageElement[]
    expect(imgs).toHaveLength(2)
    for (const img of imgs) expect(img.src).toMatch(/^blob:/)
    expect(URL.createObjectURL).toHaveBeenCalledTimes(2)
  })

  it('still shows its thumbnail after a StrictMode remount', () => {
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n]),
      component_name: () => 'Leg',
      render_definition_thumbnail: () => new Uint8Array([1, 2, 3, 4]),
    })
    render(
      <StrictMode>
        <ComponentsPanel {...baseProps} scene={scene} />
      </StrictMode>,
    )
    act(() => {
      vi.runAllTimers()
    })
    expect(screen.getAllByTestId('components-row-thumb')).toHaveLength(1)
  })

  it('caches a thumbnail across re-renders at the same docRev — no repeat render call', () => {
    let calls = 0
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n]),
      component_name: () => 'Door',
      render_definition_thumbnail: () => {
        calls += 1
        return new Uint8Array([1, 2, 3, 4])
      },
    })
    const { rerender } = render(<ComponentsPanel {...baseProps} scene={scene} docRev={5} />)
    act(() => {
      vi.runAllTimers()
    })
    expect(calls).toBe(1)

    // A re-render at the SAME docRev (e.g. an unrelated prop changed) must
    // not re-render an already-cached, still-valid thumbnail.
    rerender(<ComponentsPanel {...baseProps} scene={scene} docRev={5} onDeleteComponent={vi.fn()} />)
    act(() => {
      vi.runAllTimers()
    })
    expect(calls).toBe(1)
  })

  it('invalidates and re-renders a thumbnail when docRev changes (the definition may have changed)', () => {
    let calls = 0
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n]),
      component_name: () => 'Door',
      render_definition_thumbnail: () => {
        calls += 1
        return new Uint8Array([calls])
      },
    })
    const { rerender } = render(<ComponentsPanel {...baseProps} scene={scene} docRev={1} />)
    act(() => {
      vi.runAllTimers()
    })
    expect(calls).toBe(1)

    rerender(<ComponentsPanel {...baseProps} scene={scene} docRev={2} />)
    act(() => {
      vi.runAllTimers()
    })
    expect(calls).toBe(2)
  })

  it('revokes cached object URLs and clears the cache when docGeneration changes (a new document loaded)', () => {
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n]),
      component_name: () => 'Door',
      render_definition_thumbnail: () => new Uint8Array([1, 2, 3, 4]),
    })
    const { rerender } = render(<ComponentsPanel {...baseProps} scene={scene} docGeneration={1} />)
    act(() => {
      vi.runAllTimers()
    })
    expect(screen.getByTestId('components-row-thumb')).toBeInTheDocument()
    expect(URL.revokeObjectURL).not.toHaveBeenCalled()

    rerender(<ComponentsPanel {...baseProps} scene={scene} docGeneration={2} />)
    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1)
    // The cache is cleared synchronously; the row falls back to the
    // placeholder until the lazy queue re-renders it.
    expect(screen.getByTestId('components-row-thumb-placeholder')).toBeInTheDocument()
  })

  it('a render that fails while a thumbnail is showing keeps the last good image and retries on the next docRev', () => {
    // The kernel refuses definition previews for EVERY definition while a
    // component is open for editing; that must not wipe the panel.
    let calls = 0
    let refuse = false
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n]),
      component_name: () => 'Door',
      render_definition_thumbnail: () => {
        calls += 1
        return refuse ? undefined : new Uint8Array([calls])
      },
    })
    const { rerender } = render(<ComponentsPanel {...baseProps} scene={scene} docRev={1} />)
    act(() => {
      vi.runAllTimers()
    })
    expect(screen.getByTestId('components-row-thumb')).toBeInTheDocument()
    const shown = screen.getByTestId('components-row-thumb').getAttribute('src')

    refuse = true // a component-edit session opened: every preview refused
    rerender(<ComponentsPanel {...baseProps} scene={scene} docRev={2} />)
    act(() => {
      vi.runAllTimers()
    })
    expect(calls).toBe(2)
    expect(screen.getByTestId('components-row-thumb').getAttribute('src')).toBe(shown) // still the old image
    expect(URL.revokeObjectURL).not.toHaveBeenCalled()

    refuse = false // the session closed: the next document change re-renders
    rerender(<ComponentsPanel {...baseProps} scene={scene} docRev={3} />)
    act(() => {
      vi.runAllTimers()
    })
    expect(calls).toBe(3)
    expect(screen.getByTestId('components-row-thumb').getAttribute('src')).not.toBe(shown)
    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1)
  })

  it('a definition with nothing to render shows no thumbnail image (honest "no thumbnail")', () => {
    const scene = makeScene({
      component_ids: () => new BigUint64Array([1n]),
      component_name: () => 'Empty',
      render_definition_thumbnail: () => undefined,
    })
    render(<ComponentsPanel {...baseProps} scene={scene} />)
    act(() => {
      vi.runAllTimers()
    })
    expect(screen.queryByTestId('components-row-thumb')).not.toBeInTheDocument()
    expect(screen.getByTestId('components-row-thumb-placeholder')).toBeInTheDocument()
  })
})
