/**
 * `useScenesController` against a fake wasm handle and a fake viewport —
 * the same "plain object stands in for `Scene`" convention as
 * `panels/scenePanels.test.tsx` (the hook imports `Scene` as a type only).
 * Covers the activation choreography (design §5), Add/Update dirtying, the
 * rename error mapping, drift refresh, and Next/Previous wrap.
 */
import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createRef } from 'react'
import type { Scene } from '../wasm/loader'
import type { ViewportApi } from '../viewport/Viewport'
import { PROP_ALL, PROP_CAMERA, SCENE_COPY } from './scenesModel'
import { useScenesController } from './useScenesController'

const CAM_JSON = JSON.stringify({ projection: 'perspective', fovDeg: 45, eye: [4, -6, 3], target: [0, 0, 0], up: [0, 0, 1] })

/* eslint-disable @typescript-eslint/no-explicit-any */
function makeResolved(overrides: Record<string, any> = {}) {
  return {
    camera_json: vi.fn().mockReturnValue(CAM_JSON),
    display_json: vi.fn().mockReturnValue(JSON.stringify({ grid: false, axes: true, guides: true })),
    has_hidden: vi.fn().mockReturnValue(true),
    hidden_object_ids: vi.fn().mockReturnValue(new BigUint64Array([7n, 8n])),
    hidden_instance_ids: vi.fn().mockReturnValue(new BigUint64Array([])),
    has_hidden_tags: vi.fn().mockReturnValue(true),
    hidden_tag_paths: vi.fn().mockReturnValue(['Hardware/Screws']),
    has_hidden_nodes: vi.fn().mockReturnValue(true),
    hidden_node_kinds: vi.fn().mockReturnValue(new Uint8Array([1])),
    hidden_node_ids: vi.fn().mockReturnValue(new BigUint64Array([5n])),
    has_section: vi.fn().mockReturnValue(true),
    section_json: vi.fn().mockReturnValue(JSON.stringify({ origin: [0, 0, 1], normal: [0, 0, 1], active: true })),
    free: vi.fn(),
    ...overrides,
  }
}

function makeScene(entries: any[] = [], overrides: Record<string, any> = {}): Scene {
  let list = entries
  return {
    scenes_json: vi.fn(() => JSON.stringify(list)),
    add_scene: vi.fn(() => {
      const sid = BigInt(100 + list.length)
      list = [...list, { sid: Number(sid), name: `Scene ${list.length + 1}`, props: PROP_ALL }]
      return sid
    }),
    update_scene: vi.fn(),
    set_scene_props: vi.fn(),
    rename_scene: vi.fn(),
    set_scene_description: vi.fn(),
    move_scene: vi.fn(),
    remove_scene: vi.fn(() => {
      list = list.slice(1)
    }),
    apply_scene: vi.fn(() => makeResolved()),
    scene_drift: vi.fn().mockReturnValue('{"camera":false,"hiddenNodes":false,"hiddenTags":false,"section":false,"display":false,"staleRefs":0}'),
    set_hidden: vi.fn(),
    ...overrides,
  } as unknown as Scene
}

function makeApi(): ViewportApi {
  return {
    getCameraState: vi.fn().mockReturnValue({ projection: 'perspective', fovDeg: 45, eye: [1, 1, 1], target: [0, 0, 0], up: [0, 0, 1] }),
    setHidden: vi.fn(),
    setSectionPlane: vi.fn(),
    tweenCameraState: vi.fn((_s: unknown, _ms: number, done?: (c: boolean) => void) => done?.(true)),
    captureFrame: vi.fn().mockReturnValue({ width: 0, height: 0, pixels: new Uint8Array() }),
  } as unknown as ViewportApi
}
/* eslint-enable @typescript-eslint/no-explicit-any */

function setup(scene: Scene, api: ViewportApi = makeApi()) {
  const viewportApi = createRef<ViewportApi | null>() as React.MutableRefObject<ViewportApi | null>
  viewportApi.current = api
  const deps = {
    scene,
    viewportApi,
    docRev: 0,
    display: { grid: true, axes: true, guides: true },
    setDisplay: vi.fn(),
    setHiddenState: vi.fn(),
    exitAllContexts: vi.fn(),
    markDirty: vi.fn(),
    onToast: vi.fn(),
  }
  const hook = renderHook((props: typeof deps) => useScenesController(props), { initialProps: deps })
  return { hook, deps, api }
}

