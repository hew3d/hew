/**
 * App — File ▸ Open onto a non-pristine window, under the Tauri desktop
 * shell (A3, window-management batch).
 *
 * `isTauri` (io/fileHost.ts) is a module-level constant resolved once, from
 * `'__TAURI_INTERNALS__' in window`, when that module first evaluates — so
 * this behavior can't be exercised inside App.test.tsx (which shares one
 * module graph, evaluated once, across ~90 other tests that all depend on
 * isTauri being false). This file gets its own, isolated module graph
 * (Vitest's default per-file isolation) with the flag set before anything
 * imports App, so isTauri is true for every test below and only these.
 *
 * The other half of the desktop behavior — actually opening a real second
 * OS window and delivering the path into it — is Tauri-shell machinery
 * (open_in_new_window / take_pending_window_open in main.rs) that a jsdom
 * test can't drive; that side is covered by the Rust command wiring itself
 * plus a hand-check in the running desktop shell. What's
 * unit-testable here, and what actually matters for App.tsx's own logic, is
 * the dispatch seam: does openDocument() correctly decide to call
 * `open_in_new_window` (and never touch the current window's scene) once
 * the window is no longer pristine.
 */

import { render, screen, fireEvent, waitFor, within, act } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'

vi.hoisted(() => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
})

const menubar = () => within(screen.getByTestId('menu-bar'))

// ---------------------------------------------------------------------------
// Mocks — vi.mock() is hoisted before imports, so these must appear first.
// ---------------------------------------------------------------------------

const mockScene = {
  // Events (docs/agents/HEW_API.md §4.5): the units mirror and the
  // document lifecycle notes.
  set_display_units: vi.fn(),
  set_ui_context: vi.fn(),
  // Selection and view state (docs/agents/HEW_API.md §7): the mirrors the
  // bus reads.
  set_ui_selection: vi.fn(),
  set_ui_isolation: vi.fn(),
  note_tool_changed: vi.fn(),
  note_document_event: vi.fn(),
  document_id: vi.fn(() => undefined),
  ensure_document_id: vi.fn(() => '00000000-0000-4000-8000-000000000000'),
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
  // (The full health_report_json is the panel's, read only while it is expanded.)
  health_summary_json: () =>
    '{"counts":{"objects":0,"solids":0,"flat_sheets":0,"bent_sheets":0,"cappable":0,"holes":0,"mixed":0,"non_manifold":0},"open":[]}',
  can_scene_undo: () => false,
  can_scene_redo: () => false,
  // Lane C (docs/design/v1.1-cycle.md): this mock is not a real, stateful
  // kernel document, so `at_saved_mark` statically mirrors the OLD
  // unconditional "any mutation dirties" behavior every test here already
  // assumes — `handleDocumentChanged` only reads it after a real mutation
  // fired (never on mount), so `false` here reproduces the pre-Lane-C
  // `afterMutation` latch exactly. No test in this file exercises Save, so
  // `mark_saved`/`undo_depth` are unexercised no-op stubs.
  at_saved_mark: () => false,
  mark_saved: vi.fn(),
  undo_depth: () => 0,
  redo_depth: () => 0,
  history_entries_json: () => JSON.stringify({ undo: [], redo: [], savedDepth: null }),
  geo_reference_json: () => 'null',
  save: () => new Uint8Array(),
  load: vi.fn(),
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

vi.mock('./viewport/Viewport', () => ({
  default: vi.fn(() => null),
}))

const openAnyMock = vi.fn()
const openForImportMock = vi.fn()
const writeToMock = vi.fn()
vi.mock('./io/fileHost', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./io/fileHost')>()
  return {
    ...actual,
    makeFileHost: () => ({
      open: vi.fn(),
      save: vi.fn(),
      saveAs: vi.fn(),
      openForImport: openForImportMock,
      openAny: openAnyMock,
      exportBinary: vi.fn(),
      writeTo: writeToMock,
    }),
  }
})

vi.mock('./io/recoveryStore', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./io/recoveryStore')>()
  return {
    ...actual,
    makeRecoveryStore: () => ({
      write: async () => {},
      list: async () => [],
      claim: async () => null,
      clear: async () => {},
      discardAll: async () => {},
    }),
  }
})

