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

import { BANNED_RE, readableText } from './health/bannedWords'
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
  // Model Health (status badge / Outliner icons): an all-solid, empty summary.
  // (The full health_report_json is the panel's, read only while its window is open.)
  health_summary_json: () =>
    '{"counts":{"objects":0,"solids":0,"flat_sheets":0,"bent_sheets":0,"cappable":0,"holes":0,"mixed":0,"non_manifold":0},"open":[]}',
  can_scene_undo: () => false,
  can_scene_redo: () => false,
  // Lane C (docs/design/v1.1-cycle.md): this mock is not a real, stateful
  // kernel document, so `at_saved_mark` statically mirrors the OLD
  // unconditional "any mutation dirties" behavior every test here already
  // assumes — `handleDocumentChanged` only reads it after a real mutation
  // fired (never on mount), so `false` here reproduces the pre-Lane-C
  // `afterMutation` latch exactly. Save-flow specifics (mark_saved actually
  // clearing dirty) are covered by documentSession.test.ts and the E2E
  // dirty-state spec, not here — no test in this file exercises Save.
  at_saved_mark: () => false,
  mark_saved: vi.fn(),
  // `.skp` document source (crash-recovery persistence): none for these tests.
  skp_source: vi.fn(() => undefined),
  set_skp_source: vi.fn(),
  commit_skp_save: vi.fn(),
  undo_depth: () => 0,
  redo_depth: () => 0,
  history_entries_json: () => JSON.stringify({ undo: [], redo: [], savedDepth: null }),
  geo_reference_json: () => 'null',
  // save() is called once to snapshot the blank scene for "New" resets.
  save: () => new Uint8Array(),
  load: vi.fn(),
  // Camera persistence (docs/design/camera.md §5) — no saved view by
  // default, so load falls back to the pre-existing default framing.
  camera_state: () => undefined,
  set_camera_state: vi.fn(),
  view_style_json: () => undefined,
  set_view_style_axes: vi.fn(),
  node_parent: () => undefined as bigint | undefined,
  material_ids: () => new BigUint64Array(),
  material_info: () => undefined,
  material_texture_bytes: () => undefined,
  set_torture_mode: vi.fn(),
  component_member_objects: () => new BigUint64Array(),
  // v1.1 assets lane: material/definition rename+delete, purge unused.
  set_material_name: vi.fn(),
  delete_material: vi.fn(),
  material_usage: () => 0,
  unused_materials: () => new BigUint64Array(),
  component_ids: () => new BigUint64Array(),
  delete_definition: vi.fn(),
  definition_usage: () => 0,
  render_definition_thumbnail: () => undefined as Uint8Array | undefined,
  unused_definitions: () => new BigUint64Array(),
  purge_unused: vi.fn().mockReturnValue({ materials: () => 0, definitions: () => 0 }),
  // Object Info's Bounding Box row (objectBounds.worldBoundsForSelection) reads
  // per-object render meshes; a mesh-less stub keeps it a no-op here.
  object_mesh: () => ({ positions: () => new Float32Array(), free: () => {} }),
  instance_pose: () => undefined as Float64Array | undefined,
  node_leaf_objects: () => new BigUint64Array(),
  // Object Info's Materials row: nothing painted by default.
  object_materials: () => new BigUint64Array(),
  instance_expanded_members: () => new BigUint64Array(),
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
    lines_added: 0,
    watertight: 0,
    leaky: 0,
    health: { objects: 0, solids: 0, flat_sheets: 0, bent_sheets: 0, cappable: 0, holes: 0, mixed: 0, non_manifold: 0, seam_families: 0 },
    skipped: [] as { name: string; reason: string }[],
    textures_missing: [] as string[],
    warnings: [] as string[],
  })),
  import_stl: vi.fn(() => ({
    objects_created: 0,
    lines_added: 0,
    watertight: 0,
    leaky: 0,
    health: { objects: 0, solids: 0, flat_sheets: 0, bent_sheets: 0, cappable: 0, holes: 0, mixed: 0, non_manifold: 0, seam_families: 0 },
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

// wasm/kernelMemory — the real module reads the memory loader.ts hands it,
// which the mocked loader never does; null (not loaded) is the default here
// and the out-of-memory open test sets a figure at the wasm32 ceiling.
vi.mock('./wasm/kernelMemory', () => ({
  kernelMemoryBytes: vi.fn((): number | null => null),
  setKernelMemory: vi.fn(),
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
// `isTauri` reads through a switch so the Revert block can exercise the
// desktop side effects (read_file, push_recent); it is off everywhere else.
const tauriMode = vi.hoisted(() => ({ on: false }))
vi.mock('./io/fileHost', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./io/fileHost')>()
  return {
    ...actual,
    makeFileHost: vi.fn(actual.makeFileHost),
    get isTauri() {
      return tauriMode.on || actual.isTauri
    },
  }
})

// The Tauri core bridge: only the Revert block's desktop path calls it.
const tauriCore = vi.hoisted(() => ({
  invoke: vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: tauriCore.invoke }))

// The live bridge is a no-op outside Tauri; the spy keeps the operation
// runner App hands it, so a test can drive a live `revert` the way the shell does.
const liveBridge = vi.hoisted(() => ({
  runOp: null as null | ((op: string, args: Record<string, unknown>) => Promise<{ ok: boolean; refusal?: string; message?: string }>),
}))
vi.mock('./api/liveBridge', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./api/liveBridge')>()
  return {
    ...actual,
    installLiveBridge: vi.fn((deps: { runOp?: typeof liveBridge.runOp }) => {
      liveBridge.runOp = deps.runOp ?? null
      return () => {}
    }),
  }
})

// io/savePanel — the desktop's native save panel. Off by default (every
// other test here runs the browser path); the native-panel block below
// switches it on and answers for the panel.
vi.mock('./io/savePanel', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./io/savePanel')>()
  return { ...actual, hasNativeSavePanel: vi.fn(() => false), pickSaveTarget: vi.fn() }
})

// io/recoveryStore — only makeRecoveryStore is mocked (jsdom has no
// IndexedDB, so the real WebRecoveryStore silently no-ops and could never
// report a snapshot). Tests seed `recoveryState.listings` to simulate a
// crash snapshot awaiting recovery; shouldPromptRecovery stays real.
//
// `write`/`clear` are `vi.fn()`s (not bare closures) so the autosave
// arm/disarm describe block below can observe call counts and override
// resolve/reject behavior per test (a write failure must re-arm the tick).
// Every OTHER describe block only cares that these no-op successfully,
// which `vi.clearAllMocks()` (already called in most `beforeEach`s here)
// preserves — it clears call history, not the resolved-value implementation
// installed once at hoist time.
const recoveryState = vi.hoisted(() => ({
  listings: [] as { slot: string; meta: { version: 1; savedAt: number; name: string; path: string | null } }[],
  write: vi.fn(async (_bytes: Uint8Array, _meta: unknown) => {}),
  clear: vi.fn(async () => {}),
  writeSource: vi.fn(async (_source: Uint8Array | null) => {}),
}))
vi.mock('./io/recoveryStore', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./io/recoveryStore')>()
  return {
    ...actual,
    makeRecoveryStore: () => ({
      write: recoveryState.write,
      list: async () => recoveryState.listings,
      claim: async () => null,
      clear: recoveryState.clear,
      discardAll: async () => {},
      writeSource: recoveryState.writeSource,
    }),
  }
})

// io/recents — jsdom has no IndexedDB, so the real implementation already
// silently no-ops there; mocked explicitly (matching io/recoveryStore's
// precedent just above) so tests can assert App.tsx calls it at the right
// seam without depending on a real IndexedDB round trip (io/recents.test.ts
// covers that).
vi.mock('./io/recents', () => ({
  listRecents: vi.fn(async () => []),
  recordRecent: vi.fn(async () => undefined),
}))

import App from './App'
import Viewport from './viewport/Viewport'
import { recordRecent } from './io/recents'
import { getTrayLayout, setTrayLayout, DEFAULT_TRAY_LAYOUT } from './settings/trayLayout'
import { setShowWelcome } from './settings/welcomeScreen'
import { resetStlImportUnitForTest } from './settings/stlImportUnit'
import { makeFileHost, type DocFormat, type FileHost } from './io/fileHost'
import * as savePanel from './io/savePanel'
import { isPristineDocument, sameSessionStackIdentity, AUTOSAVE_INTERVAL_MS } from './App'
import type { DocSessionState } from './io/documentSession'
import type { Scene } from './wasm/loader'
import { MATERIAL_SENTINEL } from './tools/PaintTool'
import { clearOpenAttempt, getFailedOpen } from './log/failedOpen'
import { clearKernelStop, getKernelStop, getPanicCapture, KERNEL_PANIC_EVENT } from './log/panicCapture'
import { kernelMemoryBytes } from './wasm/kernelMemory'

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
  // Reset call history AND any per-test resolve/reject override (the
  // autosave arm/disarm describe block installs `mockRejectedValueOnce` —
  // reset back to a clean always-succeeds no-op so no override leaks into
  // an unrelated later test).
  recoveryState.write.mockReset().mockResolvedValue(undefined)
  recoveryState.clear.mockReset().mockResolvedValue(undefined)
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

describe('App — Model Health badge reads the cheap summary', () => {
  it('shows the open count from health_summary_json and never reads the full report while the window is closed', async () => {
    const m = mockScene as unknown as Record<string, unknown>
    const defaultSummary = m.health_summary_json
    const report = vi.fn(() => '{"rows":[],"families":[],"counts":{}}')
    m.health_report_json = report
    m.health_summary_json = () =>
      JSON.stringify({
        counts: { objects: 4, solids: 2, flat_sheets: 1, bent_sheets: 0, cappable: 1, holes: 0, mixed: 0, non_manifold: 0 },
        open: [['3', 'flat_sheet', false], ['4', 'cappable', false]],
      })
    try {
      await renderAndLoad()
      const badge = await waitFor(() => {
        const el = document.querySelector('[data-health-badge]')
        if (el === null) throw new Error('no badge yet')
        return el
      })
      expect(badge).toHaveAttribute('data-health-badge', 'open')
      // Neutral, and it counts every object that isn't solid (the panel included).
      expect(badge.textContent).toBe('4 objects · 2 not solid')
      expect(badge).toHaveAttribute('title', "List the objects that aren't solid")
      expect((badge as HTMLElement).style.background).toBe('var(--surface-overlay)')
      expect(readableText(badge.parentElement as HTMLElement)).not.toMatch(BANNED_RE)
      expect(screen.queryByRole('dialog', { name: 'Model Health' })).not.toBeInTheDocument()
      expect(report).not.toHaveBeenCalled()
    } finally {
      m.health_summary_json = defaultSummary
      delete m.health_report_json
    }
  })
})

describe('App — Make Solid entry points', () => {
  const m = mockScene as unknown as Record<string, unknown>
  const OPEN_ROW = {
    object: '4', name: 'Wall', definition: null, definition_name: null, placements: 1, instances: [], accepted: false, family: null,
    verdict: { kind: 'cappable' }, rims: [{ role: 'outer', planar: true, closed: true, cappable: true, perimeter: 4, extent: 1, edges: 4 }],
    branching_vertices: 0, islands: 1,
  }
  const report = vi.fn()
  const lastImport = vi.fn(() => new BigUint64Array([4n]))
  let fakeFileHost: FileHost
  const defaultSummary = m.health_summary_json
  const defaultImportSkp = mockScene.import_skp

  beforeEach(() => {
    vi.clearAllMocks()
    setTrayLayout(DEFAULT_TRAY_LAYOUT)
    report.mockImplementation(() =>
      JSON.stringify({ rows: [OPEN_ROW], families: [], counts: { objects: 4, solids: 3, flat_sheets: 0, bent_sheets: 0, cappable: 1, holes: 0, mixed: 0, non_manifold: 0 } }),
    )
    m.health_report_json = report
    m.last_import_object_ids = lastImport
    m.health_summary_json = () =>
      JSON.stringify({
        counts: { objects: 4, solids: 3, flat_sheets: 0, bent_sheets: 0, cappable: 1, holes: 0, mixed: 0, non_manifold: 0 },
        open: [['4', 'cappable', false]],
      })
    fakeFileHost = {
      open: vi.fn(),
      save: vi.fn(),
      saveAs: vi.fn(),
      openForImport: vi.fn().mockResolvedValue({ kind: 'skp', name: 'house.skp', bytes: new Uint8Array() }),
      openAny: vi.fn(),
      exportBinary: vi.fn(),
    }
    vi.mocked(makeFileHost).mockReturnValue(fakeFileHost)
  })

  afterEach(() => {
    m.health_summary_json = defaultSummary
    mockScene.import_skp = defaultImportSkp
    delete m.health_report_json
    delete m.last_import_object_ids
    vi.mocked(makeFileHost).mockReset()
  })

  it('the status badge is neutral and opens the Model Health window; it never opens the wizard', async () => {
    await renderAndLoad()
    expect(screen.queryByRole('dialog', { name: 'Make Solid' })).not.toBeInTheDocument()
    const badge = await waitFor(() => {
      const el = document.querySelector('[data-health-badge="open"]')
      if (el === null) throw new Error('no badge yet')
      return el as HTMLElement
    })
    expect(badge.textContent).toBe('4 objects · 1 not solid')
    expect(badge).toHaveAttribute('title', "List the objects that aren't solid")
    expect(badge.style.background).toBe('var(--surface-overlay)')
    expect(badge.style.color).toBe('var(--text-secondary)')
    expect(screen.queryByRole('dialog', { name: 'Model Health' })).not.toBeInTheDocument()
    fireEvent.click(badge)
    expect(await screen.findByRole('dialog', { name: 'Model Health' })).toBeInTheDocument()
    expect(screen.queryByRole('dialog', { name: 'Make Solid' })).not.toBeInTheDocument()
  })

  it('the badge with every object solid is plain and neutral, not a button', async () => {
    m.health_summary_json = () =>
      JSON.stringify({
        counts: { objects: 4, solids: 4, flat_sheets: 0, bent_sheets: 0, cappable: 0, holes: 0, mixed: 0, non_manifold: 0 },
        open: [],
      })
    await renderAndLoad()
    const label = await screen.findByText('4 objects ✓ solid')
    expect(label.tagName).toBe('SPAN')
    expect((label as HTMLElement).style.background).toBe('var(--surface-overlay)')
  })

  it('Object ▸ Make Solid… with nothing selected opens it on the whole model', async () => {
    await renderAndLoad()
    fireEvent.click(screen.getByRole('button', { name: /^object$/i }))
    fireEvent.mouseDown(menubar().getByText('Make Solid…'))
    expect(await screen.findByRole('dialog', { name: 'Make Solid' })).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Make this model solid' })).toBeInTheDocument()
    expect(report).toHaveBeenLastCalledWith(null)
    expect(screen.queryByRole('dialog', { name: 'Model Health' })).not.toBeInTheDocument()
  })

  describe('the Model Health window', () => {
    const setFramingInset = vi.fn()
    const setHealthOverlay = vi.fn()
    const fakeViewportApi = {
      setFramingInset,
      setHealthOverlay,
      clearIsolation: vi.fn(),
      isolate: vi.fn(),
      getEffectiveHiddenIds: () => ({ objects: [], instances: [] }),
    }
    beforeEach(() => {
      vi.mocked(Viewport).mockImplementation(((props: { apiRef?: { current: unknown } }) => {
        if (props.apiRef !== undefined) props.apiRef.current = fakeViewportApi
        return null
      }) as never)
    })
    afterEach(() => {
      vi.mocked(Viewport).mockImplementation((() => null) as never)
    })
    const openFromObjectMenu = () => {
      fireEvent.click(screen.getByRole('button', { name: /^object$/i }))
      fireEvent.mouseDown(menubar().getByText('Model Health…'))
    }

    it('is closed at launch and is not a tray section', async () => {
      await renderAndLoad()
      expect(screen.queryByRole('dialog', { name: 'Model Health' })).not.toBeInTheDocument()
      expect(screen.queryByRole('button', { name: /^model health$/i })).not.toBeInTheDocument()
      expect(document.querySelector('[data-tray-section-body="Model Health"]')).toBeNull()
      expect(report).not.toHaveBeenCalled()
    })

    it('is no longer in the View menu', async () => {
      await renderAndLoad()
      fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
      expect(menubar().getByText('Changes')).toBeInTheDocument()
      expect(menubar().queryByText('Model Health')).not.toBeInTheDocument()
      expect(menubar().queryByText('Model Health…')).not.toBeInTheDocument()
    })

    it('Object ▸ Model Health… opens it, and opening again keeps one window', async () => {
      await renderAndLoad()
      openFromObjectMenu()
      const win = await screen.findByRole('dialog', { name: 'Model Health' })
      expect(win).toHaveAttribute('aria-modal', 'false')
      expect(win).toHaveAttribute('data-model-health-window')
      expect(within(win).getByText('Wall')).toBeInTheDocument()
      // Opening again brings the same window forward: it takes focus back.
      screen.getByRole('button', { name: /^object$/i }).focus()
      openFromObjectMenu()
      expect(screen.getAllByRole('dialog', { name: 'Model Health' })).toHaveLength(1)
      await waitFor(() => expect(screen.getByRole('dialog', { name: 'Model Health' })).toHaveFocus())
    })

    it('closing it ends only an isolation it made, never one from elsewhere', async () => {
      await renderAndLoad()
      // Nothing isolated by the window: closing it leaves isolation alone
      // (the Make Solid wizard's, say).
      openFromObjectMenu()
      let win = await screen.findByRole('dialog', { name: 'Model Health' })
      fakeViewportApi.clearIsolation.mockClear()
      fireEvent.click(within(win).getByRole('button', { name: 'Close' }))
      expect(fakeViewportApi.clearIsolation).not.toHaveBeenCalled()
      // A row it isolated: closing it ends that isolation.
      openFromObjectMenu()
      win = await screen.findByRole('dialog', { name: 'Model Health' })
      fakeViewportApi.isolate.mockClear()
      fireEvent.click(within(win).getByText('Wall'))
      expect(fakeViewportApi.isolate).toHaveBeenCalled()
      // The row now shows its own Close (cap) button: use the title bar's ✕.
      fireEvent.click(win.querySelector('[data-floating-window-title] button')!)
      expect(fakeViewportApi.clearIsolation).toHaveBeenCalledTimes(1)
    })

    it('the ✕ closes it: the panel unmounts and its overlay clears', async () => {
      await renderAndLoad()
      openFromObjectMenu()
      const win = await screen.findByRole('dialog', { name: 'Model Health' })
      expect(win.querySelector('[data-model-health]')).not.toBeNull()
      setHealthOverlay.mockClear()
      fireEvent.click(within(win).getByRole('button', { name: 'Close' }))
      expect(screen.queryByRole('dialog', { name: 'Model Health' })).not.toBeInTheDocument()
      expect(document.querySelector('[data-model-health]')).toBeNull()
      expect(setHealthOverlay).toHaveBeenLastCalledWith(null)
    })

    it('reports the side it covers to the camera framing, and clears it on close', async () => {
      const cw = vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(1000)
      const ch = vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(700)
      try {
        window.localStorage.removeItem('hew.ui.modelHealthWindow')
        await renderAndLoad()
        setFramingInset.mockClear()
        openFromObjectMenu()
        const win = await screen.findByRole('dialog', { name: 'Model Health' })
        // Default place: 340 wide, 12 px from the right edge of a 1000 px viewport.
        await waitFor(() => expect(setFramingInset).toHaveBeenLastCalledWith(352, 0))
        fireEvent.click(within(win).getByRole('button', { name: 'Close' }))
        await waitFor(() => expect(setFramingInset).toHaveBeenLastCalledWith(0, 0))
      } finally {
        cw.mockRestore()
        ch.mockRestore()
        window.localStorage.removeItem('hew.ui.modelHealthWindow')
      }
    })
  })

  const importHouse = async () => {
    mockScene.import_skp = vi.fn(() => ({
      objects_created: 4,
      lines_added: 0,
      watertight: 3,
      leaky: 1,
      health: { objects: 4, solids: 3, flat_sheets: 0, bent_sheets: 0, cappable: 1, holes: 0, mixed: 0, non_manifold: 0, seam_families: 0 },
      skipped: [] as { name: string; reason: string }[],
      textures_missing: [] as string[],
      warnings: [] as string[],
    }))
    await renderAndLoad()
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('Import…'))
    const dialog = await screen.findByRole('dialog', { name: /import report/i })
    expect(dialog).toHaveTextContent('Imported 4 objects. 3 are solid. The other one is a panel, a sheet, or an open shape')
    return dialog
  }

  it('the import report\'s Make Solid… opens the wizard on the last import', async () => {
    const dialog = await importHouse()
    fireEvent.click(within(dialog).getByRole('button', { name: 'Make Solid…' }))
    expect(await screen.findByRole('dialog', { name: 'Make Solid' })).toBeInTheDocument()
    expect(screen.queryByRole('dialog', { name: /import report/i })).not.toBeInTheDocument()
    expect(lastImport).toHaveBeenCalled()
    expect(report).toHaveBeenLastCalledWith(BigUint64Array.from([4n]))
    expect(screen.getByRole('heading', { name: 'Make this import solid' })).toBeInTheDocument()
  })

  it('OK just closes the report; the wizard never opens on its own', async () => {
    const dialog = await importHouse()
    const ok = within(dialog).getByRole('button', { name: 'OK' })
    expect(ok).toHaveFocus()
    fireEvent.click(ok)
    expect(screen.queryByRole('dialog', { name: /import report/i })).not.toBeInTheDocument()
    expect(screen.queryByRole('dialog', { name: 'Make Solid' })).not.toBeInTheDocument()
  })

  it('the STL warning\'s Make Solid… cancels the export and opens the wizard on the offenders', async () => {
    const defaults = { ids: mockScene.object_ids, solid: mockScene.object_solid, name: mockScene.object_name }
    mockScene.object_ids = () => new BigUint64Array([4n, 5n])
    mockScene.object_solid = () => false
    mockScene.object_name = ((id: bigint) => (id === 4n ? 'Wall' : 'Roof')) as typeof mockScene.object_name
    try {
      await renderAndLoad()
      fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
      fireEvent.mouseDown(menubar().getByText('Export…'))
      const exportDialog = await screen.findByRole('dialog', { name: 'Export' })
      fireEvent.change(within(exportDialog).getByLabelText('Format'), { target: { value: 'stl' } })
      fireEvent.click(within(exportDialog).getByRole('button', { name: 'Export' }))
      const warning = await screen.findByRole('dialog', { name: /export stl warning/i })
      expect(warning).toHaveTextContent("These objects aren't solid, so the STL may not print cleanly:")
      expect(within(warning).getByRole('button', { name: 'Export Anyway' })).toHaveFocus()
      fireEvent.click(within(warning).getByRole('button', { name: 'Make Solid…' }))
      expect(await screen.findByRole('dialog', { name: 'Make Solid' })).toBeInTheDocument()
      expect(screen.queryByRole('dialog', { name: /export stl warning/i })).not.toBeInTheDocument()
      // KB-6: the objects were never selected, so the window must not say so.
      expect(screen.getByRole('heading', { name: 'Make these objects solid' })).toBeInTheDocument()
      expect(screen.queryByText(/you selected/)).not.toBeInTheDocument()
      expect(report).toHaveBeenLastCalledWith(BigUint64Array.from([4n, 5n]))
    } finally {
      mockScene.object_ids = defaults.ids
      mockScene.object_solid = defaults.solid
      mockScene.object_name = defaults.name
    }
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

describe('App — Camera menu/rail check-state after a walkthrough handoff (playtest finding 2)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  /** The props App handed the (mocked) Viewport on its last render — mirrors
   * `latestViewportProps`'s pattern in the Section Plane describe block
   * above. `onInternalToolChange` is what the REAL Viewport's switchToolRef
   * calls, at the end of EVERY invocation, with the tool that just actually
   * became active — an explicit prop-driven switch, an auto-handoff, an
   * Escape-to-Select, or a one-shot revert alike. Simulating that call here
   * stands in for the internal transition the mock can't run itself
   * (Viewport/three.js don't work under jsdom); it is what makes
   * `activeTool` — and therefore every menu checkmark, the rail highlight,
   * and the contextual dock, which all read it directly — truthful. */
  function latestViewportToolProps(): {
    onInternalToolChange?: (name: string) => void
    activeTool?: string
    activeToolSeq?: number
  } {
    const calls = vi.mocked(Viewport).mock.calls
    return calls[calls.length - 1][0] as never
  }

  function openCameraMenu() {
    fireEvent.click(screen.getByRole('button', { name: /^camera$/i }))
  }

  // CheckMenuItem commits on mousedown, not click (so the outside-mousedown
  // closer can't race it) — same as the existing "clicking Draw > Arc"
  // test below. `getByText` still matches the label alone once checked:
  // the checkmark lives in its OWN nested `<span>`, and RTL's default text
  // matching only considers an element's DIRECT text-node children, not
  // descendants' — the wrapping label span's checkmark sibling is excluded.
  function clickMenuItem(label: string): void {
    fireEvent.mouseDown(menubar().getByText(label))
  }

  function isChecked(label: string): boolean {
    return (menubar().getByText(label).closest('div')?.textContent ?? '').includes('✓')
  }

  /** What the REAL Viewport's switchToolRef does at the end of EVERY
   * invocation: report the tool that's now actually active through
   * `onInternalToolChange`. The mock can't run switchToolRef itself, so an
   * explicit reselect (which Viewport would immediately re-confirm this
   * way) needs it simulated by hand too. */
  function reportToolChange(name: string): void {
    const { onInternalToolChange } = latestViewportToolProps()
    act(() => { onInternalToolChange?.(name) })
  }

  it('shows Look Around checked (not Position Camera) after the real auto-handoff, and nothing checked after Escape', async () => {
    await renderAndLoad()

    openCameraMenu()
    clickMenuItem('Position Camera') // closes the menu (withClose)
    reportToolChange('Position Camera') // what the real switchToolRef reports on activation
    openCameraMenu()
    expect(isChecked('Position Camera')).toBe(true)

    // The auto-handoff: Position Camera places the eye then switches itself
    // to Look Around via switchToolRef, which now reports the change through
    // onInternalToolChange too — App's `activeTool` state is told, not just
    // the status-bar signal. No menu interaction happens here, so the menu
    // (already open) stays open.
    reportToolChange('Look Around')
    expect(isChecked('Look Around')).toBe(true)
    expect(isChecked('Position Camera')).toBe(false)

    // Escape returns to Select the same way (switchToolRef →
    // onInternalToolChange) — the Camera group should show NOTHING checked.
    reportToolChange('Select')
    expect(isChecked('Look Around')).toBe(false)
    expect(isChecked('Position Camera')).toBe(false)
    expect(isChecked('Walk')).toBe(false)
  })

  it('re-choosing the (visually unchecked, but state-stale) Position Camera entry is NOT a no-op', async () => {
    await renderAndLoad()

    openCameraMenu()
    clickMenuItem('Position Camera') // closes the menu (withClose)
    reportToolChange('Position Camera')
    const seqAfterFirstActivation = latestViewportToolProps().activeToolSeq

    // Simulate the auto-handoff exactly as above — `activeTool` now follows
    // it immediately, becoming 'Look Around'.
    reportToolChange('Look Around')

    // Re-choosing "Position Camera" from the menu: without `toolActivationSeq`
    // forcing it through, a plain setState('Position Camera') would be a
    // completely ordinary (non-stale) state change here and WOULD re-render
    // regardless — so this test's real job is confirming the seq counter
    // still advances on every explicit reselect, the behavior
    // `toolActivationSeq` exists to guarantee independent of whether
    // `activeTool` happened to already match.
    openCameraMenu()
    clickMenuItem('Position Camera') // closes the menu again
    const seqAfterReselect = latestViewportToolProps().activeToolSeq
    expect(seqAfterReselect).toBeGreaterThan(seqAfterFirstActivation ?? -1)

    // The real switchToolRef would immediately re-confirm the activation.
    reportToolChange('Position Camera')

    // And the menu now shows it checked again.
    openCameraMenu()
    expect(isChecked('Position Camera')).toBe(true)
  })

  it('finding A: Escape from Look Around highlights Select in the tool rail too, not just the menus', async () => {
    await renderAndLoad()

    openCameraMenu()
    clickMenuItem('Position Camera')
    reportToolChange('Position Camera')
    reportToolChange('Look Around') // auto-handoff

    // Escape-to-Select — the rail (which reads `activeTool` directly, not a
    // menu-only derivation) must pick this up exactly like the menus do.
    reportToolChange('Select')
    expect(screen.getByTitle('Select (Spc)')).toHaveAttribute('aria-checked', 'true')
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

  it('a composing keystroke never switches tools, and the physical key wins over the produced character', async () => {
    await renderAndLoad()
    // Japanese IME mid-composition: Chromium reports key "Process" with
    // isComposing; nothing may dispatch.
    fireEvent.keyDown(document, { key: 'Process', code: 'KeyR', isComposing: true })
    fireEvent.click(screen.getByRole('button', { name: /^draw$/i }))
    expect(menubar().getByText('Rectangle').closest('div')?.textContent).not.toContain('✓')
    fireEvent.keyDown(document, { key: 'Escape' })
    // IME off but a full-width character produced: the physical R key still
    // means Rectangle.
    fireEvent.keyDown(document, { key: 'ｒ', code: 'KeyR' })
    fireEvent.click(screen.getByRole('button', { name: /^draw$/i }))
    expect(menubar().getByText('Rectangle').closest('div')?.textContent).toContain('✓')
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
    // The bare-letter shortcut goes through `activateTool('Pan')`, which
    // sets `activeTool` directly — every menu checkmark (Camera included)
    // reads `activeTool` itself now, with no separate derivation step, so
    // no simulated Viewport callback is needed to see it reflected here.
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

  it('Ctrl+/ opens the command palette', async () => {
    await renderAndLoad()
    expect(screen.queryByRole('dialog', { name: /command palette/i })).not.toBeInTheDocument()
    fireEvent.keyDown(document, { key: '/', ctrlKey: true })
    expect(screen.getByRole('dialog', { name: /command palette/i })).toBeInTheDocument()
  })

  it('Cmd+/ opens the command palette (the same key on a Mac)', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: '/', metaKey: true })
    expect(screen.getByRole('dialog', { name: /command palette/i })).toBeInTheDocument()
  })

  // Ctrl+K was the documented key before `/` unified the binding across
  // platforms; it stays wired so the muscle memory (and the e2e specs that
  // still press it) keep working.
  it('Ctrl+K still opens the command palette as a legacy alias', async () => {
    await renderAndLoad()
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

  it('View menu checkmark tracks the Model Info visibility', async () => {
    await renderAndLoad()
    // Initially showModelInfo=true — View > Model Info has checkmark
    fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
    const modelInfoItem = menubar().getByText('Model Info').closest('div')
    expect(modelInfoItem?.textContent).toContain('✓')

    // Toggle off via keyboard
    fireEvent.keyDown(document, { key: 'I', ctrlKey: true, shiftKey: true })
    // Close the open dropdown first (fireEvent.mouseDown on outside element)
    fireEvent.mouseDown(document.body)
    // Re-open View menu
    await waitFor(() => {
      fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
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
    setTrayLayout({ modelInfo: false, objectInfo: false, materials: true, components: false, tags: false, scenes: false, changes: false })
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
    expect(getTrayLayout().components).toBe(false)
    expect(getTrayLayout().tags).toBe(false)
  })

  it('the Ctrl+Shift+I shortcut also persists (shortcuts keep working unchanged)', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'I', ctrlKey: true, shiftKey: true })
    await waitFor(() => expect(getTrayLayout().modelInfo).toBe(false))
  })

  it('expanding the Components section persists to the trayLayout singleton', async () => {
    await renderAndLoad()
    expect(getTrayLayout().components).toBe(false)
    fireEvent.click(screen.getByRole('button', { name: /^components$/i }))
    await waitFor(() => expect(getTrayLayout().components).toBe(true))
  })

  it('the Ctrl+Shift+M shortcut also toggles the Components section', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: 'M', ctrlKey: true, shiftKey: true })
    await waitFor(() => expect(getTrayLayout().components).toBe(true))
  })
})

describe('App — hiding the tray and compacting the tool rail', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })
  // Both are persisted; clear them so later suites start with the default
  // shell (tray shown, rail labeled).
  afterEach(() => {
    window.localStorage.removeItem('hew.trayCollapsed')
    window.localStorage.removeItem('hew.railNarrow')
    window.localStorage.removeItem('hew.dockHidden')
  })

  const tray = () => screen.queryByRole('complementary', { name: 'Tray' })
  /** Opens the View menu (closing whatever menu is already open — a click
   *  on an open trigger would toggle it shut) and returns an item's row. */
  const viewItem = (label: string) => {
    fireEvent.mouseDown(document.body)
    fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
    return menubar().getByText(label, { exact: true }).closest('div')!
  }

  it('View ▸ Tray hides the tray and the Show tray tab brings it back', async () => {
    await renderAndLoad()
    expect(viewItem('Tray').textContent).toContain('✓')
    fireEvent.mouseDown(viewItem('Tray'))
    await waitFor(() => expect(tray()).toBeNull())
    expect(window.localStorage.getItem('hew.trayCollapsed')).toBe('1')
    expect(viewItem('Tray').textContent).not.toContain('✓')
    fireEvent.mouseDown(document.body)
    fireEvent.click(screen.getByRole('button', { name: 'Show tray' }))
    await waitFor(() => expect(tray()).not.toBeNull())
    expect(screen.queryByRole('button', { name: 'Show tray' })).toBeNull()
  })

  it('takes no keyboard shortcut — Ctrl+\\ leaves the tray alone', async () => {
    await renderAndLoad()
    fireEvent.keyDown(document, { key: '\\', code: 'Backslash', ctrlKey: true })
    fireEvent.keyDown(document, { key: '\\', code: 'Backslash', metaKey: true })
    expect(tray()).not.toBeNull()
  })

  it('with the tray hidden, section check marks read unchecked and choosing one reopens the tray with it', async () => {
    await renderAndLoad()
    expect(screen.getByRole('button', { name: /outliner/i })).toHaveAttribute('aria-expanded', 'true')
    fireEvent.click(screen.getByRole('button', { name: 'Hide tray' }))
    await waitFor(() => expect(tray()).toBeNull())

    const item = viewItem('Model Info')
    expect(item.textContent).not.toContain('✓')
    fireEvent.mouseDown(item)
    await waitFor(() => expect(tray()).not.toBeNull())
    // Reopened with the section open — not toggled shut behind the scenes.
    expect(screen.getByRole('button', { name: /outliner/i })).toHaveAttribute('aria-expanded', 'true')
  })

  it('hiding the tray from a menu blurs a field focused inside it first, then focuses the Show tray tab', async () => {
    await renderAndLoad()
    const filter = screen.getByLabelText('Filter outliner')
    filter.focus()
    // Blur while still in the document — the moment a blur-to-commit field
    // (Object Info's Name, a dimension) saves what was typed.
    let blurredWhileMounted = false
    filter.addEventListener('blur', () => { blurredWhileMounted = filter.isConnected })
    // A menu item that doesn't move focus, as a macOS native accelerator
    // (or a WebKit menu click) doesn't.
    fireEvent.mouseDown(viewItem('Tray'))
    await waitFor(() => expect(tray()).toBeNull())
    expect(blurredWhileMounted).toBe(true)
    await waitFor(() => expect(screen.getByRole('button', { name: 'Show tray' })).toHaveFocus())

    // And back: showing it from the focused tab hands focus to Hide tray.
    fireEvent.click(screen.getByRole('button', { name: 'Show tray' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Hide tray' })).toHaveFocus())
  })

  it('hiding the tray from elsewhere leaves focus where it was', async () => {
    await renderAndLoad()
    const view = screen.getByRole('button', { name: /^view$/i })
    view.focus()
    fireEvent.mouseDown(viewItem('Tray'))
    await waitFor(() => expect(tray()).toBeNull())
    expect(view).toHaveFocus()
  })

  it('closing a section from a menu blurs a field focused inside that section', async () => {
    await renderAndLoad()
    const filter = screen.getByLabelText('Filter outliner')
    filter.focus()
    let blurredWhileMounted = false
    filter.addEventListener('blur', () => { blurredWhileMounted = filter.isConnected })
    fireEvent.mouseDown(viewItem('Model Info'))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /outliner/i })).toHaveAttribute('aria-expanded', 'false'),
    )
    expect(blurredWhileMounted).toBe(true)
  })

  it('closing a different section leaves a focused field alone', async () => {
    await renderAndLoad()
    const filter = screen.getByLabelText('Filter outliner')
    filter.focus()
    fireEvent.mouseDown(viewItem('Object Info'))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /object info/i })).toHaveAttribute('aria-expanded', 'false'),
    )
    expect(filter).toHaveFocus()
  })

  it('View ▸ Compact Tool Rail compacts the rail and persists it', async () => {
    await renderAndLoad()
    expect(viewItem('Compact Tool Rail').textContent).not.toContain('✓')
    fireEvent.mouseDown(viewItem('Compact Tool Rail'))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Expand tool rail' })).toBeInTheDocument())
    expect(window.localStorage.getItem('hew.railNarrow')).toBe('1')
    expect(viewItem('Compact Tool Rail').textContent).toContain('✓')
  })

  it('View ▸ Contextual Dock turns the dock off and back on, and persists it', async () => {
    await renderAndLoad()
    const dock = () => document.querySelector('[data-dock-context]')
    expect(dock()).not.toBeNull()
    expect(viewItem('Contextual Dock').textContent).toContain('✓')
    fireEvent.mouseDown(viewItem('Contextual Dock'))
    await waitFor(() => expect(dock()).toBeNull())
    expect(window.localStorage.getItem('hew.dockHidden')).toBe('1')
    expect(viewItem('Contextual Dock').textContent).not.toContain('✓')
    fireEvent.mouseDown(viewItem('Contextual Dock'))
    await waitFor(() => expect(dock()).not.toBeNull())
    expect(window.localStorage.getItem('hew.dockHidden')).toBe('0')
  })

  it('a dock turned off in an earlier session starts off', async () => {
    window.localStorage.setItem('hew.dockHidden', '1')
    await renderAndLoad()
    expect(document.querySelector('[data-dock-context]')).toBeNull()
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
    lines_added: 0,
    watertight: 0,
    leaky: 0,
    health: { objects: 0, solids: 0, flat_sheets: 0, bent_sheets: 0, cappable: 0, holes: 0, mixed: 0, non_manifold: 0, seam_families: 0 },
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

  it('latches the kernel-panic banner when the import traps the wasm kernel', async () => {
    // A wasm trap mid-import (an out-of-memory abort on a model too big for
    // the browser kernel) surfaces as the engine's "unreachable" error and
    // leaves the Scene borrow-locked: the toast still shows, and the sticky
    // banner (latched by handleToast's panic-text check) says the session
    // is unusable. Pins the import path against a toast that stops latching.
    mockScene.import_skp = vi.fn(() => {
      throw new Error('Unreachable code should not be executed')
    })

    await renderAndLoad()
    triggerImport()

    await waitFor(() => expect(mockScene.import_skp).toHaveBeenCalled())
    expect(await screen.findByText(/Import failed: .*unreachable/i)).toBeInTheDocument()
    expect(screen.getByText(/kernel error occurred/i)).toBeInTheDocument()
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
    lines_added: 0,
    watertight: 1,
    leaky: 0,
    health: { objects: 0, solids: 0, flat_sheets: 0, bent_sheets: 0, cappable: 0, holes: 0, mixed: 0, non_manifold: 0, seam_families: 0 },
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
    lines_added: 0,
    watertight: 1,
    leaky: 0,
    health: { objects: 0, solids: 0, flat_sheets: 0, bent_sheets: 0, cappable: 0, holes: 0, mixed: 0, non_manifold: 0, seam_families: 0 },
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
    // The kernel-stop tests below leave module state behind if an
    // assertion fails partway; clear it here so one failure stays one.
    delete (globalThis as { __hewLastPanic?: unknown }).__hewLastPanic
    clearKernelStop()
    vi.mocked(kernelMemoryBytes).mockReturnValue(null)
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

  it('a .hew that fails to load is kept for a bug report, and the next open that lands drops it', async () => {
    clearOpenAttempt()
    mockScene.load = vi.fn(() => {
      throw new Error('LoadError: bad manifest')
    })
    vi.mocked(fakeFileHost.openAny).mockResolvedValue({
      kind: 'hew',
      name: 'broken.hew',
      bytes: new Uint8Array([9, 9]),
      handle: null,
    })

    await renderAndLoad()
    triggerOpen()

    await waitFor(() => expect(getFailedOpen()).not.toBeNull())
    const held = getFailedOpen()!
    expect(held.name).toBe('broken.hew')
    expect(held.label).toBe('Hew document (.hew)')
    expect(held.bytes).toEqual(new Uint8Array([9, 9]))
    expect(held.error).toMatch(/bad manifest/)

    mockScene.load = defaultLoad
    vi.mocked(fakeFileHost.openAny).mockResolvedValue({
      kind: 'hew',
      name: 'good.hew',
      bytes: new Uint8Array([7]),
      handle: null,
    })
    triggerOpen()
    await waitFor(() => expect(screen.getByText('good.hew')).toBeInTheDocument())
    expect(getFailedOpen()).toBeNull()
  })

  it('an open that runs the kernel out of memory names the cause and hands the page to the crash screen', async () => {
    clearOpenAttempt()
    clearKernelStop()
    delete (globalThis as { __hewLastPanic?: unknown }).__hewLastPanic
    // Rust's out-of-memory abort is a bare wasm trap with memory at the
    // 4 GiB wasm32 ceiling: no panic hook runs, so the page is what names it.
    vi.mocked(kernelMemoryBytes).mockReturnValue(4095.9 * 1024 * 1024)
    mockScene.load = vi.fn(() => {
      throw new WebAssembly.RuntimeError('unreachable')
    })
    vi.mocked(fakeFileHost.openAny).mockResolvedValue({
      kind: 'hew',
      name: 'Big.hew',
      bytes: new Uint8Array(1024),
      handle: null,
    })
    const panicEvent = vi.fn()
    window.addEventListener(KERNEL_PANIC_EVENT, panicEvent)

    try {
      await renderAndLoad()
      triggerOpen()
      await waitFor(() => expect(panicEvent).toHaveBeenCalledTimes(1))
    } finally {
      window.removeEventListener(KERNEL_PANIC_EVENT, panicEvent)
      vi.mocked(kernelMemoryBytes).mockReturnValue(null)
    }

    // The file is kept for the report, with the cause as its error.
    const held = getFailedOpen()!
    expect(held.name).toBe('Big.hew')
    expect(held.error).toMatch(/ran out of memory opening it; kernel memory 4095\.9 MB/)
    // The crash screen gets the explanation; the capture reads like the hook's.
    expect(getKernelStop()?.headline).toBe('This model is too large for Hew to open')
    expect(getKernelStop()?.detail).toMatch(/opening Big\.hew \(1\.0 KB\)/)
    expect(getPanicCapture()?.message).toBe('Out of memory opening Big.hew (1.0 KB); kernel memory 4095.9 MB')
    expect(getPanicCapture()?.recording).toBeNull()
    delete (globalThis as { __hewLastPanic?: unknown }).__hewLastPanic
    clearKernelStop()
  })

  it('an open that traps with memory to spare is a stop, not an out-of-memory', async () => {
    clearOpenAttempt()
    clearKernelStop()
    delete (globalThis as { __hewLastPanic?: unknown }).__hewLastPanic
    vi.mocked(kernelMemoryBytes).mockReturnValue(512 * 1024 * 1024)
    mockScene.load = vi.fn(() => {
      throw new WebAssembly.RuntimeError('memory access out of bounds')
    })
    vi.mocked(fakeFileHost.openAny).mockResolvedValue({
      kind: 'hew',
      name: 'Odd.hew',
      bytes: new Uint8Array(16),
      handle: null,
    })
    const panicEvent = vi.fn()
    window.addEventListener(KERNEL_PANIC_EVENT, panicEvent)

    try {
      await renderAndLoad()
      triggerOpen()
      await waitFor(() => expect(panicEvent).toHaveBeenCalledTimes(1))
    } finally {
      window.removeEventListener(KERNEL_PANIC_EVENT, panicEvent)
      vi.mocked(kernelMemoryBytes).mockReturnValue(null)
    }

    expect(getFailedOpen()!.error).toMatch(/Hew stopped while opening it \(memory access out of bounds; kernel memory 512\.0 MB\)/)
    expect(getKernelStop()?.headline).toBe('Hew stopped while opening the file')
    expect(getPanicCapture()?.message).toMatch(/^Kernel trap opening Odd\.hew \(16 B\): memory access out of bounds/)
    delete (globalThis as { __hewLastPanic?: unknown }).__hewLastPanic
    clearKernelStop()
  })

  it('a trap after a real panic keeps the panic as the cause, not the memory', async () => {
    clearOpenAttempt()
    // The Rust panic hook runs before the trap reaches the catch: by then
    // the capture (with the real message) exists and the crash screen is up.
    ;(globalThis as { __hewLastPanic?: unknown }).__hewLastPanic = {
      at: '2026-01-01T00:00:00.000Z',
      message: 'panicked at crates/kernel/src/document.rs:42: capacity overflow',
      recording: '{"version":2,"calls":[],"golden_hash":0}',
    }
    vi.mocked(kernelMemoryBytes).mockReturnValue(4095.9 * 1024 * 1024)
    mockScene.load = vi.fn(() => {
      throw new WebAssembly.RuntimeError('unreachable')
    })
    vi.mocked(fakeFileHost.openAny).mockResolvedValue({
      kind: 'hew',
      name: 'Big.hew',
      bytes: new Uint8Array(1024),
      handle: null,
    })
    const panicEvent = vi.fn()
    window.addEventListener(KERNEL_PANIC_EVENT, panicEvent)
    try {
      await renderAndLoad()
      triggerOpen()
      await waitFor(() => expect(getFailedOpen()?.error).not.toBeNull())
    } finally {
      window.removeEventListener(KERNEL_PANIC_EVENT, panicEvent)
    }

    expect(getFailedOpen()!.error).toMatch(/internal error opening it \(panicked at .*capacity overflow; kernel memory 4095\.9 MB\)/)
    expect(getKernelStop()).toBeNull()
    expect(panicEvent).not.toHaveBeenCalled()
    // The hook's capture, recording included, is left alone.
    expect(getPanicCapture()?.recording).toBe('{"version":2,"calls":[],"golden_hash":0}')
  })

  it('a typed refusal leaves the kernel running: no stop, no panic event', async () => {
    clearOpenAttempt()
    mockScene.load = vi.fn(() => {
      throw new Error('LoadError: bad manifest')
    })
    vi.mocked(fakeFileHost.openAny).mockResolvedValue({
      kind: 'hew',
      name: 'broken.hew',
      bytes: new Uint8Array([9, 9]),
      handle: null,
    })
    const panicEvent = vi.fn()
    window.addEventListener(KERNEL_PANIC_EVENT, panicEvent)
    try {
      await renderAndLoad()
      triggerOpen()
      await waitFor(() => expect(getFailedOpen()).not.toBeNull())
    } finally {
      window.removeEventListener(KERNEL_PANIC_EVENT, panicEvent)
    }
    expect(panicEvent).not.toHaveBeenCalled()
    expect(getKernelStop()).toBeNull()
    expect(getPanicCapture()).toBeNull()
  })

  it('records a successful .hew open into the web recents store (io/recents.ts)', async () => {
    vi.mocked(fakeFileHost.openAny).mockResolvedValue({
      kind: 'hew',
      name: 'my-house.hew',
      bytes: new Uint8Array([7, 7, 7]),
      handle: null, // web build: no filesystem path/handle
    })

    await renderAndLoad()
    triggerOpen()

    await waitFor(() => expect(mockScene.load).toHaveBeenCalledWith(new Uint8Array([7, 7, 7])))
    expect(vi.mocked(recordRecent)).toHaveBeenCalledWith(new Uint8Array([7, 7, 7]), 'my-house.hew')
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

    // Cancelling the in-app UnsavedChangesDialog (Lane C) leaves the open
    // document completely untouched — the pick's own openAny() is never
    // even reached (confirmDiscard runs before it, on the web fallback
    // path), so it's still just the one call from opening my-house.hew.
    triggerOpen()
    const cancelDialog = await screen.findByRole('dialog', { name: /unsaved changes/i })
    fireEvent.click(within(cancelDialog).getByRole('button', { name: /^cancel$/i }))
    await waitFor(() => expect(screen.queryByRole('dialog', { name: /unsaved changes/i })).not.toBeInTheDocument())
    expect(vi.mocked(fakeFileHost.openAny)).toHaveBeenCalledTimes(1)
    expect(screen.getByText('my-house.hew')).toBeInTheDocument()
    expect(mockScene.load).toHaveBeenCalledTimes(1)

    // Don't Save discards it and replaces the document in place — the web
    // build has no Tauri window to open the pick into instead.
    triggerOpen()
    const discardDialog = await screen.findByRole('dialog', { name: /unsaved changes/i })
    fireEvent.click(within(discardDialog).getByRole('button', { name: /don.t save/i }))
    await waitFor(() => expect(mockScene.load).toHaveBeenCalledTimes(2))
    expect(await screen.findByText('other.hew')).toBeInTheDocument()
  })

  describe('File ▸ Revert to Saved', () => {
    const savedBytes = new Uint8Array([4, 5, 6])
    // A web file handle the way the picker hands one back.
    const savedHandle = { getFile: vi.fn(async () => new File([savedBytes], 'my-house.hew')) }
    const clickRevert = () => {
      // A click on a disabled item leaves the menu open: open it only once.
      if (menubar().queryByText('Revert to Saved') === null) {
        fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
      }
      fireEvent.mouseDown(menubar().getByText('Revert to Saved'))
    }
    const dirtyIt = () =>
      act(() => {
        ;(window as unknown as { __hew_test: { addNodeTag: (k: string, i: string, p: string[]) => void } })
          .__hew_test.addNodeTag('object', '1', ['tag'])
      })
    const openSaved = async () => {
      vi.mocked(fakeFileHost.openAny).mockResolvedValueOnce({
        kind: 'hew',
        name: 'my-house.hew',
        bytes: new Uint8Array([7, 7, 7]),
        handle: savedHandle,
      })
      await renderAndLoad()
      triggerOpen()
      expect(await screen.findByText('my-house.hew')).toBeInTheDocument()
    }

    it('does nothing while the document is clean, then asks once with Cancel as the default', async () => {
      await openSaved()
      clickRevert()
      expect(screen.queryByRole('dialog', { name: /revert to saved/i })).not.toBeInTheDocument()

      dirtyIt()
      clickRevert()
      const dialog = await screen.findByRole('dialog', { name: /revert to saved/i })
      expect(within(dialog).getByText('This throws away everything you changed since the last save.')).toBeInTheDocument()
      expect(within(dialog).getByRole('button', { name: 'Cancel' })).toHaveFocus()

      // Cancel changes nothing: no reload.
      fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }))
      await waitFor(() => expect(screen.queryByRole('dialog', { name: /revert to saved/i })).not.toBeInTheDocument())
      expect(mockScene.load).toHaveBeenCalledTimes(1)
    })

    it('does nothing for a document with no file to return to', async () => {
      vi.mocked(fakeFileHost.openAny).mockResolvedValueOnce({
        kind: 'hew',
        name: 'my-house.hew',
        bytes: new Uint8Array([7, 7, 7]),
        handle: null, // the browser download fallback leaves no handle
      })
      await renderAndLoad()
      triggerOpen()
      expect(await screen.findByText('my-house.hew')).toBeInTheDocument()
      dirtyIt()
      clickRevert()
      expect(screen.queryByRole('dialog', { name: /revert to saved/i })).not.toBeInTheDocument()
    })

    it('Revert re-reads the saved file into the window, clears dirty and history, and says so', async () => {
      const priorCanUndo = mockScene.can_scene_undo
      let history = false
      mockScene.can_scene_undo = () => history
      // A load is a fresh document: the kernel's history is empty again.
      mockScene.load = vi.fn(() => {
        history = false
      })
      try {
        await openSaved()
        history = true
        dirtyIt()
        expect(mockScene.can_scene_undo()).toBe(true)
        clickRevert()
        const dialog = await screen.findByRole('dialog', { name: /revert to saved/i })
        fireEvent.click(within(dialog).getByRole('button', { name: 'Revert' }))

        await waitFor(() => expect(mockScene.load).toHaveBeenLastCalledWith(savedBytes))
        expect(savedHandle.getFile).toHaveBeenCalled()
        expect(await screen.findByText('Reverted to the saved file.')).toBeInTheDocument()
        // Same window, same file, no longer dirty: Revert is off again.
        expect(screen.getByText('my-house.hew')).toBeInTheDocument()
        clickRevert()
        expect(screen.queryByRole('dialog', { name: /revert to saved/i })).not.toBeInTheDocument()
        // The history is gone: nothing left to undo.
        expect(mockScene.can_scene_undo()).toBe(false)
      } finally {
        mockScene.can_scene_undo = priorCanUndo
      }
    })

    // ---- review follow-ups -------------------------------------------------
    const kept = { tag: 'camera-before-revert' }
    // A viewport that has a camera to keep; every other call is a spy.
    const cameraViewport = () => {
      const api: Record<string, unknown> = {
        getCameraState: vi.fn(() => kept),
        applyCameraState: vi.fn(),
        zoomExtents: vi.fn(),
        setRenderMode: vi.fn(),
        getEffectiveHiddenIds: () => ({ objects: [], instances: [] }),
      }
      const proxy = new Proxy(api, {
        get: (t, k) => {
          if (typeof k === 'string' && !(k in t)) t[k] = vi.fn()
          return t[k as string]
        },
      })
      vi.mocked(Viewport).mockImplementation(((props: { apiRef?: { current: unknown } }) => {
        if (props.apiRef !== undefined) props.apiRef.current = proxy
        return null
      }) as never)
      return api as { getCameraState: ReturnType<typeof vi.fn>; applyCameraState: ReturnType<typeof vi.fn>; zoomExtents: ReturnType<typeof vi.fn>; setRenderMode: ReturnType<typeof vi.fn> }
    }
    const confirmRevert = async () => {
      clickRevert()
      const dialog = await screen.findByRole('dialog', { name: /revert to saved/i })
      fireEvent.click(within(dialog).getByRole('button', { name: 'Revert' }))
    }
    afterEach(() => {
      vi.mocked(Viewport).mockImplementation((() => null) as never)
      tauriMode.on = false
      mockScene.at_saved_mark = () => false
    })

    it('keeps the camera the person was looking through (a reload would otherwise reframe)', async () => {
      await openSaved()
      dirtyIt()
      // Installed after the edit: the harness's edit path only reaches the
      // document through a real viewport, which this one is not.
      const api = cameraViewport()
      await confirmRevert()
      expect(await screen.findByText('Reverted to the saved file.')).toBeInTheDocument()
      await waitFor(() => expect(api.applyCameraState).toHaveBeenCalledWith(kept))
    })

    describe('a .skp document', () => {
      const report = {
        container: 'skp2026',
        objects_created: 0,
        lines_added: 0,
        watertight: 0,
        leaky: 0,
        health: { objects: 0, solids: 0, flat_sheets: 0, bent_sheets: 0, cappable: 0, holes: 0, mixed: 0, non_manifold: 0, seam_families: 0 },
        skipped: [],
        textures_missing: [],
        warnings: [],
      }
      const scene = mockScene as unknown as Record<string, unknown>
      beforeEach(() => {
        scene.open_skp_begin = vi.fn(() => ({ phase: 'done', done: 0, total: 0, report }))
        scene.open_skp_step = vi.fn(() => ({ phase: 'done', done: 0, total: 0, report }))
      })
      afterEach(() => {
        delete scene.open_skp_begin
        delete scene.open_skp_step
      })

      it('revert keeps the camera, does not zoom to fit, and does not reopen the import report', async () => {
        vi.mocked(fakeFileHost.openAny).mockResolvedValueOnce({
          kind: 'skp',
          name: 'house.skp',
          bytes: new Uint8Array([8, 8]),
          handle: savedHandle,
        })
        await renderAndLoad()
        triggerOpen()
        // Opening shows the report once; the person dismisses it.
        const opened = await screen.findByRole('dialog', { name: 'Open report' })
        fireEvent.click(within(opened).getByRole('button', { name: /close|ok|done|continue/i }))
        await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Open report' })).not.toBeInTheDocument())
        dirtyIt()
        const api = cameraViewport()

        await confirmRevert()
        expect(await screen.findByText('Reverted to the saved file.')).toBeInTheDocument()
        await waitFor(() => expect(api.applyCameraState).toHaveBeenCalledWith(kept))
        expect(api.zoomExtents).not.toHaveBeenCalled()
        expect(screen.queryByRole('dialog', { name: 'Open report' })).not.toBeInTheDocument()
      })

      it('refuses with a toast while another .skp open is still stepping', async () => {
        vi.mocked(fakeFileHost.openAny).mockResolvedValueOnce({
          kind: 'skp',
          name: 'house.skp',
          bytes: new Uint8Array([8, 8]),
          handle: savedHandle,
        })
        await renderAndLoad()
        triggerOpen()
        const opened = await screen.findByRole('dialog', { name: 'Open report' })
        fireEvent.click(within(opened).getByRole('button', { name: /close|ok|done|continue/i }))
        dirtyIt()
        // The first revert's open keeps stepping until the test lets it end.
        let finish = false
        scene.open_skp_begin = vi.fn(() => ({ phase: 'read', done: 0, total: 1, report }))
        scene.open_skp_step = vi.fn(() =>
          finish ? { phase: 'done', done: 1, total: 1, report } : { phase: 'read', done: 0, total: 1 },
        )
        try {
          await confirmRevert()
          await waitFor(() => expect(scene.open_skp_begin).toHaveBeenCalled())
          // A second revert while that open is still stepping is refused out loud.
          await confirmRevert()
          expect(await screen.findByText('Another file is still opening; try again in a moment.')).toBeInTheDocument()
        } finally {
          finish = true
        }
        await screen.findByText('Reverted to the saved file.')
      })
    })

    it('a change to the document while the file is being read cancels the revert and says so', async () => {
      await openSaved()
      dirtyIt()
      let finishRead: (f: File) => void = () => {}
      savedHandle.getFile.mockImplementationOnce(() => new Promise<File>((r) => { finishRead = r }))
      await confirmRevert()
      await waitFor(() => expect(savedHandle.getFile).toHaveBeenCalledTimes(1))

      // Meanwhile the person starts a New document, discarding the edits.
      fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
      fireEvent.mouseDown(menubar().getByText('New'))
      const discard = await screen.findByRole('dialog', { name: /unsaved changes/i })
      fireEvent.click(within(discard).getByRole('button', { name: /don.t save/i }))
      expect(await screen.findByText('Untitled')).toBeInTheDocument()
      const loadsBefore = mockScene.load.mock.calls.length

      finishRead(new File([savedBytes], 'my-house.hew'))
      // A generous wait: under a loaded full-suite run the toast can take
      // longer than the 1 s default to land, and it is not the thing under test.
      expect(
        await screen.findByText('Revert canceled: the document changed while the file was being read.', undefined, { timeout: 10000 }),
      ).toBeInTheDocument()
      expect(mockScene.load).toHaveBeenCalledTimes(loadsBefore)
      expect(screen.queryByText('Reverted to the saved file.')).not.toBeInTheDocument()
    })

    it('a document that went clean while the file was being read is left alone, quietly', async () => {
      await openSaved()
      dirtyIt()
      let finishRead: (f: File) => void = () => {}
      savedHandle.getFile.mockImplementationOnce(() => new Promise<File>((r) => { finishRead = r }))
      await confirmRevert()
      await waitFor(() => expect(savedHandle.getFile).toHaveBeenCalledTimes(1))
      // Undo back to the saved mark: the next change reports "at saved mark".
      mockScene.at_saved_mark = () => true
      dirtyIt()
      await act(async () => {})
      // The document reads clean again: Revert is off.
      clickRevert()
      expect(screen.queryByRole('dialog', { name: /revert to saved/i })).not.toBeInTheDocument()
      const loadsBefore = mockScene.load.mock.calls.length

      finishRead(new File([savedBytes], 'my-house.hew'))
      await act(async () => { await Promise.resolve() })
      await new Promise((r) => setTimeout(r, 20))
      expect(mockScene.load).toHaveBeenCalledTimes(loadsBefore)
      expect(screen.queryByText('Reverted to the saved file.')).not.toBeInTheDocument()
      expect(screen.queryByText(/revert canceled/i)).not.toBeInTheDocument()
      // The refusal is spoken, once.
      expect(screen.getAllByText('Nothing to revert: the document is already at its saved state.')).toHaveLength(1)
    })

    it('an edit made while the file is being read cancels the revert and keeps the edit', async () => {
      await openSaved()
      dirtyIt()
      let depth = 3
      const priorDepth = mockScene.undo_depth
      mockScene.undo_depth = () => depth
      try {
        let finishRead: (f: File) => void = () => {}
        savedHandle.getFile.mockImplementationOnce(() => new Promise<File>((r) => { finishRead = r }))
        await confirmRevert()
        await waitFor(() => expect(savedHandle.getFile).toHaveBeenCalledTimes(1))
        // Same file, still dirty, but one more edit lands while the read is pending.
        depth = 4
        const loadsBefore = mockScene.load.mock.calls.length

        finishRead(new File([savedBytes], 'my-house.hew'))
        expect(
          await screen.findByText('Revert canceled: the document changed while the file was being read.'),
        ).toBeInTheDocument()
        expect(mockScene.load).toHaveBeenCalledTimes(loadsBefore)
        expect(screen.queryByText('Reverted to the saved file.')).not.toBeInTheDocument()
      } finally {
        mockScene.undo_depth = priorDepth
      }
    })

    describe('View ▸ X-ray and Wireframe', () => {
      const openView = () => {
        if (menubar().queryByText('X-ray') === null) fireEvent.click(screen.getByRole('button', { name: /^view$/i }))
      }
      const toggle = (label: string) => {
        openView()
        fireEvent.mouseDown(menubar().getByText(label))
      }
      const checked = (label: string) => {
        openView()
        return (menubar().getByText(label).closest('div')?.textContent ?? '').includes('✓')
      }

      it('X-ray and Wireframe are one mode: choosing one turns the other off, choosing it again returns to Shaded', async () => {
        const api = cameraViewport()
        await renderAndLoad()
        toggle('X-ray')
        await waitFor(() => expect(api.setRenderMode).toHaveBeenLastCalledWith('xray'))
        expect(checked('X-ray')).toBe(true)
        expect(checked('Wireframe')).toBe(false)
        toggle('Wireframe')
        await waitFor(() => expect(api.setRenderMode).toHaveBeenLastCalledWith('wireframe'))
        expect(checked('Wireframe')).toBe(true)
        expect(checked('X-ray')).toBe(false)
        toggle('X-ray')
        await waitFor(() => expect(api.setRenderMode).toHaveBeenLastCalledWith('xray'))
        expect(checked('Wireframe')).toBe(false)
        toggle('X-ray')
        await waitFor(() => expect(api.setRenderMode).toHaveBeenLastCalledWith('shaded'))
        expect(checked('X-ray')).toBe(false)
        expect(checked('Wireframe')).toBe(false)
      })

      it('File ▸ New resets the mode to Shaded', async () => {
        const api = cameraViewport()
        await renderAndLoad()
        toggle('Wireframe')
        await waitFor(() => expect(api.setRenderMode).toHaveBeenLastCalledWith('wireframe'))
        fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
        fireEvent.mouseDown(menubar().getByText('New'))
        const discard = screen.queryByRole('dialog', { name: /unsaved changes/i })
        if (discard !== null) fireEvent.click(within(discard).getByRole('button', { name: /don.t save/i }))
        await waitFor(() => expect(api.setRenderMode).toHaveBeenLastCalledWith('shaded'))
        expect(checked('X-ray')).toBe(false)
        expect(checked('Wireframe')).toBe(false)
      })

      it('Revert to Saved keeps the render mode the window is in', async () => {
        await openSaved()
        dirtyIt()
        const api = cameraViewport()
        toggle('Wireframe')
        await waitFor(() => expect(api.setRenderMode).toHaveBeenLastCalledWith('wireframe'))
        await confirmRevert()
        expect(await screen.findByText('Reverted to the saved file.')).toBeInTheDocument()
        expect(api.setRenderMode).toHaveBeenLastCalledWith('wireframe')
        expect(checked('Wireframe')).toBe(true)
      })
    })

    describe('the live hew.doc.revert operation', () => {
      it('says the document was never saved when it has changes but no file', async () => {
        vi.mocked(fakeFileHost.openAny).mockResolvedValueOnce({
          kind: 'hew',
          name: 'my-house.hew',
          bytes: new Uint8Array([7, 7, 7]),
          handle: null,
        })
        await renderAndLoad()
        triggerOpen()
        expect(await screen.findByText('my-house.hew')).toBeInTheDocument()
        dirtyIt()
        await waitFor(() => expect(liveBridge.runOp).not.toBeNull())
        expect(await liveBridge.runOp!('revert', {})).toEqual({
          ok: false,
          refusal: 'nothing_to_revert',
          message: 'There is nothing to revert to: this document has not been saved to a file yet.',
        })
      })

      it('says there are no changes when the document is clean', async () => {
        await openSaved()
        await waitFor(() => expect(liveBridge.runOp).not.toBeNull())
        expect(await liveBridge.runOp!('revert', {})).toEqual({
          ok: false,
          refusal: 'nothing_to_revert',
          message: 'There is nothing to revert: the document has no changes since the last save.',
        })
      })

      it('reverts without asking, and the crash snapshot and recents entry follow (web)', async () => {
        await openSaved()
        dirtyIt()
        recoveryState.clear.mockClear()
        vi.mocked(recordRecent).mockClear()
        await waitFor(() => expect(liveBridge.runOp).not.toBeNull())
        expect(await liveBridge.runOp!('revert', {})).toEqual({ ok: true })
        expect(screen.queryByRole('dialog', { name: /revert to saved/i })).not.toBeInTheDocument()
        expect(recoveryState.clear).toHaveBeenCalled()
        expect(vi.mocked(recordRecent)).toHaveBeenCalledWith(savedBytes, 'my-house.hew')
      })

      it('reads a desktop path with read_file and pushes it to the recents list', async () => {
        vi.mocked(fakeFileHost.openAny).mockResolvedValueOnce({
          kind: 'hew',
          name: 'my-house.hew',
          bytes: new Uint8Array([7, 7, 7]),
          handle: '/tmp/my-house.hew',
        })
        await renderAndLoad()
        triggerOpen()
        expect(await screen.findByText('my-house.hew')).toBeInTheDocument()
        dirtyIt()
        tauriMode.on = true
        tauriCore.invoke.mockClear()
        tauriCore.invoke.mockImplementation(async (cmd: string) =>
          cmd === 'read_file' ? savedBytes.buffer.slice(0) : undefined,
        )
        recoveryState.clear.mockClear()
        await waitFor(() => expect(liveBridge.runOp).not.toBeNull())
        expect(await liveBridge.runOp!('revert', {})).toEqual({ ok: true })
        expect(tauriCore.invoke).toHaveBeenCalledWith('read_file', { path: '/tmp/my-house.hew' })
        await waitFor(() =>
          expect(tauriCore.invoke).toHaveBeenCalledWith('push_recent', { path: '/tmp/my-house.hew' }),
        )
        expect(recoveryState.clear).toHaveBeenCalled()
      })
    })

    it('a saved file that cannot be read leaves the document as it was', async () => {
      await openSaved()
      dirtyIt()
      savedHandle.getFile.mockRejectedValueOnce(new Error('gone'))
      clickRevert()
      const dialog = await screen.findByRole('dialog', { name: /revert to saved/i })
      fireEvent.click(within(dialog).getByRole('button', { name: 'Revert' }))
      expect(await screen.findByText(/revert failed/i)).toBeInTheDocument()
      expect(mockScene.load).toHaveBeenCalledTimes(1)
    })
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

  const blankClean: DocSessionState = { currentRef: null, dirty: false, lastEditAt: null, lastSavedAt: null, nonUndoableReasons: new Set() }
  const blankDirty: DocSessionState = { currentRef: null, dirty: true, lastEditAt: 1, lastSavedAt: null, nonUndoableReasons: new Set() }
  const namedClean: DocSessionState = {
    currentRef: { name: 'house.hew', format: 'hew', handle: '/tmp/house.hew' },
    dirty: false,
    lastEditAt: null,
    lastSavedAt: 1,
    nonUndoableReasons: new Set(),
  }
  const namedDirty: DocSessionState = {
    currentRef: { name: 'house.hew', format: 'hew', handle: '/tmp/house.hew' },
    dirty: true,
    lastEditAt: 1,
    lastSavedAt: 1,
    nonUndoableReasons: new Set(),
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
    // Tokenized (Library effort — jsdom doesn't resolve custom properties on
    // inline styles, so the literal var(...) reference is what's observable
    // here; the resolved value is themed via tokens.css's
    // --status-leaky-bg, unit-tested visually by the app's own theme specs).
    expect(bubble.style.background).toBe('var(--status-leaky-bg)')
  })

  it('renders warning-level refusals as neutral bubbles', async () => {
    await renderAndLoad()
    act(() => latestOnToast()('nothing to undo', 'NothingToUndo'))
    const bubble = screen.getByText(/nothing to undo/).closest('div')!
    expect(bubble.style.background).toBe('var(--surface-overlay)')
  })
})

