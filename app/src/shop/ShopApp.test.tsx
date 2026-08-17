/**
 * ShopApp orchestration tests — mirrors App.test.tsx's mocking strategy
 * (stub `wasm/loader` since the real `.wasm` doesn't load in CI/jsdom, and
 * stub `viewport/Viewport` since three.js/WebGL don't work in jsdom).
 *
 * Covers: the loading→empty-state transition, opening a `.hew` document
 * (name shows, empty state clears), refusing a non-`.hew` pick, "Use full
 * editor" persisting the shell-mode override, the 3-tool switcher, and
 * tap-to-inspect rendering a card from the (captured) `onSelect`/
 * `onSelectSnap` props the mocked Viewport would otherwise call internally.
 *
 * Window size: every test in this file EXCEPT the dedicated "landscape
 * orientation" describe block near the bottom runs under a stubbed PORTRAIT
 * viewport (`stubWindowSize(390, 844)` in the shared `beforeEach` below) —
 * jsdom's own default (`1024x768`, wider than tall) would otherwise read as
 * LANDSCAPE under `orientation.ts`'s `aspect > 1` rule and silently swap
 * every one of these tests from the fused portrait dock+sheet chrome they
 * assert on to the landscape rail+centered-sheet one, with no test failure
 * pointing at why. The landscape describe block re-stubs a landscape size
 * for its own tests.
 */
import { render, screen, fireEvent, waitFor, act, within } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'

/** Sets `window.innerWidth`/`innerHeight` (jsdom's own are read-only own
 *  properties, not plain assignable fields) — matches `orientation.test.ts`'s
 *  own stubbing helper. Callers that need `useShopOrientation` to react to a
 *  size change AFTER mount (not just at it) still need to dispatch a
 *  `resize` event themselves; this alone only affects what a FRESH mount
 *  reads. */
function stubWindowSize(widthPx: number, heightPx: number): void {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: widthPx })
  Object.defineProperty(window, 'innerHeight', { configurable: true, value: heightPx })
}

/** Mutable per-test document content (top-level objects + their meshes) —
 *  plain object rather than vi.fn() mocks, since PartsSheet.tsx recomputes
 *  its sections on every render and a `mockReturnValueOnce` would only
 *  satisfy the first of several calls. Reset in `beforeEach`, so a test
 *  that wants a populated document just assigns into it directly. */
let fixture: {
  objects: bigint[]
  objectMesh: Record<string, Float32Array>
  objectName: Record<string, string>
  /** `{id: [tagPath, ...]}` — read by `node_tags` below. Empty by default
   *  (`beforeEach`); a test that wants a tagged section assigns into it. */
  nodeTags: Record<string, string[]>
}

/** A minimal fake of the wasm-bound `CameraState` getter object
 *  `Scene.camera_state()` returns — `applyOpenedBytes`'s saved-camera branch
 *  reads every one of these getters, then calls `.free()`. Only task 4b's
 *  own recents-thumbnail test (`mockScene.camera_state.mockReturnValueOnce`)
 *  ever needs a real one; every other test leaves the default `undefined`
 *  ("no saved camera") alone. */
function makeFakeCameraState() {
  return {
    projection: () => 'perspective' as const,
    fov_deg: () => 35,
    eye_x: () => 1, eye_y: () => 2, eye_z: () => 3,
    target_x: () => 0, target_y: () => 0, target_z: () => 0,
    up_x: () => 0, up_y: () => 0, up_z: () => 1,
    free: vi.fn(),
  }
}

/** A minimal fake of the wasm-bound `ResolvedSceneJs` `Scene.resolve_scene()`
 *  returns (docs/design/scenes.md §3.1) — every getter defaults to "not
 *  captured" (mirrors a Scene with nothing checked), so a bare
 *  `resolve_scene` call is a no-op until a test overrides specific fields.
 *  `free()` is a spy so tests can assert it's always released. */
function makeFakeResolvedScene(overrides: {
  cameraJson?: string
  hiddenObjectIds?: bigint[]
  hiddenInstanceIds?: bigint[]
  hiddenTagPaths?: string[]
  hiddenNodeKinds?: number[]
  hiddenNodeIds?: bigint[]
  hasHiddenTags?: boolean
  hasHiddenNodes?: boolean
  sectionJson?: string
  hasSection?: boolean
} = {}) {
  const hasHidden = (overrides.hasHiddenTags ?? false) || (overrides.hasHiddenNodes ?? false)
  return {
    camera_json: () => overrides.cameraJson,
    display_json: () => undefined as string | undefined,
    has_hidden: () => hasHidden,
    has_hidden_nodes: () => overrides.hasHiddenNodes ?? false,
    has_hidden_tags: () => overrides.hasHiddenTags ?? false,
    has_section: () => overrides.hasSection ?? false,
    hidden_instance_ids: () => new BigUint64Array(overrides.hiddenInstanceIds ?? []),
    hidden_node_ids: () => new BigUint64Array(overrides.hiddenNodeIds ?? []),
    hidden_node_kinds: () => new Uint8Array(overrides.hiddenNodeKinds ?? []),
    hidden_object_ids: () => new BigUint64Array(overrides.hiddenObjectIds ?? []),
    hidden_tag_paths: () => overrides.hiddenTagPaths ?? [],
    section_json: () => overrides.sectionJson,
    free: vi.fn(),
  }
}

const mockScene = {
  object_ids: () => new BigUint64Array(fixture.objects),
  group_ids: () => new BigUint64Array(),
  instance_ids: () => new BigUint64Array(),
  sketch_ids: () => new BigUint64Array(),
  top_level_nodes: (): { kind: string; id: bigint }[] => fixture.objects.map((id) => ({ kind: 'object', id })),
  group_members: (): { kind: string; id: bigint }[] => [],
  object_name: (id: bigint) => fixture.objectName[String(id)],
  group_name: () => undefined as string | undefined,
  instance_name: () => undefined as string | undefined,
  instance_def: () => undefined as bigint | undefined,
  component_name: () => undefined as string | undefined,
  node_tags: (_kind: number, id: bigint) => fixture.nodeTags[String(id)] ?? [],
  object_mesh: (id: bigint) => ({
    positions: () => fixture.objectMesh[String(id)] ?? new Float32Array([0, 0, 0, 1, 1, 1]),
    free: () => {},
  }),
  instance_pose: () => undefined as Float64Array | undefined,
  instance_expanded_members: () => new BigUint64Array(),
  instance_expanded_local_poses: () => new Float64Array(),
  load: vi.fn(),
  // vi.fn() (not a plain closure) — task 4b's own saved-camera recents-
  // thumbnail test overrides this via `mockReturnValueOnce`; every other
  // test leaves it at the default `undefined` ("no saved camera", the
  // common case every other open-a-document test exercises).
  camera_state: vi.fn((): ReturnType<typeof makeFakeCameraState> | undefined => undefined),
  set_hidden: vi.fn(),
  // Round-3 playtest finding 2's root-cause fix (`applyOpenedBytes` — every
  // open re-issues this, since `Scene.load` itself resets the kernel's
  // inference state, silently re-enabling axis snapping otherwise).
  set_axes_snappable: vi.fn(),
  // Registry accessors the seed fix (documentLoad.ts's
  // seedHiddenKeysFromRegistry/seedHiddenTagPathsFromRegistry) reads on
  // every open — vi.fn() (not plain closures) so individual tests can
  // override the return value for one open via mockReturnValueOnce;
  // default is an empty document, nothing seeded as hidden.
  user_hidden_kinds: vi.fn(() => new Uint8Array()),
  user_hidden_ids: vi.fn(() => new BigUint64Array()),
  tag_meta_paths: vi.fn(() => [] as string[]),
  tag_meta_hidden: vi.fn(() => new Uint8Array()),
  // Scenes (docs/design/scenes.md §6) — read-only in Shop Mode. Defaults to
  // "no Scenes, no persisted section plane" so every EXISTING test (none of
  // which know about Scenes) is unaffected; the dedicated "Scenes" describe
  // block below overrides these per test via mockReturnValueOnce/mockReturnValue.
  scenes_json: vi.fn(() => '[]'),
  section_plane_json: vi.fn((): string | undefined => undefined),
  resolve_scene: vi.fn((_sid: bigint) => makeFakeResolvedScene()),
  scene_drift: vi.fn(() => '{}'),
}

vi.mock('../wasm/loader', () => ({
  loadKernel: vi.fn(() =>
    Promise.resolve({
      version: () => '0.1.0-test',
      newScene: () => mockScene,
    }),
  ),
}))

vi.mock('../viewport/Viewport', () => ({
  default: vi.fn(() => null),
}))

// isArQuickLookCandidate's real UA/touch-point detection is arQuickLook.test.ts's
// job (it needs a real jsdom navigator to matrix-test) — here it's a plain
// vi.fn() so each "View in AR" test controls whether the button exists
// without faking a whole User-Agent string. launchArQuickLook is mocked too,
// so these tests assert ShopApp calls it correctly rather than exercising its
// real Blob/anchor mechanics (also arQuickLook.test.ts's job).
vi.mock('./arQuickLook', () => ({
  isArQuickLookCandidate: vi.fn(() => false),
  launchArQuickLook: vi.fn(),
}))

import { ShopApp, LONG_PRESS_MS, RECENT_THUMB_SIZE_PX } from './ShopApp'
import Viewport from '../viewport/Viewport'
import { makeFileHost } from '../io/fileHost'
import { listRecents, recordRecent } from '../io/recents'
import { isArQuickLookCandidate, launchArQuickLook } from './arQuickLook'
import { setLengthUnit } from '../settings/units'
import { getSceneTransitions, SCENE_TRANSITION_MS } from '../settings/sceneTransitions'
import { HINT_STORAGE_KEYS } from './hints'
import { loadKernel } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import type { Snap } from '../tools/types'

vi.mock('../io/fileHost', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../io/fileHost')>()
  return { ...actual, makeFileHost: vi.fn(actual.makeFileHost) }
})

// jsdom has no real indexedDB, so io/recents.ts's own guards would silently
// no-op every call anyway — mocked explicitly instead so tests can assert
// ShopApp actually calls recordRecent/listRecents at the right seams (real
// IndexedDB round-tripping is io/recents.test.ts's job, not this file's).
vi.mock('../io/recents', () => ({
  listRecents: vi.fn(async () => []),
  recordRecent: vi.fn(async () => undefined),
}))

/** The imperative surface ShopApp calls on the (real) Viewport's `apiRef` —
 *  a plain object of spies, the same "populate apiRef.current manually"
 *  pattern App.test.tsx uses against its own mocked Viewport. */
function makeViewportApiStub() {
  return {
    setHidden: vi.fn(),
    notifyLoaded: vi.fn(),
    zoomExtents: vi.fn(),
    zoomToWorldBounds: vi.fn(),
    applyCameraState: vi.fn(),
    cancelPendingRescale: vi.fn(),
    // Finding 5 (snap hysteresis) — called once per tap release
    // (`handleWrapperPointerUp`); every test that fires a pointerup on the
    // viewport wrapper exercises this, so it needs a real stub or those
    // tests throw on the un-mocked call.
    clearSnapHold: vi.fn(),
    exportUsdz: vi.fn(async (): Promise<Uint8Array | null> => new Uint8Array([1, 2, 3])),
    // Gesture hint (a)'s dot re-projection (`ShopApp.tsx`'s `hintDotScreen`
    // memo) — a fixed, off-screen-free stub so tests that open a document
    // with a target part don't crash rendering `HintOverlay`; hints.ts's own
    // tests cover the engine's state machine and HintOverlay.test.tsx
    // covers what each hint variant renders — this file only needs
    // rendering not to crash.
    worldToScreen: vi.fn(() => ({ x: 100, y: 100, behind: false })),
    // Playtest finding 4's thumbnail capture (applyOpenedBytes) — a 0×0
    // frame so `buildRecentThumbnail` returns null (no thumbnail recorded)
    // rather than every existing test needing to know about this at all;
    // the dedicated "recents thumbnails" describe block below overrides it.
    captureFrame: vi.fn(() => ({ width: 0, height: 0, pixels: new Uint8Array(0) })),
    // Playtest finding 12 (Views sheet).
    setStandardView: vi.fn(),
    toggleProjection: vi.fn(),
    getProjection: vi.fn(() => 'perspective' as const),
    // Scenes (docs/design/scenes.md §6) — activation's own renderer/camera
    // calls. `tweenCameraState` invokes its `onDone` synchronously (no real
    // animation frame in jsdom) so `refreshSceneDrift` runs deterministically
    // within the same test tick.
    setSectionPlane: vi.fn(),
    getCameraState: vi.fn(() => ({
      projection: 'perspective' as const, fovDeg: 45, eye: [1, 2, 3] as [number, number, number],
      target: [0, 0, 0] as [number, number, number], up: [0, 0, 1] as [number, number, number],
    })),
    tweenCameraState: vi.fn((_cam: unknown, _ms: number, onDone?: () => void) => onDone?.()),
    cancelCameraTween: vi.fn(),
  }
}

