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
 *   - Object Info renders no boilerplate when nothing is selected
 *
 * Stubs only:
 *   - src/wasm/loader  (loadKernel — calls a .wasm file that doesn't exist in CI)
 *   - src/viewport/Viewport  (three.js / WebGL do not work in jsdom)
 *
 * All other imports (io, log, settings, recording, etc.) run their real code
 * against the in-memory Storage polyfill provided by src/test/setup.ts.
 */

import { render, screen, fireEvent, waitFor, within, act } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'

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
  top_level_nodes: (): { kind: string; id: bigint }[] => [],
  object_name: () => undefined as string | undefined,
  group_name: () => undefined as string | undefined,
  instance_name: () => undefined as string | undefined,
  node_tags: () => [] as string[],
  tag_meta_paths: () => [] as string[],
  tag_meta_hidden: () => new Uint8Array(),
  set_tag_hidden: vi.fn(),
  user_hidden_kinds: () => new Uint8Array(),
  user_hidden_ids: () => new BigUint64Array(),
  node_user_hidden: () => false,
  set_node_user_hidden: vi.fn(),
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
  // Object Info's Bounding Box row (objectBounds.worldBoundsForSelection) reads
  // per-object render meshes; a mesh-less stub keeps it a no-op here.
  object_mesh: () => ({ positions: () => new Float32Array(), free: () => {} }),
  instance_pose: () => undefined as Float64Array | undefined,
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
  import_skp: vi.fn(() => ({
    objects_created: 0,
    watertight: 0,
    leaky: 0,
    skipped: [] as { name: string; reason: string }[],
    textures_missing: [] as string[],
    warnings: [] as string[],
  })),
  import_stl: vi.fn(() => ({
    objects_created: 0,
    watertight: 0,
    leaky: 0,
    skipped: [] as { name: string; reason: string }[],
    textures_missing: [] as string[],
    warnings: [] as string[],
  })),
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

// io/fileHost — only makeFileHost is mocked (as a spy defaulting to the real
// implementation) so the "import seeds hidden state" test can substitute a
// FileHost stub whose openForImport() resolves without a real file dialog.
// Every other test never triggers Open/Import, so the real WebFileHost is
// harmless there.
vi.mock('./io/fileHost', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./io/fileHost')>()
  return { ...actual, makeFileHost: vi.fn(actual.makeFileHost) }
})

// io/recoveryStore — only makeRecoveryStore is mocked (jsdom has no
// IndexedDB, so the real WebRecoveryStore silently no-ops and could never
// report a snapshot). Tests seed `recoveryState.listings` to simulate a
// crash snapshot awaiting recovery; shouldPromptRecovery stays real.
const recoveryState = vi.hoisted(() => ({
  listings: [] as { slot: string; meta: { version: 1; savedAt: number; name: string; path: string | null } }[],
}))
vi.mock('./io/recoveryStore', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./io/recoveryStore')>()
  return {
    ...actual,
    makeRecoveryStore: () => ({
      write: async () => {},
      list: async () => recoveryState.listings,
      claim: async () => null,
      clear: async () => {},
      discardAll: async () => {},
    }),
  }
})

import App from './App'
import Viewport from './viewport/Viewport'
import { getTrayLayout, setTrayLayout, DEFAULT_TRAY_LAYOUT } from './settings/trayLayout'
import { setShowWelcome } from './settings/welcomeScreen'
import { resetStlImportUnitForTest } from './settings/stlImportUnit'
import { makeFileHost, type FileHost } from './io/fileHost'
import { isPristineDocument } from './App'
import type { DocSessionState } from './io/documentSession'
import type { Scene } from './wasm/loader'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// Tray-section state persists across renders now ( trayLayout
// singleton), so tests that collapse a section would otherwise leak that
// state into every later render of <App />. Reset before each test —
// individual tests then seed their own layout where needed.
beforeEach(() => {
  setTrayLayout(DEFAULT_TRAY_LAYOUT)
  // A bare launch would (correctly) open the welcome screen over the app;
  // these tests exercise other surfaces, so opt out. The welcome screen's
  // own launch test re-enables it explicitly.
  setShowWelcome(false)
  // No crash snapshot unless a test seeds one.
  recoveryState.listings = []
})

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
    expect(screen.getByTitle('Arc (A)')).toBeInTheDocument()
    expect(screen.getByTitle('Push/Pull (P)')).toBeInTheDocument()
  })

  it('renders the web MenuBar (nativeMenuBar=false in jsdom)', async () => {
    await renderAndLoad()
    expect(screen.getByTestId('menu-bar')).toBeInTheDocument()
  })

  it('docked tray: Object Info and Outliner expanded by default, Materials and Tags collapsed', async () => {
    await renderAndLoad()
    expect(screen.getByRole('button', { name: /object info/i })).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByRole('button', { name: /outliner/i })).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByRole('button', { name: /^materials$/i })).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByRole('button', { name: /^tags$/i })).toHaveAttribute('aria-expanded', 'false')
  })

  it('Object Info panel shows no boilerplate when selection is empty', async () => {
    await renderAndLoad()
    // The empty-selection Object Info panel renders nothing at all now — the
    // old "Select an object." prompt was removed as boilerplate.
    expect(screen.queryByText(/select an object/i)).not.toBeInTheDocument()
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

  it('bare A activates the Arc tool (SketchUp-for-Windows arc key)', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'a' })
    // Arc lives in the Draw menu.
    fireEvent.click(screen.getByRole('button', { name: /^draw$/i }))
    const arcItem = menubar().getByText('Arc').closest('div')
    expect(arcItem?.textContent).toContain('✓')
  })

  it('clicking Draw > Arc activates the Arc tool', async () => {
    await renderAndLoad()
    fireEvent.click(screen.getByRole('button', { name: /^draw$/i }))
    // CheckMenuItem commits on mousedown (so the outside-mousedown closer
    // can't race it), not click.
    fireEvent.mouseDown(menubar().getByText('Arc'))
    // Re-open the Draw menu — Arc should now be checked.
    fireEvent.click(screen.getByRole('button', { name: /^draw$/i }))
    const arcItem = menubar().getByText('Arc').closest('div')
    expect(arcItem?.textContent).toContain('✓')
  })

  it('bare M activates the Move tool (SketchUp-for-Windows scheme)', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'm' })
    fireEvent.click(screen.getByRole('button', { name: /^tools$/i }))
    const moveItem = menubar().getByText('Move').closest('div')
    expect(moveItem?.textContent).toContain('✓')
  })

  it('bare H activates the Pan camera tool (SketchUp camera keys)', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'h' })
    fireEvent.click(screen.getByRole('button', { name: /^camera$/i }))
    const panItem = menubar().getByText('Pan').closest('div')
    expect(panItem?.textContent).toContain('✓')
  })

  it('modified letters do not hit the bare-letter tools (Ctrl+R is not Rectangle)', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'r', ctrlKey: true })
    fireEvent.click(screen.getByRole('button', { name: /^draw$/i }))
    const rectItem = menubar().getByText('Rectangle').closest('div')
    expect(rectItem?.textContent).not.toContain('✓')
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

