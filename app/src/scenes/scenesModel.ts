/**
 * Scenes — the pure model layer (docs/design/scenes.md §3.3, §5).
 *
 * Everything here is a plain function over the JSON shapes the wasm
 * boundary speaks (`Scene.scenes_json()`, `scene_drift()`, camera/display
 * JSON) — no React, no wasm import, so it unit-tests with a plain string
 * and the UI components can `import type` from here without a loader.
 *
 * Property bitmask (`Scene.add_scene`'s `props` and every sibling): bit 1
 * camera, 2 hidden objects, 4 visible tags, 8 section plane, 16 display.
 */

export const PROP_CAMERA = 1
export const PROP_HIDDEN_NODES = 2
export const PROP_HIDDEN_TAGS = 4
export const PROP_SECTION = 8
export const PROP_DISPLAY = 16
export const PROP_ALL = PROP_CAMERA | PROP_HIDDEN_NODES | PROP_HIDDEN_TAGS | PROP_SECTION | PROP_DISPLAY

/** The five capturable properties, in the order the details view lists them. */
export type SceneProp = 'camera' | 'hiddenNodes' | 'hiddenTags' | 'section' | 'display'
export const SCENE_PROPS: readonly SceneProp[] = ['camera', 'hiddenNodes', 'hiddenTags', 'section', 'display']
export const PROP_BIT: Record<SceneProp, number> = {
  camera: PROP_CAMERA,
  hiddenNodes: PROP_HIDDEN_NODES,
  hiddenTags: PROP_HIDDEN_TAGS,
  section: PROP_SECTION,
  display: PROP_DISPLAY,
}
/** UI labels per property (SPEC.md §1 "Captured properties"). */
export const PROP_LABEL: Record<SceneProp, string> = {
  camera: 'Camera',
  hiddenNodes: 'Hidden objects',
  hiddenTags: 'Visible tags',
  section: 'Section plane',
  display: 'Display (grid, axes, guides)',
}

export interface CameraStateJson {
  projection: 'perspective' | 'parallel'
  fovDeg: number
  eye: [number, number, number]
  target: [number, number, number]
  up: [number, number, number]
}

export interface DisplayStateJson {
  grid: boolean
  axes: boolean
  guides: boolean
}

export interface SectionPlaneJson {
  origin: [number, number, number]
  /** Unit; points at the side the cut REMOVES (`sectionManager.ts`). */
  normal: [number, number, number]
  active: boolean
}

/** One Scene as the wasm boundary lists it (`Scene.scenes_json()`). */
export interface SceneEntry {
  sid: number
  name: string
  description: string
  /** Capture bitmask — see `PROP_*`. */
  props: number
  camera?: CameraStateJson
  display?: DisplayStateJson
  /** `null` = captured with no plane; absent = not captured. */
  section?: SectionPlaneJson | null
}

/** `Scene.scene_drift()`'s answer. */
export interface SceneDrift {
  camera: boolean
  hiddenNodes: boolean
  hiddenTags: boolean
  section: boolean
  display: boolean
  staleRefs: number
}

export function driftAny(d: SceneDrift | null): boolean {
  return d !== null && (d.camera || d.hiddenNodes || d.hiddenTags || d.section || d.display || d.staleRefs > 0)
}

export function hasProp(entry: SceneEntry, prop: SceneProp): boolean {
  return (entry.props & PROP_BIT[prop]) !== 0
}

function isTriple(v: unknown): v is [number, number, number] {
  return Array.isArray(v) && v.length === 3 && v.every((x) => typeof x === 'number' && Number.isFinite(x))
}

export function parseCameraJson(v: unknown): CameraStateJson | undefined {
  if (typeof v !== 'object' || v === null) return undefined
  const o = v as Record<string, unknown>
  if (o.projection !== 'perspective' && o.projection !== 'parallel') return undefined
  if (typeof o.fovDeg !== 'number' || !isTriple(o.eye) || !isTriple(o.target) || !isTriple(o.up)) return undefined
  return { projection: o.projection, fovDeg: o.fovDeg, eye: o.eye, target: o.target, up: o.up }
}