describe('App — boolean auto-explode failure surfacing (finding 2)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('treats a runBoolean "mutated-failed" result as a committed mutation, not a no-op — it re-derives from the document instead of leaving the stale pre-explode state on screen', async () => {
    // Two plain object operands so Object ▸ Union's gate (canBoolean) opens.
    const priorObjectIds = mockScene.object_ids
    mockScene.object_ids = vi.fn(() => BigUint64Array.from([1n, 2n]))
    // `canUndo` (Edit ▸ Undo's gate) is read directly in App's render body
    // every render — `sceneRef.current?.can_scene_undo() ?? false` — never
    // memoized. The bug this guards against is `handleBoolean` treating
    // `runBoolean`'s failure-after-mutation signal like a bare no-op and
    // skipping `setDocRev` entirely, so nothing downstream ever re-read
    // scene truth (finding 2). If — and only if — `setDocRev` actually
    // fires, App re-renders and this mock is invoked again.
    const priorCanUndo = mockScene.can_scene_undo
    const canUndoMock = vi.fn(() => false)
    mockScene.can_scene_undo = canUndoMock
    try {
      await renderAndLoad()
      act(() => window.__hew_test!.selectObjects(['1', '2']))

      // Populate the (mocked) Viewport's imperative apiRef with a
      // `runBoolean` that reproduces finding 2's exact shape: auto-explode
      // committed real mutations, but the retried boolean still failed.
      const calls = vi.mocked(Viewport).mock.calls
      const { apiRef } = calls[calls.length - 1][0] as unknown as {
        apiRef?: { current: { runBoolean: (op: number, a: unknown, b: unknown) => unknown } | null }
      }
      act(() => {
        if (apiRef !== undefined) apiRef.current = { runBoolean: vi.fn(() => 'mutated-failed') }
      })

      const callsBeforeClick = canUndoMock.mock.calls.length

      fireEvent.click(screen.getByRole('button', { name: /^object$/i }))
      fireEvent.mouseDown(screen.getByText('Union'))

      // A re-render happened — `setDocRev` fired, exactly as the success
      // path already did, instead of the failure silently doing nothing.
      expect(canUndoMock.mock.calls.length).toBeGreaterThan(callsBeforeClick)
    } finally {
      mockScene.object_ids = priorObjectIds
      mockScene.can_scene_undo = priorCanUndo
    }
  })
})

