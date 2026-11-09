/**
 * App — Place Image: File ▸ Import Image… and a picture dropped on the
 * viewport both arm the placement gesture (`ViewportApi.armImagePlacement`)
 * with the picture's bytes, name and pixel size; neither replaces the
 * document, and a picture that can't be placed is refused with a toast
 * before anything is armed.
 *
 * Its own file (an isolated module graph) so its Viewport and FileHost
 * stubs stay out of App.test.tsx's.
 */

import { render, screen, fireEvent, createEvent, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'

const menubar = () => within(screen.getByTestId('menu-bar'))

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

vi.mock('./wasm/kernelMemory', () => ({
  kernelMemoryBytes: vi.fn((): number | null => null),
  setKernelMemory: vi.fn(),
}))

// The Viewport renders a marker inside the drop target and hands App a
// fake API whose armImagePlacement the tests observe.
const armImagePlacement = vi.fn()
vi.mock('./viewport/Viewport', () => ({
  default: vi.fn((props: { apiRef?: { current: unknown } }) => {
    if (props.apiRef !== undefined) {
      props.apiRef.current = {
        armImagePlacement,
        setFramingInset: vi.fn(),
        refreshScene: vi.fn(),
        notifyLoaded: vi.fn(),
        zoomExtents: vi.fn(),
      }
    }
    return <div data-testid="viewport-stub" />
  }),
}))

const openImageMock = vi.fn()
vi.mock('./io/fileHost', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./io/fileHost')>()
  return {
    ...actual,
    makeFileHost: () => ({
      open: vi.fn(),
      save: vi.fn(),
      saveAs: vi.fn(),
      openForImport: vi.fn(),
      openAny: vi.fn(),
      openImage: openImageMock,
      exportBinary: vi.fn(),
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
      writeSource: async () => {},
    }),
  }
})

vi.mock('./io/recents', () => ({
  listRecents: vi.fn(async () => []),
  recordRecent: vi.fn(async () => undefined),
}))

import App from './App'
import { setShowWelcome } from './settings/welcomeScreen'
import { setTrayLayout, DEFAULT_TRAY_LAYOUT } from './settings/trayLayout'
import { FileReadError } from './io/fileHost'

/** A PNG header `w` x `h` pixels (all Place Image reads). */
function png(w: number, h: number): Uint8Array<ArrayBuffer> {
  const be32 = (v: number) => [(v >>> 24) & 255, (v >>> 16) & 255, (v >>> 8) & 255, v & 255]
  return new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, ...be32(w), ...be32(h), 8, 6, 0, 0, 0])
}

beforeEach(() => {
  vi.clearAllMocks()
  setTrayLayout(DEFAULT_TRAY_LAYOUT)
  setShowWelcome(false)
})

async function renderAndLoad() {
  render(<App />)
  await waitFor(() => screen.getByTitle('Rectangle (R)'), { timeout: 2000 })
}

function dropFile(file: File, at = { clientX: 120, clientY: 80 }) {
  const target = screen.getByTestId('viewport-stub').parentElement as HTMLElement
  // jsdom has no DragEvent, so the pointer position is set by hand.
  const ev = createEvent.drop(target, { dataTransfer: { files: [file], items: [] } })
  Object.defineProperty(ev, 'clientX', { value: at.clientX })
  Object.defineProperty(ev, 'clientY', { value: at.clientY })
  fireEvent(target, ev)
}

describe('App — Place Image', () => {
  it('File ▸ Import Image… arms the placement with the picked picture, named after its file', async () => {
    await renderAndLoad()
    openImageMock.mockResolvedValueOnce({ name: 'Kitchen wall.png', bytes: png(400, 300) })
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('Import Image…'))
    await waitFor(() => expect(armImagePlacement).toHaveBeenCalledTimes(1))
    const [placement, at] = armImagePlacement.mock.calls[0]
    expect(placement).toMatchObject({ format: 0, name: 'Kitchen wall', pixelWidth: 400, pixelHeight: 300 })
    expect(at).toBeUndefined()
    expect(mockScene.load).not.toHaveBeenCalled()
  })

  it('a picture dropped on the viewport arms the placement at the drop point; it does not place by itself', async () => {
    await renderAndLoad()
    dropFile(new File([png(200, 100)], 'plan.PNG', { type: 'image/png' }))
    await waitFor(() => expect(armImagePlacement).toHaveBeenCalledTimes(1))
    const [placement, at] = armImagePlacement.mock.calls[0]
    expect(placement).toMatchObject({ name: 'plan', pixelWidth: 200, pixelHeight: 100 })
    expect(at).toEqual({ clientX: 120, clientY: 80 })
    expect(mockScene.load).not.toHaveBeenCalled()
  })

  it('a picture that cannot be read is refused with a toast, and nothing is armed', async () => {
    await renderAndLoad()
    dropFile(new File([new Uint8Array([1, 2, 3])], 'broken.jpg', { type: 'image/jpeg' }))
    expect(await screen.findByText(/can't read that picture/i)).toBeInTheDocument()
    expect(armImagePlacement).not.toHaveBeenCalled()
  })

  it('a picked picture that cannot be read says so instead of doing nothing', async () => {
    await renderAndLoad()
    openImageMock.mockRejectedValueOnce(new FileReadError('wall.png', new Error('NotReadableError')))
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('Import Image…'))
    expect(await screen.findByText(/Couldn't read that picture\./)).toBeInTheDocument()
    expect(armImagePlacement).not.toHaveBeenCalled()
  })

  it('a picture over 64 megapixels is refused with a toast saying what to do', async () => {
    await renderAndLoad()
    openImageMock.mockResolvedValueOnce({ name: 'huge.png', bytes: png(10000, 9000) })
    fireEvent.click(screen.getByRole('button', { name: /^file$/i }))
    fireEvent.mouseDown(menubar().getByText('Import Image…'))
    expect(await screen.findByText(/Save a smaller copy/i)).toBeInTheDocument()
    expect(armImagePlacement).not.toHaveBeenCalled()
  })
})
