/**
 *  — App orchestration tests.
 *
 * Covers:
 *   - Loading state shown before kernel resolves
 *   - Full UI appears after kernel loads
 *   - Tool switching via tool-rail button clicks
 *   - Keyboard shortcuts: Space→Select, bare R→Rectangle (SketchUp-for-Windows
 *     scheme,  — this test file runs under jsdom, which resolves as
 *     non-Mac), Ctrl+Shift+I→toggle Model Info
 *   - Docked tray section collapse/expand state ( — replaced the old
 *     floating, draggable panels; FloatingPanel.tsx deleted)
 *   - Object Info shows "Select an object." when nothing is selected
 *
 * Stubs only:
 *   - src/wasm/loader  (loadKernel — calls a .wasm file that doesn't exist in CI)
 *   - src/viewport/Viewport  (three.js / WebGL do not work in jsdom)
 *
 * All other imports (io, log, settings, recording, etc.) run their real code
 * against the in-memory Storage polyfill provided by src/test/setup.ts.
 */

import { render, screen, fireEvent, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'

/**
 * Scope a query to the top menu bar. Tool/panel names (e.g. "Select",
 * "Model Info") also appear in the status bar and docked-tray section
 * headers, so a bare screen.getByText is ambiguous in the full App; the
 * checkmark state we care about lives only inside the open dropdown, which
 * is within the menu bar.
 */
const menubar = () => within(screen.getByTestId('menu-bar'))

// ---------------------------------------------------------------------------
// Mocks — vi.mock() is hoisted before imports, so these must appear first.
// ---------------------------------------------------------------------------

// A minimal mock Scene sufficient for App to start and render its full UI.
// Shared across all tests in this file; vi.clearAllMocks() resets call counts.
const mockScene = {
  object_ids: () => new BigUint64Array(),
  group_ids: () => new BigUint64Array(),
  instance_ids: () => new BigUint64Array(),
  sketch_ids: () => new BigUint64Array(),
  top_level_nodes: () => [],
  object_name: () => undefined as string | undefined,
  group_name: () => undefined as string | undefined,
  instance_name: () => undefined as string | undefined,
  node_tags: () => [] as string[],
  object_solid: () => true,
  can_scene_undo: () => false,
  can_scene_redo: () => false,
  // save() is called once to snapshot the blank scene for "New" resets.
  save: () => new Uint8Array(),
  load: vi.fn(),
  node_parent: () => undefined as bigint | undefined,
  material_ids: () => new BigUint64Array(),
  material_info: () => undefined,
  material_texture_bytes: () => undefined,
  set_torture_mode: vi.fn(),
  component_member_objects: () => new BigUint64Array(),
  node_leaf_objects: () => new BigUint64Array(),
  set_hidden: vi.fn(),
  group_members: () => [] as { kind: string; id: bigint }[],
  component_name: () => undefined as string | undefined,
  instance_def: () => undefined as bigint | undefined,
  add_node_tag: vi.fn(),
  remove_node_tag: vi.fn(),
  set_node_name: vi.fn(),
  add_material: vi.fn(),
  add_texture_material: vi.fn(),
  set_object_material: vi.fn(),
}

vi.mock('./wasm/loader', () => ({
  loadKernel: vi.fn(() =>
    Promise.resolve({
      version: () => '0.1.0-test',
      demo_mesh: vi.fn(),
      newScene: () => mockScene,
    }),
  ),
}))

// Viewport — renders nothing; viewportApi.current stays null (App guards
// all viewportApi calls with optional chaining).
vi.mock('./viewport/Viewport', () => ({
  default: vi.fn(() => null),
}))

import App from './App'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Render <App /> and wait until the kernel has loaded (i.e. the tool rail
 * becomes visible). Returns when the "Rectangle (R)" row is in DOM.
 */
async function renderAndLoad() {
  render(<App />)
  // The loading state says "Loading kernel…". Wait for it to clear.
  await waitFor(() => screen.getByTitle('Rectangle (R)'), { timeout: 2000 })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('App — loading state', () => {
  it('shows "Loading kernel…" before the kernel resolves', () => {
    // Delay the resolution so we can see the loading state.
    // We still need to let it eventually resolve or the afterEach cleanup
    // will complain about pending state updates — so just check the initial DOM.
    render(<App />)
    expect(screen.getByText(/loading kernel/i)).toBeInTheDocument()
  })
})

describe('App — loaded state', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders the tool rail rows after the kernel loads', async () => {
    await renderAndLoad()
    // A sampling of the rail's rows (title = tool name + shortcut). jsdom
    // resolves as non-Mac, so these use the bare-letter Windows/Linux/Web
    // scheme, not macOS's Cmd-combo one.
    expect(screen.getByTitle('Select (Spc)')).toBeInTheDocument()
    expect(screen.getByTitle('Rectangle (R)')).toBeInTheDocument()
    expect(screen.getByTitle('Push/Pull (P)')).toBeInTheDocument()
  })

  it('renders the web MenuBar (nativeMenuBar=false in jsdom)', async () => {
    await renderAndLoad()
    expect(screen.getByTestId('menu-bar')).toBeInTheDocument()
  })

  it('docked tray: Entity Info and Outliner expanded by default, Materials and Tags collapsed', async () => {
    await renderAndLoad()
    expect(screen.getByRole('button', { name: /entity info/i })).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByRole('button', { name: /outliner/i })).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByRole('button', { name: /^materials$/i })).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByRole('button', { name: /^tags$/i })).toHaveAttribute('aria-expanded', 'false')
  })

  it('Object Info panel shows "Select an object." when selection is empty', async () => {
    await renderAndLoad()
    expect(screen.getByText(/select an object/i)).toBeInTheDocument()
  })
})