describe('App — tray layout persistence', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('restores each tray section\'s collapsed/expanded state from the persisted layout', async () => {
    setTrayLayout({ modelInfo: false, objectInfo: false, materials: true, tags: false })
    await renderAndLoad()
    expect(screen.getByRole('button', { name: /outliner/i })).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByRole('button', { name: /object info/i })).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByRole('button', { name: /^materials$/i })).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByRole('button', { name: /^tags$/i })).toHaveAttribute('aria-expanded', 'false')
  })

  it('collapsing a section via its header persists to the trayLayout singleton', async () => {
    await renderAndLoad()
    expect(getTrayLayout().modelInfo).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: /outliner/i }))
    await waitFor(() => expect(getTrayLayout().modelInfo).toBe(false))
    // The other sections are untouched.
    expect(getTrayLayout().objectInfo).toBe(true)
    expect(getTrayLayout().materials).toBe(false)
    expect(getTrayLayout().tags).toBe(false)
  })

  it('the Ctrl+Shift+I shortcut also persists (shortcuts keep working unchanged)', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'I', ctrlKey: true, shiftKey: true })
    await waitFor(() => expect(getTrayLayout().modelInfo).toBe(false))
  })
})

// ---------------------------------------------------------------------------
// App — hidden-by-default tags: seed from the document's tag registry on
// load, and persist the eye toggle back to it.
// ---------------------------------------------------------------------------

describe('App — hidden-by-default tag registry', () => {
  // mockScene is a shared singleton across the whole file — restore these
  // overridable methods after each test so other describe blocks (and other
  // tests here) always see the plain defaults.
  const defaultTagMetaPaths = mockScene.tag_meta_paths
  const defaultTagMetaHidden = mockScene.tag_meta_hidden
  const defaultNodeTags = mockScene.node_tags
  const defaultObjectIds = mockScene.object_ids

  beforeEach(() => {
    vi.clearAllMocks()
    setTrayLayout(DEFAULT_TRAY_LAYOUT)
  })

  afterEach(() => {
    mockScene.tag_meta_paths = defaultTagMetaPaths
    mockScene.tag_meta_hidden = defaultTagMetaHidden
    mockScene.node_tags = defaultNodeTags
    mockScene.object_ids = defaultObjectIds
  })

  it('seeds hiddenTagPaths from the registry on File ▸ New and pushes the hide to the kernel', async () => {
    // The registry knows a tag no node carries yet (e.g. an imported .skp
    // layer) and marks it hidden-by-default.
    mockScene.tag_meta_paths = () => ['Imported/HiddenLayer']
    mockScene.tag_meta_hidden = () => new Uint8Array([1])

    await renderAndLoad()
    fireEvent.click(screen.getByRole('button', { name: /^tags$/i }))

    // Before any load reconciles the registry, the session-only hidden set is
    // still empty — the tag renders as visible.
    expect(screen.getByText('HiddenLayer')).toBeInTheDocument()
    expect(screen.queryByTitle('Show tagged objects')).not.toBeInTheDocument()

    // File ▸ New runs applyLoadedBytes, which must re-seed hiddenTagPaths from
    // the (just-loaded) document's tag registry.
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('New'))

    await waitFor(() => {
      expect(screen.getByTitle('Show tagged objects')).toBeInTheDocument()
    })
    // The union push reached the kernel, not just the panel UI.
    expect(mockScene.set_hidden).toHaveBeenCalled()
  })

  it('clicking a tag eye toggle calls scene.set_tag_hidden with the path and new hidden state', async () => {
    // A single tagged object is enough — node_tags is called once per node,
    // and there's only one node in the scene, so a fixed return is unambiguous.
    mockScene.object_ids = () => new BigUint64Array([1n])
    mockScene.node_tags = () => ['Walls']

    await renderAndLoad()
    fireEvent.click(screen.getByRole('button', { name: /^tags$/i }))

    fireEvent.click(screen.getByTitle('Hide tagged objects'))
    expect(mockScene.set_tag_hidden).toHaveBeenCalledWith('Walls', true)

    // Toggling again shows it — set_tag_hidden persists the flip back to false.
    fireEvent.click(screen.getByTitle('Show tagged objects'))
    expect(mockScene.set_tag_hidden).toHaveBeenCalledWith('Walls', false)
  })
})