/** The props ShopApp most recently passed to the mocked Viewport. */
function latestViewportProps(): {
  activeTool?: string
  selectedIds?: NodeRef[]
  onSelect?: (node: NodeRef | null, additive: boolean) => void
  onSelectSnap?: (snap: Snap | null) => void
  onIsolateRequest?: (node: NodeRef) => void
  onInternalToolChange?: (name: string) => void
  onTapeMeasurePoints?: (points: readonly [number, number, number][]) => void
  onProjectionChange?: (projection: 'perspective' | 'parallel') => void
  onCameraSettled?: () => void
  apiRef?: { current: ReturnType<typeof makeViewportApiStub> | null }
  background?: string
  showGrid?: boolean
  showAxes?: boolean
  readOnly?: boolean
} {
  const calls = vi.mocked(Viewport).mock.calls
  return calls[calls.length - 1][0] as never
}

async function renderAndLoad() {
  render(<ShopApp />)
  await waitFor(() => screen.getByRole('button', { name: /^open a model…$/i }))
}

const ORIGINAL_WINDOW_WIDTH = window.innerWidth
const ORIGINAL_WINDOW_HEIGHT = window.innerHeight

beforeEach(() => {
  vi.clearAllMocks()
  localStorage.clear()
  fixture = { objects: [], objectMesh: {}, objectName: {}, nodeTags: {} }
  // clearAllMocks() only resets calls/results, not a mockReturnValue an
  // earlier test set — pin this explicitly every test so "View in AR"
  // visibility never bleeds across tests regardless of run order.
  vi.mocked(isArQuickLookCandidate).mockReturnValue(false)
  // Portrait by default (module doc) — the "landscape orientation" describe
  // block re-stubs this for its own tests, each of which renders its own
  // fresh ShopApp AFTER doing so.
  stubWindowSize(390, 844)
})

afterEach(() => {
  stubWindowSize(ORIGINAL_WINDOW_WIDTH, ORIGINAL_WINDOW_HEIGHT)
})

describe('ShopApp — boot', () => {
  it('shows the empty state (Open button) once the kernel loads with nothing open', async () => {
    await renderAndLoad()
    expect(screen.getByRole('button', { name: /^open a model…$/i })).toBeInTheDocument()
  })

  it('mounts Viewport with the loaded scene once the kernel resolves', async () => {
    await renderAndLoad()
    expect(vi.mocked(Viewport)).toHaveBeenCalled()
    expect(latestViewportProps()).toMatchObject({ activeTool: 'Select' })
  })

  // design_handoff_shop_mode/README.md Design Tokens' --viewport-bg — Shop
  // Mode opts into the gradient backdrop; the editor (App.tsx) never passes
  // this prop at all, so it stays on Viewport's 'editor-default' default.
  it('opts the viewport into the Shop Mode gradient background', async () => {
    await renderAndLoad()
    expect(latestViewportProps().background).toBe('shop-gradient')
  })

  // Shop-mode playtest finding 1 (CRITICAL, contract violation): the actual
  // drag-arm/kernel-mutation logic lives inside Viewport's own internal
  // pointer handlers, which this file can't exercise (`Viewport` is mocked
  // above — see the `e2e/shop-mode.spec.ts` "CONTRACT" test for the real,
  // full-stack regression coverage of the gesture itself). This is the
  // shallow half of that proof: ShopApp actually threads the opt-in prop
  // through as a mount-time value, same convention as `background`/
  // `showGrid`/`showAxes` just above.
  it('passes readOnly to the viewport — Shop Mode issues zero kernel mutations (module doc)', async () => {
    await renderAndLoad()
    expect(latestViewportProps().readOnly).toBe(true)
  })

  // The grid/axes hide is load-bearing for the gradient itself
  // (InfiniteGrid's ground plane is opaque and would otherwise sit in front
  // of it) — passed as mount-time PROPS, not a post-mount ViewportApi call,
  // so this is correct even across a React StrictMode dev double-mount.
  it('hides the ground grid and origin axes (a clean product-photo viewer, not the editor\'s drafting canvas)', async () => {
    await renderAndLoad()
    expect(latestViewportProps()).toMatchObject({ showGrid: false, showAxes: false })
  })
})

describe('ShopApp — open document', () => {
  it('opening a .hew pick shows the document name and clears the empty state', async () => {
    const host = { openAny: vi.fn(async () => ({ kind: 'hew' as const, name: 'Cafe Table.hew', bytes: new Uint8Array([1]), handle: null })) }
    vi.mocked(makeFileHost).mockReturnValue(host as never)

    await renderAndLoad()
    fireEvent.click(screen.getByRole('button', { name: /^open a model…$/i }))
    await waitFor(() => expect(screen.getByText('Cafe Table.hew')).toBeInTheDocument())
    expect(screen.queryByRole('button', { name: /^open a model…$/i })).not.toBeInTheDocument()
    expect(mockScene.load).toHaveBeenCalled()
    // Round-3 playtest finding 2's root cause: `Scene.load` resets the
    // kernel's inference state (wasm-api's own `load`), silently
    // re-enabling axis snapping — re-suppressed on every open, not just at
    // Viewport's own one-time mount-time call (which only ever covers the
    // empty scene BEFORE any real document exists).
    expect(mockScene.set_axes_snappable).toHaveBeenCalledWith(false)
  })

  it('refuses a non-.hew pick with a toast instead of importing', async () => {
    const host = { openAny: vi.fn(async () => ({ kind: 'stl' as const, name: 'part.stl', bytes: new Uint8Array([1]) })) }
    vi.mocked(makeFileHost).mockReturnValue(host as never)

    await renderAndLoad()
    fireEvent.click(screen.getByRole('button', { name: /^open a model…$/i }))
    await waitFor(() => expect(screen.getByText(/only open \.hew files/i)).toBeInTheDocument())
    // Still no document open — the empty state stays.
    expect(screen.getByRole('button', { name: /^open a model…$/i })).toBeInTheDocument()
  })
})

describe('ShopApp — seed hidden state on open (SEED FIX)', () => {
  it('pushes the document\'s persisted user-hidden node into the renderer hidden set on open', async () => {
    const host = { openAny: vi.fn(async () => ({ kind: 'hew' as const, name: 'Cafe Table.hew', bytes: new Uint8Array([1]), handle: null })) }
    vi.mocked(makeFileHost).mockReturnValue(host as never)
    // kind 0 = object (documentLoad.ts's seedHiddenKeysFromRegistry kindNames).
    mockScene.user_hidden_kinds.mockReturnValueOnce(new Uint8Array([0]))
    mockScene.user_hidden_ids.mockReturnValueOnce(new BigUint64Array([7n]))

    await renderAndLoad()
    const api = makeViewportApiStub()
    const { apiRef } = latestViewportProps()
    if (apiRef !== undefined) apiRef.current = api

    fireEvent.click(screen.getByRole('button', { name: /^open a model…$/i }))
    await waitFor(() => expect(screen.getByText('Cafe Table.hew')).toBeInTheDocument())

    // setHidden is called twice: once to clear stale ids ([],[]) before
    // notifyLoaded, once with the seeded hides after — the last call is the
    // one that matters for what actually renders hidden.
    expect(api.setHidden).toHaveBeenCalled()
    const lastCall = api.setHidden.mock.calls[api.setHidden.mock.calls.length - 1]
    expect(lastCall[0]).toEqual([7n])
    expect(lastCall[1]).toEqual([])
  })

  it('seeds nothing hidden for a document with an empty user-hidden registry', async () => {
    const host = { openAny: vi.fn(async () => ({ kind: 'hew' as const, name: 'Wall Clock.hew', bytes: new Uint8Array([1]), handle: null })) }
    vi.mocked(makeFileHost).mockReturnValue(host as never)

    await renderAndLoad()
    const api = makeViewportApiStub()
    const { apiRef } = latestViewportProps()
    if (apiRef !== undefined) apiRef.current = api

    fireEvent.click(screen.getByRole('button', { name: /^open a model…$/i }))
    await waitFor(() => expect(screen.getByText('Wall Clock.hew')).toBeInTheDocument())

    const lastCall = api.setHidden.mock.calls[api.setHidden.mock.calls.length - 1]
    expect(lastCall).toEqual([[], []])
  })
})

describe('ShopApp — "Use full editor"', () => {
  // The original test replaced `window.location` with a plain object
  // (`{ ...window.location, reload }`) and never restored it — permanently
  // clobbering every LATER test in this file (and any file run in the same
  // worker after it) that relies on `window.location` being a REAL
  // `Location` (e.g. `window.history.replaceState` actually updating
  // `window.location.hash`, as the receive-gate tests near the bottom of
  // this file do). Captured/restored here so this test's own stub stays
  // scoped to itself. `configurable: true` on the stub is required for the
  // restore to be possible at all — `Object.defineProperty` defaults
  // `configurable` to `false` when omitted, which would make the ORIGINAL
  // descriptor (captured below, itself configurable — that's what let this
  // test replace it in the first place) permanently un-redefinable too.
  const originalLocationDescriptor = Object.getOwnPropertyDescriptor(window, 'location')!
  afterEach(() => {
    Object.defineProperty(window, 'location', originalLocationDescriptor)
  })

  it('persists the editor override and reloads', async () => {
    const reload = vi.fn()
    Object.defineProperty(window, 'location', { value: { ...window.location, reload }, writable: true, configurable: true })

    await renderAndLoad()
    fireEvent.click(screen.getByLabelText(/^document menu/i))
    fireEvent.click(screen.getByRole('button', { name: /use full editor/i }))

    expect(localStorage.getItem('hew:shellMode')).toBe('editor')
    expect(reload).toHaveBeenCalled()
  })
})

describe('ShopApp — tool switcher', () => {
  // The tool-switcher segments live in the workbench dock, which — like the
  // old floating HUD chips it replaces — only renders once a document is
  // open (design_handoff_shop_mode §1/§8: the empty state is its own
  // full-bleed screen, no dock beneath it), so these need a loaded fixture
  // rather than the bare `renderAndLoad()` empty state.
  it('switching to Orbit updates the activeTool prop Viewport receives', async () => {
    await renderAndOpenWithFixture()
    fireEvent.click(screen.getByRole('button', { name: /^orbit$/i }))
    await waitFor(() => expect(latestViewportProps().activeTool).toBe('Orbit'))
  })

  it('switching to Tape Measure updates the activeTool prop Viewport receives', async () => {
    await renderAndOpenWithFixture()
    fireEvent.click(screen.getByRole('button', { name: /^tape measure$/i }))
    await waitFor(() => expect(latestViewportProps().activeTool).toBe('Tape Measure'))
  })

  // Adversarial-review finding 1 (CRITICAL), defense-in-depth half: even if
  // Viewport's own `switchToolRef` allowlist (Viewport.test.tsx's
  // `isToolSwitchAllowedUnderReadOnly`) somehow let a non-Shop-Mode tool
  // name through, `onInternalToolChange` — Viewport reporting back
  // whatever it just settled on — used to blind-cast ANY string into
  // `ShopToolName` and set it as `activeTool`. `handleInternalToolChange`'s
  // own `SHOP_TOOL_NAMES` allowlist now ignores anything outside the
  // 3-tool registry instead.
  it('ignores an onInternalToolChange report for a tool outside the 3-tool registry', async () => {
    await renderAndOpenWithFixture()
    expect(latestViewportProps().activeTool).toBe('Select')

    act(() => { latestViewportProps().onInternalToolChange?.('Move') })
    expect(latestViewportProps().activeTool).toBe('Select')

    // A legitimate report still goes through — the allowlist isn't just
    // silently freezing `activeTool` altogether.
    act(() => { latestViewportProps().onInternalToolChange?.('Orbit') })
    expect(latestViewportProps().activeTool).toBe('Orbit')
  })
})