export function parseDisplayJson(v: unknown): DisplayStateJson | undefined {
  if (typeof v !== 'object' || v === null) return undefined
  const o = v as Record<string, unknown>
  if (typeof o.grid !== 'boolean' || typeof o.axes !== 'boolean' || typeof o.guides !== 'boolean') return undefined
  return { grid: o.grid, axes: o.axes, guides: o.guides }
}

export function parseSectionJson(v: unknown): SectionPlaneJson | undefined {
  if (typeof v !== 'object' || v === null) return undefined
  const o = v as Record<string, unknown>
  if (!isTriple(o.origin) || !isTriple(o.normal) || typeof o.active !== 'boolean') return undefined
  return { origin: o.origin, normal: o.normal, active: o.active }
}

/**
 * Parse `Scene.scenes_json()`. Malformed entries are dropped rather than
 * thrown on — the kernel wrote this JSON, so a shape mismatch is a version
 * skew we would rather degrade on than crash the tray for.
 */
export function parseScenesJson(json: string): SceneEntry[] {
  let raw: unknown
  try {
    raw = JSON.parse(json)
  } catch {
    return []
  }
  if (!Array.isArray(raw)) return []
  const out: SceneEntry[] = []
  for (const item of raw) {
    if (typeof item !== 'object' || item === null) continue
    const o = item as Record<string, unknown>
    if (typeof o.sid !== 'number' || typeof o.name !== 'string' || typeof o.props !== 'number') continue
    const entry: SceneEntry = {
      sid: o.sid,
      name: o.name,
      description: typeof o.description === 'string' ? o.description : '',
      props: o.props,
    }
    const camera = parseCameraJson(o.camera)
    if (camera !== undefined) entry.camera = camera
    const display = parseDisplayJson(o.display)
    if (display !== undefined) entry.display = display
    if ('section' in o) {
      if (o.section === null) entry.section = null
      else {
        const section = parseSectionJson(o.section)
        if (section !== undefined) entry.section = section
      }
    }
    out.push(entry)
  }
  return out
}

export function parseDriftJson(json: string): SceneDrift | null {
  let raw: unknown
  try {
    raw = JSON.parse(json)
  } catch {
    return null
  }
  if (typeof raw !== 'object' || raw === null) return null
  const o = raw as Record<string, unknown>
  const flag = (k: string): boolean => o[k] === true
  return {
    camera: flag('camera'),
    hiddenNodes: flag('hiddenNodes'),
    hiddenTags: flag('hiddenTags'),
    section: flag('section'),
    display: flag('display'),
    staleRefs: typeof o.staleRefs === 'number' ? o.staleRefs : 0,
  }
}

/** Index of `sid` in `entries`, or -1. */
export function sceneIndex(entries: readonly SceneEntry[], sid: number): number {
  return entries.findIndex((e) => e.sid === sid)
}

/**
 * The Scene after `activeSid` in tab order (wrapping), or the first when
 * none is active; `null` when there are no Scenes. `dir` −1 = previous.
 */
export function neighborScene(entries: readonly SceneEntry[], activeSid: number | null, dir: 1 | -1): SceneEntry | null {
  if (entries.length === 0) return null
  const i = activeSid === null ? -1 : sceneIndex(entries, activeSid)
  if (i < 0) return dir === 1 ? entries[0] : entries[entries.length - 1]
  return entries[(i + dir + entries.length) % entries.length]
}

/** The auto-name Add Scene uses when the kernel is unavailable (mirrors `next_scene_name`). */
export function nextSceneName(entries: readonly SceneEntry[]): string {
  const taken = new Set(entries.map((e) => e.name))
  for (let n = 1; ; n++) {
    const candidate = `Scene ${n}`
    if (!taken.has(candidate)) return candidate
  }
}

/** Copy strings (SPEC.md §5). */
export const SCENE_COPY = {
  duplicateName: (name: string) => `A Scene named "${name}" already exists.`,
  deleteTitle: (name: string) => `Delete "${name}"?`,
  deleteBody: "Deleting a Scene can't be undone.",
  staleRefs: (n: number) => `${n} captured ${n === 1 ? 'object' : 'objects'} no longer ${n === 1 ? 'exists' : 'exist'}.`,
  emptyState: 'No Scenes yet. Add Scene saves the current camera, visibility, and section cut.',
  descriptionPlaceholder: 'Add a description — it shows on the phone.',
  updateTooltip: 'Update Scene — the view has changed since capture.',
} as const