describe('App — tool rail tool switching', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('clicking a rail row activates that tool (reflected in Tools menu)', async () => {
    await renderAndLoad()
    // Click the Push/Pull rail row
    fireEvent.click(screen.getByTitle('Push/Pull (P)'))
    // Open the Tools menu
    fireEvent.click(screen.getByRole('button', { name: /^tools$/i }))
    // Push/Pull should now have a checkmark. Scope to the menu bar — the
    // tool rail also renders a visible "Push/Pull" text node now, so
    // a bare screen.getByText is ambiguous.
    const pushPullItem = menubar().getByText('Push/Pull').closest('div')
    expect(pushPullItem?.textContent).toContain('✓')
  })

  it('switching to a new tool clears the checkmark on the previous tool', async () => {
    await renderAndLoad()
    // Activate Rectangle, then switch to Move
    fireEvent.click(screen.getByTitle('Rectangle (R)'))
    fireEvent.click(screen.getByTitle('Move (M)'))
    // Rectangle lives in the Draw menu — it should no longer be checked.
    fireEvent.click(screen.getByRole('button', { name: /^draw$/i }))
    const rectangleItem = menubar().getByText('Rectangle').closest('div')
    expect(rectangleItem?.textContent).not.toContain('✓')
    // Move lives in the Tools menu — it should now be checked (opening Tools
    // closes the Draw dropdown).
    fireEvent.click(screen.getByRole('button', { name: /^tools$/i }))
    const moveItem = menubar().getByText('Move').closest('div')
    expect(moveItem?.textContent).toContain('✓')
  })
})

