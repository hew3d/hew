import { render, screen, fireEvent, act } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { SelectionPathStrip } from './SelectionPathStrip'
import type { PathCrumb } from './selectionPath'
import type { NodeRef } from './treeModel'

const g = (id: number): NodeRef => ({ kind: 'group', id: BigInt(id) })
const i = (id: number): NodeRef => ({ kind: 'instance', id: BigInt(id) })

function crumb(over: Partial<PathCrumb> & Pick<PathCrumb, 'key' | 'label'>): PathCrumb {
  return {
    kind: 'group', state: 'above', node: null, frameDepth: null, enterNode: null, ...over,
  }
}

const crumbs: PathCrumb[] = [
  crumb({ key: 'model', label: 'Model', kind: 'model' }),
  crumb({ key: 'g1', label: 'Kitchen', node: g(1), enterNode: g(1), frameDepth: 0 }),
  crumb({ key: 'g2', label: 'Base Cabinets', node: g(2), enterNode: g(2), state: 'selected' }),
  crumb({ key: 'g3', label: 'Cabinet', node: g(3), enterNode: g(3), state: 'below' }),
  crumb({ key: 'm', label: 'Door Panel', kind: 'member', state: 'member', enterNode: i(20) }),
]

function setup(list: PathCrumb[] = crumbs) {
  const h = { onSelect: vi.fn(), onModel: vi.fn(), onFrame: vi.fn(), onEnter: vi.fn() }
  render(<SelectionPathStrip crumbs={list} {...h} />)
  return h
}
const byLabel = (label: string) =>
  screen.getAllByTestId('selection-path-crumb').find((c) => c.title.startsWith(label)) as HTMLElement