beforeEach(() => {
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    cb(0)
    return 1
  })
})

describe('useScenesController', () => {
  it('lists the kernel entries and starts with nothing active', () => {
    const { hook } = setup(makeScene([{ sid: 1, name: 'A', props: PROP_ALL }]))
    expect(hook.result.current.entries.map((e) => e.name)).toEqual(['A'])
    expect(hook.result.current.activeSid).toBeNull()
    expect(hook.result.current.drift).toBeNull()
  })

  it('activate: exits contexts, applies in the kernel, pushes hidden state, section, display, camera', () => {
    const scene = makeScene([{ sid: 1, name: 'A', props: PROP_ALL }])
    const { hook, deps, api } = setup(scene)
    act(() => hook.result.current.activate(1))
    expect(deps.exitAllContexts).toHaveBeenCalledTimes(1)
    expect(scene.apply_scene).toHaveBeenCalledWith(1n)
    // Panel state from the kernel's resolution — no app-side union walk.
    expect(deps.setHiddenState).toHaveBeenCalledWith({
      hiddenTagPaths: new Set([JSON.stringify(['Hardware', 'Screws'])]),
      hiddenKeys: new Set(['group:5']),
    })
    // Leaf ids straight to renderer + kernel inference.
    expect(api.setHidden).toHaveBeenCalledWith([7n, 8n], [])
    expect(scene.set_hidden).toHaveBeenCalled()
    expect(deps.setDisplay).toHaveBeenCalledWith({ grid: false, axes: true, guides: true })
    expect(api.setSectionPlane).toHaveBeenCalledWith({ origin: [0, 0, 1], normal: [0, 0, 1], active: true })
    expect(api.tweenCameraState).toHaveBeenCalled()
    expect((api.tweenCameraState as ReturnType<typeof vi.fn>).mock.calls[0][1]).toBe(600)
    expect(hook.result.current.activeSid).toBe(1)
    // Activation is not a dirtying change.
    expect(deps.markDirty).not.toHaveBeenCalled()
  })

  it('activate: an uncaptured property is left alone', () => {
    const resolved = makeResolved({
      display_json: vi.fn().mockReturnValue(undefined),
      has_section: vi.fn().mockReturnValue(false),
      has_hidden: vi.fn().mockReturnValue(false),
      has_hidden_tags: vi.fn().mockReturnValue(false),
      has_hidden_nodes: vi.fn().mockReturnValue(false),
    })
    const scene = makeScene([{ sid: 1, name: 'A', props: PROP_CAMERA }], { apply_scene: vi.fn(() => resolved) })
    const { hook, deps, api } = setup(scene)
    act(() => hook.result.current.activate(1))
    expect(deps.setDisplay).not.toHaveBeenCalled()
    expect(deps.setHiddenState).not.toHaveBeenCalled()
    expect(api.setHidden).not.toHaveBeenCalled()
    expect(api.setSectionPlane).not.toHaveBeenCalled()
    expect(api.tweenCameraState).toHaveBeenCalled()
    expect(resolved.free).toHaveBeenCalled()
  })

  it('add: captures all five with the live camera/display, dirties, activates, thumbnails', () => {
    const scene = makeScene([])
    const { hook, deps } = setup(scene)
    let sid: number | null = null
    act(() => {
      sid = hook.result.current.add()
    })
    expect(sid).toBe(100)
    expect(scene.add_scene).toHaveBeenCalledWith(undefined, PROP_ALL, expect.stringContaining('"eye":[1,1,1]'), JSON.stringify({ grid: true, axes: true, guides: true }), undefined)
    expect(deps.markDirty).toHaveBeenCalledTimes(1)
    expect(hook.result.current.activeSid).toBe(100)
    expect(hook.result.current.entries).toHaveLength(1)
    // A second add inserts after the active one.
    act(() => {
      hook.result.current.add()
    })
    expect((scene.add_scene as ReturnType<typeof vi.fn>).mock.calls[1][4]).toBe(100n)
  })

  it('update re-captures the entry props and dirties; rename maps the duplicate error', () => {
    const scene = makeScene([{ sid: 1, name: 'A', props: PROP_CAMERA }], {
      rename_scene: vi.fn(() => {
        throw new Error('DuplicateSceneName: a scene with that name already exists')
      }),
    })
    const { hook, deps } = setup(scene)
    act(() => hook.result.current.update(1))
    expect(scene.update_scene).toHaveBeenCalledWith(1n, PROP_CAMERA, expect.any(String), expect.any(String))
    expect(deps.markDirty).toHaveBeenCalledTimes(1)
    let err: string | null = null
    act(() => {
      err = hook.result.current.rename(1, 'B')
    })
    expect(err).toBe(SCENE_COPY.duplicateName('B'))
    expect(deps.markDirty).toHaveBeenCalledTimes(1)
  })

  it('remove clears the active Scene; next/previous wrap through the list', () => {
    const scene = makeScene([
      { sid: 1, name: 'A', props: PROP_ALL },
      { sid: 2, name: 'B', props: PROP_ALL },
    ])
    const { hook } = setup(scene)
    act(() => hook.result.current.activate(2))
    act(() => hook.result.current.next())
    expect(hook.result.current.activeSid).toBe(1)
    act(() => hook.result.current.previous())
    expect(hook.result.current.activeSid).toBe(2)
    act(() => hook.result.current.remove(2))
    expect(scene.remove_scene).toHaveBeenCalledWith(2n)
    // The fake drops the FIRST entry; the controller re-reads and notices
    // the active sid vanished either way once the list no longer holds it.
    act(() => hook.result.current.remove(1))
    expect(hook.result.current.activeSid).toBeNull()
  })

  it('resetForDocument forgets the active Scene, drift, and thumbnails (the wasm handle is reused across Open/New)', () => {
    const scene = makeScene([{ sid: 1, name: 'A', props: PROP_ALL }])
    const { hook } = setup(scene)
    act(() => hook.result.current.activate(1))
    act(() => hook.result.current.mergeThumbnails(new Map([[1, 'data:image/jpeg;base64,x']])))
    expect(hook.result.current.activeSid).toBe(1)
    expect(hook.result.current.thumbnails.size).toBe(1)
    act(() => hook.result.current.resetForDocument())
    expect(hook.result.current.activeSid).toBeNull()
    expect(hook.result.current.drift).toBeNull()
    expect(hook.result.current.thumbnails.size).toBe(0)
  })

  it('activateFirstOnLoad activates the first Scene instantly (no tween) and reports whether one existed', () => {
    const scene = makeScene([{ sid: 7, name: 'First', props: PROP_ALL }, { sid: 8, name: 'Second', props: PROP_ALL }])
    const { hook, api } = setup(scene)
    let did = false
    act(() => {
      did = hook.result.current.activateFirstOnLoad()
    })
    expect(did).toBe(true)
    expect(scene.apply_scene).toHaveBeenCalledWith(7n)
    expect(hook.result.current.activeSid).toBe(7)
    expect((api.tweenCameraState as ReturnType<typeof vi.fn>).mock.calls[0][1]).toBe(0)
    const empty = setup(makeScene([]))
    let didEmpty = true
    act(() => {
      didEmpty = empty.hook.result.current.activateFirstOnLoad()
    })
    expect(didEmpty).toBe(false)
  })

  it('refreshDrift asks the kernel with the live camera and display', () => {
    const scene = makeScene([{ sid: 1, name: 'A', props: PROP_ALL }], {
      scene_drift: vi.fn().mockReturnValue('{"camera":true,"staleRefs":1}'),
    })
    const { hook } = setup(scene)
    act(() => hook.result.current.activate(1))
    act(() => hook.result.current.refreshDrift())
    expect(scene.scene_drift).toHaveBeenCalledWith(1n, expect.stringContaining('"projection"'), JSON.stringify({ grid: true, axes: true, guides: true }))
    expect(hook.result.current.drift?.camera).toBe(true)
    expect(hook.result.current.drift?.staleRefs).toBe(1)
    expect(hook.result.current.activeDrifted).toBe(true)
  })
})
