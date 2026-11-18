import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { ContextualDock } from './ContextualDock'
import type { NodeRef } from './treeModel'
import { requiresPreview } from '../test/requiresPreview'
import { toolAllowed } from '../preview'

const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
const group = (id: bigint): NodeRef => ({ kind: 'group', id })
const instance = (id: bigint): NodeRef => ({ kind: 'instance', id })
const sketch = (id: bigint): NodeRef => ({ kind: 'sketch', id })

describe('ContextualDock', () => {
  it('shows the Empty (draw-tool) verb set when nothing is selected', () => {
    render(<ContextualDock selectedIds={[]} onRun={vi.fn()} />)
    expect(screen.getByText('DRAW')).toBeInTheDocument()
    expect(screen.getByText('Rectangle')).toBeInTheDocument()
    expect(screen.getByText('Line')).toBeInTheDocument()
    expect(screen.getByText('Circle')).toBeInTheDocument()
    expect(screen.getByText('Arc')).toBeInTheDocument()
  })

  it('shows the Object verb set for a single selected Object', () => {
    render(<ContextualDock selectedIds={[obj(1n)]} onRun={vi.fn()} />)
    expect(screen.getByText('OBJECT')).toBeInTheDocument()
    expect(screen.getByText('Move')).toBeInTheDocument()
    expect(screen.getByText('Push/Pull')).toBeInTheDocument()
    expect(screen.getByText('Paint')).toBeInTheDocument()
    expect(screen.getByText('Make Component')).toBeInTheDocument()
    expect(screen.getByText('Erase')).toBeInTheDocument()
    expect(screen.queryByText('Position Texture')).not.toBeInTheDocument()
  })

  it('offers Fillet and Shell for a single selected Object', (ctx) => {
    requiresPreview(ctx, 'fillet', 'shell')
    render(<ContextualDock selectedIds={[obj(1n)]} onRun={vi.fn()} />)
    expect(screen.getByText('Fillet')).toBeInTheDocument()
    expect(screen.getByText('Shell')).toBeInTheDocument()
  })

  it('Object row order is Push/Pull, Fillet, Move, Paint, Shell, Make Component, Save to Library, Erase, each with an icon', () => {
    render(<ContextualDock selectedIds={[obj(1n)]} onRun={vi.fn()} />)
    const buttons = screen.getAllByRole('button')
    // Fillet and Shell appear only where the Preview wall lets them.
    expect(buttons.map((b) => b.getAttribute('title'))).toEqual([
      'Push/Pull', 'Fillet', 'Move', 'Paint', 'Shell', 'Make Component', 'Save to Library', 'Erase',
    ].filter((t) => toolAllowed(t)))
    for (const b of buttons) expect(b.querySelector('svg'), b.getAttribute('title') ?? '').not.toBeNull()
  })

  it('clicking Fillet and Shell dispatches their tool ids', (ctx) => {
    requiresPreview(ctx, 'fillet', 'shell')
    const onRun = vi.fn()
    render(<ContextualDock selectedIds={[obj(1n)]} onRun={onRun} />)
    fireEvent.click(screen.getByText('Fillet'))
    expect(onRun).toHaveBeenLastCalledWith('tool-fillet')
    fireEvent.click(screen.getByText('Shell'))
    expect(onRun).toHaveBeenLastCalledWith('tool-shell')
  })

  it('shows the Group verb set for a single selected Group', () => {
    render(<ContextualDock selectedIds={[group(1n)]} onRun={vi.fn()} />)
    expect(screen.getByText('GROUP')).toBeInTheDocument()
    expect(screen.getByText('Edit')).toBeInTheDocument()
    expect(screen.getByText('Make Component')).toBeInTheDocument()
    expect(screen.getByText('Ungroup')).toBeInTheDocument()
  })

  it('shows the Instance verb set for a single selected Instance', () => {
    render(<ContextualDock selectedIds={[instance(1n)]} onRun={vi.fn()} />)
    expect(screen.getByText('COMPNT')).toBeInTheDocument()
    expect(screen.getByText('Make Unique')).toBeInTheDocument()
    expect(screen.getByText('Explode')).toBeInTheDocument()
  })

  it('shows the Multi verb set for more than one selected node', () => {
    render(<ContextualDock selectedIds={[obj(1n), group(2n)]} onRun={vi.fn()} />)
    expect(screen.getByText('MULTI')).toBeInTheDocument()
    expect(screen.getByText('Move')).toBeInTheDocument()
    expect(screen.getByText('Group')).toBeInTheDocument()
    expect(screen.getByText('Erase')).toBeInTheDocument()
    expect(screen.queryByText('Paint')).not.toBeInTheDocument()
  })

  it('Multi row shows Union, Subtract, and Intersect (with icons) only when canBoolean is true, and dispatches them', () => {
    const onRun = vi.fn()
    const sel = [obj(1n), obj(2n)]
    const { unmount } = render(
      <ContextualDock selectedIds={sel} onRun={onRun} gates={{ canGroup: true, canMakeComponent: true, canBoolean: false }} />,
    )
    for (const label of ['Union', 'Subtract', 'Intersect']) expect(screen.queryByText(label)).not.toBeInTheDocument()
    unmount()
    render(
      <ContextualDock selectedIds={sel} onRun={onRun} gates={{ canGroup: true, canMakeComponent: true, canBoolean: true }} />,
    )
    expect(screen.getAllByRole('button').map((b) => b.getAttribute('title'))).toEqual([
      'Move', 'Union', 'Subtract', 'Intersect', 'Group', 'Erase',
    ])
    for (const [label, id] of [['Union', 'edit-union'], ['Subtract', 'edit-subtract'], ['Intersect', 'edit-intersect']]) {
      expect(screen.getByRole('button', { name: label }).querySelector('svg')).not.toBeNull()
      fireEvent.click(screen.getByText(label))
      expect(onRun).toHaveBeenLastCalledWith(id)
    }
  })

  // --- Structural verbs (Group / Make Component) dispatch + gating --------

  it('clicking Group on a multi-selection dispatches edit-group', () => {
    const onRun = vi.fn()
    render(
      <ContextualDock
        selectedIds={[obj(1n), obj(2n)]}
       
        onRun={onRun}
        gates={{ canGroup: true, canMakeComponent: true }}
      />,
    )
    fireEvent.click(screen.getByText('Group'))
    expect(onRun).toHaveBeenCalledWith('edit-group')
  })

  it('clicking Make Component on an object dispatches edit-make-component', () => {
    const onRun = vi.fn()
    render(
      <ContextualDock
        selectedIds={[obj(1n)]}
       
        onRun={onRun}
        gates={{ canGroup: false, canMakeComponent: true }}
      />,
    )
    fireEvent.click(screen.getByText('Make Component'))
    expect(onRun).toHaveBeenCalledWith('edit-make-component')
  })

  it('hides the structural verbs when their gates are off', () => {
    render(
      <ContextualDock
        selectedIds={[obj(1n), sketch(2n)]}
       
        onRun={vi.fn()}
        gates={{ canGroup: false, canMakeComponent: false }}
      />,
    )
    // A multi-selection containing a sketch sub-entity is not groupable.
    expect(screen.getByText('MULTI')).toBeInTheDocument()
    expect(screen.queryByText('Group')).not.toBeInTheDocument()
    expect(screen.queryByText('Make Component')).not.toBeInTheDocument()
  })

  // Contract change: a selected sketch used to render nothing — it
  // now gets its own 'sketch' verb row (Push/Pull primary, then the
  // transform trio, then Erase).
  it('shows the Sketch verb set for a single selected free-standing sketch', () => {
    render(<ContextualDock selectedIds={[sketch(1n)]} onRun={vi.fn()} />)
    expect(screen.getByText('SKETCH')).toBeInTheDocument()
    expect(screen.getByText('Push/Pull')).toBeInTheDocument()
    expect(screen.getByText('Follow Me')).toBeInTheDocument()
    expect(screen.getByText('Move')).toBeInTheDocument()
    expect(screen.getByText('Rotate')).toBeInTheDocument()
    expect(screen.queryByText('Scale')).not.toBeInTheDocument()
    expect(screen.getByText('Erase')).toBeInTheDocument()
  })

  it('offers Revolve and Loft for a single selected free-standing sketch', (ctx) => {
    requiresPreview(ctx, 'revolve', 'loft')
    render(<ContextualDock selectedIds={[sketch(1n)]} onRun={vi.fn()} />)
    expect(screen.getByText('Revolve')).toBeInTheDocument()
    expect(screen.getByText('Loft')).toBeInTheDocument()
  })

  it('clicking Follow Me on a sketch dispatches its tool id', () => {
    const onRun = vi.fn()
    render(<ContextualDock selectedIds={[sketch(1n)]} onRun={onRun} />)
    fireEvent.click(screen.getByText('Follow Me'))
    expect(onRun).toHaveBeenLastCalledWith('tool-follow-me')
  })

  it('clicking Revolve and Loft on a sketch dispatches their tool ids', (ctx) => {
    requiresPreview(ctx, 'revolve', 'loft')
    const onRun = vi.fn()
    render(<ContextualDock selectedIds={[sketch(1n)]} onRun={onRun} />)
    for (const [label, id] of [['Revolve', 'tool-revolve'], ['Loft', 'tool-loft']]) {
      fireEvent.click(screen.getByText(label))
      expect(onRun).toHaveBeenLastCalledWith(id)
    }
  })

  it('renders nothing when only guides or annotations are selected', () => {
    const { container } = render(<ContextualDock selectedIds={[]} others={2} onRun={vi.fn()} />)
    expect(container.firstChild).toBeNull()
  })

  it('offers the nodes verbs alone when guides or annotations are selected beside them', () => {
    const { container } = render(<ContextualDock selectedIds={[{ kind: 'object', id: 1n }]} others={2} onRun={vi.fn()} />)
    const alone = render(<ContextualDock selectedIds={[{ kind: 'object', id: 1n }]} onRun={vi.fn()} />)
    expect(container.firstChild).not.toBeNull()
    expect(container.innerHTML).toBe(alone.container.innerHTML)
  })

  it('clicking the primary verb calls onRun with its id', () => {
    const onRun = vi.fn()
    render(<ContextualDock selectedIds={[obj(1n)]} onRun={onRun} />)
    fireEvent.click(screen.getByText('Push/Pull'))
    expect(onRun).toHaveBeenCalledWith('tool-pushpull')
  })

  it('clicking a secondary verb calls onRun with its id', () => {
    const onRun = vi.fn()
    render(<ContextualDock selectedIds={[obj(1n)]} onRun={onRun} />)
    fireEvent.click(screen.getByText('Erase'))
    expect(onRun).toHaveBeenCalledWith('edit-delete')
  })

  // --- Save to Library (Library effort) ---

  it('hides Save to Library when gates are given but canSaveToLibrary is omitted/false', () => {
    render(
      <ContextualDock
        selectedIds={[obj(1n)]}
       
        onRun={vi.fn()}
        gates={{ canGroup: false, canMakeComponent: false }}
      />,
    )
    expect(screen.queryByText('Save to Library')).not.toBeInTheDocument()
  })

  it('shows and dispatches Save to Library when canSaveToLibrary is true', () => {
    const onRun = vi.fn()
    render(
      <ContextualDock
        selectedIds={[obj(1n)]}
       
        onRun={onRun}
        gates={{ canGroup: false, canMakeComponent: false, canSaveToLibrary: true }}
      />,
    )
    fireEvent.click(screen.getByText('Save to Library'))
    expect(onRun).toHaveBeenCalledWith('save-to-library')
  })

  // --- Active-tool highlight  ---

  it('does NOT highlight the first (primary) verb when it is not the active tool', () => {
    render(<ContextualDock selectedIds={[]} onRun={vi.fn()} activeToolId="tool-arc" />)
    // Rectangle is first in EMPTY_VERBS (the old "primary" slot) but Arc is
    // the actual active tool here — Rectangle must render unselected.
    const rectangleBtn = screen.getByRole('button', { name: 'Rectangle' })
    expect(rectangleBtn.style.border).toBe('1px solid transparent')
    expect(rectangleBtn).toHaveAttribute('aria-pressed', 'false')
  })

  it('highlights the verb matching activeToolId, wherever it sits in the list', () => {
    render(<ContextualDock selectedIds={[]} onRun={vi.fn()} activeToolId="tool-arc" />)
    const arcBtn = screen.getByRole('button', { name: 'Arc' })
    expect(arcBtn.style.border).toContain('var(--accent-border)')
    // aria-pressed mirrors the highlight — the machine-readable contract the
    // E2E ui-chrome spec asserts through.
    expect(arcBtn).toHaveAttribute('aria-pressed', 'true')
  })

  it('highlights nothing when activeToolId is undefined', () => {
    render(<ContextualDock selectedIds={[]} onRun={vi.fn()} />)
    for (const label of ['Rectangle', 'Line', 'Circle', 'Arc']) {
      const btn = screen.getByRole('button', { name: label })
      expect(btn.style.border).toBe('1px solid transparent')
    }
  })

  it('highlights nothing when activeToolId is not in the current context\'s verb set', () => {
    // Arc has no dock verb in the Object context — nothing should light up.
    render(<ContextualDock selectedIds={[obj(1n)]} onRun={vi.fn()} activeToolId="tool-arc" />)
    for (const label of ['Push/Pull', 'Move', 'Paint', 'Erase']) {
      const btn = screen.getByRole('button', { name: label })
      expect(btn.style.border).toBe('1px solid transparent')
    }
  })

  it('cross-fades on context change: container carries.hew-dock and remounts when the context swaps', () => {
    const { container, rerender } = render(
      <ContextualDock selectedIds={[obj(1n)]} onRun={vi.fn()} />,
    )
    const before = container.firstChild as HTMLElement
    expect(before.className).toContain('hew-dock')

    rerender(<ContextualDock selectedIds={[group(2n)]} onRun={vi.fn()} />)
    const after = container.firstChild as HTMLElement
    // The container is keyed on the context, so a context swap replaces the
    // DOM node — that remount is what replays the CSS cross-fade animation.
    expect(after).not.toBe(before)
    expect(after.className).toContain('hew-dock')
    expect(screen.getByText('GROUP')).toBeInTheDocument()
  })

  it('does NOT remount (no cross-fade restart) when the selection changes but the context stays the same', () => {
    const { container, rerender } = render(
      <ContextualDock selectedIds={[obj(1n)]} onRun={vi.fn()} />,
    )
    const before = container.firstChild as HTMLElement
    rerender(<ContextualDock selectedIds={[obj(2n)]} onRun={vi.fn()} />)
    expect(container.firstChild).toBe(before)
  })

  it('hidden (camera drag): stays mounted but fades to opacity 0 and becomes click-through', () => {
    const { container } = render(
      <ContextualDock selectedIds={[obj(1n)]} onRun={vi.fn()} hidden />,
    )
    const dock = container.firstChild as HTMLElement
    expect(dock).not.toBeNull() // mounted, so the fade can transition back in
    expect(dock.style.opacity).toBe('0')
    expect(dock.style.pointerEvents).toBe('none')
  })

  it('reappears when hidden flips back off (drag released)', () => {
    const { container, rerender } = render(
      <ContextualDock selectedIds={[obj(1n)]} onRun={vi.fn()} hidden />,
    )
    const before = container.firstChild as HTMLElement
    rerender(<ContextualDock selectedIds={[obj(1n)]} onRun={vi.fn()} hidden={false} />)
    const dock = container.firstChild as HTMLElement
    // Same node (no remount — this is the transition path, not the cross-fade
    // path) and fully interactive again.
    expect(dock).toBe(before)
    expect(dock.style.opacity).toBe('1')
    expect(dock.style.pointerEvents).toBe('')
  })

  // --- Hover-preview of the Sketch verbs --------------------------

  it('shows the Sketch verb set when selection is empty and hoveringSketchRegion is set', () => {
    render(
      <ContextualDock selectedIds={[]} onRun={vi.fn()} hoveringSketchRegion />,
    )
    expect(screen.getByText('SKETCH')).toBeInTheDocument()
    expect(screen.getByText('Push/Pull')).toBeInTheDocument()
    expect(screen.queryByText('Rectangle')).not.toBeInTheDocument()
  })

  it('shows the ordinary empty (draw-tool) verb set when hoveringSketchRegion is false', () => {
    render(
      <ContextualDock selectedIds={[]} onRun={vi.fn()} hoveringSketchRegion={false} />,
    )
    expect(screen.getByText('DRAW')).toBeInTheDocument()
    expect(screen.getByText('Rectangle')).toBeInTheDocument()
  })

  it('ignores hoveringSketchRegion entirely once something is actually selected', () => {
    render(
      <ContextualDock selectedIds={[obj(1n)]} onRun={vi.fn()} hoveringSketchRegion />,
    )
    // An explicit Object selection always wins — still the Object row, not Sketch.
    expect(screen.getByText('OBJECT')).toBeInTheDocument()
    expect(screen.queryByText('SKETCH')).not.toBeInTheDocument()
  })

  it('ignores hoveringSketchRegion when only a guide is selected (still no dock)', () => {
    const { container } = render(
      <ContextualDock selectedIds={[]} others={1} onRun={vi.fn()} hoveringSketchRegion />,
    )
    expect(container.firstChild).toBeNull()
  })

  it('clicking the hover-previewed Push/Pull verb dispatches the same id as a real Sketch selection', () => {
    const onRun = vi.fn()
    render(
      <ContextualDock selectedIds={[]} onRun={onRun} hoveringSketchRegion />,
    )
    fireEvent.click(screen.getByText('Push/Pull'))
    expect(onRun).toHaveBeenCalledWith('tool-pushpull')
  })

  // --- Hover-preview disables everything except Push/Pull ------------------

  it('hover-preview: Push/Pull, Revolve, and Loft are enabled where offered; the other sketch verbs render disabled', () => {
    render(
      <ContextualDock selectedIds={[]} onRun={vi.fn()} hoveringSketchRegion />,
    )
    for (const label of ['Push/Pull', 'Revolve', 'Loft'].filter((l) => toolAllowed(l))) {
      expect(screen.getByRole('button', { name: label })).not.toBeDisabled()
    }
    for (const label of ['Follow Me', 'Move', 'Rotate', 'Erase']) {
      const btn = screen.getByRole('button', { name: label })
      expect(btn).toBeDisabled()
      expect(btn).toHaveAttribute('aria-disabled', 'true')
      // Visually quieted and non-interactive.
      expect(btn.style.opacity).toBe('0.4')
      expect(btn.style.pointerEvents).toBe('none')
    }
  })

  it('hover-preview: clicking a disabled verb does not dispatch', () => {
    const onRun = vi.fn()
    render(
      <ContextualDock selectedIds={[]} onRun={onRun} hoveringSketchRegion />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Move' }))
    expect(onRun).not.toHaveBeenCalled()
  })

  it('hover-preview: disabled verbs do not light up on hover', () => {
    render(
      <ContextualDock selectedIds={[]} onRun={vi.fn()} hoveringSketchRegion />,
    )
    const btn = screen.getByRole('button', { name: 'Move' })
    fireEvent.mouseEnter(btn)
    expect(btn.style.background).toBe('transparent')
  })

  it('a real sketch SELECTION keeps every sketch verb enabled', () => {
    render(<ContextualDock selectedIds={[sketch(1n)]} onRun={vi.fn()} />)
    for (const label of ['Push/Pull', 'Revolve', 'Loft', 'Follow Me', 'Move', 'Rotate', 'Erase'].filter((l) => toolAllowed(l))) {
      const btn = screen.getByRole('button', { name: label })
      expect(btn).not.toBeDisabled()
      expect(btn).toHaveAttribute('aria-disabled', 'false')
    }
  })

  it('a real sketch selection keeps every verb enabled even while hoveringSketchRegion is set', () => {
    const onRun = vi.fn()
    render(
      <ContextualDock selectedIds={[sketch(1n)]} onRun={onRun} hoveringSketchRegion />,
    )
    const move = screen.getByRole('button', { name: 'Move' })
    expect(move).not.toBeDisabled()
    fireEvent.click(move)
    expect(onRun).toHaveBeenCalledWith('tool-move')
  })
})