// Adversarial-review finding 3 (MAJOR): Tape Measure markers used to
// survive a document swap. `TapeMeasureTool.test.ts` covers the tool's own
// `cancel()`/`onDocumentReset()` fix at the source; this covers
// `ShopApp.tsx`'s own belt-and-suspenders clear in `applyOpenedBytes`.
describe('ShopApp — Tape Measure markers (adversarial-review finding 3)', () => {
  it('clears on a document swap (applyOpenedBytes)', async () => {
    // Inlined rather than `renderAndOpenWithFixture()` — that helper's own
    // `host` (with `openAny` pinned to always resolve "Doc.hew") is a
    // closure-local `useRef` capture the MOMENT `ShopApp` mounts, so a
    // `mockReturnValue` set AFTER mount can't reach it; a SECOND, DIFFERENT
    // open needs the same host object across both picks, just re-armed with
    // `mockResolvedValueOnce`.
    const host = {
      openAny: vi.fn(async () => ({ kind: 'hew' as const, name: 'Doc.hew', bytes: new Uint8Array([1]), handle: null })),
    }
    vi.mocked(makeFileHost).mockReturnValue(host as never)

    await renderAndLoad()
    const api = makeViewportApiStub()
    const { apiRef } = latestViewportProps()
    if (apiRef !== undefined) apiRef.current = api

    fireEvent.click(screen.getByRole('button', { name: /^open a model…$/i }))
    await waitFor(() => expect(screen.getByText('Doc.hew')).toBeInTheDocument())

    act(() => {
      latestViewportProps().onTapeMeasurePoints?.([[0, 0, 0], [1, 0, 0]])
    })
    // `r="7"` — the tape marker's own radius, distinct from the ⋯ menu
    // button's own 3 decorative dots (`r="1.8"`) also present in the DOM.
    await waitFor(() => expect(document.querySelectorAll('circle[r="7"]').length).toBeGreaterThan(0))

    host.openAny.mockResolvedValueOnce({ kind: 'hew', name: 'Second.hew', bytes: new Uint8Array([2]), handle: null })
    fireEvent.click(screen.getByLabelText(/^document menu/i))
    fireEvent.click(screen.getByRole('button', { name: /^open…$/i }))
    await waitFor(() => expect(screen.getByText('Second.hew')).toBeInTheDocument())

    expect(document.querySelectorAll('circle[r="7"]').length).toBe(0)
  })
})

describe('ShopApp — tap-to-inspect', () => {
  it('renders an inspect card for a tapped object, naming it and its dimensions', async () => {
    await renderAndLoad()
    // The Orbit button used to be a reliable landmark for the viewport
    // wrapper regardless of document state (the old floating HUD chips
    // rendered unconditionally); the redesigned workbench dock only renders
    // once a document is open (design_handoff_shop_mode §1/§8 — the empty
    // state is its own full-bleed screen with no dock beneath it), so these
    // tap-to-inspect tests — which don't otherwise care about document
    // state — grab the wrapper directly by its own testid instead.
    const container = screen.getByTestId('shop-viewport')
    fireEvent.pointerDown(container, { clientX: 50, clientY: 60 })

    const { onSelect, onSelectSnap } = latestViewportProps()
    onSelectSnap?.(null)
    onSelect?.({ kind: 'object', id: 1n }, false)

    await waitFor(() => expect(screen.getByText('Object 1')).toBeInTheDocument())
  })

  it('dismisses the inspect card when the tap resolves to nothing', async () => {
    await renderAndLoad()
    // The Orbit button used to be a reliable landmark for the viewport
    // wrapper regardless of document state (the old floating HUD chips
    // rendered unconditionally); the redesigned workbench dock only renders
    // once a document is open (design_handoff_shop_mode §1/§8 — the empty
    // state is its own full-bleed screen with no dock beneath it), so these
    // tap-to-inspect tests — which don't otherwise care about document
    // state — grab the wrapper directly by its own testid instead.
    const container = screen.getByTestId('shop-viewport')
    fireEvent.pointerDown(container, { clientX: 50, clientY: 60 })
    const { onSelect, onSelectSnap } = latestViewportProps()
    onSelectSnap?.(null)
    onSelect?.({ kind: 'object', id: 1n }, false)
    await waitFor(() => expect(screen.getByText('Object 1')).toBeInTheDocument())

    onSelectSnap?.(null)
    onSelect?.(null, false)
    await waitFor(() => expect(screen.queryByText('Object 1')).not.toBeInTheDocument())
  })
})

/** Open a `.hew` pick into a freshly rendered ShopApp, with `fixture`
 *  already populated by the caller — returns the (now-populated) viewport
 *  api stub, wired into the mocked Viewport's `apiRef` before the open so
 *  it captures every `setHidden` call the open itself makes. */
async function renderAndOpenWithFixture(): Promise<ReturnType<typeof makeViewportApiStub>> {
  const host = { openAny: vi.fn(async () => ({ kind: 'hew' as const, name: 'Doc.hew', bytes: new Uint8Array([1]), handle: null })) }
  vi.mocked(makeFileHost).mockReturnValue(host as never)

  await renderAndLoad()
  const api = makeViewportApiStub()
  const { apiRef } = latestViewportProps()
  if (apiRef !== undefined) apiRef.current = api

  fireEvent.click(screen.getByRole('button', { name: /^open a model…$/i }))
  await waitFor(() => expect(screen.getByText('Doc.hew')).toBeInTheDocument())
  return api
}

describe('ShopApp — Parts sheet', () => {
  it('renders one row per top-level object, with its dimensions', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 0.1, 0.2, 0.3])
    await renderAndOpenWithFixture()

    expect(screen.getByText('Object 1')).toBeInTheDocument()
    // Just check the row mounted with a dimensions readout, not the exact
    // formatted string — which format (metric columns vs. imperial stacked)
    // the test environment's locale seeds by default isn't this test's
    // concern (localeUnits.test.ts's job).
    const row = screen.getByText('Object 1').closest('div') as HTMLElement
    expect(row.textContent).toMatch(/\d/)
  })

  it('tapping a row highlights (selects) the part and zooms to it', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    const api = await renderAndOpenWithFixture()

    const row = screen.getByText('Object 1').closest('div') as HTMLElement
    fireEvent.pointerDown(row, { clientX: 10, clientY: 10 })
    fireEvent.pointerUp(row, { clientX: 10, clientY: 10 })

    await waitFor(() => expect(latestViewportProps().selectedIds).toEqual([{ kind: 'object', id: 1n }]))
    expect(api.zoomToWorldBounds).toHaveBeenCalledWith([0, 0, 0], [1, 1, 1])
  })

  // A row tap is a PROGRAMMATIC camera jump (`zoomToNodeOrExtents`'s own
  // doc comment) — same "camera movement dismisses instantly" rule the
  // viewport's own camera DRAG already enforced. An adversarial-review
  // finding: this used to leave a stale card up (only the drag path
  // dismissed). `InspectCard`'s `role="status"` is the one element in this
  // tree that identifies it uniquely — with a document loaded, both the
  // card AND the sheet's own row render the tapped part's NAME text, so
  // `getByText('Object 1')` alone can't tell them apart.
  it('tapping a Parts-sheet row dismisses an already-open inspect card instantly', async () => {
    fixture.objects = [1n, 2n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    fixture.objectMesh['2'] = new Float32Array([0, 0, 0, 1, 1, 1])
    await renderAndOpenWithFixture()

    // Open the card the same way the tap-to-inspect tests do — a viewport
    // tap resolving to Object 1.
    const container = screen.getByTestId('shop-viewport')
    fireEvent.pointerDown(container, { clientX: 50, clientY: 60 })
    const { onSelect, onSelectSnap } = latestViewportProps()
    onSelectSnap?.(null)
    onSelect?.({ kind: 'object', id: 1n }, false)
    await waitFor(() => expect(screen.getByRole('status')).toBeInTheDocument())

    // A DIFFERENT row's tap — Object 2, so this isn't just re-tapping the
    // same already-inspected part.
    const row2 = screen.getByText('Object 2').closest('div') as HTMLElement
    fireEvent.pointerDown(row2, { clientX: 10, clientY: 10 })
    fireEvent.pointerUp(row2, { clientX: 10, clientY: 10 })

    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })

  it('toggling a row\'s eye hides the part (renderer-only — no scene.set_hidden call)', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    const api = await renderAndOpenWithFixture()
    api.setHidden.mockClear()

    fireEvent.click(screen.getByRole('button', { name: /^hide object 1$/i }))

    await waitFor(() => expect(api.setHidden).toHaveBeenLastCalledWith([1n], []))
    // Shop Mode issues zero kernel transactions — this must be a pure
    // renderer-level hide, never scene.set_hidden (App.tsx's equivalent DOES
    // call it; ShopApp deliberately never does — module doc).
    expect(mockScene.set_hidden).not.toHaveBeenCalled()
  })

  it('long-press isolates a row, and "Show all" undoes ONLY the isolate — a sheet-driven hide survives it', async () => {
    fixture.objects = [1n, 2n, 3n]
    for (const id of [1n, 2n, 3n]) fixture.objectMesh[String(id)] = new Float32Array([0, 0, 0, 1, 1, 1])
    const api = await renderAndOpenWithFixture()

    // Sheet-driven hide: Object 3 hidden via its own eye toggle.
    fireEvent.click(screen.getByRole('button', { name: /^hide object 3$/i }))
    await waitFor(() => expect(api.setHidden).toHaveBeenLastCalledWith([3n], []))

    // Long-press Object 1 → isolate (hides everything outside its leaf set,
    // i.e. Object 2 — Object 3 stays hidden too, but that's the sheet's
    // doing, not isolate's, going into the assertion below).
    const row1 = screen.getByText('Object 1').closest('div') as HTMLElement
    fireEvent.pointerDown(row1, { clientX: 10, clientY: 10 })
    await new Promise((resolve) => setTimeout(resolve, LONG_PRESS_MS + 50))
    fireEvent.pointerUp(row1, { clientX: 10, clientY: 10 })
    await waitFor(() => {
      const [hiddenObjects] = api.setHidden.mock.calls[api.setHidden.mock.calls.length - 1]
      expect(new Set(hiddenObjects)).toEqual(new Set([2n, 3n]))
    })

    // "Show all" — Object 2 (hidden only by the isolate) comes back; Object 3
    // (hidden by the sheet toggle) stays hidden. This is the coherence the
    // design requires: the isolate-undo chip must never clobber sheet-driven
    // hides. Isolate/show-all both opt into the fade (design_handoff_shop_
    // mode/README.md §4) — a THIRD `{fadeMs}` argument the sheet's own eye
    // toggle above never passed.
    fireEvent.click(screen.getByRole('button', { name: /show all/i }))
    await waitFor(() => expect(api.setHidden).toHaveBeenLastCalledWith([3n], [], { fadeMs: 240 }))
  })
})

