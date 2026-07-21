/**
 *  — component tests for MenuBar.
 *
 * MenuBar is the in-app web menu bar (rendered when nativeMenuBar=false).
 * Tests assert real interaction: menu items invoke callbacks, tool checkmarks
 * update on activeTool, and the native-menu guard returns null.
 */

import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { MenuBar, type MenuBarProps } from './MenuBar'
import { TOOLS } from '../tools/toolRegistry'

const defaultProps: MenuBarProps = {
  name: 'Untitled',
  saveState: '',
  nativeMenuBar: false,
  onNew: vi.fn(),
  onOpen: vi.fn(),
  onSave: vi.fn(),
  onSaveAs: vi.fn(),
  onImport: vi.fn(),
  onExport: vi.fn(),
  onUndo: vi.fn(),
  onRedo: vi.fn(),
  canUndo: false,
  canRedo: false,
}

describe('MenuBar', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders nothing when nativeMenuBar=true', () => {
    const { container } = render(<MenuBar {...defaultProps} nativeMenuBar />)
    expect(container.firstChild).toBeNull()
  })

  it('renders the bar element when nativeMenuBar=false', () => {
    render(<MenuBar {...defaultProps} />)
    expect(screen.getByTestId('menu-bar')).toBeInTheDocument()
  })

  it('shows the document name in the bar', () => {
    render(<MenuBar {...defaultProps} name="myfile.hew" />)
    expect(screen.getByText('myfile.hew')).toBeInTheDocument()
  })

  it('shows the save-state indicator when non-empty', () => {
    render(<MenuBar {...defaultProps} name="myfile.hew" saveState="Saved 2 minutes ago" />)
    expect(screen.getByText('Saved 2 minutes ago')).toBeInTheDocument()
  })

  it('omits the save-state indicator when empty', () => {
    render(<MenuBar {...defaultProps} name="myfile.hew" saveState="" />)
    expect(screen.queryByText(/edited|saved/i)).not.toBeInTheDocument()
  })

  // --- Command palette resting field: moved to ToolRail ---

  it('never renders a palette search field (it lives in ToolRail since)', () => {
    render(<MenuBar {...defaultProps} />)
    expect(screen.queryByText(/search/i)).not.toBeInTheDocument()
  })

  // --- File menu ---

  it('opens the File dropdown when File is clicked', () => {
    render(<MenuBar {...defaultProps} />)
    expect(screen.queryByText('New')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /file/i }))
    expect(screen.getByText('New')).toBeInTheDocument()
  })

  it('calls onNew when File > New is mousedown-clicked', () => {
    const onNew = vi.fn()
    render(<MenuBar {...defaultProps} onNew={onNew} />)
    fireEvent.click(screen.getByRole('button', { name: /file/i }))
    fireEvent.mouseDown(screen.getByText('New'))
    expect(onNew).toHaveBeenCalledOnce()
  })

  it('calls onSave when File > Save is mousedown-clicked', () => {
    const onSave = vi.fn()
    render(<MenuBar {...defaultProps} onSave={onSave} />)
    fireEvent.click(screen.getByRole('button', { name: /file/i }))
    fireEvent.mouseDown(screen.getByText('Save'))
    expect(onSave).toHaveBeenCalledOnce()
  })

  it('calls onOpen when File > Open… is mousedown-clicked', () => {
    const onOpen = vi.fn()
    render(<MenuBar {...defaultProps} onOpen={onOpen} />)
    fireEvent.click(screen.getByRole('button', { name: /file/i }))
    fireEvent.mouseDown(screen.getByText('Open…'))
    expect(onOpen).toHaveBeenCalledOnce()
  })

  // --- Edit menu ---

  it('shows Undo disabled when canUndo=false', () => {
    render(<MenuBar {...defaultProps} canUndo={false} />)
    fireEvent.click(screen.getByRole('button', { name: /edit/i }))
    // Disabled menu items have grey color (opacity check is harder, but they exist)
    expect(screen.getByText('Undo')).toBeInTheDocument()
  })

  it('calls onUndo when Edit > Undo is mousedown-clicked and canUndo=true', () => {
    const onUndo = vi.fn()
    render(<MenuBar {...defaultProps} canUndo={true} onUndo={onUndo} />)
    fireEvent.click(screen.getByRole('button', { name: /edit/i }))
    fireEvent.mouseDown(screen.getByText('Undo'))
    expect(onUndo).toHaveBeenCalledOnce()
  })

  it('does NOT call onUndo when Edit > Undo is clicked but canUndo=false', () => {
    const onUndo = vi.fn()
    render(<MenuBar {...defaultProps} canUndo={false} onUndo={onUndo} />)
    fireEvent.click(screen.getByRole('button', { name: /edit/i }))
    fireEvent.mouseDown(screen.getByText('Undo'))
    expect(onUndo).not.toHaveBeenCalled()
  })

  // --- Edit menu: object commands (relocated here from the Outliner's
  // per-object buttons — this menu is now their only static UI surface) ---

  const openGates = {
    canGroup: true,
    canUngroup: true,
    canMakeComponent: true,
    canPlaceCopy: true,
    canExplode: true,
    canMakeUnique: true,
    canBoolean: true,
  }

  it('dispatches every object command id through onEditAction when its gate is open', () => {
    const onEditAction = vi.fn()
    render(<MenuBar {...defaultProps} editGates={openGates} onEditAction={onEditAction} />)
    const commands: Array<[string, string]> = [
      ['Group', 'edit-group'],
      ['Ungroup', 'edit-ungroup'],
      ['Make Component', 'edit-make-component'],
      ['Place Copy', 'edit-place-copy'],
      ['Explode', 'edit-explode'],
      ['Make Unique', 'edit-make-unique'],
      ['Union', 'edit-union'],
      ['Subtract', 'edit-subtract'],
      ['Intersect', 'edit-intersect'],
    ]
    for (const [label, action] of commands) {
      // Each dispatch closes the dropdown (withClose), so re-open per item.
      fireEvent.click(screen.getByRole('button', { name: /edit/i }))
      fireEvent.mouseDown(screen.getByText(label))
      expect(onEditAction).toHaveBeenLastCalledWith(action)
    }
    expect(onEditAction).toHaveBeenCalledTimes(commands.length)
  })

  it('does NOT dispatch a gated-off object command (Group with canGroup=false)', () => {
    const onEditAction = vi.fn()
    render(
      <MenuBar
        {...defaultProps}
        editGates={{ ...openGates, canGroup: false }}
        onEditAction={onEditAction}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /edit/i }))
    fireEvent.mouseDown(screen.getByText('Group'))
    expect(onEditAction).not.toHaveBeenCalled()
  })

  // --- Tools menu: active tool checkmark ---

  it('shows a checkmark next to the active tool in the Tools menu', () => {
    render(<MenuBar {...defaultProps} activeTool="Move" onSelectTool={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /tools/i }))
    // The checked item has a ✓ prefix rendered as text
    // Find the Move item's parent span that contains the checkmark span
    const moveItem = screen.getByText('Move').closest('div')
    expect(moveItem?.textContent).toContain('✓')
  })

  it('shows no checkmark next to inactive tools', () => {
    render(<MenuBar {...defaultProps} activeTool="Move" onSelectTool={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /tools/i }))
    // Rotate is in the list but not active — its container should not have ✓
    const rotateEl = screen.getByText('Rotate')
    // The rotate item div's full text should not contain ✓
    expect(rotateEl.closest('div')?.textContent).not.toContain('✓')
  })

  it('calls onSelectTool with "Push/Pull" when that menu item is mousedown-clicked', () => {
    const onSelectTool = vi.fn()
    render(<MenuBar {...defaultProps} activeTool="Select" onSelectTool={onSelectTool} />)
    fireEvent.click(screen.getByRole('button', { name: /tools/i }))
    fireEvent.mouseDown(screen.getByText('Push/Pull'))
    expect(onSelectTool).toHaveBeenCalledWith('Push/Pull')
  })

  // --- Rail ↔ menu parity ---

  it('every registry tool is reachable from the Draw/Tools/Camera dropdowns (no rail↔menu drift)', () => {
    // This menu bar is the ONLY menu on the web build and Windows/Linux
    // Tauri, and its items are hand-written — a tool added to the registry
    // (rail, palette, shortcuts) but not here would be invisible in the
    // menus on those platforms. Same drift-pinning spirit as the palette's
    // registry test.
    render(<MenuBar {...defaultProps} activeTool="Select" onSelectTool={vi.fn()} />)
    const seen = new Set<string>()
    for (const menu of [/draw/i, /^tools$/i, /camera/i]) {
      fireEvent.click(screen.getByRole('button', { name: menu }))
      for (const tool of TOOLS) {
        if (screen.queryByText(tool) !== null) seen.add(tool)
      }
    }
    for (const tool of TOOLS) {
      expect(seen.has(tool), `tool "${tool}" is missing from the web menu bar`).toBe(true)
    }
  })

  // --- Window menu: panel toggles ---

  it('shows a checkmark next to Model Info when showModelInfo=true', () => {
    render(<MenuBar {...defaultProps} showModelInfo={true} onToggleModelInfo={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /window/i }))
    const modelInfoEl = screen.getByText('Model Info')
    expect(modelInfoEl.closest('div')?.textContent).toContain('✓')
  })

  it('calls onToggleModelInfo when Window > Model Info is mousedown-clicked', () => {
    const onToggleModelInfo = vi.fn()
    render(<MenuBar {...defaultProps} showModelInfo={true} onToggleModelInfo={onToggleModelInfo} />)
    fireEvent.click(screen.getByRole('button', { name: /window/i }))
    fireEvent.mouseDown(screen.getByText('Model Info'))
    expect(onToggleModelInfo).toHaveBeenCalledOnce()
  })

  it('calls onToggleMaterials when Window > Materials is mousedown-clicked', () => {
    const onToggleMaterials = vi.fn()
    render(<MenuBar {...defaultProps} showMaterials={false} onToggleMaterials={onToggleMaterials} />)
    fireEvent.click(screen.getByRole('button', { name: /window/i }))
    fireEvent.mouseDown(screen.getByText('Materials'))
    expect(onToggleMaterials).toHaveBeenCalledOnce()
  })

  // --- Close on outside click ---

  it('closes an open dropdown when the user clicks outside the bar', () => {
    render(
      <div>
        <MenuBar {...defaultProps} />
        <div data-testid="outside">Outside</div>
      </div>,
    )
    fireEvent.click(screen.getByRole('button', { name: /file/i }))
    expect(screen.getByText('New')).toBeInTheDocument()
    fireEvent.mouseDown(screen.getByTestId('outside'))
    expect(screen.queryByText('New')).not.toBeInTheDocument()
  })

  // --- View menu ---

  it('shows a checkmark for Axes when showAxes=true', () => {
    render(<MenuBar {...defaultProps} showAxes={true} onToggleAxes={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
    const axesEl = screen.getByText('Axes')
    expect(axesEl.closest('div')?.textContent).toContain('✓')
  })

  it('calls onToggleAxes when View > Axes is mousedown-clicked', () => {
    const onToggleAxes = vi.fn()
    render(<MenuBar {...defaultProps} showAxes={true} onToggleAxes={onToggleAxes} />)
    fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
    fireEvent.mouseDown(screen.getByText('Axes'))
    expect(onToggleAxes).toHaveBeenCalledOnce()
  })

  it('shows a checkmark for Grid when showGrid=true', () => {
    render(<MenuBar {...defaultProps} showGrid={true} onToggleGrid={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
    const gridEl = screen.getByText('Grid')
    expect(gridEl.closest('div')?.textContent).toContain('✓')
  })

  it('calls onToggleGrid when View > Grid is mousedown-clicked', () => {
    const onToggleGrid = vi.fn()
    render(<MenuBar {...defaultProps} showGrid={false} onToggleGrid={onToggleGrid} />)
    fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
    const gridEl = screen.getByText('Grid')
    expect(gridEl.closest('div')?.textContent).not.toContain('✓')
    fireEvent.mouseDown(gridEl)
    expect(onToggleGrid).toHaveBeenCalledOnce()
  })

  // --- View > Section Plane (D3, section-plane-polish) ---

  it('shows no checkmark for Section Plane when no section is placed (sectionPlaneChecked=false)', () => {
    render(<MenuBar {...defaultProps} sectionPlaneChecked={false} sectionPlaneExists={false} />)
    fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
    const el = screen.getByText('Section Plane')
    expect(el.closest('div')?.textContent).not.toContain('✓')
  })

  it('shows a checkmark for Section Plane only when a section is placed AND active', () => {
    render(<MenuBar {...defaultProps} sectionPlaneChecked={true} sectionPlaneExists={true} />)
    fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
    const el = screen.getByText('Section Plane')
    expect(el.closest('div')?.textContent).toContain('✓')
  })

  it('a section placed but INACTIVE (dashed widget) shows unchecked, not checked', () => {
    render(<MenuBar {...defaultProps} sectionPlaneChecked={false} sectionPlaneExists={true} />)
    fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
    const el = screen.getByText('Section Plane')
    expect(el.closest('div')?.textContent).not.toContain('✓')
  })

  it('calls onToggleSectionActive when View > Section Plane is mousedown-clicked while a section exists', () => {
    const onToggleSectionActive = vi.fn()
    render(
      <MenuBar
        {...defaultProps}
        sectionPlaneChecked={true}
        sectionPlaneExists={true}
        onToggleSectionActive={onToggleSectionActive}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
    fireEvent.mouseDown(screen.getByText('Section Plane'))
    expect(onToggleSectionActive).toHaveBeenCalledOnce()
  })

  it('View > Section Plane is disabled and swallows clicks when no section is placed', () => {
    const onToggleSectionActive = vi.fn()
    render(
      <MenuBar
        {...defaultProps}
        sectionPlaneChecked={false}
        sectionPlaneExists={false}
        onToggleSectionActive={onToggleSectionActive}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
    fireEvent.mouseDown(screen.getByText('Section Plane'))
    expect(onToggleSectionActive).not.toHaveBeenCalled()
  })

  it('Tools menu no longer has a standalone "Toggle Section Active" command (moved to View)', () => {
    render(<MenuBar {...defaultProps} />)
    fireEvent.click(screen.getByRole('button', { name: /tools/i }))
    expect(screen.queryByText('Toggle Section Active')).not.toBeInTheDocument()
    // The Section Plane TOOL selector itself is untouched.
    expect(screen.getByText('Section Plane')).toBeInTheDocument()
  })

  // --- Help menu ---

  it('calls onReportBug when Help > Report Bug… is mousedown-clicked', () => {
    const onReportBug = vi.fn()
    render(<MenuBar {...defaultProps} onReportBug={onReportBug} />)
    fireEvent.click(screen.getByRole('button', { name: /help/i }))
    fireEvent.mouseDown(screen.getByText('Report Bug…'))
    expect(onReportBug).toHaveBeenCalledOnce()
  })

  // --- Native menu-mode semantics ---

  it('hovering another trigger while a menu is open switches to it (menu mode)', () => {
    render(<MenuBar {...defaultProps} />)
    fireEvent.click(screen.getByRole('button', { name: /file/i }))
    expect(screen.getByText('New')).toBeInTheDocument()
    // Hover Edit — no click — and the Edit menu replaces the File menu.
    fireEvent.mouseEnter(screen.getByRole('button', { name: /edit/i }))
    expect(screen.queryByText('New')).not.toBeInTheDocument()
    expect(screen.getByText('Undo')).toBeInTheDocument()
  })

  it('hovering a trigger with no menu open does NOT open one', () => {
    render(<MenuBar {...defaultProps} />)
    fireEvent.mouseEnter(screen.getByRole('button', { name: /file/i }))
    expect(screen.queryByText('New')).not.toBeInTheDocument()
  })

  it('clicking the empty part of the bar leaves menu mode', () => {
    render(<MenuBar {...defaultProps} />)
    fireEvent.click(screen.getByRole('button', { name: /file/i }))
    expect(screen.getByText('New')).toBeInTheDocument()
    // Dead-space clicks have the bar itself as target.
    fireEvent.mouseDown(screen.getByTestId('menu-bar'))
    expect(screen.queryByText('New')).not.toBeInTheDocument()
  })

  it('clicking outside the bar leaves menu mode', () => {
    render(<MenuBar {...defaultProps} />)
    fireEvent.click(screen.getByRole('button', { name: /file/i }))
    expect(screen.getByText('New')).toBeInTheDocument()
    fireEvent.mouseDown(document.body)
    expect(screen.queryByText('New')).not.toBeInTheDocument()
  })

  it('Escape leaves menu mode', () => {
    render(<MenuBar {...defaultProps} />)
    fireEvent.click(screen.getByRole('button', { name: /file/i }))
    expect(screen.getByText('New')).toBeInTheDocument()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByText('New')).not.toBeInTheDocument()
  })

  // --- Settings gear ---

  it('shows the settings gear when onOpenSettings is provided, and it opens settings', () => {
    const onOpenSettings = vi.fn()
    render(<MenuBar {...defaultProps} onOpenSettings={onOpenSettings} />)
    fireEvent.click(screen.getByRole('button', { name: /^settings$/i }))
    expect(onOpenSettings).toHaveBeenCalledOnce()
  })

  it('omits the settings gear when onOpenSettings is not provided', () => {
    render(<MenuBar {...defaultProps} />)
    expect(screen.queryByRole('button', { name: /^settings$/i })).not.toBeInTheDocument()
  })

  it('no longer offers Settings… in the Window menu (the gear replaced it)', () => {
    render(<MenuBar {...defaultProps} onOpenSettings={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /window/i }))
    expect(screen.getByText('Model Info')).toBeInTheDocument()
    expect(screen.queryByText('Settings…')).not.toBeInTheDocument()
  })

  // --- Rail <-> menu parity ---

  it('menu parity: every TOOL_REGISTRY tool is selectable from the Draw/Tools/Camera menus', () => {
    // The tool registry is the single source of truth for which tools
    // exist; this hand-written menu must never silently fall behind it
    // (Follow Me shipped wired into the dispatcher but missing from both
    // menu surfaces — this pins the web one).
    const onSelectTool = vi.fn()
    render(<MenuBar {...defaultProps} activeTool="Select" onSelectTool={onSelectTool} />)
    for (const tool of TOOLS) {
      let item: HTMLElement | null = null
      for (const menu of ['draw', 'tools', 'camera']) {
        fireEvent.click(screen.getByRole('button', { name: new RegExp(`^${menu}$`, 'i') }))
        item = screen.queryByText(tool)
        if (item !== null) break
      }
      expect(item, `tool "${tool}" is missing from every menu`).not.toBeNull()
      fireEvent.mouseDown(item as HTMLElement)
      expect(onSelectTool).toHaveBeenLastCalledWith(tool)
    }
  })
})