describe('App — session-stack menu gating (docs/design/group-session.md)', () => {
  // Matches every other top-level describe block's own reset (e.g. the
  // Section Plane block above): without it, `vi.mocked(Viewport).mock.calls`
  // accumulates across the whole file.
  beforeEach(() => {
    vi.clearAllMocks()
  })

  /** The `onSessionChange` callback and the (mutable) `apiRef` App handed
   *  the (mocked) Viewport on its last render — mirrors the Section Plane
   *  block's `latestViewportProps`. The real Viewport calls this whenever
   *  the session stack changes; the mock does neither, so the test drives
   *  it by hand. */
  function latestViewportProps(): {
    onSessionChange?: (frames: { node: { kind: string; id: bigint }; label: string }[]) => void
    apiRef?: { current: { runGroup: (nodes: unknown[]) => unknown } | null }
  } {
    const calls = vi.mocked(Viewport).mock.calls
    return calls[calls.length - 1][0] as never
  }

  /** Open the Edit menu — a no-op if it's already open. `MenuItem`'s
   *  `withClose` wrapper only closes the dropdown on an ENABLED item's
   *  click (its `onClick` fires); a disabled item's `onMouseDown` never
   *  calls `onClick`, so the menu stays open after probing one — clicking
   *  the "Edit" trigger again would TOGGLE it closed instead of leaving it
   *  open, breaking a second probe in the same test. Checking for "Group"
   *  first (present iff the Edit menu is open) makes opening idempotent
   *  regardless of which path the previous probe took. */
  function openEditMenu(): void {
    if (menubar().queryByText('Group') === null) {
      fireEvent.click(screen.getByRole('button', { name: /^object$/i }))
    }
  }

  it('Edit > Group stays enabled while only a GROUP frame is open — its product folds into the group at close', async () => {
    const priorObjectIds = mockScene.object_ids
    mockScene.object_ids = vi.fn(() => BigUint64Array.from([1n, 2n]))
    try {
      await renderAndLoad()

      const { apiRef, onSessionChange } = latestViewportProps()
      const runGroup = vi.fn(() => 99n)
      act(() => {
        if (apiRef !== undefined) apiRef.current = { runGroup }
        onSessionChange?.([{ node: { kind: 'group', id: 50n }, label: 'Group 1' }])
      })
      // Select AFTER the session opens: `handleSessionChange` clears the
      // selection on every innermost-frame identity change (design: opening
      // a session on B while some OTHER node was selected must not leave a
      // stale, out-of-scope selection lighting up commands), so selecting
      // first would be wiped out by the session-open itself.
      act(() => window.__hew_test!.selectObjects(['1', '2']))

      openEditMenu()
      fireEvent.mouseDown(menubar().getByText('Group'))
      expect(runGroup).toHaveBeenCalledTimes(1)
    } finally {
      mockScene.object_ids = priorObjectIds
    }
  })

  it('Edit > Group disables once the INNERMOST frame is a COMPONENT — the kernel refuses ExplodeSessionScope for it', async () => {
    const priorObjectIds = mockScene.object_ids
    mockScene.object_ids = vi.fn(() => BigUint64Array.from([1n, 2n]))
    try {
      await renderAndLoad()

      const { apiRef, onSessionChange } = latestViewportProps()
      const runGroup = vi.fn(() => 99n)
      act(() => {
        if (apiRef !== undefined) apiRef.current = { runGroup }
        onSessionChange?.([{ node: { kind: 'instance', id: 60n }, label: 'Box' }])
      })
      act(() => window.__hew_test!.selectObjects(['1', '2']))

      openEditMenu()
      fireEvent.mouseDown(menubar().getByText('Group'))
      // The disabled item's own onMouseDown never calls onClick — runGroup
      // must never be reached.
      expect(runGroup).not.toHaveBeenCalled()
    } finally {
      mockScene.object_ids = priorObjectIds
    }
  })

  it('re-enables Edit > Group once the component frame closes back to a group-only stack (resync, not a one-shot latch)', async () => {
    const priorObjectIds = mockScene.object_ids
    mockScene.object_ids = vi.fn(() => BigUint64Array.from([1n, 2n]))
    try {
      await renderAndLoad()

      const { apiRef, onSessionChange } = latestViewportProps()
      const runGroup = vi.fn(() => 99n)
      act(() => {
        if (apiRef !== undefined) apiRef.current = { runGroup }
        // Nested stack: an outer group with a component frame innermost.
        onSessionChange?.([
          { node: { kind: 'group', id: 50n }, label: 'Group 1' },
          { node: { kind: 'instance', id: 60n }, label: 'Box' },
        ])
      })
      act(() => window.__hew_test!.selectObjects(['1', '2']))
      openEditMenu()
      fireEvent.mouseDown(menubar().getByText('Group'))
      expect(runGroup).not.toHaveBeenCalled()

      // The component frame closes — back to a group-only stack.
      act(() => {
        onSessionChange?.([{ node: { kind: 'group', id: 50n }, label: 'Group 1' }])
      })
      // Re-select (the innermost frame's identity changed, which clears the
      // selection — design: `handleSessionChange` clears on every innermost
      // identity change) before re-checking the gate.
      act(() => window.__hew_test!.selectObjects(['1', '2']))
      openEditMenu()
      fireEvent.mouseDown(menubar().getByText('Group'))
      expect(runGroup).toHaveBeenCalledTimes(1)
    } finally {
      mockScene.object_ids = priorObjectIds
    }
  })
})