// ---------------------------------------------------------------------------
// App — user-hidden node registry (manifest v6 per-node persisted hide):
// seed hiddenKeys from the registry on load, and persist the eye toggle
// back to it.
// ---------------------------------------------------------------------------

describe('App — user-hidden node registry', () => {
  // mockScene is a shared singleton across the whole file — restore these
  // overridable methods after each test so other describe blocks (and other
  // tests here) always see the plain defaults.
  const defaultUserHiddenKinds = mockScene.user_hidden_kinds
  const defaultUserHiddenIds = mockScene.user_hidden_ids
  const defaultTopLevelNodes = mockScene.top_level_nodes
  const defaultGroupIds = mockScene.group_ids

  beforeEach(() => {
    vi.clearAllMocks()
    setTrayLayout(DEFAULT_TRAY_LAYOUT)
  })

  afterEach(() => {
    mockScene.user_hidden_kinds = defaultUserHiddenKinds
    mockScene.user_hidden_ids = defaultUserHiddenIds
    mockScene.top_level_nodes = defaultTopLevelNodes
    mockScene.group_ids = defaultGroupIds
  })

  it('seeds hiddenKeys from the registry on File ▸ New and pushes the hide to the kernel', async () => {
    // One top-level group node throughout — before New, the registry hasn't
    // been consulted yet (hiddenKeys starts empty), so it renders visible.
    mockScene.top_level_nodes = () => [{ kind: 'group', id: 7n }]
    mockScene.group_ids = () => new BigUint64Array([7n])

    await renderAndLoad()
    expect(screen.getByTitle('Hide')).toBeInTheDocument()

    // The registry says this group is user-hidden (e.g. a hidden imported
    // .skp component) — File ▸ New re-loads and must re-seed hiddenKeys from
    // the (just-loaded) document's registry.
    mockScene.user_hidden_kinds = () => new Uint8Array([1]) // 1 = group
    mockScene.user_hidden_ids = () => new BigUint64Array([7n])

    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('New'))

    await waitFor(() => {
      expect(screen.getByTitle('Show')).toBeInTheDocument()
    })
    // The union push reached the kernel, not just the tree UI.
    expect(mockScene.set_hidden).toHaveBeenCalled()
  })

  it('clicking a node eye toggle calls scene.set_node_user_hidden with kind/id/flag', async () => {
    mockScene.top_level_nodes = () => [{ kind: 'group', id: 3n }]
    mockScene.group_ids = () => new BigUint64Array([3n])

    await renderAndLoad()

    fireEvent.click(screen.getByTitle('Hide'))
    expect(mockScene.set_node_user_hidden).toHaveBeenCalledWith(1, 3n, true)

    // Toggling again shows it — set_node_user_hidden persists the flip back.
    fireEvent.click(screen.getByTitle('Show'))
    expect(mockScene.set_node_user_hidden).toHaveBeenCalledWith(1, 3n, false)
  })
})

// ---------------------------------------------------------------------------
// App — import: the .skp import path must seed BOTH the hidden-tag registry
// and the user-hidden-node registry, since imported hidden layers/components
// arrive through the same document registries a load populates.
// ---------------------------------------------------------------------------