// Every Tauri surface App.tsx touches, stubbed permissively: unsubscribe
// functions for every listener, and a single invoke mock that resolves a
// sensible default per command (null/[] for queries, undefined for
// fire-and-forget) unless a test overrides it — most of App's Tauri-gated
// effects (title push, menu sync, startup handoff, window-list fetch) fire
// on every render regardless of what a given test cares about, and must not
// throw or hang the test.
const invokeMock = vi.fn(async (cmd: string) => {
  switch (cmd) {
    case 'get_recents':
    case 'list_windows':
      return []
    case 'take_pending_recovery':
    case 'take_pending_window_open':
    case 'take_pending_open':
    case 'updater_available':
      return null
    case 'read_file':
      // openPath's `.hew` branch reads this and feeds it straight to
      // mockScene.load (a no-op mock — content is irrelevant to the tests
      // below, only that the read succeeds).
      return new ArrayBuffer(0)
    default:
      return undefined
  }
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
// Needed so App.tsx's menu-action/menu-open-path listener-registration
// effects resolve cleanly at mount instead of throwing against the real,
// un-Tauri'd package (see the coverage note further down on why these two
// listeners aren't asserted on directly here).
const webviewListenMock = vi.fn(async (_event: string, _handler: (e: { payload: string }) => void) =>
  () => {},
)
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ listen: webviewListenMock }),
}))
// A single stable mock (not a fresh vi.fn() per getCurrentWebview() call) so
// tests can grab the registered drag-drop handler and invoke it directly to
// simulate a real OS file drop.
const dragDropMock = vi.fn(
  async (_handler: (e: { payload: { type: string; paths: string[] } }) => void) => () => {},
)
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: dragDropMock }),
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    label: 'main',
    setTitle: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    onFocusChanged: vi.fn(async () => () => {}),
    onCloseRequested: vi.fn(async () => () => {}),
  }),
  getAllWindows: vi.fn(async () => []),
}))
// Stable reference so tests can assert the discard prompt was (or, for the
// new-window route, was NOT) shown — "nothing is discarded when a new window
// opens" is the whole point of routing through open_in_new_window instead of
// a guarded replace.
const askMock = vi.fn(async () => true)
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask: askMock }))

import App from './App'

beforeEach(() => {
  vi.clearAllMocks()
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case 'get_recents':
      case 'list_windows':
        return []
      case 'take_pending_recovery':
      case 'take_pending_window_open':
      case 'take_pending_open':
      case 'updater_available':
        return null
      default:
        return undefined
    }
  })
})

async function renderAndLoad() {
  render(<App />)
  await waitFor(() => screen.getByTitle('Rectangle (R)'), { timeout: 2000 })
}

const triggerOpen = () => {
  fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
  fireEvent.mouseDown(menubar().getByText('Open…'))
}