describe('App — enterNode entry convergence (adversarial-review finding 1)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    // jsdom does not implement scrollIntoView; DocumentTree calls it on the
    // selected row's mount effect (see panels/scenePanels.test.tsx's own
    // polyfill for the same gap).
    Element.prototype.scrollIntoView = vi.fn()
  })

  /** Mirrors the session-stack menu-gating block's own helper. */
  function latestViewportProps(): {
    onSessionChange?: (frames: { node: { kind: string; id: bigint }; label: string }[]) => void
    apiRef?: { current: Record<string, unknown> | null }
  } {
    const calls = vi.mocked(Viewport).mock.calls
    return calls[calls.length - 1][0] as never
  }

  it('repro (a): double-clicking a session MEMBER row keeps the open session open and pushes the member as an object context, instead of closing the session', async () => {
    await renderAndLoad()
    const { apiRef, onSessionChange } = latestViewportProps()
    const runCloseInnermostSession = vi.fn(() => true)
    const runOpenGroupSession = vi.fn(() => true)
    const runOpenExplodeSessionOrFallback = vi.fn()
    act(() => {
      if (apiRef !== undefined) {
        apiRef.current = {
          // The group's session is open — its only member (object 1) has
          // had its parent link erased by the ungroup posture, so
          // `scene.node_parent` (the mock's default) reports it as a bare
          // top-level object with no ancestor: exactly the state that used
          // to fool `enterNode`'s walk into reading it as "outside every
          // open frame."
          sessionStack: () => [{ kind: 'group', id: 50n }],
          sessionMembers: () => [{ kind: 'object', id: 1n }],
          hasArmedGesture: () => false,
          runCloseInnermostSession,
          runOpenGroupSession,
          runOpenExplodeSessionOrFallback,
        }
      }
      onSessionChange?.([{ node: { kind: 'group', id: 50n }, label: 'Group 1' }])
    })

    // The session-member row nests under the group's synthetic header,
    // labeled positionally ("Object 1" — the group's only, unnamed member).
    fireEvent.doubleClick(screen.getByText('Object 1'))

    // The bug this fixes closed the WHOLE stack (reading the member's
    // parent-erased chain as "outside every open frame") before re-homing
    // the object at the top level. Fixed: the session stays open — no close
    // call at all — and the member is pushed as a plain object context.
    expect(runCloseInnermostSession).not.toHaveBeenCalled()
    expect(runOpenGroupSession).not.toHaveBeenCalled()
    expect(runOpenExplodeSessionOrFallback).not.toHaveBeenCalled()
    expect(window.__hew_test!.getSelection()).toEqual([{ kind: 'object', id: '1' }])
  })

  it('repro (b): double-clicking a nested member GROUP mid-session opens its session directly, without closing the outer frame first', async () => {
    await renderAndLoad()
    const { apiRef, onSessionChange } = latestViewportProps()
    const runCloseInnermostSession = vi.fn(() => true)
    const runOpenGroupSession = vi.fn(() => true)
    const runOpenExplodeSessionOrFallback = vi.fn()
    act(() => {
      if (apiRef !== undefined) {
        apiRef.current = {
          // Group G (id 50) is open; nested group H (id 70) is one of its
          // live top-level members now (not yet opened itself) — H's OWN
          // parent link (originally G) was erased the same way, so
          // `node_parent` reports it with no ancestor either.
          sessionStack: () => [{ kind: 'group', id: 50n }],
          sessionMembers: () => [{ kind: 'group', id: 70n }],
          hasArmedGesture: () => false,
          runCloseInnermostSession,
          runOpenGroupSession,
          runOpenExplodeSessionOrFallback,
        }
      }
      // A distinct label for G's own session-frame header row, so its text
      // can't collide with H's positional member-row label below.
      onSessionChange?.([{ node: { kind: 'group', id: 50n }, label: 'Group G' }])
    })

    // H's positional label as G's sole, unnamed member: "Group 1".
    fireEvent.doubleClick(screen.getByText('Group 1'))

    // The bug this fixes closed G first (reading H's parent-erased chain as
    // "outside every open frame"), then tried to open H — hitting the
    // kernel's `ExplodeSessionNestedGroup` refusal for a session opened on
    // a target that's no longer top-level. Fixed: G is never closed, and H
    // opens directly on top of it.
    expect(runCloseInnermostSession).not.toHaveBeenCalled()
    expect(runOpenGroupSession).toHaveBeenCalledWith(70n)
    expect(runOpenExplodeSessionOrFallback).not.toHaveBeenCalled()
  })

  it('a target genuinely outside the open session still closes the whole stack (unchanged fallback behavior)', async () => {
    const priorTopLevel = mockScene.top_level_nodes
    // An unrelated top-level group (id 90), disjoint from the open session
    // (id 50) and not among its members — the walked chain's root (the
    // group itself) matches none of `sessionMembers()`.
    mockScene.top_level_nodes = () => [{ kind: 'group', id: 90n }]
    try {
      await renderAndLoad()
      const { apiRef, onSessionChange } = latestViewportProps()
      const runCloseInnermostSession = vi.fn(() => true)
      const runOpenGroupSession = vi.fn(() => true)
      act(() => {
        if (apiRef !== undefined) {
          apiRef.current = {
            sessionStack: () => [{ kind: 'group', id: 50n }],
            sessionMembers: () => [{ kind: 'object', id: 1n }],
            hasArmedGesture: () => false,
            runCloseInnermostSession,
            runOpenGroupSession,
            runOpenExplodeSessionOrFallback: vi.fn(),
          }
        }
        // A distinct label for the OPEN frame's header row, so it can't
        // collide with the unrelated top-level group's own positional
        // "Group 1" label below.
        onSessionChange?.([{ node: { kind: 'group', id: 50n }, label: 'Open Group' }])
      })

      // The unrelated top-level group's positional label — rendered
      // undimmed on the path per design, but its walked chain root (itself)
      // matches none of the open session's members.
      fireEvent.doubleClick(screen.getByText('Group 1'))

      expect(runCloseInnermostSession).toHaveBeenCalledTimes(1)
    } finally {
      mockScene.top_level_nodes = priorTopLevel
    }
  })

  it('finding 3: an armed tool gesture makes enterNode a silent no-op, even for a session member', async () => {
    await renderAndLoad()
    const { apiRef, onSessionChange } = latestViewportProps()
    const runCloseInnermostSession = vi.fn(() => true)
    act(() => {
      if (apiRef !== undefined) {
        apiRef.current = {
          sessionStack: () => [{ kind: 'group', id: 50n }],
          sessionMembers: () => [{ kind: 'object', id: 1n }],
          hasArmedGesture: () => true,
          runCloseInnermostSession,
          runOpenGroupSession: vi.fn(() => true),
          runOpenExplodeSessionOrFallback: vi.fn(),
        }
      }
      onSessionChange?.([{ node: { kind: 'group', id: 50n }, label: 'Group 1' }])
    })

    fireEvent.doubleClick(screen.getByText('Object 1'))

    // No close, no open, no selection change — matches Escape's own
    // refusal posture while a gesture is armed (silently ignored).
    expect(runCloseInnermostSession).not.toHaveBeenCalled()
    expect(window.__hew_test!.getSelection()).toEqual([])
  })
})

