import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { ContextualDock } from './ContextualDock'
import type { NodeRef } from './treeModel'

const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
const group = (id: bigint): NodeRef => ({ kind: 'group', id })
const instance = (id: bigint): NodeRef => ({ kind: 'instance', id })
const sketch = (id: bigint): NodeRef => ({ kind: 'sketch', id })

describe('ContextualDock', () => {
  it('shows the Empty (draw-tool) verb set when nothing is selected', () => {
    render(<ContextualDock selectedIds={[]} selectedGuide={null} onRun={vi.fn()} />)
    expect(screen.getByText('DRAW')).toBeInTheDocument()
    expect(screen.getByText('Rectangle')).toBeInTheDocument()
    expect(screen.getByText('Line')).toBeInTheDocument()
    expect(screen.getByText('Circle')).toBeInTheDocument()
    expect(screen.getByText('Arc')).toBeInTheDocument()
  })

  it('shows the Object verb set for a single selected Object', () => {
    render(<ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={vi.fn()} />)
    expect(screen.getByText('OBJECT')).toBeInTheDocument()
    expect(screen.getByText('Move')).toBeInTheDocument()
    expect(screen.getByText('Push/Pull')).toBeInTheDocument()
    expect(screen.getByText('Paint')).toBeInTheDocument()
    expect(screen.getByText('Erase')).toBeInTheDocument()
  })

  it('shows the Group verb set for a single selected Group', () => {
    render(<ContextualDock selectedIds={[group(1n)]} selectedGuide={null} onRun={vi.fn()} />)
    expect(screen.getByText('GROUP')).toBeInTheDocument()
    expect(screen.getByText('Edit')).toBeInTheDocument()
    expect(screen.getByText('Ungroup')).toBeInTheDocument()
  })

  it('shows the Instance verb set for a single selected Instance', () => {
    render(<ContextualDock selectedIds={[instance(1n)]} selectedGuide={null} onRun={vi.fn()} />)
    expect(screen.getByText('COMPNT')).toBeInTheDocument()
    expect(screen.getByText('Make Unique')).toBeInTheDocument()
    expect(screen.getByText('Explode')).toBeInTheDocument()
  })

  it('shows the Multi verb set for more than one selected node', () => {
    render(<ContextualDock selectedIds={[obj(1n), group(2n)]} selectedGuide={null} onRun={vi.fn()} />)
    expect(screen.getByText('MULTI')).toBeInTheDocument()
    expect(screen.getByText('Move')).toBeInTheDocument()
    expect(screen.getByText('Erase')).toBeInTheDocument()
    expect(screen.queryByText('Paint')).not.toBeInTheDocument()
  })

  // Contract change: a selected sketch used to render nothing — it
  // now gets its own 'sketch' verb row (Push/Pull primary, then the
  // transform trio, then Erase).
  it('shows the Sketch verb set for a single selected free-standing sketch', () => {
    render(<ContextualDock selectedIds={[sketch(1n)]} selectedGuide={null} onRun={vi.fn()} />)
    expect(screen.getByText('SKETCH')).toBeInTheDocument()
    expect(screen.getByText('Push/Pull')).toBeInTheDocument()
    expect(screen.getByText('Move')).toBeInTheDocument()
    expect(screen.getByText('Rotate')).toBeInTheDocument()
    expect(screen.getByText('Scale')).toBeInTheDocument()
    expect(screen.getByText('Erase')).toBeInTheDocument()
  })

  it('renders nothing when a construction guide is selected', () => {
    const { container } = render(<ContextualDock selectedIds={[]} selectedGuide={5n} onRun={vi.fn()} />)
    expect(container.firstChild).toBeNull()
  })

  it('clicking the primary verb calls onRun with its id', () => {
    const onRun = vi.fn()
    render(<ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={onRun} />)
    fireEvent.click(screen.getByText('Push/Pull'))
    expect(onRun).toHaveBeenCalledWith('tool-pushpull')
  })

  it('clicking a secondary verb calls onRun with its id', () => {
    const onRun = vi.fn()
    render(<ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={onRun} />)
    fireEvent.click(screen.getByText('Erase'))
    expect(onRun).toHaveBeenCalledWith('edit-delete')
  })

  // --- Active-tool highlight  ---

  it('does NOT highlight the first (primary) verb when it is not the active tool', () => {
    render(<ContextualDock selectedIds={[]} selectedGuide={null} onRun={vi.fn()} activeToolId="tool-arc" />)
    // Rectangle is first in EMPTY_VERBS (the old "primary" slot) but Arc is
    // the actual active tool here — Rectangle must render unselected.
    const rectangleBtn = screen.getByRole('button', { name: 'Rectangle' })
    expect(rectangleBtn.style.border).toBe('1px solid transparent')
    expect(rectangleBtn).toHaveAttribute('aria-pressed', 'false')
  })

  it('highlights the verb matching activeToolId, wherever it sits in the list', () => {
    render(<ContextualDock selectedIds={[]} selectedGuide={null} onRun={vi.fn()} activeToolId="tool-arc" />)
    const arcBtn = screen.getByRole('button', { name: 'Arc' })
    expect(arcBtn.style.border).toContain('var(--accent-border)')
    // aria-pressed mirrors the highlight — the machine-readable contract the
    // E2E ui-chrome spec asserts through.
    expect(arcBtn).toHaveAttribute('aria-pressed', 'true')
  })

  it('highlights nothing when activeToolId is undefined', () => {
    render(<ContextualDock selectedIds={[]} selectedGuide={null} onRun={vi.fn()} />)
    for (const label of ['Rectangle', 'Line', 'Circle', 'Arc']) {
      const btn = screen.getByRole('button', { name: label })
      expect(btn.style.border).toBe('1px solid transparent')
    }
  })

  it('highlights nothing when activeToolId is not in the current context\'s verb set', () => {
    // Arc has no dock verb in the Object context — nothing should light up.
    render(<ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={vi.fn()} activeToolId="tool-arc" />)
    for (const label of ['Push/Pull', 'Move', 'Paint', 'Erase']) {
      const btn = screen.getByRole('button', { name: label })
      expect(btn.style.border).toBe('1px solid transparent')
    }
  })

  it('cross-fades on context change: container carries.hew-dock and remounts when the context swaps', () => {
    const { container, rerender } = render(
      <ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={vi.fn()} />,
    )
    const before = container.firstChild as HTMLElement
    expect(before.className).toContain('hew-dock')

    rerender(<ContextualDock selectedIds={[group(2n)]} selectedGuide={null} onRun={vi.fn()} />)
    const after = container.firstChild as HTMLElement
    // The container is keyed on the context, so a context swap replaces the
    // DOM node — that remount is what replays the CSS cross-fade animation.
    expect(after).not.toBe(before)
    expect(after.className).toContain('hew-dock')
    expect(screen.getByText('GROUP')).toBeInTheDocument()
  })

  it('does NOT remount (no cross-fade restart) when the selection changes but the context stays the same', () => {
    const { container, rerender } = render(
      <ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={vi.fn()} />,
    )
    const before = container.firstChild as HTMLElement
    rerender(<ContextualDock selectedIds={[obj(2n)]} selectedGuide={null} onRun={vi.fn()} />)
    expect(container.firstChild).toBe(before)
  })

  it('hidden (camera drag): stays mounted but fades to opacity 0 and becomes click-through', () => {
    const { container } = render(
      <ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={vi.fn()} hidden />,
    )
    const dock = container.firstChild as HTMLElement
    expect(dock).not.toBeNull() // mounted, so the fade can transition back in
    expect(dock.style.opacity).toBe('0')
    expect(dock.style.pointerEvents).toBe('none')
  })

  it('reappears when hidden flips back off (drag released)', () => {
    const { container, rerender } = render(
      <ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={vi.fn()} hidden />,
    )
    const before = container.firstChild as HTMLElement
    rerender(<ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={vi.fn()} hidden={false} />)
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
      <ContextualDock selectedIds={[]} selectedGuide={null} onRun={vi.fn()} hoveringSketchRegion />,
    )
    expect(screen.getByText('SKETCH')).toBeInTheDocument()
    expect(screen.getByText('Push/Pull')).toBeInTheDocument()
    expect(screen.queryByText('Rectangle')).not.toBeInTheDocument()
  })

  it('shows the ordinary empty (draw-tool) verb set when hoveringSketchRegion is false', () => {
    render(
      <ContextualDock selectedIds={[]} selectedGuide={null} onRun={vi.fn()} hoveringSketchRegion={false} />,
    )
    expect(screen.getByText('DRAW')).toBeInTheDocument()
    expect(screen.getByText('Rectangle')).toBeInTheDocument()
  })

  it('ignores hoveringSketchRegion entirely once something is actually selected', () => {
    render(
      <ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={vi.fn()} hoveringSketchRegion />,
    )
    // An explicit Object selection always wins — still the Object row, not Sketch.
    expect(screen.getByText('OBJECT')).toBeInTheDocument()
    expect(screen.queryByText('SKETCH')).not.toBeInTheDocument()
  })

  it('ignores hoveringSketchRegion when a construction guide is selected (still no dock)', () => {
    const { container } = render(
      <ContextualDock selectedIds={[]} selectedGuide={5n} onRun={vi.fn()} hoveringSketchRegion />,
    )
    expect(container.firstChild).toBeNull()
  })

  it('clicking the hover-previewed Push/Pull verb dispatches the same id as a real Sketch selection', () => {
    const onRun = vi.fn()
    render(
      <ContextualDock selectedIds={[]} selectedGuide={null} onRun={onRun} hoveringSketchRegion />,
    )
    fireEvent.click(screen.getByText('Push/Pull'))
    expect(onRun).toHaveBeenCalledWith('tool-pushpull')
  })
})