describe('App — import seeds hidden tags and hidden node keys', () => {
  const defaultTagMetaPaths = mockScene.tag_meta_paths
  const defaultTagMetaHidden = mockScene.tag_meta_hidden
  const defaultUserHiddenKinds = mockScene.user_hidden_kinds
  const defaultUserHiddenIds = mockScene.user_hidden_ids
  const defaultTopLevelNodes = mockScene.top_level_nodes
  const defaultGroupIds = mockScene.group_ids
  const defaultImportSkp = mockScene.import_skp

  let fakeFileHost: FileHost

  beforeEach(() => {
    vi.clearAllMocks()
    setTrayLayout(DEFAULT_TRAY_LAYOUT)
    fakeFileHost = {
      open: vi.fn(),
      save: vi.fn(),
      saveAs: vi.fn(),
      openForImport: vi.fn().mockResolvedValue({
        kind: 'skp',
        name: 'theater.skp',
        bytes: new Uint8Array(),
      }),
      openAny: vi.fn(),
      exportBinary: vi.fn(),
    }
    vi.mocked(makeFileHost).mockReturnValue(fakeFileHost)
  })

  afterEach(() => {
    mockScene.tag_meta_paths = defaultTagMetaPaths
    mockScene.tag_meta_hidden = defaultTagMetaHidden
    mockScene.user_hidden_kinds = defaultUserHiddenKinds
    mockScene.user_hidden_ids = defaultUserHiddenIds
    mockScene.top_level_nodes = defaultTopLevelNodes
    mockScene.group_ids = defaultGroupIds
    mockScene.import_skp = defaultImportSkp
    vi.mocked(makeFileHost).mockReset()
  })

  it('File ▸ Import… seeds both hiddenTagPaths and hiddenKeys from the post-import registries', async () => {
    // The import populates one hidden tag and one hidden (imported) group —
    // both registries only reflect this *after* scene.import_skp runs.
    mockScene.tag_meta_paths = () => ['Imported/HiddenLayer']
    mockScene.tag_meta_hidden = () => new Uint8Array([1])
    mockScene.user_hidden_kinds = () => new Uint8Array([1]) // 1 = group
    mockScene.user_hidden_ids = () => new BigUint64Array([9n])
    mockScene.top_level_nodes = () => [{ kind: 'group', id: 9n }]
    mockScene.group_ids = () => new BigUint64Array([9n])

    await renderAndLoad()

    // Before Import, the outliner renders the group visible.
    expect(screen.getByTitle('Hide')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('Import…'))

    await waitFor(() => {
      expect(mockScene.import_skp).toHaveBeenCalled()
    })

    // Hidden node: the group's eye toggle now reads "Show" (hidden).
    await waitFor(() => {
      expect(screen.getByTitle('Show')).toBeInTheDocument()
    })

    // Hidden tag: the Tags panel shows the imported layer as hidden too.
    fireEvent.click(screen.getByRole('button', { name: /^tags$/i }))
    expect(screen.getByTitle('Show tagged objects')).toBeInTheDocument()

    // Both hides reached the kernel via the same union push.
    expect(mockScene.set_hidden).toHaveBeenCalled()
  })
})

// ---------------------------------------------------------------------------
// App — import overlay lifecycle: the ImportingOverlay (role="status") must
// be committed to the DOM *before* the blocking scene.import_* call runs, and
// must always be cleared afterwards — on success, on throw, and on a picker
// cancel it must never appear at all.  (Whether the browser actually paints
// the committed overlay before the freeze is the double-rAF nextPaint()
// barrier's job — see paint.test.ts; here we assert the state sequencing.)
// ---------------------------------------------------------------------------