describe('sameSessionStackIdentity (adversarial-review finding 5 helper)', () => {
  it('true for two empty stacks', () => {
    expect(sameSessionStackIdentity([], [])).toBe(true)
  })

  it('false when lengths differ', () => {
    expect(sameSessionStackIdentity([{ node: { kind: 'group', id: 1n } }], [])).toBe(false)
  })

  it('false when any frame identity differs, even deep in the stack', () => {
    const a = [{ node: { kind: 'group' as const, id: 1n } }, { node: { kind: 'group' as const, id: 2n } }]
    const b = [{ node: { kind: 'group' as const, id: 1n } }, { node: { kind: 'group' as const, id: 3n } }]
    expect(sameSessionStackIdentity(a, b)).toBe(false)
  })

  it('true for the same frame identities, ignoring an unrelated label field', () => {
    const a = [{ node: { kind: 'instance' as const, id: 9n }, label: 'Box' }]
    const b = [{ node: { kind: 'instance' as const, id: 9n }, label: 'Box (renamed)' }]
    expect(sameSessionStackIdentity(a, b)).toBe(true)
  })
})

describe('App — Tape Measure rescale confirmation invalidation (adversarial-review finding 5)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  function latestViewportProps(): {
    onSessionChange?: (frames: { node: { kind: string; id: bigint }; label: string }[]) => void
    onRescaleArmed?: (info: { currentDistance: number; typedDistance: number; factor: number; anchor: [number, number, number] }) => void
    apiRef?: { current: Record<string, unknown> | null }
  } {
    const calls = vi.mocked(Viewport).mock.calls
    return calls[calls.length - 1][0] as never
  }

  it('any session-stack change while the dialog is pending cancels it instead of leaving it armed against a stale scope', async () => {
    await renderAndLoad()
    const { apiRef, onSessionChange } = latestViewportProps()
    const cancelPendingRescale = vi.fn()
    const confirmPendingRescale = vi.fn()
    act(() => {
      if (apiRef !== undefined) {
        apiRef.current = {
          sessionStack: () => [{ kind: 'group', id: 50n }],
          hasArmedGesture: () => false,
          cancelPendingRescale,
          confirmPendingRescale,
          runCloseInnermostSession: vi.fn(() => true),
        }
      }
      // Arm inside a group session — the dialog captures its innermost
      // frame (group 50) as the scoped-rescale target.
      onSessionChange?.([{ node: { kind: 'group', id: 50n }, label: 'Group 1' }])
    })
    // Re-fetch: `handleRescaleArmed`'s `[sessionStack]` dep means Viewport
    // re-rendered with a FRESH `onRescaleArmed` closure after the state
    // update above — the one destructured before it read the pre-open
    // (empty) `sessionStack` and would arm the whole-model, unscoped path.
    act(() => {
      latestViewportProps().onRescaleArmed?.({ currentDistance: 1, typedDistance: 2, factor: 2, anchor: [0, 0, 0] })
    })
    expect(screen.getByRole('dialog', { name: 'Resize Group 1' })).toBeTruthy()

    // The stack changes while the dialog is still up — e.g. Ctrl/Cmd+Z
    // popped the group session (still live behind the modal). ANY change,
    // not just the innermost frame's identity, must invalidate the arm.
    act(() => {
      onSessionChange?.([])
    })

    expect(cancelPendingRescale).toHaveBeenCalledTimes(1)
    expect(confirmPendingRescale).not.toHaveBeenCalled()
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('a session-stack refresh that leaves every frame identity unchanged does NOT cancel the pending dialog', async () => {
    await renderAndLoad()
    const { apiRef, onSessionChange } = latestViewportProps()
    const cancelPendingRescale = vi.fn()
    act(() => {
      if (apiRef !== undefined) {
        apiRef.current = {
          sessionStack: () => [{ kind: 'group', id: 50n }],
          hasArmedGesture: () => false,
          cancelPendingRescale,
          confirmPendingRescale: vi.fn(),
          runCloseInnermostSession: vi.fn(() => true),
        }
      }
      onSessionChange?.([{ node: { kind: 'group', id: 50n }, label: 'Group 1' }])
    })
    act(() => {
      latestViewportProps().onRescaleArmed?.({ currentDistance: 1, typedDistance: 2, factor: 2, anchor: [0, 0, 0] })
    })
    expect(screen.getByRole('dialog', { name: 'Resize Group 1' })).toBeTruthy()

    // A same-identity re-push (e.g. a mid-session fold-in re-deriving the
    // same one-frame stack, unrelated to the dialog) must leave it alone.
    act(() => {
      onSessionChange?.([{ node: { kind: 'group', id: 50n }, label: 'Group 1' }])
    })

    expect(cancelPendingRescale).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog', { name: 'Resize Group 1' })).toBeTruthy()
  })

  it('Confirm re-checks the live stack and declines instead of applying when it no longer matches the captured scope', async () => {
    await renderAndLoad()
    const { apiRef, onSessionChange } = latestViewportProps()
    const cancelPendingRescale = vi.fn()
    const confirmPendingRescale = vi.fn()
    // A mutable box `sessionStack()` reads live — the dialog's onConfirm
    // handler re-queries this at click time, independent of what
    // `onSessionChange` last pushed.
    let liveStack: { kind: string; id: bigint }[] = [{ kind: 'group', id: 50n }]
    act(() => {
      if (apiRef !== undefined) {
        apiRef.current = {
          sessionStack: () => liveStack,
          hasArmedGesture: () => false,
          cancelPendingRescale,
          confirmPendingRescale,
          runCloseInnermostSession: vi.fn(() => true),
        }
      }
      onSessionChange?.([{ node: { kind: 'group', id: 50n }, label: 'Group 1' }])
    })
    act(() => {
      latestViewportProps().onRescaleArmed?.({ currentDistance: 1, typedDistance: 2, factor: 2, anchor: [0, 0, 0] })
    })
    const dialog = screen.getByRole('dialog', { name: 'Resize Group 1' })
    expect(dialog).toBeTruthy()

    // The live stack diverges WITHOUT an intervening `onSessionChange` push
    // (the belt-and-suspenders scenario this re-check exists for).
    liveStack = [{ kind: 'group', id: 99n }]

    fireEvent.click(within(dialog).getByRole('button', { name: /resize/i }))

    expect(confirmPendingRescale).not.toHaveBeenCalled()
    expect(cancelPendingRescale).toHaveBeenCalledTimes(1)
  })
})