describe('App — keyboard shortcuts', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('Space activates the Select tool', async () => {
    await renderAndLoad()
    // First switch away from Select so we can detect the change
    fireEvent.click(screen.getByTitle('Rectangle (R)'))
    // Now press Space
    fireEvent.keyDown(document, { key: ' ' })
    // Open Tools menu and check Select has the checkmark
    fireEvent.click(screen.getByRole('button', { name: /^tools$/i }))
    const selectItem = menubar().getByText('Select').closest('div')
    expect(selectItem?.textContent).toContain('✓')
  })

  it('bare R activates the Rectangle tool (SketchUp-for-Windows scheme)', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'r' })
    // Rectangle lives in the Draw menu, not Tools.
    fireEvent.click(screen.getByRole('button', { name: /^draw$/i }))
    const rectangleItem = menubar().getByText('Rectangle').closest('div')
    expect(rectangleItem?.textContent).toContain('✓')
  })

  it('bare M activates the Move tool (SketchUp-for-Windows scheme)', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'm' })
    fireEvent.click(screen.getByRole('button', { name: /^tools$/i }))
    const moveItem = menubar().getByText('Move').closest('div')
    expect(moveItem?.textContent).toContain('✓')
  })

  it('Ctrl+R (modified) still activates the Pan camera tool, not bare-letter Rectangle', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'r', ctrlKey: true })
    fireEvent.click(screen.getByRole('button', { name: /^camera$/i }))
    const panItem = menubar().getByText('Pan').closest('div')
    expect(panItem?.textContent).toContain('✓')
  })

  it('Ctrl+K opens the command palette', async () => {
    await renderAndLoad()
    expect(screen.queryByRole('dialog', { name: /command palette/i })).not.toBeInTheDocument()
    fireEvent.keyDown(document, { key: 'k', ctrlKey: true })
    expect(screen.getByRole('dialog', { name: /command palette/i })).toBeInTheDocument()
  })

  it('selecting a palette result runs the same action as its rail row', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'k', ctrlKey: true })
    fireEvent.change(screen.getByPlaceholderText(/search tools, actions, help/i), { target: { value: 'rotate' } })
    // Scope to the dialog — the rail underneath also renders a "Rotate" row.
    fireEvent.click(within(screen.getByRole('dialog')).getByText('Rotate'))
    // Palette closes and the same menuActionRef dispatch the rail/menu use ran.
    expect(screen.queryByRole('dialog', { name: /command palette/i })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^tools$/i }))
    const rotateItem = menubar().getByText('Rotate').closest('div')
    expect(rotateItem?.textContent).toContain('✓')
  })
})

describe('App — panel toggle', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('Ctrl+Shift+I collapses the Outliner tray section', async () => {
    await renderAndLoad()
    expect(screen.getByRole('button', { name: /outliner/i })).toHaveAttribute('aria-expanded', 'true')
    fireEvent.keyDown(document, { key: 'I', ctrlKey: true, shiftKey: true })
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /outliner/i })).toHaveAttribute('aria-expanded', 'false'),
    )
  })

  it('Ctrl+Shift+I re-expands the Outliner section when pressed again', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'I', ctrlKey: true, shiftKey: true })
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /outliner/i })).toHaveAttribute('aria-expanded', 'false'),
    )
    fireEvent.keyDown(document, { key: 'I', ctrlKey: true, shiftKey: true })
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /outliner/i })).toHaveAttribute('aria-expanded', 'true'),
    )
  })

  it('clicking the Outliner section header collapses it (the tray has no close button)', async () => {
    await renderAndLoad()
    fireEvent.click(screen.getByRole('button', { name: /outliner/i }))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /outliner/i })).toHaveAttribute('aria-expanded', 'false'),
    )
  })

  it('Window menu checkmark tracks the Model Info visibility', async () => {
    await renderAndLoad()
    // Initially showModelInfo=true — Window > Model Info has checkmark
    fireEvent.click(screen.getByRole('button', { name: /^window$/i }))
    const modelInfoItem = menubar().getByText('Model Info').closest('div')
    expect(modelInfoItem?.textContent).toContain('✓')

    // Toggle off via keyboard
    fireEvent.keyDown(document, { key: 'I', ctrlKey: true, shiftKey: true })
    // Close the open dropdown first (fireEvent.mouseDown on outside element)
    fireEvent.mouseDown(document.body)
    // Re-open Window menu
    await waitFor(() => {
      fireEvent.click(screen.getByRole('button', { name: /^window$/i }))
      const item = menubar().getByText('Model Info').closest('div')
      expect(item?.textContent).not.toContain('✓')
    })
  })
})