describe('App — import overlay lifecycle', () => {
  const defaultImportSkp = mockScene.import_skp
  const emptyReport = {
    objects_created: 0,
    watertight: 0,
    leaky: 0,
    skipped: [] as { name: string; reason: string }[],
    textures_missing: [] as string[],
    warnings: [] as string[],
  }

  let fakeFileHost: FileHost

  beforeEach(() => {
    vi.clearAllMocks()
    setTrayLayout(DEFAULT_TRAY_LAYOUT)
    fakeFileHost = {
      open: vi.fn(),
      save: vi.fn(),
      saveAs: vi.fn(),
      openForImport: vi.fn().mockResolvedValue({
        kind: 'skp',
        name: 'guest-house.skp',
        bytes: new Uint8Array(),
      }),
      openAny: vi.fn(),
      exportBinary: vi.fn(),
    }
    vi.mocked(makeFileHost).mockReturnValue(fakeFileHost)
  })

  afterEach(() => {
    mockScene.import_skp = defaultImportSkp
    vi.mocked(makeFileHost).mockReset()
  })

  const triggerImport = () => {
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('Import…'))
  }

  it('shows the overlay before scene.import_skp runs and clears it after success', async () => {
    // Snapshot overlay visibility at the exact moment the blocking import
    // starts — the overlay must already be in the DOM by then.
    let overlayVisibleAtImport = false
    mockScene.import_skp = vi.fn(() => {
      overlayVisibleAtImport = screen.queryByRole('status') !== null
      return emptyReport
    })

    await renderAndLoad()
    triggerImport()

    await waitFor(() => expect(mockScene.import_skp).toHaveBeenCalled())
    expect(overlayVisibleAtImport).toBe(true)

    // finally-clause: the overlay clears once the import completes.
    await waitFor(() => expect(screen.queryByRole('status')).not.toBeInTheDocument())
  })

  it('clears the overlay when the import throws, and surfaces a toast', async () => {
    mockScene.import_skp = vi.fn(() => {
      throw new Error('corrupt chunk')
    })

    await renderAndLoad()
    triggerImport()

    await waitFor(() => expect(mockScene.import_skp).toHaveBeenCalled())
    await waitFor(() => expect(screen.queryByRole('status')).not.toBeInTheDocument())
    expect(screen.getByText(/Import failed: corrupt chunk/)).toBeInTheDocument()
  })

  it('never shows the overlay when the user cancels the file picker', async () => {
    vi.mocked(fakeFileHost.openForImport).mockResolvedValue(null)

    await renderAndLoad()
    triggerImport()

    await waitFor(() => expect(fakeFileHost.openForImport).toHaveBeenCalled())
    expect(mockScene.import_skp).not.toHaveBeenCalled()
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// App — STL import: the units-chooser modal (DESIGN §5) must appear
// BEFORE the blocking overlay, and its resolved unit_scale must be exactly
// what reaches scene.import_stl. Cancelling the chooser must behave like
// cancelling the file picker (no import, no overlay, document untouched).
// ---------------------------------------------------------------------------

describe('App — STL import units chooser', () => {
  const defaultImportStl = mockScene.import_stl
  const emptyReport = {
    objects_created: 1,
    watertight: 1,
    leaky: 0,
    skipped: [] as { name: string; reason: string }[],
    textures_missing: [] as string[],
    warnings: [] as string[],
  }

  let fakeFileHost: FileHost

  beforeEach(() => {
    vi.clearAllMocks()
    setTrayLayout(DEFAULT_TRAY_LAYOUT)
    // The last-STL-unit singleton persists across tests; reset so the chooser
    // defaults to Millimeters regardless of what a prior test picked.
    resetStlImportUnitForTest()
    fakeFileHost = {
      open: vi.fn(),
      save: vi.fn(),
      saveAs: vi.fn(),
      openForImport: vi.fn().mockResolvedValue({
        kind: 'stl',
        name: 'bracket.stl',
        bytes: new Uint8Array([1, 2, 3]),
      }),
      openAny: vi.fn(),
      exportBinary: vi.fn(),
    }
    vi.mocked(makeFileHost).mockReturnValue(fakeFileHost)
  })

  afterEach(() => {
    mockScene.import_stl = defaultImportStl
    vi.mocked(makeFileHost).mockReset()
  })

  const triggerImport = () => {
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('Import…'))
  }

  it('shows the units chooser before the overlay, defaulting to Millimeters', async () => {
    mockScene.import_stl = vi.fn(() => emptyReport)

    await renderAndLoad()
    triggerImport()

    const dialog = await screen.findByRole('dialog', { name: /stl import units/i })
    expect(within(dialog).getByRole('radio', { name: /millimeters/i })).toBeChecked()
    // The blocking overlay must not appear until AFTER the unit is chosen.
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    expect(mockScene.import_stl).not.toHaveBeenCalled()
  })

  it('threads the default Millimeters choice through to scene.import_stl as unit_scale 0.001', async () => {
    mockScene.import_stl = vi.fn(() => emptyReport)

    await renderAndLoad()
    triggerImport()

    const dialog = await screen.findByRole('dialog', { name: /stl import units/i })
    fireEvent.click(within(dialog).getByRole('button', { name: /^import$/i }))

    await waitFor(() => expect(mockScene.import_stl).toHaveBeenCalled())
    expect(mockScene.import_stl).toHaveBeenCalledWith(new Uint8Array([1, 2, 3]), 0.001, 'bracket')
  })

  it('threads a non-default choice (Inches) through as unit_scale 0.0254', async () => {
    mockScene.import_stl = vi.fn(() => emptyReport)

    await renderAndLoad()
    triggerImport()

    const dialog = await screen.findByRole('dialog', { name: /stl import units/i })
    fireEvent.click(within(dialog).getByRole('radio', { name: /inches/i }))
    fireEvent.click(within(dialog).getByRole('button', { name: /^import$/i }))

    await waitFor(() => expect(mockScene.import_stl).toHaveBeenCalled())
    expect(mockScene.import_stl).toHaveBeenCalledWith(new Uint8Array([1, 2, 3]), 0.0254, 'bracket')
  })

  it('cancelling the units chooser never imports and leaves the document untouched', async () => {
    mockScene.import_stl = vi.fn(() => emptyReport)

    await renderAndLoad()
    triggerImport()

    const dialog = await screen.findByRole('dialog', { name: /stl import units/i })
    fireEvent.click(within(dialog).getByRole('button', { name: /cancel/i }))

    await waitFor(() =>
      expect(screen.queryByRole('dialog', { name: /stl import units/i })).not.toBeInTheDocument(),
    )
    expect(mockScene.import_stl).not.toHaveBeenCalled()
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })

  it('a second import triggered while the units chooser is open is refused (no hang, first import survives)', async () => {
    mockScene.import_stl = vi.fn(() => emptyReport)

    await renderAndLoad()
    triggerImport()

    // First chooser is open.
    const dialog = await screen.findByRole('dialog', { name: /stl import units/i })
    expect(vi.mocked(fakeFileHost.openForImport)).toHaveBeenCalledTimes(1)

    // Fire a SECOND import while the first chooser is still open. The
    // re-entrancy guard must refuse it: no second file dialog, and the first
    // chooser stays exactly as it was (not clobbered).
    triggerImport()
    await Promise.resolve()
    expect(vi.mocked(fakeFileHost.openForImport)).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('dialog', { name: /stl import units/i })).toBe(dialog)

    // Completing the FIRST chooser still drives its import to completion —
    // proof the first call was never orphaned.
    fireEvent.click(within(dialog).getByRole('button', { name: /^import$/i }))
    await waitFor(() => expect(mockScene.import_stl).toHaveBeenCalledTimes(1))
    expect(mockScene.import_stl).toHaveBeenCalledWith(new Uint8Array([1, 2, 3]), 0.001, 'bracket')
  })
})

// ---------------------------------------------------------------------------
// App — the unified Open dialog (File ▸ Open… / Welcome's "Open a file…"):
// ONE dialog (`FileHost.openAny()`) covering `.hew` plus every import format,
// dispatching by the picked extension. Exercises the actual openDocument()
// dispatch — not just openForImport()'s pre-existing File ▸ Import… path
// the tests above cover — including the fix for a stale docSession after a
// failed import (see runImportPick's catch block in App.tsx).
// ---------------------------------------------------------------------------
describe('App — unified Open dialog', () => {
  const defaultImportStl = mockScene.import_stl
  const defaultLoad = mockScene.load
  const emptyReport = {
    objects_created: 1,
    watertight: 1,
    leaky: 0,
    skipped: [] as { name: string; reason: string }[],
    textures_missing: [] as string[],
    warnings: [] as string[],
  }

  let fakeFileHost: FileHost

  beforeEach(() => {
    vi.clearAllMocks()
    setTrayLayout(DEFAULT_TRAY_LAYOUT)
    resetStlImportUnitForTest()
    fakeFileHost = {
      open: vi.fn(),
      save: vi.fn(),
      saveAs: vi.fn(),
      openForImport: vi.fn(),
      openAny: vi.fn(),
      exportBinary: vi.fn(),
    }
    vi.mocked(makeFileHost).mockReturnValue(fakeFileHost)
  })

  afterEach(() => {
    mockScene.import_stl = defaultImportStl
    mockScene.load = defaultLoad
    vi.mocked(makeFileHost).mockReset()
  })

  const triggerOpen = () => {
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('Open…'))
  }
  const triggerImport = () => {
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('Import…'))
  }

  it('a picked .hew file loads straight through — no STL units chooser, no import report', async () => {
    vi.mocked(fakeFileHost.openAny).mockResolvedValue({
      kind: 'hew',
      name: 'my-house.hew',
      bytes: new Uint8Array([7, 7, 7]),
      handle: '/tmp/my-house.hew',
    })

    await renderAndLoad()
    triggerOpen()

    await waitFor(() => expect(mockScene.load).toHaveBeenCalledWith(new Uint8Array([7, 7, 7])))
    // The document name now reflects the opened file (proof the hew branch,
    // not the import branch, ran) — and neither import-only surface appeared.
    expect(await screen.findByText('my-house.hew')).toBeInTheDocument()
    expect(screen.queryByRole('dialog', { name: /stl import units/i })).not.toBeInTheDocument()
    expect(screen.queryByRole('dialog', { name: /import report/i })).not.toBeInTheDocument()
    expect(mockScene.import_stl).not.toHaveBeenCalled()
  })

  it('a picked .stl file routes through the same units chooser as File ▸ Import…', async () => {
    mockScene.import_stl = vi.fn(() => emptyReport)
    vi.mocked(fakeFileHost.openAny).mockResolvedValue({
      kind: 'stl',
      name: 'bracket.stl',
      bytes: new Uint8Array([1, 2, 3]),
    })

    await renderAndLoad()
    triggerOpen()

    const dialog = await screen.findByRole('dialog', { name: /stl import units/i })
    fireEvent.click(within(dialog).getByRole('button', { name: /^import$/i }))

    await waitFor(() => expect(mockScene.import_stl).toHaveBeenCalled())
    expect(mockScene.import_stl).toHaveBeenCalledWith(new Uint8Array([1, 2, 3]), 0.001, 'bracket')
    expect(await screen.findByRole('dialog', { name: /import report/i })).toBeInTheDocument()
  })

  it('the re-entrancy guard is shared: File ▸ Open… while File ▸ Import…\'s chooser is open is refused', async () => {
    mockScene.import_stl = vi.fn(() => emptyReport)
    vi.mocked(fakeFileHost.openForImport).mockResolvedValue({
      kind: 'stl',
      name: 'bracket.stl',
      bytes: new Uint8Array([1, 2, 3]),
    })

    await renderAndLoad()
    triggerImport()

    const dialog = await screen.findByRole('dialog', { name: /stl import units/i })
    expect(vi.mocked(fakeFileHost.openForImport)).toHaveBeenCalledTimes(1)

    // Fire the UNIFIED Open path (a different entry point) while the first
    // gesture's chooser is still open — the shared guard must refuse it too.
    triggerOpen()
    await Promise.resolve()
    expect(vi.mocked(fakeFileHost.openAny)).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog', { name: /stl import units/i })).toBe(dialog)

    // The first (Import) gesture still completes normally — proof it wasn't
    // orphaned by the refused cross-entry-point second call.
    fireEvent.click(within(dialog).getByRole('button', { name: /^import$/i }))
    await waitFor(() => expect(mockScene.import_stl).toHaveBeenCalledTimes(1))
  })

  it('a failed import through the unified Open dialog resets the session to Untitled — no silent overwrite of the prior file on the next Save', async () => {
    // First, a real .hew is open (so currentRef/name point at a real file).
    vi.mocked(fakeFileHost.openAny).mockResolvedValueOnce({
      kind: 'hew',
      name: 'my-house.hew',
      bytes: new Uint8Array([7, 7, 7]),
      handle: '/tmp/my-house.hew',
    })
    await renderAndLoad()
    triggerOpen()
    expect(await screen.findByText('my-house.hew')).toBeInTheDocument()

    // Now the same unified dialog picks a corrupt STL — the import throws.
    mockScene.import_stl = vi.fn(() => {
      throw new Error('bad data')
    })
    vi.mocked(fakeFileHost.openAny).mockResolvedValueOnce({
      kind: 'stl',
      name: 'corrupt.stl',
      bytes: new Uint8Array([9, 9, 9]),
    })
    triggerOpen()

    const chooser = await screen.findByRole('dialog', { name: /stl import units/i })
    fireEvent.click(within(chooser).getByRole('button', { name: /^import$/i }))

    // The failure toasts, AND the session no longer claims "my-house.hew" is
    // still the open file — it reverts to Untitled, matching the live scene
    // (which the blank-then-import replace already emptied before the throw).
    await waitFor(() => expect(screen.getByText(/import failed/i)).toBeInTheDocument())
    expect(screen.queryByText('my-house.hew')).not.toBeInTheDocument()
    expect(await screen.findByText('Untitled')).toBeInTheDocument()
  })

  it('a further File ▸ Open onto an already-open, DIRTY document confirms discarding first (web build — no Tauri window support to fall back to)', async () => {
    // Open the first document — the window is no longer pristine afterward
    // (isPristineDocument requires currentRef === null).
    vi.mocked(fakeFileHost.openAny).mockResolvedValueOnce({
      kind: 'hew',
      name: 'my-house.hew',
      bytes: new Uint8Array([7, 7, 7]),
      handle: '/tmp/my-house.hew',
    })
    await renderAndLoad()
    triggerOpen()
    expect(await screen.findByText('my-house.hew')).toBeInTheDocument()

    // Dirty it via the semantic harness (a real mutation, not a flag flip) —
    // confirmDiscard is a no-op-true while clean, so this is what makes the
    // discard prompt actually fire below. addNodeTag is a convenient
    // lightweight mutation (mockScene.add_node_tag is already a no-op spy).
    const harness = (window as unknown as {
      __hew_test: { addNodeTag: (kind: string, id: string, path: string[]) => void }
    }).__hew_test
    act(() => harness.addNodeTag('object', '1', ['tag']))

    vi.mocked(fakeFileHost.openAny).mockResolvedValueOnce({
      kind: 'hew',
      name: 'other.hew',
      bytes: new Uint8Array([1, 1, 1]),
      handle: '/tmp/other.hew',
    })

    // Cancelling the prompt leaves the open document completely untouched —
    // the dialog is never even shown (confirmDiscard runs before it, on the
    // web fallback path).
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValueOnce(false)
    triggerOpen()
    await Promise.resolve()
    expect(confirmSpy).toHaveBeenCalledTimes(1)
    // Still just the one call from opening my-house.hew — cancelling the
    // discard prompt means the dialog for THIS gesture never even opens.
    expect(vi.mocked(fakeFileHost.openAny)).toHaveBeenCalledTimes(1)
    expect(screen.getByText('my-house.hew')).toBeInTheDocument()
    expect(mockScene.load).toHaveBeenCalledTimes(1)

    // Confirming discards it and replaces the document in place — the web
    // build has no Tauri window to open the pick into instead.
    confirmSpy.mockReturnValueOnce(true)
    triggerOpen()
    await waitFor(() => expect(mockScene.load).toHaveBeenCalledTimes(2))
    expect(await screen.findByText('other.hew')).toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// isPristineDocument — the predicate File ▸ Open (and, aligned, File ▸ New)
// use to decide whether the current window may be reused in place. Stricter
// than a bare "scene is empty" check: a saved-then-emptied document
// (currentRef set, but every entity since deleted) must NOT be treated as
// pristine, or reusing the window would silently abandon that file's editing
// session (undo history, its crash-recovery snapshot, the association
// itself) with no prompt at all.
// ---------------------------------------------------------------------------
describe('isPristineDocument', () => {
  const emptyIds = () => new BigUint64Array()
  const oneId = () => BigUint64Array.from([1n])
  const emptyScene = { object_ids: emptyIds, group_ids: emptyIds, instance_ids: emptyIds, sketch_ids: emptyIds } as unknown as Scene
  const nonEmptyScene = { object_ids: oneId, group_ids: emptyIds, instance_ids: emptyIds, sketch_ids: emptyIds } as unknown as Scene

  const blankClean: DocSessionState = { currentRef: null, dirty: false, lastEditAt: null, lastSavedAt: null }
  const blankDirty: DocSessionState = { currentRef: null, dirty: true, lastEditAt: 1, lastSavedAt: null }
  const namedClean: DocSessionState = {
    currentRef: { name: 'house.hew', handle: '/tmp/house.hew' },
    dirty: false,
    lastEditAt: null,
    lastSavedAt: 1,
  }
  const namedDirty: DocSessionState = {
    currentRef: { name: 'house.hew', handle: '/tmp/house.hew' },
    dirty: true,
    lastEditAt: 1,
    lastSavedAt: 1,
  }

  it('a fresh blank document (no file, clean, empty scene) is pristine', () => {
    expect(isPristineDocument(blankClean, emptyScene)).toBe(true)
  })

  it('an untitled document with unsaved edits is not pristine, even with an empty scene', () => {
    expect(isPristineDocument(blankDirty, emptyScene)).toBe(false)
  })

  it('a document backed by a named file is not pristine, even clean and empty (saved-then-emptied)', () => {
    expect(isPristineDocument(namedClean, emptyScene)).toBe(false)
  })

  it('a blank/clean session with geometry in the scene is not pristine', () => {
    expect(isPristineDocument(blankClean, nonEmptyScene)).toBe(false)
  })

  it('a named, dirty document with an empty scene is not pristine (both conditions violated at once)', () => {
    expect(isPristineDocument(namedDirty, emptyScene)).toBe(false)
  })
})

describe('App — toast severity', () => {
  /** The onToast prop the App handed the (mocked) Viewport on its last render. */
  function latestOnToast(): (message: string, code?: string) => void {
    const calls = vi.mocked(Viewport).mock.calls
    const props = calls[calls.length - 1][0] as { onToast: (m: string, c?: string) => void }
    return props.onToast
  }

  it('renders error-level kernel refusals as red bubbles, from the single classification source', async () => {
    await renderAndLoad()
    // A group-boolean refusal must render exactly like its sibling
    // OperandNotSolid — one classification source (isErrorLevelCode), used
    // by the log level AND the bubble color alike.
    act(() => latestOnToast()('not solid', 'BooleanOperandNotSolid'))
    const bubble = screen.getByText(/not solid/).closest('div')!
    expect(bubble.style.background).toBe('rgb(204, 51, 34)')
  })

  it('renders warning-level refusals as neutral bubbles', async () => {
    await renderAndLoad()
    act(() => latestOnToast()('nothing to undo', 'NothingToUndo'))
    const bubble = screen.getByText(/nothing to undo/).closest('div')!
    expect(bubble.style.background).toBe('rgb(51, 51, 51)')
  })
})

describe('App — welcome screen', () => {
  it('opens on a bare launch and closes into the blank document', async () => {
    setShowWelcome(true)
    await renderAndLoad()
    const dialog = await screen.findByRole('dialog', { name: /welcome to hew/i })
    expect(dialog).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /start modeling/i }))
    expect(screen.queryByRole('dialog', { name: /welcome to hew/i })).not.toBeInTheDocument()
  })

  it('yields to the crash-recovery prompt (recovery wins the startup handoff)', async () => {
    setShowWelcome(true)
    recoveryState.listings = [
      { slot: 'web', meta: { version: 1, savedAt: Date.now(), name: 'Crashed Doc', path: null } },
    ]
    await renderAndLoad()
    await screen.findByRole('dialog', { name: /recover unsaved document/i })
    expect(screen.queryByRole('dialog', { name: /welcome to hew/i })).not.toBeInTheDocument()
  })

  it('swallows app shortcuts while open — no tool switch, no palette stacked underneath', async () => {
    setShowWelcome(true)
    await renderAndLoad()
    await screen.findByRole('dialog', { name: /welcome to hew/i })

    // Bare-letter tool shortcut and the Ctrl+K palette must both be inert
    // behind the modal (its overlay blocks the pointer but not the keyboard).
    fireEvent.keyDown(document, { key: 'r' })
    fireEvent.keyDown(document, { key: 'k', ctrlKey: true })
    expect(screen.queryByRole('dialog', { name: /command palette/i })).not.toBeInTheDocument()

    // Escape still dismisses the welcome screen itself…
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog', { name: /welcome to hew/i })).not.toBeInTheDocument()

    // …and the earlier 'r' never reached the tool registry.
    fireEvent.click(screen.getByRole('button', { name: /^draw$/i }))
    const rectangleItem = menubar().getByText('Rectangle').closest('div')
    expect(rectangleItem?.textContent).not.toContain('✓')
  })
})