describe('App — File ▸ Open onto a non-pristine window (Tauri)', () => {
  it('opens a picked .hew file into a NEW window, leaving the current (non-pristine) window untouched', async () => {
    await renderAndLoad()

    // Make the window non-pristine the same way a real session would: open
    // a document into it first.
    openAnyMock.mockResolvedValueOnce({
      kind: 'hew',
      name: 'my-house.hew',
      bytes: new Uint8Array([7, 7, 7]),
      handle: '/tmp/my-house.hew',
    })
    triggerOpen()
    expect(await screen.findByText('my-house.hew')).toBeInTheDocument()
    expect(mockScene.load).toHaveBeenCalledTimes(1)
    // Let the docSessionRef mirror catch up before the next gesture — see
    // makeNonPristine below on why the title rendering isn't enough.
    await act(async () => {})

    // A further Open, now that the window holds a named file, must route
    // into a new window instead of replacing this one.
    openAnyMock.mockResolvedValueOnce({
      kind: 'hew',
      name: 'other.hew',
      bytes: new Uint8Array([1, 1, 1]),
      handle: '/tmp/other.hew',
    })
    triggerOpen()

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('open_in_new_window', { path: '/tmp/other.hew' }),
    )
    // The current window's document is untouched: still my-house.hew, and
    // scene.load was never called a second time.
    expect(screen.getByText('my-house.hew')).toBeInTheDocument()
    expect(mockScene.load).toHaveBeenCalledTimes(1)
  })

  it('a pristine window (fresh launch) still reuses itself in place — no new window for the first Open', async () => {
    await renderAndLoad()

    openAnyMock.mockResolvedValueOnce({
      kind: 'hew',
      name: 'my-house.hew',
      bytes: new Uint8Array([7, 7, 7]),
      handle: '/tmp/my-house.hew',
    })
    triggerOpen()

    expect(await screen.findByText('my-house.hew')).toBeInTheDocument()
    expect(mockScene.load).toHaveBeenCalledTimes(1)
    expect(invokeMock).not.toHaveBeenCalledWith('open_in_new_window', expect.anything())
  })

  it('announces a picked .hew as opened, with its path', async () => {
    await renderAndLoad()
    vi.mocked(mockScene.note_document_event).mockClear()
    openAnyMock.mockResolvedValueOnce({
      kind: 'hew',
      name: 'my-house.hew',
      bytes: new Uint8Array([7, 7, 7]),
      handle: '/tmp/my-house.hew',
    })
    triggerOpen()
    expect(await screen.findByText('my-house.hew')).toBeInTheDocument()
    expect(mockScene.note_document_event).toHaveBeenCalledWith(
      'opened',
      'my-house.hew',
      '/tmp/my-house.hew',
      undefined,
    )
  })

  it('an import pick (no .hew handle) onto a non-pristine window routes through its `path` field', async () => {
    await renderAndLoad()

    openAnyMock.mockResolvedValueOnce({
      kind: 'hew',
      name: 'my-house.hew',
      bytes: new Uint8Array([7, 7, 7]),
      handle: '/tmp/my-house.hew',
    })
    triggerOpen()
    expect(await screen.findByText('my-house.hew')).toBeInTheDocument()
    await act(async () => {}) // docSessionRef mirror; see makeNonPristine

    openAnyMock.mockResolvedValueOnce({
      kind: 'stl',
      name: 'bracket.stl',
      bytes: new Uint8Array([1, 2, 3]),
      path: '/tmp/bracket.stl',
    })
    triggerOpen()

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('open_in_new_window', { path: '/tmp/bracket.stl' }),
    )
    // No STL units chooser in this window — the import never ran here.
    expect(screen.queryByRole('dialog', { name: /stl import units/i })).not.toBeInTheDocument()
  })
})

const triggerImport = () => {
  fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
  fireEvent.mouseDown(menubar().getByText('Import…'))
}

// File ▸ Import… (importDocument) is a SEPARATE function from the unified
// Open dialog (openDocument) — its own explicit, import-only picker — and
// had the identical pristine-bypass defect: it always replaced the current
// window in place after a legacy discard prompt, with no pristine check and
// no `open_in_new_window` routing at all, even on Tauri.
describe('App — File ▸ Import… onto a non-pristine window (Tauri)', () => {
  it('opens a picked import file into a NEW window, leaving the current (non-pristine) window untouched, no discard prompt', async () => {
    await renderAndLoad()
    await makeNonPristine()

    openForImportMock.mockResolvedValueOnce({
      kind: 'stl',
      name: 'bracket.stl',
      bytes: new Uint8Array([1, 2, 3]),
      path: '/tmp/bracket.stl',
    })
    triggerImport()

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('open_in_new_window', { path: '/tmp/bracket.stl' }),
    )
    expect(screen.getByText('my-house.hew')).toBeInTheDocument()
    expect(mockScene.load).toHaveBeenCalledTimes(1) // only the earlier makeNonPristine() open
    expect(askMock).not.toHaveBeenCalled() // nothing discarded — no prompt
    // No STL units chooser in this window — the import never ran here.
    expect(screen.queryByRole('dialog', { name: /stl import units/i })).not.toBeInTheDocument()
  })

  it('onto a pristine window: applies in place, no new window', async () => {
    await renderAndLoad()

    openForImportMock.mockResolvedValueOnce({
      kind: 'stl',
      name: 'bracket.stl',
      bytes: new Uint8Array([1, 2, 3]),
      path: '/tmp/bracket.stl',
    })
    triggerImport()

    // The STL units chooser DOES appear here — the import runs in this
    // (pristine) window, unlike the non-pristine case above.
    expect(await screen.findByRole('dialog', { name: /stl import units/i })).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('open_in_new_window', expect.anything())
  })
})