describe('ShopApp — isolate fade (design_handoff_shop_mode §4)', () => {
  it('long-press isolate passes fadeMs — the Parts sheet\'s own eye toggle never does', async () => {
    fixture.objects = [1n, 2n]
    for (const id of [1n, 2n]) fixture.objectMesh[String(id)] = new Float32Array([0, 0, 0, 1, 1, 1])
    const api = await renderAndOpenWithFixture()

    // Eye toggle: plain 2-argument call, unchanged.
    fireEvent.click(screen.getByRole('button', { name: /^hide object 2$/i }))
    await waitFor(() => expect(api.setHidden).toHaveBeenLastCalledWith([2n], []))
    expect(api.setHidden.mock.calls[api.setHidden.mock.calls.length - 1]).toHaveLength(2)

    // Long-press isolate: a third `{fadeMs}` argument.
    const row1 = screen.getByText('Object 1').closest('div') as HTMLElement
    fireEvent.pointerDown(row1, { clientX: 10, clientY: 10 })
    await new Promise((resolve) => setTimeout(resolve, LONG_PRESS_MS + 50))
    fireEvent.pointerUp(row1, { clientX: 10, clientY: 10 })
    await waitFor(() => {
      const lastCall = api.setHidden.mock.calls[api.setHidden.mock.calls.length - 1]
      expect(lastCall[2]).toEqual({ fadeMs: 240 })
    })
  })

  it('"Show all" also passes fadeMs (symmetric fade back in)', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    const api = await renderAndOpenWithFixture()

    const row1 = screen.getByText('Object 1').closest('div') as HTMLElement
    fireEvent.pointerDown(row1, { clientX: 10, clientY: 10 })
    await new Promise((resolve) => setTimeout(resolve, LONG_PRESS_MS + 50))
    fireEvent.pointerUp(row1, { clientX: 10, clientY: 10 })
    await waitFor(() => screen.getByRole('button', { name: /show all/i }))

    fireEvent.click(screen.getByRole('button', { name: /show all/i }))
    await waitFor(() => {
      const lastCall = api.setHidden.mock.calls[api.setHidden.mock.calls.length - 1]
      expect(lastCall[2]).toEqual({ fadeMs: 240 })
    })
  })
})

// A concurrent Viewport.tsx change (shop-mode playtest) drives Tape
// Measure's own HELD-press isolate from the LIVE press snap under that tool
// (`onIsolateRequest`), rather than the chrome-level long-press timer this
// file's other isolate tests exercise via `lastTapNodeRef` — that timer's
// own target is only ever set by the Select tool's `onSelect`, so it goes
// stale (still whatever was last tapped in Select mode) the instant Tape
// Measure becomes active. `handleWrapperPointerDownCapture` now disarms
// itself for that one tool so the two isolate paths never race.
describe('ShopApp — Tape Measure isolate wiring (task 1)', () => {
  it('onIsolateRequest isolates a whole-part node (object/group/instance)', async () => {
    fixture.objects = [1n, 2n]
    for (const id of [1n, 2n]) fixture.objectMesh[String(id)] = new Float32Array([0, 0, 0, 1, 1, 1])
    const api = await renderAndOpenWithFixture()

    act(() => {
      latestViewportProps().onIsolateRequest?.({ kind: 'object', id: 1n })
    })
    await waitFor(() => {
      const lastCall = api.setHidden.mock.calls[api.setHidden.mock.calls.length - 1]
      expect(new Set(lastCall[0])).toEqual(new Set([2n]))
      expect(lastCall[2]).toEqual({ fadeMs: 240 })
    })
  })

  it('onIsolateRequest ignores a non-whole-part node (e.g. a sketch)', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    const api = await renderAndOpenWithFixture()
    api.setHidden.mockClear()

    act(() => {
      latestViewportProps().onIsolateRequest?.({ kind: 'sketch', id: 9n })
    })
    // No isolate call landed — every setHidden call since the clear (if
    // any) is a plain 2-argument one, never isolate's own `{fadeMs}` shape.
    expect(api.setHidden.mock.calls.every((call) => call[2] === undefined)).toBe(true)
  })

  it('the wrapper long-press does NOT isolate while Tape Measure is active — its own target (lastTapNodeRef) would be stale', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    const api = await renderAndOpenWithFixture()

    // A Select-mode tap on Object 1 — the ONLY thing that ever sets
    // `lastTapNodeRef`, the wrapper timer's own isolate target.
    const container = screen.getByTestId('shop-viewport')
    fireEvent.pointerDown(container, { clientX: 50, clientY: 60 })
    const { onSelect, onSelectSnap } = latestViewportProps()
    onSelectSnap?.(null)
    onSelect?.({ kind: 'object', id: 1n }, false)
    fireEvent.pointerUp(container, { clientX: 50, clientY: 60 })
    await waitFor(() => expect(screen.getByText('Object 1')).toBeInTheDocument())
    api.setHidden.mockClear()

    // Switch to Tape Measure, then hold a press on the wrapper — the OLD
    // behavior would isolate the now-stale Object 1 from the Select tap
    // above; the fix disarms the wrapper's own long-press timer for Tape.
    fireEvent.click(screen.getByRole('button', { name: /^tape measure$/i }))
    fireEvent.pointerDown(container, { clientX: 50, clientY: 60 })
    await new Promise((resolve) => setTimeout(resolve, LONG_PRESS_MS + 50))
    fireEvent.pointerUp(container, { clientX: 50, clientY: 60 })

    expect(api.setHidden.mock.calls.every((call) => call[2] === undefined)).toBe(true)
  })

  it('the wrapper long-press still isolates in Select mode, unchanged', async () => {
    fixture.objects = [1n, 2n]
    for (const id of [1n, 2n]) fixture.objectMesh[String(id)] = new Float32Array([0, 0, 0, 1, 1, 1])
    const api = await renderAndOpenWithFixture()

    const container = screen.getByTestId('shop-viewport')
    fireEvent.pointerDown(container, { clientX: 50, clientY: 60 })
    const { onSelect, onSelectSnap } = latestViewportProps()
    onSelectSnap?.(null)
    onSelect?.({ kind: 'object', id: 1n }, false)
    fireEvent.pointerUp(container, { clientX: 50, clientY: 60 })
    await waitFor(() => expect(screen.getByText('Object 1')).toBeInTheDocument())
    api.setHidden.mockClear()

    fireEvent.pointerDown(container, { clientX: 50, clientY: 60 })
    await new Promise((resolve) => setTimeout(resolve, LONG_PRESS_MS + 50))
    fireEvent.pointerUp(container, { clientX: 50, clientY: 60 })

    await waitFor(() => {
      const lastCall = api.setHidden.mock.calls[api.setHidden.mock.calls.length - 1]
      expect(new Set(lastCall[0])).toEqual(new Set([2n]))
      expect(lastCall[2]).toEqual({ fadeMs: 240 })
    })
  })
})

describe('ShopApp — Parts sheet header (design_handoff_shop_mode §1)', () => {
  it('the "N of M shown" pill reflects hides, and the unit chip shows the current format', async () => {
    setLengthUnit('m')
    fixture.objects = [1n, 2n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    fixture.objectMesh['2'] = new Float32Array([0, 0, 0, 1, 1, 1])
    await renderAndOpenWithFixture()

    expect(screen.getByText('2 of 2 shown')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^units: meters$/i })).toHaveTextContent('m')

    fireEvent.click(screen.getByRole('button', { name: /^hide object 2$/i }))
    await waitFor(() => expect(screen.getByText('1 of 2 shown')).toBeInTheDocument())
  })

  it('opens the unit picker from the header chip, and picking a unit re-renders row dims live', async () => {
    setLengthUnit('m')
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1.5, 1.5, 1.5])
    await renderAndOpenWithFixture()

    // Metric mode: three separate mono columns, no per-row L/W/H letters.
    // All three dims are equal (1.5 m each), so all three columns render the
    // identical string.
    expect(screen.getAllByText('1.5 m').length).toBe(3)

    fireEvent.click(screen.getByRole('button', { name: /^units: meters$/i }))
    fireEvent.click(screen.getByRole('button', { name: /^decimal inches$/i }))

    // The picker closes and every row re-renders in the newly picked
    // format — no page reload, no re-fetch of the document.
    await waitFor(() => expect(screen.queryByRole('dialog', { name: /units/i })).not.toBeInTheDocument())
    expect(screen.getByRole('button', { name: /^units: decimal inches$/i })).toHaveTextContent('in.d')
    expect(screen.queryByText('1.5 m')).not.toBeInTheDocument()
    // 1.5m == 59.055...in, rounded to 3 decimals and trimmed by formatLengthIn
    // — an imperial format switches the row to the STACKED layout (design
    // §2), a single line with all three dims, not three separate columns.
    expect(screen.getByText((_, el) => el?.textContent === 'L 59.055" · W 59.055" · H 59.055"')).toBeInTheDocument()
  })

  it('a tag section renders a "Hide all"/"Show all" master toggle that hides every row under it', async () => {
    setLengthUnit('m')
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    fixture.nodeTags = { '1': ['Structure'] }
    await renderAndOpenWithFixture()

    expect(screen.getByText(/^Structure · 1$/)).toBeInTheDocument()
    const hideAll = screen.getByRole('button', { name: /^hide all$/i })
    fireEvent.click(hideAll)
    await waitFor(() => expect(screen.getByRole('button', { name: /^show all$/i })).toBeInTheDocument())
  })
})

// The maintainer-approved "Idea 2" split of the old combined overflow menu:
// document actions live behind the pill's own `DocumentMenu`, settings
// behind the ⋯ button's `SettingsMenu`. `DocumentMenu.test.tsx` covers the
// component in isolation (rows, header, empty-state degradation, corner
// anchoring); these integration tests only prove ShopApp wires its real
// state (docName gating, unit format, theme persistence) through correctly.
describe('ShopApp — document menu', () => {
  it('shows Open…/Use full editor with Save a copy gated on a loaded document', async () => {
    await renderAndLoad()
    fireEvent.click(screen.getByLabelText(/^document menu/i))
    expect(screen.getByRole('button', { name: /^open…$/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^use full editor$/i })).toBeInTheDocument()
    // Nothing loaded yet in the empty state — no "Save a copy" item.
    expect(screen.queryByRole('button', { name: /^save a copy/i })).not.toBeInTheDocument()
  })

  it('shows "Save a copy (.hew)" once a document is loaded', async () => {
    fixture.objects = [1n]
    await renderAndOpenWithFixture()
    fireEvent.click(screen.getByLabelText(/^document menu/i))
    expect(screen.getByRole('button', { name: /^save a copy \(\.hew\)$/i })).toBeInTheDocument()
  })

  it('closes on a scrim tap', async () => {
    await renderAndLoad()
    fireEvent.click(screen.getByLabelText(/^document menu/i))
    expect(screen.getByRole('button', { name: /^open…$/i })).toBeInTheDocument()
    fireEvent.click(screen.getByTestId('shop-document-scrim'))
    await waitFor(() => expect(screen.queryByRole('button', { name: /^open…$/i })).not.toBeInTheDocument())
  })
})

describe('ShopApp — settings menu', () => {
  it('Units row opens the SAME unit picker the sheet header chip does, and its current label tracks the live format', async () => {
    setLengthUnit('m')
    fixture.objects = [1n]
    await renderAndOpenWithFixture()

    fireEvent.click(screen.getByLabelText('Settings'))
    expect(screen.getByText('Meters')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^units$/i }))
    await waitFor(() => expect(screen.getByRole('dialog', { name: /units/i })).toBeInTheDocument())

    fireEvent.click(screen.getByRole('button', { name: /^centimeters$/i }))
    await waitFor(() => expect(screen.queryByRole('dialog', { name: /units/i })).not.toBeInTheDocument())

    fireEvent.click(screen.getByLabelText('Settings'))
    expect(screen.getByText('Centimeters')).toBeInTheDocument()
  })

  it('closes on a scrim tap', async () => {
    await renderAndLoad()
    fireEvent.click(screen.getByLabelText('Settings'))
    expect(screen.getByText('GESTURES')).toBeInTheDocument()
    fireEvent.click(screen.getByTestId('shop-settings-scrim'))
    await waitFor(() => expect(screen.queryByText('GESTURES')).not.toBeInTheDocument())
  })

  it('Theme segmented control persists via the SAME settings/theme.ts singleton the desktop Settings window uses — no shop-local store', async () => {
    await renderAndLoad()
    fireEvent.click(screen.getByLabelText('Settings'))

    expect(localStorage.getItem('hew.settings.theme')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: /^dark$/i }))
    expect(localStorage.getItem('hew.settings.theme')).toBe('dark')

    fireEvent.click(screen.getByRole('button', { name: /^light$/i }))
    expect(localStorage.getItem('hew.settings.theme')).toBe('light')
  })
})

