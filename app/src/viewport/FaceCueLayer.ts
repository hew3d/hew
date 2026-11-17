/**
 * FaceCueLayer — lights the face (or sketch region) a tool's click would act
 * on, from the active tool's `hoverFace()` (OW-14, Push/Pull today):
 *
 * 1. A tint over the face: its own triangles drawn again with a flat unlit
 *    material at 0.22 opacity on the preview rung of the depth ladder,
 *    depth-tested. Holes stay holes and a curved face tints correctly,
 *    because these are the triangles already on screen.
 * 2. An outline of the outer boundary: a 1.5 px fat line at full opacity.
 *
 * The color says what a click would do, in the preview language
 * (`previewTint.ts`): orange (`--preview-pushpull`) pushes or pulls the
 * face's own solid, blue (`--preview-extrude-new`) makes a new object. Not a
 * selection: selection lights the edges of a whole object, this is a fill on
 * one face. Purely visual: never pickable, never a snap participant.
 *
 * It rebuilds only when the target's identity (or color, or theme) changes,
 * not per pointer move. A face with more than `FACE_CUE_MAX_VERTICES`
 * vertices gets the outline only. A face the source cannot answer for (a
 * stale handle) draws nothing rather than a misplaced cue.
 *
 * The Viewport calls `update()` with `tool.hoverFace()` after every pointer
 * move, key and view change, and `clear()` on pointer down (the tool then
 * reports null), pointer leave, Escape, tool switch, document reset,
 * entering or leaving an editing session, and unmount.
 */