describe('App — handleSessionChange selection ordering (adversarial-review finding 4)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    Element.prototype.scrollIntoView = vi.fn()
  })

  function latestViewportProps(): {
    onSessionChange?: (frames: { node: { kind: string; id: bigint }; label: string }[]) => void
    apiRef?: { current: Record<string, unknown> | null }
  } {
    const calls = vi.mocked(Viewport).mock.calls
    return calls[calls.length - 1][0] as never
  }

  it('enterNode opening a session (which fires onSessionChange synchronously, clearing the selection) still ends with the target selected, not clobbered', async () => {
    const priorTopLevel = mockScene.top_level_nodes
    mockScene.top_level_nodes = () => [{ kind: 'group', id: 50n }]
    try {
      await renderAndLoad()
      const { apiRef } = latestViewportProps()
      // A prior selection that must NOT survive the session opening —
      // `handleSessionChange`'s own clear-on-innermost-change contract.
      act(() => window.__hew_test!.selectObjects(['7']))
      expect(window.__hew_test!.getSelection()).toEqual([{ kind: 'object', id: '7' }])

      act(() => {
        if (apiRef !== undefined) {
          apiRef.current = {
            sessionStack: () => [],
            sessionMembers: () => null,
            hasArmedGesture: () => false,
            runCloseInnermostSession: vi.fn(() => true),
            // Mirrors the real Viewport: opening the session fires
            // `onSessionChange` SYNCHRONOUSLY, inside this very call —
            // exactly the reentrant ordering the old `setSessionStack`
            // updater-based clear used to lose to `enterNode`'s own
            // trailing `setSelectedIds([target])`.
            runOpenGroupSession: (id: bigint) => {
              latestViewportProps().onSessionChange?.([{ node: { kind: 'group', id }, label: 'Group 1' }])
              return true
            },
            runOpenExplodeSessionOrFallback: vi.fn(),
          }
        }
      })

      fireEvent.doubleClick(screen.getByText('Group 1'))

      // The target (the group just opened) is selected — the session-open
      // reentrant clear happened first (call order), enterNode's own
      // trailing select happened last and won.
      expect(window.__hew_test!.getSelection()).toEqual([{ kind: 'group', id: '50' }])
    } finally {
      mockScene.top_level_nodes = priorTopLevel
    }
  })
})

describe('App — armed-gesture guard on breadcrumb close paths (adversarial-review finding 3)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    Element.prototype.scrollIntoView = vi.fn()
  })

  function latestViewportProps(): {
    onSessionChange?: (frames: { node: { kind: string; id: bigint }; label: string }[]) => void
    apiRef?: { current: Record<string, unknown> | null }
  } {
    const calls = vi.mocked(Viewport).mock.calls
    return calls[calls.length - 1][0] as never
  }

  it('a non-root crumb spanning a session-close is a no-op while a gesture is armed, and works once it is not', async () => {
    await renderAndLoad()
    const { apiRef, onSessionChange } = latestViewportProps()
    const runCloseInnermostSession = vi.fn(() => true)
    let armed = true
    act(() => {
      if (apiRef !== undefined) {
        apiRef.current = {
          sessionStack: () => [{ kind: 'group', id: 50n }, { kind: 'group', id: 60n }],
          sessionMembers: () => null,
          hasArmedGesture: () => armed,
          runCloseInnermostSession,
          runOpenGroupSession: vi.fn(() => true),
          runOpenExplodeSessionOrFallback: vi.fn(),
        }
      }
      // Two nested group frames — the outer's own crumb (depth 0) is not
      // the terminal breadcrumb entry, so it renders as a clickable button.
      onSessionChange?.([
        { node: { kind: 'group', id: 50n }, label: 'Outer Group' },
        { node: { kind: 'group', id: 60n }, label: 'Inner Group' },
      ])
    })

    fireEvent.click(screen.getByRole('button', { name: 'Outer Group' }))
    expect(runCloseInnermostSession).not.toHaveBeenCalled()

    armed = false
    fireEvent.click(screen.getByRole('button', { name: 'Outer Group' }))
    expect(runCloseInnermostSession).toHaveBeenCalledTimes(1)
  })

  it('the root "Model" crumb (handleExitToModel) is a no-op while a gesture is armed, and works once it is not', async () => {
    await renderAndLoad()
    const { apiRef, onSessionChange } = latestViewportProps()
    const runCloseInnermostSession = vi.fn(() => true)
    let armed = true
    act(() => {
      if (apiRef !== undefined) {
        apiRef.current = {
          sessionStack: () => [{ kind: 'group', id: 50n }],
          sessionMembers: () => null,
          hasArmedGesture: () => armed,
          runCloseInnermostSession,
          runOpenGroupSession: vi.fn(() => true),
          runOpenExplodeSessionOrFallback: vi.fn(),
        }
      }
      onSessionChange?.([{ node: { kind: 'group', id: 50n }, label: 'Group 1' }])
    })

    // Two `Model` buttons now: the Outliner's breadcrumb and the viewport
    // selection-path strip's root crumb. Both reach `handleExitToModel`.
    const [outlinerModel, stripModel] = ['not-strip', 'strip'].map((which) =>
      screen.getAllByRole('button', { name: 'Model' }).find(
        (b) => (b.dataset.testid === 'selection-path-crumb') === (which === 'strip'),
      ) as HTMLElement,
    )

    fireEvent.click(outlinerModel)
    fireEvent.click(stripModel)
    expect(runCloseInnermostSession).not.toHaveBeenCalled()

    armed = false
    fireEvent.click(outlinerModel)
    expect(runCloseInnermostSession).toHaveBeenCalledTimes(1)
    fireEvent.click(stripModel)
    expect(runCloseInnermostSession).toHaveBeenCalledTimes(2)
  })
})

describe('App — View > Section Cut menu state (D3, section-plane-polish)', () => {
  // Matches every other top-level describe block in this file (e.g. `App —
  // loaded state` above): without its own reset, `vi.mocked(Viewport).mock.calls`
  // accumulates across the WHOLE file, so `latestViewportProps()` below could
  // read a call from an earlier, unrelated test (adversarial-review finding —
  // flagged as a plausible flake source, not reproduced, fixed defensively).
  beforeEach(() => {
    vi.clearAllMocks()
  })

  /** Section-state shape `Viewport.getSectionState()` returns. */
  type SectionState = { origin: [number, number, number]; normal: [number, number, number]; active: boolean } | null

  /** The `onSectionChanged` callback and the (mutable) `apiRef` App handed
   * the (mocked) Viewport on its last render — mirrors `latestOnToast`'s
   * pattern. The real Viewport populates `apiRef.current` imperatively and
   * calls `onSectionChanged` whenever the section's existence/active state
   * actually changes; the mock does neither, so tests drive both by hand to
   * exercise App's re-derive-from-truth wiring (not a shadow boolean). */
  function latestViewportProps(): {
    onSectionChanged?: () => void
    apiRef?: { current: { getSectionState: () => SectionState; toggleSectionActive?: () => void } | null }
  } {
    const calls = vi.mocked(Viewport).mock.calls
    return calls[calls.length - 1][0] as never
  }

  /** Simulate the viewport reporting a new section state and notifying App. */
  function reportSectionState(state: SectionState, extra: { toggleSectionActive?: () => void } = {}) {
    const { onSectionChanged, apiRef } = latestViewportProps()
    act(() => {
      if (apiRef !== undefined) apiRef.current = { getSectionState: () => state, ...extra }
      onSectionChanged?.()
    })
  }

  it('starts unchecked with no section placed', async () => {
    await renderAndLoad()
    fireEvent.click(menubar().getByRole('button', { name: /^view$/i }))
    const el = menubar().getByText('Section Cut')
    expect(el.closest('div')?.textContent).not.toContain('✓')
  })

  it('checks View > Section Cut after the viewport reports an active section', async () => {
    await renderAndLoad()
    reportSectionState({ origin: [0, 0, 0], normal: [0, 0, 1], active: true })
    fireEvent.click(menubar().getByRole('button', { name: /^view$/i }))
    const el = menubar().getByText('Section Cut')
    expect(el.closest('div')?.textContent).toContain('✓')
  })

  it('a section placed but INACTIVE reports unchecked, not checked', async () => {
    await renderAndLoad()
    reportSectionState({ origin: [0, 0, 0], normal: [0, 0, 1], active: false })
    fireEvent.click(menubar().getByRole('button', { name: /^view$/i }))
    const el = menubar().getByText('Section Cut')
    expect(el.closest('div')?.textContent).not.toContain('✓')
  })

  it('clears back to unchecked when the section is deleted', async () => {
    await renderAndLoad()
    reportSectionState({ origin: [0, 0, 0], normal: [0, 0, 1], active: true })
    reportSectionState(null)
    fireEvent.click(menubar().getByRole('button', { name: /^view$/i }))
    const el = menubar().getByText('Section Cut')
    expect(el.closest('div')?.textContent).not.toContain('✓')
  })

  it('View > Section Cut dispatches the SAME toggle-section-active command as before (D3 keeps the action id)', async () => {
    await renderAndLoad()
    const toggleSectionActive = vi.fn()
    reportSectionState({ origin: [0, 0, 0], normal: [0, 0, 1], active: true }, { toggleSectionActive })
    fireEvent.click(menubar().getByRole('button', { name: /^view$/i }))
    fireEvent.mouseDown(menubar().getByText('Section Cut'))
    expect(toggleSectionActive).toHaveBeenCalledOnce()
  })
})

describe('App — View > Reset Axes command (tool-parity §4, finding 1)', () => {
  // Reset Axes used to be a web-only MenuBar item wired to a direct callback
  // prop (`onResetAxes={() => viewportApi.current?.resetAxes()}`) with no
  // 'reset-axes' menuActionRef case and no native-menu counterpart — so it
  // rendered fine but silently did nothing on macOS, whose Tauri shell
  // renders the native menu exclusively and never falls back to this
  // in-app MenuBar. App.tsx now gives it a real command id ('reset-axes')
  // routed through the same menuActionRef switch every other menu command
  // uses; this test drives the WEB path (MenuBar click) end to end and
  // proves it reaches `viewportApi.current.resetAxes()` — the native-menu
  // side of the same path is covered by nativeMenuParity.test.ts's
  // "non-tool commands" suite (main.rs has no JS-visible surface to click).
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('clicking View > Reset Drawing Axes calls viewportApi.current.resetAxes()', async () => {
    await renderAndLoad()
    const resetAxes = vi.fn()
    const calls = vi.mocked(Viewport).mock.calls
    const { apiRef } = calls[calls.length - 1][0] as { apiRef?: { current: { resetAxes: () => void } | null } }
    act(() => {
      if (apiRef !== undefined) apiRef.current = { resetAxes }
    })
    fireEvent.click(menubar().getByRole('button', { name: /^view$/i }))
    fireEvent.mouseDown(menubar().getByText('Reset Drawing Axes'))
    expect(resetAxes).toHaveBeenCalledOnce()
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

describe('App — autosave arm/disarm (recovery-snapshot timer, Lane F adversarial-review finding)', () => {
  /**
   * `armAutosaveTick`/`runAutosaveTick`/`clearRecoverySnapshot` (App.tsx
   * ~540-720, ~1862-1869) are component-internal closures — nothing to
   * import and unit-test directly. Their only externally-observable effect
   * is exactly two calls: `recoveryState.write` (the mocked
   * `io/recoveryStore`'s write, above) and `recoveryState.clear`. Driving a
   * REAL document mutation through the harness and asserting on those call
   * counts under fake timers tests the real arm/disarm state machine (a
   * used-to-be-unconditional `setInterval` replaced with an armed-on-dirty/
   * disarmed-on-clean `setTimeout`) end to end, rather than reimplementing
   * its internals in the test.
   *
   * `mutateDocument()` uses `undo()` purely as a MUTATION TRIGGER, not
   * because undo itself matters: in this headless (viewport-less) test
   * environment the harness's `undo()` falls back to `act((s) =>
   * s.scene_undo().free())` (harness.ts), which calls `deps.reconcile()` —
   * the SAME `handleDocumentChanged` choke point every real edit (draw,
   * push/pull, paint, …) funnels through, including the `armAutosaveTick()`
   * call at its end (mirrors the "document changes prune dead handles"
   * describe block above, which established this exact pattern).
   * `can_scene_undo()` is never consulted on this headless path, so the
   * stub scene needs nothing beyond a `scene_undo` stub.
   */
  function mutateDocument(): void {
    ;(mockScene as Record<string, unknown>).scene_undo = () => ({ free: () => { /* no-op */ } })
    const harness = (window as unknown as { __hew_test: { undo(): void } }).__hew_test
    act(() => harness.undo())
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    // Belt-and-suspenders: a test that throws mid-body before reaching its
    // own `finally` must not leave fake timers installed for the NEXT
    // test/describe block in this file.
    vi.useRealTimers()
  })

  it('a document mutation arms exactly one autosave timer (repeated mutations coalesce)', async () => {
    await renderAndLoad()
    vi.useFakeTimers()

    mutateDocument()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTOSAVE_INTERVAL_MS / 2)
    })
    // A second mutation while the first timer is still pending must NOT
    // arm an independent second timer (armAutosaveTick's `if
    // (autosaveTimerRef.current === null)` guard).
    mutateDocument()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTOSAVE_INTERVAL_MS / 2)
    })
    // The one (coalesced) timer fires here, 12s after the FIRST mutation.
    expect(recoveryState.write).toHaveBeenCalledTimes(1)
    // If the second mutation HAD armed its own independent timer (due 12s
    // after ITS OWN arm time, i.e. 6s from here), it would fire during this
    // advance — it must not.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTOSAVE_INTERVAL_MS / 2)
    })
    expect(recoveryState.write).toHaveBeenCalledTimes(1)
  })

  it('a successful recovery write disarms the tick until the next mutation', async () => {
    await renderAndLoad()
    vi.useFakeTimers()

    mutateDocument()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTOSAVE_INTERVAL_MS)
    })
    expect(recoveryState.write).toHaveBeenCalledTimes(1)

    // No new mutation — advancing further must not fire another write: the
    // successful write's `.then()` left the tick disarmed.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTOSAVE_INTERVAL_MS * 2)
    })
    expect(recoveryState.write).toHaveBeenCalledTimes(1)

    // A fresh mutation re-arms it.
    mutateDocument()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTOSAVE_INTERVAL_MS)
    })
    expect(recoveryState.write).toHaveBeenCalledTimes(2)
  })

  it('a write failure re-arms the tick (retries on the next interval, not silently)', async () => {
    await renderAndLoad()
    recoveryState.write.mockRejectedValueOnce(new Error('write failed'))
    vi.useFakeTimers()

    mutateDocument()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTOSAVE_INTERVAL_MS)
    })
    expect(recoveryState.write).toHaveBeenCalledTimes(1)

    // No new mutation this time — the FAILED write's `.catch()` must have
    // re-armed the tick on its own (unlike the successful-write case
    // above, where nothing re-arms without a fresh edit).
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTOSAVE_INTERVAL_MS)
    })
    expect(recoveryState.write).toHaveBeenCalledTimes(2)
  })

  it('no autosave write ever fires on a clean, never-edited document', async () => {
    await renderAndLoad()
    vi.useFakeTimers()

    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTOSAVE_INTERVAL_MS * 3)
    })
    expect(recoveryState.write).not.toHaveBeenCalled()
  })

  it('File ▸ New (discarding a dirty document) cancels the pending autosave timer outright', async () => {
    await renderAndLoad()
    vi.useFakeTimers()

    // Dirty the document — armAutosaveTick fires, a timer is pending.
    mutateDocument()

    // File ▸ New on a dirty (non-pristine) document: `isPristineDocument`
    // is false, so `newDocument()` awaits `confirmDiscard()` — the in-app
    // Unsaved Changes dialog (Lane C) — and Don't Save resets in place and
    // calls `clearRecoverySnapshot()`.
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('New'))
    // Flush to the dialog, answer it, then flush the applyLoadedBytes/
    // clearRecoverySnapshot chain — small fake-time advances (rather than
    // `waitFor`, which polls on REAL timers and would hang here) cover both
    // pending microtasks and any short setTimeout/rAF step along the way.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50)
    })
    const dialog = screen.getByRole('dialog', { name: /unsaved changes/i })
    fireEvent.click(within(dialog).getByRole('button', { name: /don.t save/i }))
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50)
    })
    expect(recoveryState.clear).toHaveBeenCalled()

    // The ORIGINAL pending timer (armed by the pre-New mutation, still
    // ~11.95s from firing) must not go on to fire a write —
    // `clearRecoverySnapshot` cancels it outright (`clearTimeout`), rather
    // than merely relying on the next tick finding a clean document (which
    // would look identical from `write`'s call count alone at the FIRST
    // interval, but is a materially different — weaker — guarantee than
    // "cancelled").
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTOSAVE_INTERVAL_MS * 2)
    })
    expect(recoveryState.write).not.toHaveBeenCalled()
  })
})