describe('ShopApp — offline recents (empty state)', () => {
  it('lists recorded recents once the kernel boots, and stays quiet with none', async () => {
    vi.mocked(listRecents).mockResolvedValue([])
    await renderAndLoad()
    expect(screen.queryByText(/^recents$/i)).not.toBeInTheDocument()
  })

  it('shows a Recents row with name + relative time, and tapping it loads it directly (no picker)', async () => {
    const bytes = new Uint8Array([9, 9, 9])
    vi.mocked(listRecents).mockResolvedValue([
      { id: 'r1', name: 'Wall Clock.hew', timestamp: Date.now() - 60_000, bytes, source: 'open' },
    ])
    const host = { openAny: vi.fn() }
    vi.mocked(makeFileHost).mockReturnValue(host as never)

    await renderAndLoad()
    await waitFor(() => expect(screen.getByText('Wall Clock.hew')).toBeInTheDocument())
    expect(screen.getByText(/minute ago/i)).toBeInTheDocument()

    fireEvent.click(screen.getByText('Wall Clock.hew'))
    await waitFor(() => expect(screen.getByText('Wall Clock.hew')).toBeInTheDocument())
    // The document name in the top strip now reads the recent's name, and
    // the picker was never invoked.
    expect(mockScene.load).toHaveBeenCalledWith(bytes)
    expect(host.openAny).not.toHaveBeenCalled()
  })

  it('shows a Recents row\'s part count when the entry carries one, and omits the suffix when it doesn\'t', async () => {
    vi.mocked(listRecents).mockResolvedValue([
      { id: 'r1', name: 'With Count.hew', timestamp: Date.now() - 60_000, bytes: new Uint8Array([1]), source: 'open', partCount: 9 },
      { id: 'r2', name: 'No Count.hew', timestamp: Date.now() - 60_000, bytes: new Uint8Array([2]), source: 'open' },
    ])
    await renderAndLoad()
    await waitFor(() => expect(screen.getByText('With Count.hew')).toBeInTheDocument())
    expect(screen.getByText(/minute ago · 9 parts/i)).toBeInTheDocument()
    const noCountRow = screen.getByText('No Count.hew').closest('button') as HTMLElement
    expect(noCountRow.textContent).not.toMatch(/parts/)
  })

  it('records a successful picker open into recents (web build, no isTauri gate)', async () => {
    fixture.objects = [1n]
    await renderAndOpenWithFixture()
    // The 3 trailing args are recordRecent's own defaulted/optional ones —
    // 'open' (the default source), undefined (idb, defaulted to the real
    // indexedDB), and the part count computed from the SAME fixture at open
    // time (design §8's Recents "N parts" suffix — see applyOpenedBytes's
    // doc comment).
    expect(vi.mocked(recordRecent)).toHaveBeenCalledWith(new Uint8Array([1]), 'Doc.hew', 'open', undefined, 1)
  })

  it('computes the recorded part count from the actually-loaded document, not a placeholder', async () => {
    fixture.objects = [1n, 2n, 3n]
    for (const id of [1n, 2n, 3n]) fixture.objectMesh[String(id)] = new Float32Array([0, 0, 0, 1, 1, 1])
    await renderAndOpenWithFixture()
    expect(vi.mocked(recordRecent)).toHaveBeenCalledWith(new Uint8Array([1]), 'Doc.hew', 'open', undefined, 3)
  })

  // The empty state's ghost button opens ScanSheet.tsx's in-app QR scanner
  // (ScanSheet.test.tsx covers the scanner's own camera/decode state
  // machine in isolation) — this only proves ShopApp wires the button to
  // it and that "Cancel" closes it again. jsdom has no `mediaDevices` at
  // all, so the sheet settles into its 'unavailable' state — itself a
  // useful assertion that the no-camera copy renders rather than an
  // unhandled rejection.
  it('"From your desktop…" opens the in-app scanner sheet, and Cancel closes it', async () => {
    await renderAndLoad()

    expect(screen.queryByRole('dialog', { name: 'Scan from desktop' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^from your desktop…$/i }))

    const sheet = await screen.findByRole('dialog', { name: 'Scan from desktop' })
    expect(sheet).toBeInTheDocument()
    await waitFor(() => expect(sheet).toHaveTextContent(/no camera is available/i))

    fireEvent.click(screen.getByRole('button', { name: /^cancel$/i }))
    expect(screen.queryByRole('dialog', { name: 'Scan from desktop' })).not.toBeInTheDocument()
  })

  // The document menu's "Open from desktop…" row is the SECOND seam onto
  // the identical scanner sheet (ScanSheet.tsx's own doc comment) —
  // proving it opens the same sheet (and closes the menu doing so) is
  // this test's whole job; the sheet's own behavior is covered above/in
  // ScanSheet.test.tsx.
  it('document menu "Open from desktop…" opens the same scanner sheet and closes the menu', async () => {
    await renderAndLoad()

    fireEvent.click(screen.getByLabelText(/^document menu/i))
    expect(screen.getByText('Use full editor')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /^open from desktop…$/i }))

    expect(await screen.findByRole('dialog', { name: 'Scan from desktop' })).toBeInTheDocument()
    expect(screen.queryByText('Use full editor')).not.toBeInTheDocument()
  })
})

// Task 2's toolbar reshuffle dropped the dock/rail AR button entirely; Task
// 3 moved "View in AR…" into the document menu (`DocumentMenu.tsx`'s own
// module doc) — every test below reaches it through that menu instead.
describe('ShopApp — View in AR (iOS Quick Look, now in the document menu)', () => {
  it('is absent from the document menu with no document open, even on an AR-capable browser', async () => {
    vi.mocked(isArQuickLookCandidate).mockReturnValue(true)
    await renderAndLoad()
    fireEvent.click(screen.getByLabelText(/^document menu/i))
    expect(screen.queryByRole('button', { name: /view in ar/i })).not.toBeInTheDocument()
  })

  it('is absent once a document is open on a non-candidate browser', async () => {
    vi.mocked(isArQuickLookCandidate).mockReturnValue(false)
    fixture.objects = [1n]
    await renderAndOpenWithFixture()
    fireEvent.click(screen.getByLabelText(/^document menu/i))
    expect(screen.queryByRole('button', { name: /view in ar/i })).not.toBeInTheDocument()
  })

  it('renders in the document menu once a document is open on an AR-capable browser', async () => {
    vi.mocked(isArQuickLookCandidate).mockReturnValue(true)
    fixture.objects = [1n]
    await renderAndOpenWithFixture()
    fireEvent.click(screen.getByLabelText(/^document menu/i))
    expect(screen.getByRole('button', { name: /view in ar/i })).toBeInTheDocument()
  })

  it('tapping it closes the menu, exports USDZ through the viewport API, and hands the bytes to launchArQuickLook', async () => {
    vi.mocked(isArQuickLookCandidate).mockReturnValue(true)
    fixture.objects = [1n]
    const api = await renderAndOpenWithFixture()

    fireEvent.click(screen.getByLabelText(/^document menu/i))
    fireEvent.click(screen.getByRole('button', { name: /view in ar/i }))
    // Menu closed on the tap — same "act, then close" order every other row
    // (Open…/Save a copy/etc.) already uses.
    expect(screen.queryByText('Use full editor')).not.toBeInTheDocument()

    await waitFor(() => expect(vi.mocked(launchArQuickLook)).toHaveBeenCalled())
    expect(api.exportUsdz).toHaveBeenCalledTimes(1)
    expect(vi.mocked(launchArQuickLook)).toHaveBeenCalledWith(new Uint8Array([1, 2, 3]), 'Doc')
  })

  it('shows a toast instead of launching AR when exportUsdz has nothing to export', async () => {
    vi.mocked(isArQuickLookCandidate).mockReturnValue(true)
    fixture.objects = [1n]
    const api = await renderAndOpenWithFixture()
    vi.mocked(api.exportUsdz).mockResolvedValueOnce(null)

    fireEvent.click(screen.getByLabelText(/^document menu/i))
    fireEvent.click(screen.getByRole('button', { name: /view in ar/i }))
    await waitFor(() => expect(screen.getByText(/nothing to export/i)).toBeInTheDocument())
    expect(vi.mocked(launchArQuickLook)).not.toHaveBeenCalled()
  })

  // The row itself closes the menu on every tap (unlike the old dock
  // button, which stayed put and re-tappable while busy) — the double-tap
  // busy-guard is proven here by re-opening the now-closed menu mid-export
  // and confirming BOTH halves of Task 3's ask: the row relabels itself
  // "Preparing…", and tapping it anyway is a no-op (`viewInAr`'s own
  // `if (arBusy) return` guard, unchanged from before this task).
  it('busy-guards against a re-opened-menu tap while the export is in flight, relabeling the row "Preparing…"', async () => {
    vi.mocked(isArQuickLookCandidate).mockReturnValue(true)
    fixture.objects = [1n]
    const api = await renderAndOpenWithFixture()
    let resolveExport: (bytes: Uint8Array | null) => void = () => {}
    vi.mocked(api.exportUsdz).mockReturnValueOnce(new Promise((resolve) => { resolveExport = resolve }))

    fireEvent.click(screen.getByLabelText(/^document menu/i))
    fireEvent.click(screen.getByRole('button', { name: /view in ar/i })) // closes the menu, starts the export

    fireEvent.click(screen.getByLabelText(/^document menu/i)) // re-open while still pending
    const busyRow = screen.getByRole('button', { name: /^preparing…$/i })
    fireEvent.click(busyRow)

    resolveExport(new Uint8Array([9]))
    await waitFor(() => expect(vi.mocked(launchArQuickLook)).toHaveBeenCalledTimes(1))
    expect(api.exportUsdz).toHaveBeenCalledTimes(1)
  })
})

// ---------------------------------------------------------------------------
// Gesture-discoverability hints (design_handoff_shop_mode's "Gesture
// discoverability" section; the state machine is `hints.ts`, thoroughly
// covered by `hints.test.ts` — these two tests only prove the WIRING: the
// real `handleSelect`/open-document seams actually reach the engine and the
// engine's output actually reaches the DOM (as `pointer-events: none` text,
// never intercepting the tap itself). The 8s/3rd-inspect hints are the
// engine's own job to verify, not re-proven here against a live timer.
// ---------------------------------------------------------------------------