import * as THREE from 'three'
import type { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import type { HoverFace } from '../tools/types'
import {
  PREVIEW_EXTRUDE_NEW_FALLBACK,
  PREVIEW_EXTRUDE_NEW_TOKEN,
  PREVIEW_PUSHPULL_FALLBACK,
  PREVIEW_PUSHPULL_TOKEN,
  readPreviewTint,
} from '../tools/previewTint'
import { getResolvedTheme } from '../settings/theme'
import { DEPTH_BIAS } from './depthPolicy'
import { disposeFatSegments, makeFatSegments } from './fatLine'
import { loopSegments } from './faceCueGeometry'

/** The tint's opacity. */
export const FACE_CUE_TINT_OPACITY = 0.22
/** The outline's weight (the same as the drawing-plane cue's outline). */
export const FACE_CUE_OUTLINE = { widthPx: 1.5, opacity: 1 } as const

/** Where the layer gets its geometry, all in WORLD space; `null` for what
 *  it cannot answer (the cue then draws what it can). The Viewport backs
 *  this with the renderer and the kernel; tests with plain arrays. */
export interface FaceCueSource {
  faceTriangles(object: bigint, face: bigint, instance: bigint | null): Float32Array | null
  regionTriangles(sketch: bigint, region: bigint, instance: bigint | null): Float32Array | null
  faceOutline(object: bigint, face: bigint, instance: bigint | null): Float32Array | null
  regionOutline(sketch: bigint, region: bigint, instance: bigint | null): Float32Array | null
}

/** What the layer shows right now (observe-only: the E2E harness reads it). */
export interface FaceCueShown {
  kind: 'face' | 'region'
  /** The object (face) or sketch (region) handle, and the face or region. */
  owner: bigint
  id: bigint
  instance: bigint | null
  colorToken: HoverFace['colorToken']
  /** Whether the tint and the outline were drawn. */
  tinted: boolean
  outlined: boolean
}

function tokenColor(token: HoverFace['colorToken']): number {
  return token === PREVIEW_EXTRUDE_NEW_TOKEN
    ? readPreviewTint(PREVIEW_EXTRUDE_NEW_TOKEN, PREVIEW_EXTRUDE_NEW_FALLBACK)
    : readPreviewTint(PREVIEW_PUSHPULL_TOKEN, PREVIEW_PUSHPULL_FALLBACK)
}

export class FaceCueLayer {
  readonly group: THREE.Group
  private tint: THREE.Mesh | null = null
  private outline: LineSegments2 | null = null
  private shown: FaceCueShown | null = null
  private shownKey: string | null = null
  private builds = 0

  constructor() {
    this.group = new THREE.Group()
    this.group.name = 'FaceCueLayer'
  }

  /** The cue drawn right now, or null. */
  get current(): FaceCueShown | null {
    return this.shown === null ? null : { ...this.shown }
  }

  /** How many times a cue was (re)built — observe-only, for tests: it must
   *  not grow across many moves over one face. */
  get buildCount(): number {
    return this.builds
  }

  /**
   * Show the cue for `cue` (`tool.hoverFace()`), or clear it when null. The
   * same target in the same color and theme as already shown is left alone.
   */
  update(cue: HoverFace | null, source: FaceCueSource): void {
    if (cue === null) {
      this.clear()
      return
    }
    const t = cue.target
    const key = [
      t.kind,
      t.kind === 'face' ? t.object : t.sketch,
      t.kind === 'face' ? t.face : t.region,
      t.instance ?? '-',
      cue.colorToken,
      getResolvedTheme(),
    ].join(':')
    if (key === this.shownKey) return
    this._dispose()
    this.shownKey = key
    this.builds++

    let tris: Float32Array | null = null
    let loop: Float32Array | null = null
    try {
      if (t.kind === 'face') {
        tris = source.faceTriangles(t.object, t.face, t.instance)
        loop = source.faceOutline(t.object, t.face, t.instance)
      } else {
        tris = source.regionTriangles(t.sketch, t.region, t.instance)
        loop = source.regionOutline(t.sketch, t.region, t.instance)
      }
    } catch {
      // A handle gone stale between the pick and now: nothing to light.
      tris = null
      loop = null
    }

    const color = tokenColor(cue.colorToken)
    if (tris !== null && tris.length >= 9) {
      const geo = new THREE.BufferGeometry()
      geo.setAttribute('position', new THREE.BufferAttribute(tris, 3))
      const mat = new THREE.MeshBasicMaterial({
        color,
        transparent: true,
        opacity: FACE_CUE_TINT_OPACITY,
        side: THREE.DoubleSide,
        depthTest: true,
        depthWrite: false,
        toneMapped: false,
        polygonOffset: true,
        polygonOffsetFactor: DEPTH_BIAS.PREVIEW,
        polygonOffsetUnits: DEPTH_BIAS.PREVIEW,
      })
      const mesh = new THREE.Mesh(geo, mat)
      mesh.name = 'FaceCueTint'
      mesh.renderOrder = 995
      mesh.raycast = () => { /* never pickable */ }
      mesh.frustumCulled = false
      this.tint = mesh
      this.group.add(mesh)
    }
    if (loop !== null && loop.length >= 9) {
      const line = makeFatSegments(loopSegments(loop), {
        color,
        widthPx: FACE_CUE_OUTLINE.widthPx,
        opacity: FACE_CUE_OUTLINE.opacity,
        depthTest: true,
        depthBias: DEPTH_BIAS.PREVIEW,
        renderOrder: 996,
      })
      line.name = 'FaceCueOutline'
      line.raycast = () => { /* never pickable */ }
      this.outline = line
      this.group.add(line)
    }
    this.shown = {
      kind: t.kind,
      owner: t.kind === 'face' ? t.object : t.sketch,
      id: t.kind === 'face' ? t.face : t.region,
      instance: t.instance,
      colorToken: cue.colorToken,
      tinted: this.tint !== null,
      outlined: this.outline !== null,
    }
  }

  /** Dispose and hide the cue: pointer down, leave, Escape, tool switch,
   *  document reset, a session opening or closing. */
  clear(): void {
    this._dispose()
  }

  private _dispose(): void {
    this.shown = null
    this.shownKey = null
    if (this.tint !== null) {
      this.tint.geometry.dispose()
      ;(this.tint.material as THREE.Material).dispose()
      this.group.remove(this.tint)
      this.tint = null
    }
    if (this.outline !== null) {
      disposeFatSegments(this.outline)
      this.group.remove(this.outline)
      this.outline = null
    }
  }
}