describe('App — Lane C: dirty is derived from the kernel saved mark', () => {
  // mockScene.at_saved_mark defaults to a static `false` (see its own doc
  // comment above) — mirroring the OLD unconditional "any mutation dirties"
  // behavior for every test that doesn't care about this. This suite
  // overrides it per test to exercise the actual undo-to-clean derivation
  // (`afterMutation`'s `atSavedMark` parameter, documentSession.ts).
  const originalAtSavedMark = mockScene.at_saved_mark

  afterEach(() => {
    mockScene.at_saved_mark = originalAtSavedMark
    delete (mockScene as Record<string, unknown>).scene_undo
  })

  it('undoing back to the saved mark clears the dirty title dot — no re-save needed', async () => {
    await renderAndLoad()
    const harness = (window as unknown as {
      __hew_test: { addNodeTag: (kind: string, id: string, path: string[]) => void; undo: () => void }
    }).__hew_test

    // A real mutation dirties the document (at_saved_mark's default false).
    act(() => harness.addNodeTag('object', '1', ['tag']))
    expect(document.title.startsWith('•')).toBe(true)

    // Undo lands exactly back at the saved mark: at_saved_mark now reports
    // true, and handleDocumentChanged (the choke point every undo/redo
    // reconciles through) recomputes dirty from it — no explicit "clean"
    // action, no re-save, matching every other undo-aware editor.
    mockScene.at_saved_mark = () => true
    ;(mockScene as Record<string, unknown>).scene_undo = () => ({ free: () => { /* no-op */ } })
    act(() => harness.undo())

    expect(document.title.startsWith('•')).toBe(false)
  })

  it('undoing while NOT at the saved mark leaves the title dirty', async () => {
    await renderAndLoad()
    const harness = (window as unknown as {
      __hew_test: { addNodeTag: (kind: string, id: string, path: string[]) => void; undo: () => void }
    }).__hew_test

    act(() => harness.addNodeTag('object', '1', ['tag']))
    expect(document.title.startsWith('•')).toBe(true)

    // Undo one step, but the saved mark sits somewhere else entirely
    // (at_saved_mark stays false) — still dirty.
    ;(mockScene as Record<string, unknown>).scene_undo = () => ({ free: () => { /* no-op */ } })
    act(() => harness.undo())

    expect(document.title.startsWith('•')).toBe(true)
  })

  // Session bookkeeping (Lane C follow-up, maintainer playtest): entering
  // or leaving a group/component edit session pushes a real, undoable
  // kernel action, but the kernel's `content_depth` ignores it — the
  // saved mark (and therefore `at_saved_mark()`) doesn't move. This test
  // stands in for that at the App/documentSession seam: whatever mutation
  // fires, if the kernel reports `at_saved_mark(): true` right through it
  // (exactly what a pure-bookkeeping action does), the derived `dirty`
  // must stay false — no separate bookkeeping-awareness needed above the
  // kernel's own `at_saved_mark` boundary.
  it('a mutation the kernel reports as still-at-the-saved-mark (a bookkeeping action) never dirties the title', async () => {
    await renderAndLoad()
    const harness = (window as unknown as {
      __hew_test: { addNodeTag: (kind: string, id: string, path: string[]) => void }
    }).__hew_test

    mockScene.at_saved_mark = () => true
    act(() => harness.addNodeTag('object', '1', ['tag']))

    expect(document.title.startsWith('•')).toBe(false)
  })
})

// ---------------------------------------------------------------------------
// App — confirmDiscard's Save-then-continue path (adversarial review
// finding): the in-app UnsavedChangesDialog's Save button runs the real
// save flow and only lets the original discard-triggering action (here,
// File ▸ New) proceed on an ACTUAL successful write — a cancelled/failed
// save must leave the dialog up, New un-run, and the document untouched.
// ---------------------------------------------------------------------------

describe('App — confirmDiscard: Save-then-continue (File ▸ New)', () => {
  const originalAtSavedMark = mockScene.at_saved_mark
  let fakeFileHost: FileHost

  beforeEach(() => {
    vi.clearAllMocks()
    setTrayLayout(DEFAULT_TRAY_LAYOUT)
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
    mockScene.at_saved_mark = originalAtSavedMark
    delete (mockScene as Record<string, unknown>).scene_undo
    vi.mocked(makeFileHost).mockReset()
  })

  const dirtyTheDocument = async () => {
    await renderAndLoad()
    const harness = (window as unknown as {
      __hew_test: { addNodeTag: (kind: string, id: string, path: string[]) => void }
    }).__hew_test
    // mockScene.at_saved_mark defaults to a static `false` (see its own doc
    // comment), so any real mutation dirties the title.
    act(() => harness.addNodeTag('object', '1', ['tag']))
    expect(document.title.startsWith('•')).toBe(true)
  }

  const triggerNew = () => {
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('New'))
  }

  // The document has no file yet, so its Save is a Save As: the format is
  // chosen (the browser's in-app step here) along with the place.
  const continueSaveAs = async () => {
    const saveAs = await screen.findByRole('dialog', { name: /^save as$/i })
    fireEvent.click(within(saveAs).getByRole('button', { name: /^continue$/i }))
  }

  it('a successful Save in the dialog lets File ▸ New proceed', async () => {
    await dirtyTheDocument()
    vi.mocked(fakeFileHost.saveAs).mockResolvedValue({ name: 'house.hew', format: 'hew', handle: '/tmp/house.hew' })

    triggerNew()
    const dialog = await screen.findByRole('dialog', { name: /unsaved changes/i })
    fireEvent.click(within(dialog).getByRole('button', { name: /^save$/i }))
    await continueSaveAs()

    await waitFor(() => expect(fakeFileHost.saveAs).toHaveBeenCalledOnce())
    expect(fakeFileHost.save).not.toHaveBeenCalled()
    // New proceeded: the blank-document load ran, and the dialog closed.
    await waitFor(() => expect(mockScene.load).toHaveBeenCalledOnce())
    expect(screen.queryByRole('dialog', { name: /unsaved changes/i })).not.toBeInTheDocument()
    // Written once: the dialog's Save, not again on the way to New.
    expect(fakeFileHost.saveAs).toHaveBeenCalledOnce()
    expect(fakeFileHost.save).not.toHaveBeenCalled()
  })

  it('a cancelled Save (host returns null — e.g. a dismissed Save As picker) blocks New and keeps the document', async () => {
    await dirtyTheDocument()
    vi.mocked(fakeFileHost.saveAs).mockResolvedValue(null)

    triggerNew()
    const dialog = await screen.findByRole('dialog', { name: /unsaved changes/i })
    fireEvent.click(within(dialog).getByRole('button', { name: /^save$/i }))
    await continueSaveAs()
    await waitFor(() => expect(fakeFileHost.saveAs).toHaveBeenCalledOnce())

    // The dialog stays up (Save didn't land) — New never ran, nothing was
    // discarded, and the document is exactly as dirty as before.
    expect(screen.getByRole('dialog', { name: /unsaved changes/i })).toBeInTheDocument()
    expect(mockScene.load).not.toHaveBeenCalled()
    expect(document.title.startsWith('•')).toBe(true)

    // The Save button is usable again (not stuck in "Saving…"), so the
    // user can retry or fall back to Don't Save/Cancel.
    expect(within(dialog).getByRole('button', { name: /^save$/i })).not.toBeDisabled()
  })
})

// ---------------------------------------------------------------------------
// App — Lane C: `undoRedoMenuLabels` (App.tsx) reaching the web MenuBar.
// ---------------------------------------------------------------------------

describe('App — Lane C: Edit menu shows the specific undo/redo entry label', () => {
  const originalHistoryEntriesJson = mockScene.history_entries_json
  const originalCanUndo = mockScene.can_scene_undo
  const originalCanRedo = mockScene.can_scene_redo

  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    mockScene.history_entries_json = originalHistoryEntriesJson
    mockScene.can_scene_undo = originalCanUndo
    mockScene.can_scene_redo = originalCanRedo
  })

  it('shows "Undo <label>" / "Redo <label>" derived from the top undo/redo entry', async () => {
    mockScene.can_scene_undo = () => true
    mockScene.can_scene_redo = () => true
    mockScene.history_entries_json = () =>
      JSON.stringify({
        undo: [{ label: 'Push/Pull', origin: 'user' }],
        redo: [{ label: 'Move 1 object', origin: 'user' }],
        savedDepth: 1,
      })

    await renderAndLoad()
    fireEvent.click(screen.getByRole('button', { name: /^edit$/i }))
    expect(screen.getByText('Undo Push/Pull')).toBeInTheDocument()
    expect(screen.getByText('Redo Move 1 object')).toBeInTheDocument()
  })

  it('falls back to the bare verb when the history is empty, even with canUndo/canRedo true', async () => {
    mockScene.can_scene_undo = () => true
    mockScene.can_scene_redo = () => true
    mockScene.history_entries_json = () => JSON.stringify({ undo: [], redo: [], savedDepth: 0 })

    await renderAndLoad()
    fireEvent.click(screen.getByRole('button', { name: /^edit$/i }))
    expect(screen.getByText('Undo')).toBeInTheDocument()
    expect(screen.getByText('Redo')).toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// App — recovery snapshot clearing on a dirty→clean transition (Lane C
// follow-up, maintainer playtest repro): make changes (autosave writes a
// recovery snapshot while dirty), undo them all (the document reads clean),
// quit — no unsaved-changes prompt (correct), but the STALE snapshot must
// not survive to falsely offer recovery on the next launch. Covers the
// dirty→clean effect (`wasDirtyRef`) and the write/clear ordering
// `clearRecoverySnapshot` itself guarantees; the Tauri-close and
// beforeunload call sites are exercised directly in App.tsx and aren't
// reachable from jsdom (no real window close/Tauri shell here), so this
// suite covers the transition effect plus the beforeunload listener's own
// dirty gate (which IS reachable, via a dispatched event).
// ---------------------------------------------------------------------------

describe('App — recovery snapshot clearing on clean', () => {
  const originalAtSavedMark = mockScene.at_saved_mark

  beforeEach(() => {
    // `shouldAdvanceTime` keeps the fake clock ticking in step with real
    // time (dialogs.test.tsx's own pickup-polling precedent) so the kernel
    // load promise and RTL's `waitFor` polling still resolve normally,
    // while `vi.advanceTimersByTimeAsync` fast-forwards the 12s autosave
    // interval without the test actually waiting on it.
    vi.useFakeTimers({ shouldAdvanceTime: true })
  })

  afterEach(() => {
    vi.useRealTimers()
    mockScene.at_saved_mark = originalAtSavedMark
    delete (mockScene as Record<string, unknown>).scene_undo
  })

  it('clears the recovery snapshot when an undo lands back at the saved mark after an autosave write', async () => {
    await renderAndLoad()
    const harness = (window as unknown as {
      __hew_test: { addNodeTag: (kind: string, id: string, path: string[]) => void; undo: () => void }
    }).__hew_test

    act(() => harness.addNodeTag('object', '1', ['tag']))
    expect(document.title.startsWith('•')).toBe(true)

    // Let the 12s autosave tick fire and write a recovery snapshot while dirty.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(12_000)
    })
    await waitFor(() => expect(recoveryState.write).toHaveBeenCalled())
    recoveryState.clear.mockClear()

    // Undo back to the saved mark — the document reads clean again, with
    // no explicit save/close/discard in between.
    mockScene.at_saved_mark = () => true
    ;(mockScene as Record<string, unknown>).scene_undo = () => ({ free: () => { /* no-op */ } })
    act(() => harness.undo())
    expect(document.title.startsWith('•')).toBe(false)

    await waitFor(() => expect(recoveryState.clear).toHaveBeenCalled())
  })

  it('keeps the snapshot while the document is still dirty (a close/unload check must not clear it)', async () => {
    await renderAndLoad()
    const harness = (window as unknown as {
      __hew_test: { addNodeTag: (kind: string, id: string, path: string[]) => void }
    }).__hew_test

    act(() => harness.addNodeTag('object', '1', ['tag']))
    expect(document.title.startsWith('•')).toBe(true)

    // The beforeunload listener only fires clearRecoverySnapshot when NOT
    // dirty (App.tsx) — a dirty document's snapshot is the whole point of
    // recovery, so it must survive an unload check.
    window.dispatchEvent(new Event('beforeunload', { cancelable: true }))
    await Promise.resolve()
    expect(recoveryState.clear).not.toHaveBeenCalled()
  })

  it('discards (clears) a write that resolves AFTER the document already went clean, instead of leaving the snapshot behind', async () => {
    let resolveWrite!: () => void
    recoveryState.write.mockImplementationOnce(
      () => new Promise<void>((resolve) => { resolveWrite = resolve }),
    )

    await renderAndLoad()
    const harness = (window as unknown as {
      __hew_test: { addNodeTag: (kind: string, id: string, path: string[]) => void; undo: () => void }
    }).__hew_test

    act(() => harness.addNodeTag('object', '1', ['tag']))
    expect(document.title.startsWith('•')).toBe(true)

    // The autosave tick fires; the write is deliberately left pending.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(12_000)
    })
    expect(recoveryState.write).toHaveBeenCalled()

    // Undo back to the saved mark WHILE the write is still in flight.
    mockScene.at_saved_mark = () => true
    ;(mockScene as Record<string, unknown>).scene_undo = () => ({ free: () => { /* no-op */ } })
    act(() => harness.undo())
    expect(document.title.startsWith('•')).toBe(false)

    // clearRecoverySnapshot (fired by the dirty→clean transition) is
    // awaiting the still-in-flight write — clear() hasn't run yet.
    await Promise.resolve()
    await Promise.resolve()
    expect(recoveryState.clear).not.toHaveBeenCalled()

    // The write finally lands — clear() must run right after, so the
    // snapshot it just wrote never survives the clean transition.
    resolveWrite()
    await waitFor(() => expect(recoveryState.clear).toHaveBeenCalled())
  })
})

