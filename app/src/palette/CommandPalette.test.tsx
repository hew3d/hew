/**
 * Component tests for CommandPalette.
 *
 * jsdom provides a real localStorage, so palette/recency.ts needs no mocking
 * here (unlike its own recency.test.ts, which runs under the 'node' env).
 */
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { CommandPalette } from './CommandPalette'
import { clearRecent } from './recency'

beforeEach(() => {
  clearRecent()
})

describe('CommandPalette', () => {
  it('renders nothing when closed', () => {
    const { container } = render(<CommandPalette open={false} onClose={vi.fn()} onRun={vi.fn()} />)
    expect(container.firstChild).toBeNull()
  })

  it('renders the search dialog when open', () => {
    render(<CommandPalette open onClose={vi.fn()} onRun={vi.fn()} />)
    expect(screen.getByRole('dialog', { name: /command palette/i })).toBeInTheDocument()
    expect(screen.getByPlaceholderText(/search tools, actions, help/i)).toBeInTheDocument()
  })

  it('shows tool results grouped under "Tools" by default (empty query)', () => {
    render(<CommandPalette open onClose={vi.fn()} onRun={vi.fn()} />)
    expect(screen.getByText('Tools')).toBeInTheDocument()
    expect(screen.getByText('Select')).toBeInTheDocument()
  })

  it('typing narrows the results to matches', () => {
    render(<CommandPalette open onClose={vi.fn()} onRun={vi.fn()} />)
    fireEvent.change(screen.getByPlaceholderText(/search tools, actions, help/i), { target: { value: 'rotate' } })
    expect(screen.getByText('Rotate')).toBeInTheDocument()
    expect(screen.queryByText('Select')).not.toBeInTheDocument()
  })

  it('shows a "no matches" message for a query that matches nothing', () => {
    render(<CommandPalette open onClose={vi.fn()} onRun={vi.fn()} />)
    fireEvent.change(screen.getByPlaceholderText(/search tools, actions, help/i), { target: { value: 'zzz-nope' } })
    expect(screen.getByText(/no matches/i)).toBeInTheDocument()
  })

  it('searches dynamic Model entries alongside static ones and runs their jump id', () => {
    const onRun = vi.fn()
    const extraEntries = [
      {
        id: 'jump-tag:["Projector"]',
        label: 'Tag: Projector',
        description: 'Reveal this tag in the Tags panel.',
        group: 'Model' as const,
        synonyms: ['Projector'],
      },
      {
        id: 'jump-node:instance:7',
        label: 'Component: Panasonic Projector',
        description: 'Select it and reveal it in the Outliner.',
        group: 'Model' as const,
        synonyms: ['Panasonic Projector'],
      },
    ]
    render(<CommandPalette open onClose={vi.fn()} onRun={onRun} extraEntries={extraEntries} />)
    fireEvent.change(screen.getByPlaceholderText(/search tools, actions, help/i), { target: { value: 'pr' } })
    // Static tool still matches ("pr" → Protractor) AND both Model entries do.
    expect(screen.getByText('Protractor')).toBeInTheDocument()
    expect(screen.getByText('Model')).toBeInTheDocument()
    expect(screen.getByText('Tag: Projector')).toBeInTheDocument()
    expect(screen.getByText('Component: Panasonic Projector')).toBeInTheDocument()
    fireEvent.click(screen.getByText('Tag: Projector'))
    expect(onRun).toHaveBeenCalledWith('jump-tag:["Projector"]')
  })

  it('Escape calls onClose', () => {
    const onClose = vi.fn()
    render(<CommandPalette open onClose={onClose} onRun={vi.fn()} />)
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' })
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('Enter runs the first (auto-selected) result and closes', () => {
    const onClose = vi.fn()
    const onRun = vi.fn()
    render(<CommandPalette open onClose={onClose} onRun={onRun} />)
    fireEvent.change(screen.getByPlaceholderText(/search tools, actions, help/i), { target: { value: 'rotate' } })
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Enter' })
    expect(onRun).toHaveBeenCalledWith('tool-rotate')
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('ArrowDown moves the selection before Enter runs it', () => {
    const onRun = vi.fn()
    render(<CommandPalette open onClose={vi.fn()} onRun={onRun} />)
    // Empty-query order starts with the default suggestions: Select, Push/Pull, Save, Undo.
    const dialog = screen.getByRole('dialog')
    fireEvent.keyDown(dialog, { key: 'ArrowDown' })
    fireEvent.keyDown(dialog, { key: 'Enter' })
    expect(onRun).toHaveBeenCalledWith('tool-pushpull')
  })

  it('clicking a result runs it and closes', () => {
    const onClose = vi.fn()
    const onRun = vi.fn()
    render(<CommandPalette open onClose={onClose} onRun={onRun} />)
    fireEvent.change(screen.getByPlaceholderText(/search tools, actions, help/i), { target: { value: 'rotate' } })
    fireEvent.click(screen.getByText('Rotate'))
    expect(onRun).toHaveBeenCalledWith('tool-rotate')
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('clicking the backdrop calls onClose', () => {
    const onClose = vi.fn()
    render(<CommandPalette open onClose={onClose} onRun={vi.fn()} />)
    fireEvent.mouseDown(screen.getByRole('dialog').parentElement!)
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('resets the query when reopened', () => {
    const { rerender } = render(<CommandPalette open onClose={vi.fn()} onRun={vi.fn()} />)
    fireEvent.change(screen.getByPlaceholderText(/search tools, actions, help/i), { target: { value: 'rotate' } })
    rerender(<CommandPalette open={false} onClose={vi.fn()} onRun={vi.fn()} />)
    rerender(<CommandPalette open onClose={vi.fn()} onRun={vi.fn()} />)
    expect(screen.getByPlaceholderText(/search tools, actions, help/i)).toHaveValue('')
  })
})