// ---------------------------------------------------------------------------
// openPath's other entry points — Open Recent, native drag-drop, and a live
// `menu-open-path` delivery (native recent-file click / warm macOS "open
// document" Apple event / second-instance open, which all funnel through the
// same shell event — see deliver_open/emit_to_active in main.rs) — must all
// follow the identical pristine-else-new-window rule openDocument's own Open…
// dialog already does above. Before this fix, openPath always replaced the
// current window in place regardless of pristine-ness, discard-prompting a
// dirty window instead of routing to a new one (the worst of the three
// playtest bugs this fixes).
//
// Coverage note: openRecent and the live `menu-open-path` listener are both
// thin wrappers around this exact same openPath (openRecent = `(path) =>
// openPath(path).catch(...)`; the listener = `openPathRef.current(event.
// payload)` — see App.tsx) — so their BEHAVIOR is already pinned by the
// drag-drop suite below, which drives the identical openPath through a third
// entry point. A UI-level test of openRecent/menu-open-path specifically was
// attempted and dropped: App.tsx registers TWO listeners against
// `@tauri-apps/api/webviewWindow` at mount (menu-action, then menu-open-path)
// plus a `get_recents` fetch against `@tauri-apps/api/core` that recurs on
// every docSession change alongside the title-push effect's own `core` call —
// and Vitest/vite-node's dynamic-import mock interception has a reproducible
// race for concurrent FIRST-time imports of the same specifier: whichever
// call is issued first (by source position) reliably gets the vi.mock'd
// module, the other(s) fall through to the real, unmocked package (confirmed
// by swapping the two listener effects' source order and watching the
// failure move with it — not a property of this diff, a property of the test
// harness). Real-desktop verification should specifically click File ▸ Open
// Recent and exercise a second file-association open while the app is
// already running, onto both a pristine and a dirty window.
// ---------------------------------------------------------------------------

// Opens a named file, THEN dirties it via the semantic test harness (a real
// mutation, not a flag flip — same pattern as App.test.tsx's web-build
// discard test). A merely-named-but-clean document is already non-pristine
// enough to exercise the new-window ROUTING, but it's the wrong fixture for
// asserting the discard prompt is unreachable: confirmDiscard() short-
// circuits to true without ever calling `ask` while `dirty` is false (see
// App.tsx), so a clean fixture would make that assertion pass vacuously even
// if the guard were reinstated by mistake. Genuinely dirtying it first is
// what makes "no discard prompt" a real assertion — the case that actually
// matters ("the worst of the three playtest bugs": a dirty window getting
// silently discard-prompted instead of routed to a new one).
const makeNonPristine = async () => {
  openAnyMock.mockResolvedValueOnce({
    kind: 'hew',
    name: 'my-house.hew',
    bytes: new Uint8Array([7, 7, 7]),
    handle: '/tmp/my-house.hew',
  })
  triggerOpen()
  expect(await screen.findByText('my-house.hew')).toBeInTheDocument()
  const harness = (window as unknown as {
    __hew_test: { addNodeTag: (kind: string, id: string, path: string[]) => void }
  }).__hew_test
  act(() => harness.addNodeTag('object', '1', ['tag']))
  // Flush passive effects before returning. `App` mirrors its document session
  // into `docSessionRef` from a `useEffect`, and the menu handlers this helper
  // sets up for read the REF, not the state — so until that effect has run,
  // `isPristineDocument` still sees the pristine session this helper just
  // finished replacing, and a following gesture takes the pristine branch.
  // React 18 happened to have flushed it by the time `findByText` resolved;
  // React 19 does not, so ask for it explicitly rather than rely on timing.
  await act(async () => {})
}

async function findDragDropHandler(): Promise<(e: { payload: { type: string; paths: string[] } }) => void> {
  await waitFor(() => expect(dragDropMock).toHaveBeenCalled())
  return dragDropMock.mock.calls[0][0]
}

