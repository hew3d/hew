import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { ToolRail, RAIL_NARROW_WIDTH, RAIL_WIDE_WIDTH } from './ToolRail'
import { RAIL_GROUPS, toolsInGroup } from '../tools/toolRegistry'

const railTools = RAIL_GROUPS.flatMap((group) => toolsInGroup(group).map((t) => t.name))

describe('ToolRail narrow mode', () => {
  it('keeps every rail tool one click away', () => {
    const onSelectTool = vi.fn()
    render(
      <ToolRail
        activeTool="Select"
        onSelectTool={onSelectTool}
        onOpenPalette={vi.fn()}
        onOpenLibrary={vi.fn()}
        narrow
        onToggleNarrow={vi.fn()}
      />,
    )
    expect(railTools.length).toBeGreaterThan(0)
    for (const name of railTools) {
      const row = screen.getByRole('radio', { name })
      fireEvent.click(row)
      expect(onSelectTool).toHaveBeenLastCalledWith(name)
      expect(row).toHaveAttribute('title', expect.stringContaining(name))
    }
    expect(screen.getByRole('button', { name: 'Library' })).toBeInTheDocument()
  })

  it('drops the palette field and swaps the rail width', () => {
    const { rerender } = render(
      <ToolRail activeTool="Select" onSelectTool={vi.fn()} onOpenPalette={vi.fn()} narrow onToggleNarrow={vi.fn()} />,
    )
    expect(screen.queryByRole('button', { name: 'Search tools, actions, help' })).toBeNull()
    expect(screen.getByRole('radiogroup', { name: 'Tools' })).toHaveStyle({ width: `${RAIL_NARROW_WIDTH}px` })

    rerender(
      <ToolRail activeTool="Select" onSelectTool={vi.fn()} onOpenPalette={vi.fn()} narrow={false} onToggleNarrow={vi.fn()} />,
    )
    expect(screen.getByRole('button', { name: 'Search tools, actions, help' })).toBeInTheDocument()
    expect(screen.getByRole('radiogroup', { name: 'Tools' })).toHaveStyle({ width: `${RAIL_WIDE_WIDTH}px` })
  })

  it('the chevron toggles and its label follows the state', () => {
    const onToggleNarrow = vi.fn()
    const { rerender } = render(
      <ToolRail activeTool="Select" onSelectTool={vi.fn()} narrow={false} onToggleNarrow={onToggleNarrow} />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Collapse tool rail' }))
    expect(onToggleNarrow).toHaveBeenCalledTimes(1)

    rerender(<ToolRail activeTool="Select" onSelectTool={vi.fn()} narrow onToggleNarrow={onToggleNarrow} />)
    expect(screen.getByRole('button', { name: 'Expand tool rail' })).toBeInTheDocument()
    expect(screen.getAllByRole('separator')).toHaveLength(RAIL_GROUPS.length)
  })

  it('hides the chevron when no toggle handler is given', () => {
    render(<ToolRail activeTool="Select" onSelectTool={vi.fn()} />)
    expect(screen.queryByRole('button', { name: /tool rail/ })).toBeNull()
  })
})
