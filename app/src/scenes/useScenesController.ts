/**
 * Scenes controller (docs/design/scenes.md §5) — the one place the editor
 * talks to the kernel about Scenes. Owns the entry list, the active Scene,
 * its drift, and the derived thumbnails; performs Add / Update / Activate /
 * Rename / Describe / Props / Reorder / Delete / Next / Previous against
 * the wasm handle and the viewport. The tray section (`ScenesPanel`), the
 * View ▸ Scenes menu, the palette, and Page Up/Down all drive THIS object;
 * none of them touch the kernel directly.
 *
 * Activation (design §5 "Activate"): exit any edit context, `apply_scene`
 * in the kernel, take its returned panel state (hidden tag paths, hidden
 * node refs) into App's `hiddenTagPaths`/`hiddenKeys` and its returned leaf
 * ids straight into `ViewportApi.setHidden` + `scene.set_hidden` — never
 * re-running the app's own union walk — then display toggles, section
 * clip, and finally the camera through `tweenCameraState`.
 *
 * Every editing method marks the document dirty on success (Scenes are
 * persisted; activation is not a dirtying change).
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { RefObject } from 'react'
import type { Scene } from '../wasm/loader'
import type { ViewportApi } from '../viewport/Viewport'
import { friendlyErrorText } from '../kernelErrors'
import { nodeKey } from '../panels/treeModel'
import type { NodeRef } from '../panels/treeModel'
import { tagPathKey } from '../panels/tagModel'
import { buildFrameThumbnail } from '../viewport/frameThumbnail'
import { getSceneTransitions, SCENE_TRANSITION_MS, subscribe as subscribeTransitions } from '../settings/sceneTransitions'
import {
  PROP_ALL,
  SCENE_COPY,
  driftAny,
  neighborScene,
  parseCameraJson,
  parseDisplayJson,
  parseDriftJson,
  parseScenesJson,
  parseSectionJson,
  sceneIndex,
} from './scenesModel'
import type { CameraStateJson, DisplayStateJson, SceneDrift, SceneEntry } from './scenesModel'

/** Thumbnail pixel box: 4× the row's 24×17 CSS box (SPEC.md §1) for crisp 2×. */
export const SCENE_THUMB_WIDTH_PX = 96
export const SCENE_THUMB_HEIGHT_PX = 68
const SCENE_THUMB_QUALITY = 0.72

export interface ScenesControllerDeps {
  /** The live wasm document handle (null before the kernel is up). */
  scene: Scene | null
  viewportApi: RefObject<ViewportApi | null>
  /** Bumps on every document mutation — refreshes the entry list + drift. */
  docRev: number
  /** The editor's live display toggles (App's showGrid/showAxes/showGuides). */
  display: DisplayStateJson
  /** Apply a Scene's display toggles (App pushes them into the viewport). */
  setDisplay: (d: DisplayStateJson) => void
  /** Replace App's hidden-state React sets after an activation (panel eyes). */
  setHiddenState: (next: { hiddenKeys?: Set<string>; hiddenTagPaths?: Set<string> }) => void
  /** Exit every open group/component edit context (root "Model" crumb). */
  exitAllContexts: () => void
  /** Mark the document dirty (Scene edits are persisted state). */
  markDirty: () => void
  onToast: (message: string) => void
}

export interface ScenesController {
  entries: SceneEntry[]
  activeSid: number | null
  /** Drift of the active Scene, or null when none is active. */
  drift: SceneDrift | null
  /** True when the active Scene has drifted (or has stale references). */
  activeDrifted: boolean
  /** Derived JPEG data URLs by sid (missing = placeholder). */
  thumbnails: ReadonlyMap<number, string>
  /** Add Scene: capture all five properties, auto-name, insert after the
   * active Scene, activate, return the new sid (null on failure). */
  add: () => number | null
  /** Re-capture the Scene's checked properties; refresh its thumbnail. */
  update: (sid: number) => void
  /** Activate a Scene. `instant` skips the camera tween (document load
   * activates the first Scene without animating into it). */
  activate: (sid: number, opts?: { instant?: boolean }) => void
  /** Document load (design §5, playtest): activate the first Scene, if any,
   * instantly — SketchUp opens on its first Scene, and a document with
   * Scenes always shows one as active. Returns whether one was activated. */
  activateFirstOnLoad: () => boolean
  next: () => void
  previous: () => void
  /** Returns the inline error text, or null on success. */
  rename: (sid: number, name: string) => string | null
  setDescription: (sid: number, text: string) => void
  /** Set the capture bitmask (`PROP_*`). */
  setProps: (sid: number, props: number) => void
  moveUp: (sid: number) => void
  moveDown: (sid: number) => void
  remove: (sid: number) => void
  refreshThumbnail: (sid: number) => void
  /** Recompute drift now (App calls this from camera-settled / section-changed). */
  refreshDrift: () => void
  /** Restore thumbnails (e.g. from a persistence layer) — merged by sid. */
  mergeThumbnails: (thumbs: ReadonlyMap<number, string>) => void
  /** A different document now occupies the (reused, mutated-in-place) wasm
   * handle — File ▸ New/Open/Recover: forget the active Scene, its drift,
   * and every thumbnail. App.tsx calls this from its load path; the
   * `[scene]`-keyed reset only covers a brand-new handle. */
  resetForDocument: () => void
}

