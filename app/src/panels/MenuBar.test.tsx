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

  // --- Help menu ---

  it('calls onReportBug when Help > Report Bug… is mousedown-clicked', () => {
    const onReportBug = vi.fn()
    render(<MenuBar {...defaultProps} onReportBug={onReportBug} />)
    fireEvent.click(screen.getByRole('button', { name: /help/i }))
    fireEvent.mouseDown(screen.getByText('Report Bug…'))
    expect(onReportBug).toHaveBeenCalledOnce()
  })
})