describe('App — v1.1 assets lane: viewport resync + current-material reset after delete/purge', () => {
  // Material/component delete and purge are panel-driven (MaterialPalette's
  // ×, ComponentsPanel's ×, the header's Purge Unused… button) — NOT tool
  // commits, so they don't automatically reach Viewport's internal
  // handleSceneRefresh the way a tool gesture does. doDeleteMaterial/
  // doDeleteComponent/confirmPurgeUnused must explicitly call
  // viewportApi.current?.refreshScene() (mirrors onAlphaCommitted's
  // syncMaterialOpacity precedent) or the deleted/purged geometry's
  // rendering never updates even though the kernel already changed.
  function latestViewportProps(): {
    apiRef?: { current: Record<string, unknown> | null }
  } {
    const calls = vi.mocked(Viewport).mock.calls
    return calls[calls.length - 1][0] as never
  }

  const priors: Partial<typeof mockScene> = {}
  function stub<K extends keyof typeof mockScene>(key: K, value: (typeof mockScene)[K]) {
    if (!(key in priors)) (priors as Record<string, unknown>)[key] = mockScene[key]
    ;(mockScene as Record<string, unknown>)[key] = value
  }
  afterEach(() => {
    for (const key of Object.keys(priors) as (keyof typeof mockScene)[]) {
      ;(mockScene as Record<string, unknown>)[key] = priors[key]
      delete priors[key]
    }
  })

  it('deleting an unused material calls refreshScene and, if it was the current material, resets currentMaterialId to the sentinel', async () => {
    const redInfo = { name: () => 'Red', r: () => 220, g: () => 30, b: () => 30, a: () => 255, has_texture: () => false, has_pbr: () => false, metallic: () => 0, roughness: () => 1 }
    stub('material_ids', () => BigUint64Array.from([7n]))
    stub('material_info', ((id: bigint) => (id === 7n ? redInfo : undefined)) as typeof mockScene.material_info)
    stub('material_usage', (() => 0) as typeof mockScene.material_usage)
    const deleteMaterial = vi.fn()
    stub('delete_material', deleteMaterial as typeof mockScene.delete_material)

    setTrayLayout({ ...DEFAULT_TRAY_LAYOUT, materials: true })
    await renderAndLoad()

    const { apiRef } = latestViewportProps()
    const refreshScene = vi.fn()
    act(() => { if (apiRef !== undefined) apiRef.current = { refreshScene } })

    // Select the swatch as current (Paint tool's active material) — the
    // thing that must be cleared once its material is gone.
    fireEvent.click(screen.getByTitle('Red'))
    expect(latestViewportProps().apiRef?.current).toBeTruthy()
    expect(vi.mocked(Viewport).mock.calls.at(-1)![0].currentMaterialId).toBe(7n)

    fireEvent.click(screen.getByRole('button', { name: 'Delete material Red' }))

    expect(deleteMaterial).toHaveBeenCalledWith(7n)
    expect(refreshScene, 'the viewport must re-tessellate after a material delete').toHaveBeenCalled()
    expect(
      vi.mocked(Viewport).mock.calls.at(-1)![0].currentMaterialId,
      'the deleted material must not stay "current"',
    ).toBe(MATERIAL_SENTINEL)
  })

  it('deleting a component definition with no instances calls refreshScene', async () => {
    stub('component_ids', () => BigUint64Array.from([9n]))
    stub('component_name', ((id: bigint) => (id === 9n ? 'Leg' : undefined)) as typeof mockScene.component_name)
    stub('definition_usage', (() => 0) as typeof mockScene.definition_usage)
    const deleteDefinition = vi.fn()
    stub('delete_definition', deleteDefinition as typeof mockScene.delete_definition)

    setTrayLayout({ ...DEFAULT_TRAY_LAYOUT, components: true })
    await renderAndLoad()

    const { apiRef } = latestViewportProps()
    const refreshScene = vi.fn()
    act(() => { if (apiRef !== undefined) apiRef.current = { refreshScene } })

    fireEvent.click(screen.getByRole('button', { name: /^delete component/i }))

    expect(deleteDefinition).toHaveBeenCalledWith(9n)
    expect(refreshScene, 'the viewport must re-tessellate after a definition delete').toHaveBeenCalled()
  })

  it('Purge Unused calls refreshScene and clears currentMaterialId when the current material was purged', async () => {
    const staleInfo = { name: () => 'Stray', r: () => 5, g: () => 5, b: () => 5, a: () => 255, has_texture: () => false, has_pbr: () => false, metallic: () => 0, roughness: () => 1 }
    let purged = false
    stub('material_ids', () => (purged ? new BigUint64Array() : BigUint64Array.from([11n])))
    stub('material_info', ((id: bigint) => (id === 11n && !purged ? staleInfo : undefined)) as typeof mockScene.material_info)
    stub('unused_materials', () => (purged ? new BigUint64Array() : BigUint64Array.from([11n])))
    stub('unused_definitions', () => new BigUint64Array())
    stub('purge_unused', (() => {
      purged = true
      return { materials: () => 1, definitions: () => 0 }
    }) as typeof mockScene.purge_unused)

    setTrayLayout({ ...DEFAULT_TRAY_LAYOUT, materials: true, components: true })
    await renderAndLoad()

    const { apiRef } = latestViewportProps()
    const refreshScene = vi.fn()
    act(() => { if (apiRef !== undefined) apiRef.current = { refreshScene } })

    fireEvent.click(screen.getByTitle('Stray'))
    expect(vi.mocked(Viewport).mock.calls.at(-1)![0].currentMaterialId).toBe(11n)

    fireEvent.click(screen.getByRole('button', { name: 'Purge Unused…' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Purge' }))

    expect(refreshScene, 'the viewport must re-tessellate after a purge').toHaveBeenCalled()
    expect(
      vi.mocked(Viewport).mock.calls.at(-1)![0].currentMaterialId,
      'a purged material must not stay "current"',
    ).toBe(MATERIAL_SENTINEL)
  })
})

// ---------------------------------------------------------------------------
// App — the desktop's native save panel: Save As and Export choose the
// format in the platform's own panel (io/savePanel.ts) and write to the
// path it picked, never through the in-app format step or Export dialog.
// ---------------------------------------------------------------------------

describe('App — the native save panel chooses the format', () => {
  let fakeFileHost: FileHost
  const written = new Uint8Array([7, 7, 7])

  beforeEach(() => {
    vi.clearAllMocks()
    setTrayLayout(DEFAULT_TRAY_LAYOUT)
    vi.mocked(savePanel.hasNativeSavePanel).mockReturnValue(true)
    fakeFileHost = {
      open: vi.fn(),
      save: vi.fn(),
      saveAs: vi.fn(),
      openForImport: vi.fn(),
      openAny: vi.fn(),
      exportBinary: vi.fn(),
      writeTo: vi.fn(async (path: string, _bytes: Uint8Array, format?: DocFormat) => ({
        name: path.split('/').pop() ?? path,
        format: format ?? 'hew',
        handle: path,
      })),
    }
    vi.mocked(makeFileHost).mockReturnValue(fakeFileHost)
    ;(mockScene as Record<string, unknown>).save_skp = vi.fn(() => ({ bytes: written, simplified: [], writer_notes: [] }))
    ;(mockScene as Record<string, unknown>).export_skp = vi.fn(() => ({ bytes: written, simplified: [], writer_notes: [] }))
  })

  afterEach(() => {
    vi.mocked(savePanel.hasNativeSavePanel).mockReturnValue(false)
    vi.mocked(makeFileHost).mockReset()
    delete (mockScene as Record<string, unknown>).save_skp
    delete (mockScene as Record<string, unknown>).export_skp
  })

  const fileMenu = (item: string) => {
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText(item))
  }

  it('Save As writes the format picked in the panel to the path it picked', async () => {
    vi.mocked(savePanel.pickSaveTarget).mockResolvedValue({ path: '/tmp/m.skp', format: 'skp-older', options: { release: 'skp2017' } })
    await renderAndLoad()
    fileMenu('Save As…')

    await waitFor(() => expect(fakeFileHost.writeTo).toHaveBeenCalledOnce())
    expect(savePanel.pickSaveTarget).toHaveBeenCalledWith('Save As', expect.any(String), savePanel.savePanelFormats('hew', false), 'hew')
    expect((mockScene as Record<string, unknown>).save_skp).toHaveBeenCalledWith('skp2017', expect.any(String), undefined, false)
    expect(fakeFileHost.writeTo).toHaveBeenCalledWith('/tmp/m.skp', written, 'skp2017')
    // No in-app format step, no second picker.
    expect(screen.queryByRole('dialog', { name: /^save as$/i })).not.toBeInTheDocument()
    expect(fakeFileHost.saveAs).not.toHaveBeenCalled()
    // Bound to the file it wrote (the mock's saved mark never clears the
    // dirty dot; documentSession.test.ts covers that).
    await waitFor(() => expect(document.title).toMatch(/m\.skp — Hew$/))
  })

  it('Export writes the format picked in the panel to the path it picked', async () => {
    vi.mocked(savePanel.pickSaveTarget).mockResolvedValue({ path: '/tmp/e.skp', format: 'skp-older', options: { release: 'skp2017' } })
    await renderAndLoad()
    fileMenu('Export…')

    await waitFor(() => expect(fakeFileHost.writeTo).toHaveBeenCalledOnce())
    const [title, , formats, initial] = vi.mocked(savePanel.pickSaveTarget).mock.calls[0]
    expect([title, initial]).toEqual(['Export', 'glb'])
    expect(formats.map((f) => f.id)).toEqual(['glb', 'stl', '3mf', 'usdz', 'svg', 'skp', 'skp-older'])
    expect((mockScene as Record<string, unknown>).export_skp).toHaveBeenCalledWith('skp2017', expect.any(String), false)
    expect(fakeFileHost.writeTo).toHaveBeenCalledWith('/tmp/e.skp', written)
    expect(screen.queryByRole('dialog', { name: /^export$/i })).not.toBeInTheDocument()
    expect(fakeFileHost.exportBinary).not.toHaveBeenCalled()
  })

  it('Save As passes the compat box of the 2021-and-later container to the writer', async () => {
    vi.mocked(savePanel.pickSaveTarget).mockResolvedValue({ path: '/tmp/m.skp', format: 'skp2026', options: { compat: true } })
    await renderAndLoad()
    fileMenu('Save As…')

    await waitFor(() => expect(fakeFileHost.writeTo).toHaveBeenCalledOnce())
    expect((mockScene as Record<string, unknown>).save_skp).toHaveBeenCalledWith('skp2026', expect.any(String), undefined, true)
  })

  it('Export passes the compat box only for the 2021-and-later container', async () => {
    vi.mocked(savePanel.pickSaveTarget).mockResolvedValue({ path: '/tmp/e.skp', format: 'skp', options: { compat: true } })
    await renderAndLoad()
    fileMenu('Export…')

    await waitFor(() => expect(fakeFileHost.writeTo).toHaveBeenCalledOnce())
    expect((mockScene as Record<string, unknown>).export_skp).toHaveBeenCalledWith('skp2026', expect.any(String), true)
  })

  it('a panel that could not be shown says so rather than passing for a cancel', async () => {
    vi.mocked(savePanel.pickSaveTarget).mockRejectedValue('the save panel needs the main thread')
    await renderAndLoad()
    fileMenu('Save As…')
    expect(await screen.findByText(/Save As failed: .*main thread/)).toBeInTheDocument()
    fileMenu('Export…')
    expect(await screen.findByText(/Export failed: .*main thread/)).toBeInTheDocument()
    expect(fakeFileHost.writeTo).not.toHaveBeenCalled()
  })

  it('Export to a stream release lists what the older format left out', async () => {
    vi.mocked(savePanel.pickSaveTarget).mockResolvedValue({ path: '/tmp/e.skp', format: 'skp-older', options: { release: 'skp2018' } })
    ;(mockScene as Record<string, unknown>).export_skp = vi.fn(() => ({
      bytes: written,
      simplified: [],
      writer_notes: ['a text is anchored to an entity the file does not hold and is left out'],
    }))
    await renderAndLoad()
    fileMenu('Export…')

    const notice = await screen.findByTestId('skp-loss-notice')
    expect(notice).toHaveTextContent('left out')
    expect(notice).toHaveTextContent('a text is anchored to an entity')
    expect((mockScene as Record<string, unknown>).export_skp).toHaveBeenCalledWith('skp2018', expect.any(String), false)
  })

  it.each(['skp8', 'skp3', 'skp2013'] as const)(
    'Export to release %s passes the release through as its own target',
    async (format) => {
      vi.mocked(savePanel.pickSaveTarget).mockResolvedValue({ path: '/tmp/e.skp', format: 'skp-older', options: { release: format } })
      ;(mockScene as Record<string, unknown>).export_skp = vi.fn(() => ({
        bytes: written,
        simplified: [],
        writer_notes: [],
      }))
      await renderAndLoad()
      fileMenu('Export…')
      await waitFor(() =>
        expect((mockScene as Record<string, unknown>).export_skp).toHaveBeenCalledWith(format, expect.any(String), false),
      )
    },
  )

  it('a canceled panel writes nothing', async () => {
    vi.mocked(savePanel.pickSaveTarget).mockResolvedValue(null)
    await renderAndLoad()
    fileMenu('Save As…')
    await waitFor(() => expect(savePanel.pickSaveTarget).toHaveBeenCalledOnce())
    fileMenu('Export…')
    await waitFor(() => expect(savePanel.pickSaveTarget).toHaveBeenCalledTimes(2))
    expect(fakeFileHost.writeTo).not.toHaveBeenCalled()
    expect(fakeFileHost.saveAs).not.toHaveBeenCalled()
    expect(fakeFileHost.exportBinary).not.toHaveBeenCalled()
  })
})

// ---------------------------------------------------------------------------
// App — Object Info links: a tag chip or a Materials-row entry goes through
// revealInTray, which opens the collapsed pane and selects the target in it.
// ---------------------------------------------------------------------------

describe('App — revealInTray (Object Info tag chips and Materials row)', () => {
  const saved = {
    top: mockScene.top_level_nodes,
    objectIds: mockScene.object_ids,
    objectName: mockScene.object_name,
    nodeTags: mockScene.node_tags,
    materialIds: mockScene.material_ids,
    materialInfo: mockScene.material_info,
    objectMaterials: mockScene.object_materials,
  }
  const scrolled = vi.fn()
  const originalScroll = Element.prototype.scrollIntoView

  beforeEach(() => {
    vi.clearAllMocks()
    setTrayLayout(DEFAULT_TRAY_LAYOUT) // Tags and Materials start collapsed
    Element.prototype.scrollIntoView = scrolled
    mockScene.top_level_nodes = () => [{ kind: 'object', id: 1n }]
    mockScene.object_ids = () => new BigUint64Array([1n])
    mockScene.object_name = () => 'Beam'
    mockScene.node_tags = () => ['Structure/Roof']
    mockScene.material_ids = () => new BigUint64Array([10n])
    mockScene.material_info = ((id: bigint) =>
      id === 10n
        ? { name: () => 'Oak', r: () => 200, g: () => 150, b: () => 100, a: () => 255, has_texture: () => false, has_pbr: () => false, metallic: () => 0, roughness: () => 1, world_w: () => 1, world_h: () => 1 }
        : undefined) as unknown as typeof mockScene.material_info
    mockScene.object_materials = () => new BigUint64Array([10n])
  })

  afterEach(() => {
    mockScene.top_level_nodes = saved.top
    mockScene.object_ids = saved.objectIds
    mockScene.object_name = saved.objectName
    mockScene.node_tags = saved.nodeTags
    mockScene.material_ids = saved.materialIds
    mockScene.material_info = saved.materialInfo
    mockScene.object_materials = saved.objectMaterials
    Element.prototype.scrollIntoView = originalScroll
  })

  const sectionHeader = (name: RegExp) => screen.getByRole('button', { name })

  it('a tag chip opens the collapsed Tags pane, expands the parents, and selects the tag row', async () => {
    await renderAndLoad()
    fireEvent.click(screen.getByText('Beam')) // select the object in the Outliner
    expect(sectionHeader(/^tags$/i).getAttribute('aria-expanded')).toBe('false')

    fireEvent.click(screen.getByText('Structure / Roof'))

    expect(sectionHeader(/^tags$/i).getAttribute('aria-expanded')).toBe('true')
    const row = (await screen.findByText('Roof')).closest('[data-active]') as HTMLElement
    expect(row.dataset.active).toBe('true')
    expect(scrolled).toHaveBeenCalled()
  })

  it('a Materials-row entry opens the collapsed Materials pane and makes that material the current one without picking Paint', async () => {
    await renderAndLoad()
    fireEvent.click(screen.getByText('Beam'))
    expect(sectionHeader(/^materials$/i).getAttribute('aria-expanded')).toBe('false')

    fireEvent.click(within(screen.getByTestId('object-info-materials')).getByText('Oak'))

    expect(sectionHeader(/^materials$/i).getAttribute('aria-expanded')).toBe('true')
    const swatch = await screen.findByTitle('Oak')
    expect(swatch.style.borderColor).toContain('accent-base')
    expect(scrolled).toHaveBeenCalled()
    // Selecting here does not switch tools the way picking in the palette does.
    expect(screen.getByTitle('Rectangle (R)')).toBeInTheDocument()
    expect(vi.mocked(Viewport).mock.calls.at(-1)?.[0]).toMatchObject({ activeTool: 'Select' })
  })
})