describe('ShopApp — gesture hints (Wave 5)', () => {
  it('shows hint (a) pointing at the largest visible part on first open, and a tap kills it + persists the flag', async () => {
    fixture.objects = [1n, 2n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1]) // small
    fixture.objectMesh['2'] = new Float32Array([0, 0, 0, 5, 5, 5]) // largest — the hint's target
    await renderAndOpenWithFixture()

    expect(screen.getByText('Tap a part for its size')).toBeInTheDocument()
    expect(localStorage.getItem(HINT_STORAGE_KEYS.tap)).toBe('1')

    const { onSelect, onSelectSnap } = latestViewportProps()
    onSelectSnap?.(null)
    onSelect?.({ kind: 'object', id: 2n }, false)

    await waitFor(() => expect(screen.queryByText('Tap a part for its size')).not.toBeInTheDocument())
    // Still set (it was already written the moment the hint fired at open —
    // hints.ts's own "fires once" doc comment) — the tap's job here is just
    // to prove it KILLS the on-screen hint, not the flag's origin.
    expect(localStorage.getItem(HINT_STORAGE_KEYS.tap)).toBe('1')
  })

  it('never renders while the document menu is open', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    await renderAndOpenWithFixture()
    expect(screen.getByText('Tap a part for its size')).toBeInTheDocument()

    fireEvent.click(screen.getByLabelText(/^document menu/i))
    expect(screen.queryByText('Tap a part for its size')).not.toBeInTheDocument()
  })

  it('never renders while the settings menu is open', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    await renderAndOpenWithFixture()
    expect(screen.getByText('Tap a part for its size')).toBeInTheDocument()

    fireEvent.click(screen.getByLabelText('Settings'))
    expect(screen.queryByText('Tap a part for its size')).not.toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// Landscape orientation (design_handoff_shop_mode/README.md §5) — Wave 4.
// Every test below stubs a landscape-shaped window (`stubWindowSize(844,
// 390)`) BEFORE its own render, since `useShopOrientation` reads the window
// size at mount (module doc's `stubWindowSize` comment).
// ---------------------------------------------------------------------------

/** Drags the Parts sheet's grab handle by `deltaY` px (negative = up =
 *  taller) — real `PointerEvent`s via `fireEvent`, exercising the SAME
 *  handler `PartsSheet.tsx`'s real touch drag runs through (its own
 *  `handlePointerDown` now guards `setPointerCapture` the same way the old
 *  side-sheet tab always did, specifically so this works in jsdom). jsdom
 *  has no real layout engine, so `containerRef`'s measured height is
 *  always 0 here — `sheetDetents.ts`'s `nearestDetent` still resolves any
 *  non-trivial drag to a definite non-'peek' detent against a 0px
 *  container (peek's fixed 64px is the only detent that differs from 0),
 *  which is all this helper needs to prove: SOME open detent, not which
 *  exact one. */
function dragSheetHandle(deltaY: number): void {
  const handle = screen.getByTestId('parts-sheet-handle')
  fireEvent.pointerDown(handle, { clientY: 400, pointerId: 1 })
  fireEvent.pointerMove(handle, { clientY: 400 + deltaY, pointerId: 1 })
  fireEvent.pointerUp(handle, { clientY: 400 + deltaY, pointerId: 1 })
}

/** The Parts sheet root's own inline `height` style — jsdom stores style
 *  props verbatim even though it never actually clips/reflows around them,
 *  so this is a real (non-vacuous) read of the current `detent`, unlike
 *  asserting row text presence (PartsSheet's rows sit in the DOM at EVERY
 *  detent now — `peek`'s 64px just clips them via `overflow:hidden` in a
 *  real browser, which jsdom doesn't model — module doc). Replaces the
 *  removed "Pull up"/"Pull down" link this file used to read detent off
 *  (item 6: the link itself is gone, the drag handle alone is the
 *  affordance now). */
function sheetHeightStyle(): string {
  return (screen.getByTestId('parts-sheet') as HTMLElement).style.height
}

describe('ShopApp — landscape orientation (design_handoff_shop_mode §5)', () => {
  it('boots into the right rail + a centered bottom sheet at peek height, not the old side sheet', async () => {
    stubWindowSize(844, 390)
    fixture.objects = [1n]
    await renderAndOpenWithFixture()

    // The right rail's tools share the SAME accessible names as the
    // portrait dock (module doc's "same aria names" requirement) even
    // though the rail renders them icon-only.
    expect(screen.getByRole('button', { name: 'Zoom Extents' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Select' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^orbit$/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^tape measure$/i })).toBeInTheDocument()

    // The bottom sheet itself — landscape's design correction (item 8): no
    // more left-edge tab/panel, the SAME sheet portrait uses, starting at
    // 'peek' just like portrait's own default.
    expect(screen.getByTestId('parts-sheet')).toBeInTheDocument()
    expect(sheetHeightStyle()).toBe('64px')
    // The old side sheet's tab is gone entirely — dead code, not just
    // hidden (item 8: "remove dead code, don't strand it").
    expect(screen.queryByTestId('shop-side-sheet-tab')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /cutlist/i })).not.toBeInTheDocument()

    // The "Pull up"/"Pull down" link is gone everywhere (item 6 — the drag
    // handle alone is the affordance now).
    expect(screen.queryByText(/pull (up|down)/i)).not.toBeInTheDocument()
  })

  it('a row eye toggle hides it in the landscape bottom sheet', async () => {
    stubWindowSize(844, 390)
    fixture.objects = [1n, 2n]
    await renderAndOpenWithFixture()

    // Rows sit in the DOM at every detent (module doc) — no need to open
    // the sheet first to reach one.
    expect(screen.getByText('Object 1')).toBeInTheDocument()
    expect(screen.getByText('Object 2')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Hide Object 2' }))
    expect(screen.getByRole('button', { name: 'Show Object 2' })).toBeInTheDocument()
  })

  it('the unit picker renders as a centered card, not a bottom sheet', async () => {
    setLengthUnit('m')
    stubWindowSize(844, 390)
    fixture.objects = [1n]
    await renderAndOpenWithFixture()

    fireEvent.click(screen.getByRole('button', { name: /^units: meters$/i }))
    const dialog = screen.getByRole('dialog', { name: 'Units' })
    expect(dialog).toBeInTheDocument()
    // The landscape "centered 360px card" variant (design §7) — width is
    // the one CSS property that's unambiguous proof this isn't the
    // portrait bottom sheet (which sets `left/right: 0` instead of a fixed
    // width).
    expect(dialog.style.width).toBe('360px')
  })

  it('rotating back to portrait restores the dock/bottom sheet and preserves the drag-opened detent + hides', async () => {
    stubWindowSize(844, 390)
    fixture.objects = [1n, 2n]
    await renderAndOpenWithFixture()

    // Drag the handle open — lands on a definite non-'peek' detent against
    // jsdom's always-0 container height (this helper's own doc comment).
    dragSheetHandle(-600)
    expect(sheetHeightStyle()).not.toBe('64px')

    fireEvent.click(screen.getByRole('button', { name: 'Hide Object 2' }))
    expect(screen.getByRole('button', { name: 'Show Object 2' })).toBeInTheDocument()

    // Rotate: the lifted `detent` state (`ShopApp.tsx`) must survive a
    // rotation, not silently reset to 'peek'.
    act(() => {
      stubWindowSize(390, 844)
      window.dispatchEvent(new Event('resize'))
    })

    // Portrait dock is back — the landscape rail's icon-only Zoom Extents
    // is gone, replaced by the portrait dock's own (there is exactly ONE
    // "Zoom Extents" button now, not a leftover rail + a new dock).
    expect(screen.getAllByRole('button', { name: 'Zoom Extents' })).toHaveLength(1)

    // The detent carried over — the sheet's own inline height style is
    // still the SAME non-'peek' value from before the rotation, not reset.
    expect(sheetHeightStyle()).not.toBe('64px')
    expect(screen.getByText('Object 1')).toBeInTheDocument()
    expect(screen.getByText('Object 2')).toBeInTheDocument()
    // The hide from landscape survived the rotation.
    expect(screen.getByRole('button', { name: 'Show Object 2' })).toBeInTheDocument()
  })
})

describe('ShopApp — link-arrived #recv= receive gate (adversarial-review findings 1 & 8)', () => {
  const TOKEN = 'a'.repeat(22)
  const KEY = 'k'.repeat(43)
  const ORIGINAL_HREF = window.location.href

  afterEach(() => {
    window.history.replaceState(null, '', ORIGINAL_HREF)
  })

  // Finding 8: the sensitive token+key must not linger in the address bar
  // for the whole span of `loadKernel()` — stripped at MOUNT, not gated on
  // `scene` becoming ready.
  it('strips the #recv= hash from the URL immediately at mount, independent of kernel readiness', async () => {
    // The kernel never resolves in this test — the hash strip must not be
    // waiting on it.
    vi.mocked(loadKernel).mockReturnValueOnce(new Promise(() => {}))
    window.history.replaceState(null, '', `/#recv=${TOKEN}.${KEY}.Bench`)

    render(<ShopApp />)

    await waitFor(() => expect(window.location.hash).toBe(''))
    // The kernel is still pending — no scene, no confirm gate, nothing —
    // proving the strip ran on its own, not as a side effect of the (still
    // pending) boot completing.
    expect(screen.queryByRole('dialog', { name: 'Open shared model?' })).not.toBeInTheDocument()
  })

  // Finding 1 (CRITICAL): a link-arrived handoff shows the confirmation
  // gate — naming the untrusted shared document — instead of loading
  // immediately, once the kernel is ready to act on it.
  it('shows the confirmation gate once the kernel is ready, naming the (untrusted) shared document', async () => {
    window.history.replaceState(null, '', `/#recv=${TOKEN}.${KEY}.Caf%C3%A9%20Table`)

    render(<ShopApp />)

    const dialog = await screen.findByRole('dialog', { name: 'Open shared model?' })
    expect(within(dialog).getByText(/Café Table/)).toBeInTheDocument()
  })

  it('Cancel dismisses the gate without ever fetching, and does not load a document', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch')
    window.history.replaceState(null, '', `/#recv=${TOKEN}.${KEY}.Bench`)

    render(<ShopApp />)
    const dialog = await screen.findByRole('dialog', { name: 'Open shared model?' })

    fireEvent.click(within(dialog).getByRole('button', { name: /^cancel$/i }))

    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Open shared model?' })).not.toBeInTheDocument())
    expect(fetchSpy).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: /^open a model…$/i })).toBeInTheDocument()
    fetchSpy.mockRestore()
  })

  it('Open fetches the relay, decrypts, and loads the document', async () => {
    const { encrypt, generateKey, toBase64Url } = await import('../io/shareCrypto')
    const key = generateKey()
    const ciphertext = await encrypt(key, new Uint8Array([1, 2, 3]))

    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(ciphertext.buffer as ArrayBuffer, { status: 200 }),
    )
    window.history.replaceState(null, '', `/#recv=${TOKEN}.${toBase64Url(key)}.Bench`)

    render(<ShopApp />)
    const dialog = await screen.findByRole('dialog', { name: 'Open shared model?' })
    fireEvent.click(within(dialog).getByRole('button', { name: /^open$/i }))

    await waitFor(() => expect(mockScene.load).toHaveBeenCalled())
    expect(fetchSpy).toHaveBeenCalledWith(expect.stringContaining(`/${TOKEN}`))
    await waitFor(() => expect(screen.getByText('Bench')).toBeInTheDocument())
    fetchSpy.mockRestore()
  })
})

// ---------------------------------------------------------------------------
// Playtest finding 8: tapping a dock action button used to leave the Select
// inspect chip up, visually "riding along" to whichever button was tapped.
// Every dock action button (tool segments, Zoom Extents, AR) now clears the
// selection and dismisses the inspect card instantly on the tap.
// ---------------------------------------------------------------------------

describe('ShopApp — dock actions clear a lingering inspect/selection (playtest finding 8)', () => {
  async function openInspectCard() {
    const container = screen.getByTestId('shop-viewport')
    fireEvent.pointerDown(container, { clientX: 50, clientY: 60 })
    const { onSelect, onSelectSnap } = latestViewportProps()
    onSelectSnap?.(null)
    onSelect?.({ kind: 'object', id: 1n }, false)
    await waitFor(() => expect(screen.getByRole('status')).toBeInTheDocument())
  }

  it('switching tools (Orbit) dismisses an active inspect card and clears the selection', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    await renderAndOpenWithFixture()
    await openInspectCard()
    expect(latestViewportProps().selectedIds).toEqual([{ kind: 'object', id: 1n }])

    fireEvent.click(screen.getByRole('button', { name: /^orbit$/i }))

    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    await waitFor(() => expect(latestViewportProps().selectedIds).toEqual([]))
  })

  it('tapping Zoom Extents dismisses an active inspect card and clears the selection', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    await renderAndOpenWithFixture()
    await openInspectCard()

    fireEvent.click(screen.getByRole('button', { name: 'Zoom Extents' }))

    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    await waitFor(() => expect(latestViewportProps().selectedIds).toEqual([]))
  })

  it('a targeted zoom (Parts-sheet row tap) does NOT clear the selection it just set', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    await renderAndOpenWithFixture()

    const row = screen.getByText('Object 1').closest('div') as HTMLElement
    fireEvent.pointerDown(row, { clientX: 10, clientY: 10 })
    fireEvent.pointerUp(row, { clientX: 10, clientY: 10 })

    await waitFor(() => expect(latestViewportProps().selectedIds).toEqual([{ kind: 'object', id: 1n }]))
  })

  // View in AR moved from a dock action button into the document menu (Task
  // 3) — reached through the menu now, but `viewInAr` still clears a
  // lingering inspect/selection on the tap (its own doc comment: harmless
  // and still correct even though the menu's scrim already blocks further
  // viewport taps while it's up).
  it('tapping View in AR (in the document menu) dismisses an active inspect card and clears the selection', async () => {
    vi.mocked(isArQuickLookCandidate).mockReturnValue(true)
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    await renderAndOpenWithFixture()
    await openInspectCard()

    fireEvent.click(screen.getByLabelText(/^document menu/i))
    fireEvent.click(screen.getByRole('button', { name: /view in ar/i }))

    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    await waitFor(() => expect(latestViewportProps().selectedIds).toEqual([]))
  })
})

