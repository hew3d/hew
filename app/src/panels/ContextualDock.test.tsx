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
  })

  it('shows the Multi verb set for more than one selected node', () => {
    render(<ContextualDock selectedIds={[obj(1n), group(2n)]} selectedGuide={null} onRun={vi.fn()} />)
    expect(screen.getByText('MULTI')).toBeInTheDocument()
    expect(screen.getByText('Move')).toBeInTheDocument()
    expect(screen.getByText('Erase')).toBeInTheDocument()
    expect(screen.queryByText('Paint')).not.toBeInTheDocument()
  })

  it('renders nothing for a selected free-standing sketch', () => {
    const { container } = render(<ContextualDock selectedIds={[sketch(1n)]} selectedGuide={null} onRun={vi.fn()} />)
    expect(container.firstChild).toBeNull()
  })

  it('renders nothing when a construction guide is selected', () => {
    const { container } = render(<ContextualDock selectedIds={[]} selectedGuide={5n} onRun={vi.fn()} />)
    expect(container.firstChild).toBeNull()
  })

  it('clicking the primary verb calls onRun with its id', () => {
    const onRun = vi.fn()
    render(<ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={onRun} />)
    fireEvent.click(screen.getByText('Move'))
    expect(onRun).toHaveBeenCalledWith('tool-move')
  })

  it('clicking a secondary verb calls onRun with its id', () => {
    const onRun = vi.fn()
    render(<ContextualDock selectedIds={[obj(1n)]} selectedGuide={null} onRun={onRun} />)
    fireEvent.click(screen.getByText('Erase'))
    expect(onRun).toHaveBeenCalledWith('edit-delete')
  })
})