describe('SelectionPathStrip', () => {
  it('renders nothing for an empty crumb list', () => {
    render(<SelectionPathStrip crumbs={[]} onSelect={vi.fn()} onModel={vi.fn()} onFrame={vi.fn()} onEnter={vi.fn()} />)
    expect(screen.queryByTestId('selection-path')).toBeNull()
  })

  it('renders every crumb with its state, the editing chip on frames, and the member note', () => {
    setup()
    expect(screen.getAllByTestId('selection-path-crumb')).toHaveLength(5)
    expect(byLabel('Kitchen').dataset.state).toBe('above')
    expect(byLabel('Kitchen').dataset.editing).toBe('true')
    expect(byLabel('Kitchen').textContent).toContain('editing')
    expect(byLabel('Base Cabinets').dataset.state).toBe('selected')
    expect(byLabel('Base Cabinets').textContent).not.toContain('editing')
    expect(byLabel('Cabinet').dataset.state).toBe('below')
    expect(byLabel('Door Panel').dataset.state).toBe('member')
    expect(byLabel('Door Panel').textContent).toContain('part of the component')
    expect(byLabel('Door Panel').tagName).toBe('SPAN') // not a button: not selectable
  })

  it('gives each crumb a full-name title and caps the label width', () => {
    setup()
    expect(byLabel('Base Cabinets').title).toBe('Base Cabinets')
    expect(screen.getByTestId('selection-path').style.maxWidth).toBe('100%')
    expect(screen.getByText('Base Cabinets').style.maxWidth).toBe('180px')
  })

  it('lets canvas clicks through everywhere but the crumbs', () => {
    setup()
    expect(screen.getByTestId('selection-path-band').style.pointerEvents).toBe('none')
    expect(screen.getByTestId('selection-path').style.pointerEvents).toBe('none')
    for (const c of screen.getAllByTestId('selection-path-crumb')) expect(c.style.pointerEvents).toBe('auto')
  })

  it('renders a divider crumb as a non-interactive hairline with no arrow after it', () => {
    setup([
      crumb({ key: 'model', label: 'Model', kind: 'model' }),
      crumb({ key: 'g1', label: 'Kitchen', node: g(1), frameDepth: 0 }),
      crumb({ key: 'divider', label: '', kind: 'divider' }),
      crumb({ key: 'g2', label: 'Island', node: g(2), state: 'selected' }),
    ])
    expect(screen.getByTestId('selection-path-divider').textContent).toContain('│')
    expect(screen.getAllByTestId('selection-path-crumb')).toHaveLength(3)
    expect(screen.getByTestId('selection-path').textContent?.split('›')).toHaveLength(2)
  })

  it('clicking a plain crumb replaces the selection with that node', () => {
    const h = setup()
    fireEvent.click(byLabel('Cabinet'))
    expect(h.onSelect).toHaveBeenCalledWith(g(3))
    expect(h.onModel).not.toHaveBeenCalled()
    expect(h.onFrame).not.toHaveBeenCalled()
  })

  it('clicking Model exits to the model', () => {
    const h = setup()
    fireEvent.click(byLabel('Model'))
    expect(h.onModel).toHaveBeenCalledTimes(1)
    expect(h.onSelect).not.toHaveBeenCalled()
  })

  it('clicking an open frame closes down to its depth', () => {
    const h = setup()
    fireEvent.click(byLabel('Kitchen'))
    expect(h.onFrame).toHaveBeenCalledWith(0)
    expect(h.onSelect).not.toHaveBeenCalled()
  })

  it('double-clicking a plain crumb enters it; Model and open frames do not', () => {
    const h = setup()
    fireEvent.doubleClick(byLabel('Cabinet'))
    expect(h.onEnter).toHaveBeenCalledWith(g(3))
    fireEvent.doubleClick(byLabel('Model'))
    fireEvent.doubleClick(byLabel('Kitchen'))
    expect(h.onEnter).toHaveBeenCalledTimes(1)
  })

  it('double-clicking the member crumb enters its instance; a single click does nothing', () => {
    const h = setup()
    fireEvent.click(byLabel('Door Panel'))
    expect(h.onSelect).not.toHaveBeenCalled()
    fireEvent.doubleClick(byLabel('Door Panel'))
    expect(h.onEnter).toHaveBeenCalledWith(i(20))
  })

  describe('adaptive placement', () => {
    let roCallbacks: Array<() => void> = []
    let widths = { client: 0, scroll: 0, band: 0 }
    const protoClient = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth')
    const protoScroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollWidth')

    function stubLayout() {
      roCallbacks = []
      vi.stubGlobal('ResizeObserver', class {
        constructor(cb: () => void) { roCallbacks.push(cb) }
        observe() {}
        disconnect() {}
      })
      Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
        configurable: true,
        get(this: HTMLElement) {
          const id = this.dataset.testid
          return id === 'selection-path' ? widths.client : id === 'selection-path-band' ? widths.band : 0
        },
      })
      Object.defineProperty(HTMLElement.prototype, 'scrollWidth', {
        configurable: true,
        get(this: HTMLElement) { return this.dataset.testid === 'selection-path' ? widths.scroll : 0 },
      })
    }
    afterEach(() => {
      vi.unstubAllGlobals()
      for (const [k, d] of [['clientWidth', protoClient], ['scrollWidth', protoScroll]] as const) {
        if (d !== undefined) Object.defineProperty(HTMLElement.prototype, k, d)
        else delete (HTMLElement.prototype as unknown as Record<string, unknown>)[k]
      }
    })
    const row = () => screen.getByTestId('selection-path').dataset.row
    const bandStyle = () => screen.getByTestId('selection-path-band').style

    it('stays in the top-center band when the content fits', () => {
      widths = { client: 400, scroll: 400, band: 0 }
      stubLayout()
      setup()
      expect(row()).toBe('band')
      expect(screen.getByTestId('selection-path-band').dataset.row).toBe('band')
      expect(bandStyle().top).toBe('16px')
      // 16 px inset + 76 px die + 12 px margin, on both sides to stay centered.
      expect(bandStyle().left).toBe('104px')
      expect(bandStyle().right).toBe('104px')
    })

    it('moves to a full-width second row when the content overflows the band, and back when it fits', () => {
      widths = { client: 120, scroll: 500, band: 0 }
      stubLayout()
      setup()
      expect(row()).toBe('second')
      expect(bandStyle().top).toBe('54px')
      // The second row still stops short of the die's column on the right.
      expect(bandStyle().left).toBe('16px')
      expect(bandStyle().right).toBe('104px')
      expect(bandStyle().pointerEvents).toBe('none')
      expect(screen.getByTestId('selection-path').style.maxWidth).toBe('100%')
      expect(screen.getByTestId('selection-path').style.overflow).toBe('hidden')
      for (const c of screen.getAllByTestId('selection-path-crumb')) expect(c.style.pointerEvents).toBe('auto')

      // Still too wide for the band (viewport 650 -> band 442): stays put on resize.
      widths = { client: 500, scroll: 500, band: 650 - 16 - 104 }
      act(() => roCallbacks.forEach((cb) => cb()))
      expect(row()).toBe('second')

      // Viewport grows: the band (1500 - 208 = 1292) now holds the content.
      widths = { client: 500, scroll: 500, band: 1500 - 16 - 104 }
      act(() => roCallbacks.forEach((cb) => cb()))
      expect(row()).toBe('band')
      expect(bandStyle().top).toBe('16px')
    })
  })
})