// ---------------------------------------------------------------------------
// Playtest finding 4: a real model thumbnail (captured via
// `ViewportApi.captureFrame`) replaces the striped placeholder swatch in the
// empty-state Recents list once one's been recorded for that entry.
// ---------------------------------------------------------------------------

describe('ShopApp — recents thumbnails (playtest finding 4)', () => {
  // jsdom has no real canvas 2D/encoding backend (ScanSheet.test.tsx's own
  // `stubCanvasContext` doc comment covers the same gap for its QR-decode
  // canvas) — `buildRecentThumbnail`'s two-canvas-pass pipeline needs
  // `getContext('2d')` (for `putImageData`/`drawImage`), `toDataURL` (jsdom
  // logs "Not implemented" and returns `undefined` otherwise), AND the
  // global `ImageData` constructor itself (jsdom doesn't define it AT ALL —
  // `typeof ImageData === 'undefined'`, unlike the other two which exist but
  // no-op) stubbed to exercise the pipeline's actual success path.
  const originalImageData = globalThis.ImageData
  function stubCanvas(dataUrl: string): void {
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
      putImageData: vi.fn(),
      drawImage: vi.fn(),
      // Task 4a's vertical-flip transform, bracketing the "cover"-crop
      // `drawImage` call above — the dedicated "flips the captured frame"
      // test below asserts these are actually called/ordered; every other
      // test here just needs them to exist so `buildRecentThumbnail`
      // doesn't throw on an un-mocked method.
      save: vi.fn(),
      translate: vi.fn(),
      scale: vi.fn(),
      restore: vi.fn(),
    } as unknown as CanvasRenderingContext2D)
    vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockReturnValue(dataUrl)
    // @ts-expect-error — a minimal stand-in, just enough for
    // `buildRecentThumbnail`'s constructor call to succeed; nothing reads
    // its pixel data back out (the stubbed `putImageData` above ignores its
    // argument entirely).
    globalThis.ImageData = class {
      constructor(public data: Uint8ClampedArray, public width: number, public height?: number) {}
    }
  }
  afterEach(() => {
    globalThis.ImageData = originalImageData
  })

  it('captures a frame and records it as a thumbnail after opening a document', async () => {
    stubCanvas('data:image/jpeg;base64,FAKE')
    // A non-empty fixture — the framing branch that schedules the capture
    // (applyOpenedBytes) only runs for a non-empty scene; an empty one
    // frames nothing and so captures no thumbnail either (this module's own
    // doc comment on that gate).
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    const host = { openAny: vi.fn(async () => ({ kind: 'hew' as const, name: 'Doc.hew', bytes: new Uint8Array([1]), handle: null })) }
    vi.mocked(makeFileHost).mockReturnValue(host as never)
    await renderAndLoad()
    const api = makeViewportApiStub()
    api.captureFrame.mockReturnValue({ width: 4, height: 4, pixels: new Uint8Array(4 * 4 * 4).fill(200) })
    const { apiRef } = latestViewportProps()
    if (apiRef !== undefined) apiRef.current = api

    fireEvent.click(screen.getByRole('button', { name: /^open a model…$/i }))
    await waitFor(() => expect(screen.getByText('Doc.hew')).toBeInTheDocument())

    await waitFor(() => expect(api.captureFrame).toHaveBeenCalled())
    await waitFor(() => {
      const thumbnailCall = vi.mocked(recordRecent).mock.calls.find((call) => call[5] !== undefined)
      expect(thumbnailCall?.[5]).toBe('data:image/jpeg;base64,FAKE')
    })
    vi.restoreAllMocks()
  })

  it('leaves the thumbnail unset (falls back to the placeholder) when the capture pipeline fails', async () => {
    // No canvas stub in this test — jsdom's real "not implemented" behavior
    // (`getContext` returns null) exercises `buildRecentThumbnail`'s own
    // null-guard, and `captureRecentThumbnail`'s try/catch around the whole
    // thing either way — best-effort, never breaks the open.
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    const host = { openAny: vi.fn(async () => ({ kind: 'hew' as const, name: 'Doc.hew', bytes: new Uint8Array([1]), handle: null })) }
    vi.mocked(makeFileHost).mockReturnValue(host as never)
    await renderAndLoad()
    const api = makeViewportApiStub()
    api.captureFrame.mockReturnValue({ width: 4, height: 4, pixels: new Uint8Array(4 * 4 * 4).fill(200) })
    const { apiRef } = latestViewportProps()
    if (apiRef !== undefined) apiRef.current = api

    fireEvent.click(screen.getByRole('button', { name: /^open a model…$/i }))
    await waitFor(() => expect(screen.getByText('Doc.hew')).toBeInTheDocument())
    await waitFor(() => expect(api.captureFrame).toHaveBeenCalled())

    // No SECOND recordRecent call (the thumbnail one) ever lands.
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(vi.mocked(recordRecent).mock.calls.every((call) => call[5] === undefined)).toBe(true)
  })

  // Task 4a: `captureFrame`'s pixels are in WebGL `readPixels` row order
  // (bottom-to-top), but the old code drew them straight onto the thumb
  // canvas — vertically flipped relative to what was actually on screen.
  // `buildRecentThumbnail`'s own doc comment has the fix's algebra; this
  // proves the WIRING — the thumb context's flip transform actually runs,
  // bracketing the "cover"-crop draw — since jsdom's stub can't render real
  // pixels for a visual assertion.
  it('flips the captured frame vertically before drawing the thumbnail crop', async () => {
    const ctxSpies = {
      putImageData: vi.fn(),
      drawImage: vi.fn(),
      save: vi.fn(),
      translate: vi.fn(),
      scale: vi.fn(),
      restore: vi.fn(),
    }
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(ctxSpies as unknown as CanvasRenderingContext2D)
    vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockReturnValue('data:image/jpeg;base64,FLIP')
    // @ts-expect-error — same minimal stand-in as `stubCanvas` above.
    globalThis.ImageData = class {
      constructor(public data: Uint8ClampedArray, public width: number, public height?: number) {}
    }

    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    const host = { openAny: vi.fn(async () => ({ kind: 'hew' as const, name: 'Doc.hew', bytes: new Uint8Array([1]), handle: null })) }
    vi.mocked(makeFileHost).mockReturnValue(host as never)
    await renderAndLoad()
    const api = makeViewportApiStub()
    api.captureFrame.mockReturnValue({ width: 4, height: 4, pixels: new Uint8Array(4 * 4 * 4).fill(200) })
    const { apiRef } = latestViewportProps()
    if (apiRef !== undefined) apiRef.current = api

    fireEvent.click(screen.getByRole('button', { name: /^open a model…$/i }))
    await waitFor(() => expect(screen.getByText('Doc.hew')).toBeInTheDocument())
    await waitFor(() => expect(ctxSpies.drawImage).toHaveBeenCalled())

    // save → translate(0, size) → scale(1, -1) → drawImage → restore, in
    // that order — mirroring the thumb canvas around its own horizontal
    // centerline keeps the existing "cover"-crop math's centering intact
    // (the doc comment's own algebra) while correcting the row order.
    expect(ctxSpies.translate).toHaveBeenCalledWith(0, RECENT_THUMB_SIZE_PX)
    expect(ctxSpies.scale).toHaveBeenCalledWith(1, -1)
    const order = [ctxSpies.save, ctxSpies.translate, ctxSpies.scale, ctxSpies.drawImage, ctxSpies.restore]
      .map((spy) => spy.mock.invocationCallOrder[0])
    expect(order).toEqual([...order].sort((a, b) => a - b))

    vi.restoreAllMocks()
  })

  // Task 4b: a document WITH a saved camera used to capture the thumbnail
  // at that (often zoomed-OUT) saved pose. `zoomExtents()` re-poses the
  // camera synchronously (no tween), so the fix fits the camera JUST for
  // this capture, then restores the saved view as the LAST paint — proven
  // here by the call ORDER: zoomExtents, then captureFrame, then
  // applyCameraState(the saved state).
  it('fits the camera (zoomExtents) before capturing a saved-camera document\'s thumbnail, then restores the saved view', async () => {
    stubCanvas('data:image/jpeg;base64,FITTED')
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    const host = { openAny: vi.fn(async () => ({ kind: 'hew' as const, name: 'Doc.hew', bytes: new Uint8Array([1]), handle: null })) }
    vi.mocked(makeFileHost).mockReturnValue(host as never)
    mockScene.camera_state.mockReturnValueOnce(makeFakeCameraState())
    await renderAndLoad()
    const api = makeViewportApiStub()
    api.captureFrame.mockReturnValue({ width: 4, height: 4, pixels: new Uint8Array(4 * 4 * 4).fill(200) })
    const { apiRef } = latestViewportProps()
    if (apiRef !== undefined) apiRef.current = api

    fireEvent.click(screen.getByRole('button', { name: /^open a model…$/i }))
    await waitFor(() => expect(screen.getByText('Doc.hew')).toBeInTheDocument())
    await waitFor(() => expect(api.applyCameraState).toHaveBeenCalled())

    expect(api.zoomExtents).toHaveBeenCalledTimes(1)
    expect(api.captureFrame).toHaveBeenCalledTimes(1)
    expect(api.applyCameraState).toHaveBeenCalledTimes(1)
    const zoomOrder = api.zoomExtents.mock.invocationCallOrder[0]
    const captureOrder = api.captureFrame.mock.invocationCallOrder[0]
    const applyOrder = api.applyCameraState.mock.invocationCallOrder[0]
    expect(zoomOrder).toBeLessThan(captureOrder)
    expect(captureOrder).toBeLessThan(applyOrder)

    vi.restoreAllMocks()
  })

  it('renders a real thumbnail image for a recent that has one, and the placeholder swatch for one that doesn\'t', async () => {
    vi.mocked(listRecents).mockResolvedValue([
      { id: 'r1', name: 'With Thumb.hew', timestamp: Date.now() - 60_000, bytes: new Uint8Array([1]), source: 'open', thumbnail: 'data:image/jpeg;base64,AAA' },
      { id: 'r2', name: 'No Thumb.hew', timestamp: Date.now() - 60_000, bytes: new Uint8Array([2]), source: 'open' },
    ])
    await renderAndLoad()
    await waitFor(() => expect(screen.getByText('With Thumb.hew')).toBeInTheDocument())

    const withThumbRow = screen.getByText('With Thumb.hew').closest('button') as HTMLElement
    const img = within(withThumbRow).getByTestId('shop-recent-thumb') as HTMLImageElement
    expect(img.src).toBe('data:image/jpeg;base64,AAA')

    const noThumbRow = screen.getByText('No Thumb.hew').closest('button') as HTMLElement
    expect(within(noThumbRow).queryByTestId('shop-recent-thumb')).not.toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// Playtest finding 12: a dedicated "Views" dock button opens a sheet with
// the seven standard views plus a Parallel⇄Perspective toggle, wired to the
// already-existing ViewportApi.setStandardView/toggleProjection.
// ---------------------------------------------------------------------------

describe('ShopApp — Views sheet (playtest finding 12)', () => {
  it('the Views button opens a sheet listing the seven standard views', async () => {
    fixture.objects = [1n]
    await renderAndOpenWithFixture()

    fireEvent.click(screen.getByRole('button', { name: /^views$/i }))
    expect(screen.getByRole('dialog', { name: 'Views' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'ISO' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Top' })).toBeInTheDocument()
  })

  it('picking a standard view calls ViewportApi.setStandardView and closes the sheet', async () => {
    fixture.objects = [1n]
    const api = await renderAndOpenWithFixture()

    fireEvent.click(screen.getByRole('button', { name: /^views$/i }))
    fireEvent.click(screen.getByRole('button', { name: 'Front' }))

    expect(api.setStandardView).toHaveBeenCalledWith('front')
    expect(screen.queryByRole('dialog', { name: 'Views' })).not.toBeInTheDocument()
  })

  it('toggling projection calls ViewportApi.toggleProjection without closing the sheet', async () => {
    fixture.objects = [1n]
    const api = await renderAndOpenWithFixture()

    fireEvent.click(screen.getByRole('button', { name: /^views$/i }))
    fireEvent.click(screen.getByRole('button', { name: 'Parallel' }))

    expect(api.toggleProjection).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('dialog', { name: 'Views' })).toBeInTheDocument()
  })

  it('reflects the live projection reported via onProjectionChange', async () => {
    fixture.objects = [1n]
    await renderAndOpenWithFixture()

    act(() => { latestViewportProps().onProjectionChange?.('parallel') })
    fireEvent.click(screen.getByRole('button', { name: /^views$/i }))

    expect(screen.getByRole('button', { name: 'Parallel' })).toHaveAttribute('aria-pressed', 'true')
  })

  it('opening Views dismisses an active inspect card and clears the selection (dock action, playtest finding 8)', async () => {
    fixture.objects = [1n]
    fixture.objectMesh['1'] = new Float32Array([0, 0, 0, 1, 1, 1])
    await renderAndOpenWithFixture()

    const container = screen.getByTestId('shop-viewport')
    fireEvent.pointerDown(container, { clientX: 50, clientY: 60 })
    const { onSelect, onSelectSnap } = latestViewportProps()
    onSelectSnap?.(null)
    onSelect?.({ kind: 'object', id: 1n }, false)
    await waitFor(() => expect(screen.getByRole('status')).toBeInTheDocument())

    fireEvent.click(screen.getByRole('button', { name: /^views$/i }))

    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    await waitFor(() => expect(latestViewportProps().selectedIds).toEqual([]))
  })

  it('the Views button is reachable in landscape too', async () => {
    stubWindowSize(844, 390)
    fixture.objects = [1n]
    await renderAndOpenWithFixture()

    fireEvent.click(screen.getByRole('button', { name: /^views$/i }))
    expect(screen.getByRole('dialog', { name: 'Views' })).toBeInTheDocument()
  })
})

// docs/design/scenes.md §6 / SPEC.md §2 — Shop Mode's read-only Scenes
// contract: activation goes through the PURE `resolve_scene` (never
// `apply_scene`, `set_hidden`/`set_tag_hidden`/`set_node_user_hidden`, or
// `set_section_plane` — `mockScene` above deliberately never defines
// `apply_scene`/`set_section_plane`/`set_tag_hidden`/`set_node_user_hidden`
// at all, so a stray call to any of them fails the test with a bare
// TypeError rather than needing an explicit "was not called" assertion).
describe('ShopApp — Scenes (docs/design/scenes.md §6)', () => {
  const SCENE_1_JSON = { sid: 1, name: 'Cut layout', description: 'Cut layout on a sheet', props: 31 }
  const SCENE_2_JSON = { sid: 2, name: 'Tenon section', description: '', props: 31 }
  const SCENE_1_CAMERA = { projection: 'perspective' as const, fovDeg: 40, eye: [5, 5, 5] as [number, number, number], target: [0, 0, 0] as [number, number, number], up: [0, 0, 1] as [number, number, number] }
  const SCENE_1_SECTION = { origin: [0, 0, 0] as [number, number, number], normal: [0, 1, 0] as [number, number, number], active: true }

  function resolvedForScene1() {
    return makeFakeResolvedScene({
      cameraJson: JSON.stringify(SCENE_1_CAMERA),
      hasHiddenNodes: true,
      hiddenNodeKinds: [0],
      hiddenNodeIds: [2n],
      hiddenObjectIds: [2n],
      hiddenInstanceIds: [],
      hasSection: true,
      sectionJson: JSON.stringify(SCENE_1_SECTION),
    })
  }

  it('the pill is absent until a Scene is activated', async () => {
    mockScene.scenes_json.mockReturnValue(JSON.stringify([SCENE_1_JSON]))
    fixture.objects = [1n, 2n]
    await renderAndOpenWithFixture()

    expect(screen.queryByText('Cut layout')).not.toBeInTheDocument()
  })

  it('the Views sheet lists Scenes above the standard views', async () => {
    mockScene.scenes_json.mockReturnValue(JSON.stringify([SCENE_1_JSON, SCENE_2_JSON]))
    fixture.objects = [1n, 2n]
    await renderAndOpenWithFixture()

    fireEvent.click(screen.getByRole('button', { name: /^views$/i }))
    expect(screen.getByText('Cut layout')).toBeInTheDocument()
    expect(screen.getByText('Tenon section')).toBeInTheDocument()
    expect(screen.getByText('Cut layout on a sheet')).toBeInTheDocument()
  })

  it('activating a Scene from the Views sheet resolves it (never apply_scene), pushes hidden with fade, applies the section plane, and tweens the camera', async () => {
    mockScene.scenes_json.mockReturnValue(JSON.stringify([SCENE_1_JSON]))
    mockScene.resolve_scene.mockImplementation(() => resolvedForScene1())
    fixture.objects = [1n, 2n]
    const api = await renderAndOpenWithFixture()
    api.setHidden.mockClear()

    fireEvent.click(screen.getByRole('button', { name: /^views$/i }))
    fireEvent.click(screen.getByText('Cut layout'))

    expect(mockScene.resolve_scene).toHaveBeenCalledWith(1n)
    expect(api.setHidden).toHaveBeenLastCalledWith([2n], [], { fadeMs: 240 })
    expect(api.setSectionPlane).toHaveBeenCalledWith(SCENE_1_SECTION)
    expect(api.tweenCameraState).toHaveBeenCalledWith(
      SCENE_1_CAMERA,
      getSceneTransitions() ? SCENE_TRANSITION_MS : 0,
      expect.any(Function),
    )
    // Shop Mode issues zero kernel transactions — the resolved handle's own
    // renderer-level ids are pushed via `ViewportApi.setHidden`, never a
    // kernel `scene.set_hidden` call (App.tsx's editor equivalent DOES call
    // it; ShopApp deliberately never does — module doc, and the existing
    // "renderer-only hide" assertions elsewhere in this file).
    expect(mockScene.set_hidden).not.toHaveBeenCalled()
    // The sheet act-and-closes, same as a standard-view row.
    expect(screen.queryByRole('dialog', { name: 'Views' })).not.toBeInTheDocument()
  })

  it('the pill shows the active Scene\'s name, and tapping it opens the Views sheet', async () => {
    mockScene.scenes_json.mockReturnValue(JSON.stringify([SCENE_1_JSON]))
    mockScene.resolve_scene.mockImplementation(() => resolvedForScene1())
    fixture.objects = [1n, 2n]
    await renderAndOpenWithFixture()

    fireEvent.click(screen.getByRole('button', { name: /^views$/i }))
    fireEvent.click(screen.getByText('Cut layout'))

    expect(screen.getByText('Cut layout')).toBeInTheDocument()
    fireEvent.click(screen.getByText('Cut layout'))
    expect(screen.getByRole('dialog', { name: 'Views' })).toBeInTheDocument()
  })

  it('the chevrons cycle Scenes with wrap', async () => {
    mockScene.scenes_json.mockReturnValue(JSON.stringify([SCENE_1_JSON, SCENE_2_JSON]))
    mockScene.resolve_scene.mockImplementation((sid: bigint) =>
      sid === 1n ? resolvedForScene1() : makeFakeResolvedScene(),
    )
    fixture.objects = [1n, 2n]
    await renderAndOpenWithFixture()

    // The document opens on its FIRST Scene (playtest round 1, matching the
    // desktop and SketchUp): the pill is already up, naming "Cut layout".
    const pill = () => screen.getByTestId('scene-pill')
    await waitFor(() => expect(pill()).toHaveTextContent('Cut layout'))

    fireEvent.click(screen.getByRole('button', { name: 'Next Scene' }))
    await waitFor(() => expect(pill()).toHaveTextContent('Tenon section'))

    // Wraps back to the first Scene.
    fireEvent.click(screen.getByRole('button', { name: 'Next Scene' }))
    await waitFor(() => expect(pill()).toHaveTextContent('Cut layout'))

    fireEvent.click(screen.getByRole('button', { name: 'Previous Scene' }))
    await waitFor(() => expect(pill()).toHaveTextContent('Tenon section'))
  })

  it('"Show all" while a Scene is active returns to the Scene\'s hidden set, not everything-visible', async () => {
    mockScene.scenes_json.mockReturnValue(JSON.stringify([SCENE_1_JSON]))
    mockScene.resolve_scene.mockImplementation(() => resolvedForScene1())
    fixture.objects = [1n, 2n, 3n]
    for (const id of [1n, 2n, 3n]) fixture.objectMesh[String(id)] = new Float32Array([0, 0, 0, 1, 1, 1])
    const api = await renderAndOpenWithFixture()

    // Opens on the first Scene (auto-activated on open); re-activate it
    // explicitly from the Views sheet all the same — the row is the SCENES
    // section's, distinct from the pill's own name label.
    await waitFor(() => expect(screen.getByTestId('scene-pill')).toHaveTextContent('Cut layout'))
    fireEvent.click(screen.getByRole('button', { name: /^views$/i }))
    const sheet = screen.getByRole('dialog', { name: 'Views' })
    fireEvent.click(within(sheet).getByText('Cut layout'))
    await waitFor(() => expect(api.setHidden).toHaveBeenLastCalledWith([2n], [], { fadeMs: 240 }))

    // Long-press isolate Object 1 — hides Object 3 too (outside its leaf
    // set), on TOP of the Scene's own Object 2.
    const row1 = screen.getByText('Object 1').closest('div') as HTMLElement
    fireEvent.pointerDown(row1, { clientX: 10, clientY: 10 })
    await new Promise((resolve) => setTimeout(resolve, LONG_PRESS_MS + 50))
    fireEvent.pointerUp(row1, { clientX: 10, clientY: 10 })
    await waitFor(() => {
      const [hiddenObjects] = api.setHidden.mock.calls[api.setHidden.mock.calls.length - 1]
      expect(new Set(hiddenObjects)).toEqual(new Set([2n, 3n]))
    })

    fireEvent.click(screen.getByRole('button', { name: /show all/i }))
    await waitFor(() => {
      const [hiddenObjects, hiddenInstances, opts] = api.setHidden.mock.calls[api.setHidden.mock.calls.length - 1]
      expect(new Set(hiddenObjects)).toEqual(new Set([2n]))
      expect(hiddenInstances).toEqual([])
      expect(opts).toEqual({ fadeMs: 240 })
    })
  })

  it('never calls apply_scene, set_hidden, set_tag_hidden, set_node_user_hidden, or set_section_plane — read-only contract', async () => {
    mockScene.scenes_json.mockReturnValue(JSON.stringify([SCENE_1_JSON]))
    mockScene.resolve_scene.mockImplementation(() => resolvedForScene1())
    fixture.objects = [1n, 2n]
    await renderAndOpenWithFixture()

    expect((mockScene as Record<string, unknown>).apply_scene).toBeUndefined()
    expect((mockScene as Record<string, unknown>).set_tag_hidden).toBeUndefined()
    expect((mockScene as Record<string, unknown>).set_node_user_hidden).toBeUndefined()
    expect((mockScene as Record<string, unknown>).set_section_plane).toBeUndefined()

    fireEvent.click(screen.getByRole('button', { name: /^views$/i }))
    fireEvent.click(screen.getByText('Cut layout'))

    expect(mockScene.set_hidden).not.toHaveBeenCalled()
  })
})