function cameraJson(api: ViewportApi | null): string | undefined {
  const c = api?.getCameraState()
  return c === undefined ? undefined : JSON.stringify(c)
}

function displayJson(d: DisplayStateJson): string {
  return JSON.stringify(d)
}

/** `Scene.rename_scene`'s kernel refusal → the SPEC's inline copy. */
function renameErrorText(err: unknown, name: string): string {
  const raw = err instanceof Error ? err.message : String(err)
  if (raw.startsWith('DuplicateSceneName')) return SCENE_COPY.duplicateName(name)
  return friendlyErrorText(err)
}

export function useScenesController(deps: ScenesControllerDeps): ScenesController {
  const { scene, viewportApi, docRev, display, setDisplay, setHiddenState, exitAllContexts, markDirty, onToast } =
    deps
  const [rev, setRev] = useState(0)
  const [activeSid, setActiveSid] = useState<number | null>(null)
  const [drift, setDrift] = useState<SceneDrift | null>(null)
  const [thumbnails, setThumbnails] = useState<Map<number, string>>(() => new Map())
  const [transitions, setTransitions] = useState<boolean>(() => getSceneTransitions())
  useEffect(() => subscribeTransitions(setTransitions), [])

  // Latest deps through refs, so the stable callbacks below never go stale.
  const sceneRef = useRef(scene)
  sceneRef.current = scene
  const displayRef = useRef(display)
  displayRef.current = display
  const setDisplayRef = useRef(setDisplay)
  setDisplayRef.current = setDisplay
  const setHiddenStateRef = useRef(setHiddenState)
  setHiddenStateRef.current = setHiddenState
  const exitAllContextsRef = useRef(exitAllContexts)
  exitAllContextsRef.current = exitAllContexts
  const markDirtyRef = useRef(markDirty)
  markDirtyRef.current = markDirty
  const onToastRef = useRef(onToast)
  onToastRef.current = onToast
  const activeSidRef = useRef(activeSid)
  activeSidRef.current = activeSid
  const transitionsRef = useRef(transitions)
  transitionsRef.current = transitions

  // A new document handle = a new document: nothing active, no thumbnails.
  useEffect(() => {
    setActiveSid(null)
    setDrift(null)
    setThumbnails(new Map())
  }, [scene])

  const entries = useMemo<SceneEntry[]>(() => {
    if (scene === null) return []
    try {
      return parseScenesJson(scene.scenes_json())
    } catch {
      return []
    }
    // rev: bumped after every controller op; docRev: kernel mutations.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scene, docRev, rev])
  const entriesRef = useRef(entries)
  entriesRef.current = entries

  // The active Scene vanished (deleted, or the document changed under us).
  useEffect(() => {
    if (activeSid !== null && sceneIndex(entries, activeSid) < 0) {
      setActiveSid(null)
      setDrift(null)
    }
  }, [entries, activeSid])

  const refreshDrift = useCallback(() => {
    const s = sceneRef.current
    const sid = activeSidRef.current
    if (s === null || sid === null) {
      setDrift(null)
      return
    }
    try {
      const json = s.scene_drift(BigInt(sid), cameraJson(viewportApi.current), displayJson(displayRef.current))
      setDrift(parseDriftJson(json))
    } catch {
      setDrift(null)
    }
  }, [viewportApi])

  // Drift keys off document mutations and display toggles here; the camera
  // and section paths call `refreshDrift` from App's viewport callbacks.
  useEffect(() => {
    refreshDrift()
  }, [refreshDrift, docRev, rev, activeSid, display.grid, display.axes, display.guides])

  const captureThumbnail = useCallback(
    (sid: number) => {
      // Next frame: the caller has just re-posed the camera / changed
      // visibility, and `captureFrame` renders what is current when it runs.
      requestAnimationFrame(() => {
        const api = viewportApi.current
        if (api === null) return
        const frame = api.captureFrame()
        const url = buildFrameThumbnail(frame, SCENE_THUMB_WIDTH_PX, SCENE_THUMB_HEIGHT_PX, SCENE_THUMB_QUALITY)
        if (url === null) return
        setThumbnails((prev) => {
          const next = new Map(prev)
          next.set(sid, url)
          return next
        })
      })
    },
    [viewportApi],
  )

  const bump = useCallback(() => setRev((r) => r + 1), [])

  const applyResolved = useCallback(
    (sid: number, instant: boolean, resolved: {
      camera_json(): string | undefined
      display_json(): string | undefined
      has_hidden(): boolean
      hidden_object_ids(): BigUint64Array
      hidden_instance_ids(): BigUint64Array
      has_hidden_tags(): boolean
      hidden_tag_paths(): string[]
      has_hidden_nodes(): boolean
      hidden_node_kinds(): Uint8Array
      hidden_node_ids(): BigUint64Array
      has_section(): boolean
      section_json(): string | undefined
      free(): void
    }) => {
      const s = sceneRef.current
      const api = viewportApi.current
      try {
        // Panel state first (cheap), then the renderer/kernel leaf sets.
        const hiddenState: { hiddenKeys?: Set<string>; hiddenTagPaths?: Set<string> } = {}
        if (resolved.has_hidden_tags()) {
          const paths = resolved.hidden_tag_paths()
          hiddenState.hiddenTagPaths = new Set(
            paths.map((p) => tagPathKey(p.split('/').map((seg) => seg.trim()).filter((seg) => seg.length > 0))),
          )
        }
        if (resolved.has_hidden_nodes()) {
          const kinds = resolved.hidden_node_kinds()
          const ids = resolved.hidden_node_ids()
          const kindNames: NodeRef['kind'][] = ['object', 'group', 'instance']
          const keys = new Set<string>()
          for (let i = 0; i < kinds.length; i++) {
            const kind = kindNames[kinds[i]]
            if (kind !== undefined) keys.add(nodeKey({ kind, id: ids[i] }))
          }
          hiddenState.hiddenKeys = keys
        }
        if (hiddenState.hiddenKeys !== undefined || hiddenState.hiddenTagPaths !== undefined) {
          setHiddenStateRef.current(hiddenState)
        }
        if (resolved.has_hidden()) {
          const objectIds = Array.from(resolved.hidden_object_ids())
          const instanceIds = Array.from(resolved.hidden_instance_ids())
          api?.setHidden(objectIds, instanceIds)
          s?.set_hidden(new BigUint64Array(objectIds), new BigUint64Array(instanceIds))
        }
        const dj = resolved.display_json()
        if (dj !== undefined) {
          const d = parseDisplayJson(JSON.parse(dj))
          if (d !== undefined) setDisplayRef.current(d)
        }
        if (resolved.has_section()) {
          const sj = resolved.section_json()
          const plane = sj === undefined ? null : (parseSectionJson(JSON.parse(sj)) ?? null)
          api?.setSectionPlane(plane)
        }
        const cj = resolved.camera_json()
        if (cj !== undefined && api !== null) {
          const cam = parseCameraJson(JSON.parse(cj))
          if (cam !== undefined) {
            api.tweenCameraState(cam, transitionsRef.current && !instant ? SCENE_TRANSITION_MS : 0, () => refreshDrift())
          }
        }
      } finally {
        resolved.free()
      }
      setActiveSid(sid)
      activeSidRef.current = sid
      bump()
    },
    [viewportApi, refreshDrift, bump],
  )

  const activate = useCallback(
    (sid: number, opts?: { instant?: boolean }) => {
      const s = sceneRef.current
      if (s === null) return
      exitAllContextsRef.current()
      let resolved
      try {
        resolved = s.apply_scene(BigInt(sid))
      } catch (err) {
        onToastRef.current(`Activate Scene failed: ${friendlyErrorText(err)}`)
        return
      }
      applyResolved(sid, opts?.instant === true, resolved)
    },
    [applyResolved],
  )

  const activateFirstOnLoad = useCallback((): boolean => {
    const s = sceneRef.current
    if (s === null) return false
    let first: SceneEntry | undefined
    try {
      first = parseScenesJson(s.scenes_json())[0]
    } catch {
      first = undefined
    }
    if (first === undefined) return false
    activate(first.sid, { instant: true })
    return true
  }, [activate])

  const add = useCallback((): number | null => {
    const s = sceneRef.current
    if (s === null) return null
    const api = viewportApi.current
    let sid: bigint
    try {
      sid = s.add_scene(
        undefined,
        PROP_ALL,
        cameraJson(api),
        displayJson(displayRef.current),
        activeSidRef.current === null ? undefined : BigInt(activeSidRef.current),
      )
    } catch (err) {
      onToastRef.current(`Add Scene failed: ${friendlyErrorText(err)}`)
      return null
    }
    const n = Number(sid)
    markDirtyRef.current()
    setActiveSid(n)
    activeSidRef.current = n
    bump()
    captureThumbnail(n)
    return n
  }, [viewportApi, bump, captureThumbnail])

  const update = useCallback(
    (sid: number) => {
      const s = sceneRef.current
      if (s === null) return
      const entry = entriesRef.current.find((e) => e.sid === sid)
      if (entry === undefined) return
      try {
        s.update_scene(BigInt(sid), entry.props, cameraJson(viewportApi.current), displayJson(displayRef.current))
      } catch (err) {
        onToastRef.current(`Update Scene failed: ${friendlyErrorText(err)}`)
        return
      }
      markDirtyRef.current()
      bump()
      captureThumbnail(sid)
      // Updating the active Scene clears its drift by definition.
      if (activeSidRef.current === sid) refreshDrift()
    },
    [viewportApi, bump, captureThumbnail, refreshDrift],
  )

  const rename = useCallback(
    (sid: number, name: string): string | null => {
      const s = sceneRef.current
      if (s === null) return 'No document.'
      try {
        s.rename_scene(BigInt(sid), name)
      } catch (err) {
        return renameErrorText(err, name)
      }
      markDirtyRef.current()
      bump()
      return null
    },
    [bump],
  )

  const setDescription = useCallback(
    (sid: number, text: string) => {
      const s = sceneRef.current
      if (s === null) return
      const entry = entriesRef.current.find((e) => e.sid === sid)
      if (entry === undefined || entry.description === text) return
      try {
        s.set_scene_description(BigInt(sid), text)
      } catch (err) {
        onToastRef.current(`Describe Scene failed: ${friendlyErrorText(err)}`)
        return
      }
      markDirtyRef.current()
      bump()
    },
    [bump],
  )

  const setProps = useCallback(
    (sid: number, props: number) => {
      const s = sceneRef.current
      if (s === null) return
      try {
        s.set_scene_props(BigInt(sid), props, cameraJson(viewportApi.current), displayJson(displayRef.current))
      } catch (err) {
        onToastRef.current(`Scene properties failed: ${friendlyErrorText(err)}`)
        return
      }
      markDirtyRef.current()
      bump()
    },
    [viewportApi, bump],
  )

  const move = useCallback(
    (sid: number, delta: -1 | 1) => {
      const s = sceneRef.current
      if (s === null) return
      const i = sceneIndex(entriesRef.current, sid)
      if (i < 0) return
      const to = i + delta
      if (to < 0 || to >= entriesRef.current.length) return
      try {
        s.move_scene(BigInt(sid), to)
      } catch (err) {
        onToastRef.current(`Move Scene failed: ${friendlyErrorText(err)}`)
        return
      }
      markDirtyRef.current()
      bump()
    },
    [bump],
  )

  const remove = useCallback(
    (sid: number) => {
      const s = sceneRef.current
      if (s === null) return
      try {
        s.remove_scene(BigInt(sid))
      } catch (err) {
        onToastRef.current(`Delete Scene failed: ${friendlyErrorText(err)}`)
        return
      }
      markDirtyRef.current()
      setThumbnails((prev) => {
        if (!prev.has(sid)) return prev
        const next = new Map(prev)
        next.delete(sid)
        return next
      })
      if (activeSidRef.current === sid) {
        setActiveSid(null)
        activeSidRef.current = null
      }
      bump()
    },
    [bump],
  )

  const step = useCallback(
    (dir: 1 | -1) => {
      const target = neighborScene(entriesRef.current, activeSidRef.current, dir)
      if (target !== null) activate(target.sid)
    },
    [activate],
  )

  const resetForDocument = useCallback(() => {
    setActiveSid(null)
    activeSidRef.current = null
    setDrift(null)
    setThumbnails(new Map())
    bump()
  }, [bump])

  const mergeThumbnails = useCallback((thumbs: ReadonlyMap<number, string>) => {
    setThumbnails((prev) => {
      const next = new Map(prev)
      for (const [sid, url] of thumbs) next.set(sid, url)
      return next
    })
  }, [])

  return useMemo<ScenesController>(
    () => ({
      entries,
      activeSid,
      drift,
      activeDrifted: driftAny(drift),
      thumbnails,
      add,
      update,
      activate,
      activateFirstOnLoad,
      next: () => step(1),
      previous: () => step(-1),
      rename,
      setDescription,
      setProps,
      moveUp: (sid) => move(sid, -1),
      moveDown: (sid) => move(sid, 1),
      remove,
      refreshThumbnail: captureThumbnail,
      refreshDrift,
      mergeThumbnails,
      resetForDocument,
    }),
    [
      entries,
      activeSid,
      drift,
      thumbnails,
      add,
      update,
      activate,
      activateFirstOnLoad,
      step,
      rename,
      setDescription,
      setProps,
      move,
      remove,
      captureThumbnail,
      refreshDrift,
      mergeThumbnails,
      resetForDocument,
    ],
  )
}