describe('App — native drag-drop follows the pristine-else-new-window rule (Tauri)', () => {
  it('onto a non-pristine window: opens into a NEW window, current window untouched, no discard prompt', async () => {
    await renderAndLoad()
    const handler = await findDragDropHandler()
    await makeNonPristine()

    handler({ payload: { type: 'drop', paths: ['/tmp/dropped.hew'] } })

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('open_in_new_window', { path: '/tmp/dropped.hew' }),
    )
    expect(screen.getByText('my-house.hew')).toBeInTheDocument()
    expect(mockScene.load).toHaveBeenCalledTimes(1)
    expect(askMock).not.toHaveBeenCalled()
  })

  it('onto a pristine window: applies in place, no new window', async () => {
    await renderAndLoad()
    const handler = await findDragDropHandler()

    handler({ payload: { type: 'drop', paths: ['/tmp/dropped.hew'] } })

    expect(await screen.findByText('dropped.hew')).toBeInTheDocument()
    expect(mockScene.load).toHaveBeenCalledTimes(1)
    expect(invokeMock).not.toHaveBeenCalledWith('open_in_new_window', expect.anything())
  })

  it('reads a dropped picture to arm Place Image, opening nothing', async () => {
    await renderAndLoad()
    const handler = await findDragDropHandler()

    handler({ payload: { type: 'drop', paths: ['/tmp/plan.png'], position: { x: 10, y: 20 } } } as never)

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('read_file', { path: '/tmp/plan.png' }))
    expect(mockScene.load).not.toHaveBeenCalled()
    expect(invokeMock).not.toHaveBeenCalledWith('open_in_new_window', expect.anything())
  })

  // (A .png is no longer an example: a dropped picture arms Place Image.)
  it('ignores a non-.hew path in a drop payload (unchanged pre-existing filter)', async () => {
    await renderAndLoad()
    const handler = await findDragDropHandler()

    handler({ payload: { type: 'drop', paths: ['/tmp/not-a-hew-file.txt'] } })

    // Nothing to await for a no-op; give any accidental async work a tick.
    await new Promise((r) => setTimeout(r, 0))
    expect(mockScene.load).not.toHaveBeenCalled()
    expect(invokeMock).not.toHaveBeenCalledWith('open_in_new_window', expect.anything())
  })
})

describe('App — the document id reaches the shell with the save (Tauri)', () => {
  it('Save As reports the minted document id before the save is announced', async () => {
    const id = '3f2c9107-5ed4-40a8-806b-1ec297405d83'
    const scene = mockScene as Record<string, unknown>
    scene.set_skp_source = vi.fn()
    scene.commit_skp_save = vi.fn()
    vi.mocked(mockScene.document_id).mockReturnValue(id as never)
    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case 'get_recents':
        case 'list_windows':
          return []
        case 'pick_save_target':
          return { path: '/tmp/m.hew', format: 'hew', options: {} } as unknown as ArrayBuffer
        case 'take_pending_recovery':
        case 'take_pending_window_open':
        case 'take_pending_open':
        case 'updater_available':
          return null
        default:
          return undefined
      }
    })
    writeToMock.mockResolvedValue({ name: 'm.hew', format: 'hew', handle: '/tmp/m.hew' })
    await renderAndLoad()
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('Save As…'))
    await waitFor(() =>
      expect(mockScene.note_document_event).toHaveBeenCalledWith('saved', 'm.hew', expect.anything(), 'hew'),
    )
    expect(mockScene.ensure_document_id).toHaveBeenCalled()
    const reportCall = (invokeMock.mock.calls as unknown as [string, { id?: string } | undefined][]).findIndex(
      ([cmd, args]) => cmd === 'set_document_id' && args?.id === id,
    )
    expect(reportCall).toBeGreaterThanOrEqual(0)
    const reportOrder = invokeMock.mock.invocationCallOrder[reportCall]
    const savedCall = vi
      .mocked(mockScene.note_document_event)
      .mock.calls.findIndex(([kind]) => kind === 'saved')
    const savedOrder = vi.mocked(mockScene.note_document_event).mock.invocationCallOrder[savedCall]
    expect(reportOrder).toBeLessThan(savedOrder)
  })
})