describe('App — document changes prune dead handles from the selection', () => {
  // The maintainer's repro: copy a cube, 3x the copy into an array, Undo —
  // Object Info still said "3 selected" and the dock stayed in Multi mode
  // (both read the app selection) while the Outliner (which re-reads the
  // document) correctly showed one object. handleDocumentChanged is the
  // choke point every mutation funnels through (undo/redo included), so the
  // prune lives there and covers every path that can kill selected nodes.
  const originalObjectIds = mockScene.object_ids

  afterEach(() => {
    mockScene.object_ids = originalObjectIds
    delete (mockScene as Record<string, unknown>).scene_undo
  })

  it('a selection over removed objects shrinks to the survivors after undo', async () => {
    await renderAndLoad()
    const harness = (window as unknown as {
      __hew_test: {
        selectObjects(ids: string[]): void
        getSelection(): { kind: string; id: string }[]
        undo(): void
      }
    }).__hew_test
    expect(harness).toBeDefined()

    // Three live objects, all selected (the post-array state).
    mockScene.object_ids = () => BigUint64Array.from([1n, 2n, 3n])
    act(() => harness.selectObjects(['1', '2', '3']))
    expect(harness.getSelection()).toHaveLength(3)

    // Undo removes two of them (the harness's headless arm reconciles via
    // handleDocumentChanged, the same choke point every entry drives).
    ;(mockScene as Record<string, unknown>).scene_undo = () => ({ free: () => { /* no-op */ } })
    mockScene.object_ids = () => BigUint64Array.from([1n])
    act(() => harness.undo())

    expect(harness.getSelection()).toEqual([{ kind: 'object', id: '1' }])
  })
})
