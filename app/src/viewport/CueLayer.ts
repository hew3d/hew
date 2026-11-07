/**
 * CueLayer — a THREE.Group rebuilt on every pointer move.
 *
 * Renders:
 *   - Snap marker glyph at the snap point, colored by kind:
 *       endpoint   → green   #00cc44
 *       midpoint   → cyan    #00cccc
 *       on-edge    → red     #cc2200
 *       on-intersection-line → orange-red #e05a00 (the line where two
 *                    objects' faces meet — a virtual edge, no stored geometry)
 *       on-face    → blue    #0055cc
 *       on-guide   → purple  #9933cc  (construction guide)
 *       on-axis    → axis color (X=red, Y=green, Z=blue; unknown=magenta)
 *       from-point → axis color, same as on-axis (LineTool's from-point
 *                    closing inference — a tool-local kind, never emitted
 *                    by SnapService itself)
 *       ground     → gray    #888888  (fallback — no kernel snap)
 *       plane      → gray    #888888  (constraint-plane fallback, same role)
 *       other      → white   #ffffff
 *   - Dashed guide line through the snap point along direction() when present.
 *   - The virtual edge itself, drawn solid between its two ends, when the
 *     snap lies on the line where two objects' faces meet (`meetingLine`):
 *     nothing in the model draws that line, so the cue is the only thing
 *     that shows what the cursor is sliding along.
 *
 * The group is added to the scene once; call update() on every pointer move
 * to rebuild its children; call updateMarkerScale(camera) every render frame
 * so the cross marker stays a constant screen size regardless of zoom.
 */

import * as THREE from 'three'
import type { Snap } from '../tools/types'
import { axisColorsForTheme } from './axisColors'
import { getResolvedTheme } from '../settings/theme'

const SNAP_COLORS: Record<string, number> = {
  endpoint: 0x00cc44,
  center: 0x00aa88,
  quadrant: 0x00aa88,
  tangent: 0xb050d0,
  midpoint: 0x00cccc,
  intersection: 0xffaa00,
  'on-edge': 0xcc2200,
  'on-intersection-line': 0xe05a00,
  'on-face': 0x0055cc,
  'on-guide': 0x9933cc,
  ground: 0x888888,
  plane: 0x888888,
}

/** Half-length of the dashed guide line (meters) */
const GUIDE_HALF_LENGTH = 5

function snapColor(kind: string): number {
  if (kind in SNAP_COLORS) return SNAP_COLORS[kind]
  if (kind === 'on-axis') return 0xcc00cc // magenta fallback if direction unknown
  return 0xffffff
}

function buildGuideLine(
  pos: THREE.Vector3,
  direction: [number, number, number],
  color: number,
): THREE.LineSegments {
  const [dx, dy, dz] = direction
  const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
  if (len < 1e-9) return new THREE.LineSegments()

  const nx = (dx / len) * GUIDE_HALF_LENGTH
  const ny = (dy / len) * GUIDE_HALF_LENGTH
  const nz = (dz / len) * GUIDE_HALF_LENGTH

  // Dashed guide line: 10 segments alternating solid/gap
  const SEGMENTS = 20
  const pts: number[] = []
  for (let i = 0; i < SEGMENTS; i++) {
    // Alternate: even=solid, odd=gap
    if (i % 2 !== 0) continue
    const t0 = (i / SEGMENTS) * 2 - 1 // -1 to +1
    const t1 = ((i + 1) / SEGMENTS) * 2 - 1
    pts.push(
      pos.x + t0 * nx, pos.y + t0 * ny, pos.z + t0 * nz,
      pos.x + t1 * nx, pos.y + t1 * ny, pos.z + t1 * nz,
    )
  }

  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pts), 3))
  const mat = new THREE.LineBasicMaterial({
    color,
    depthTest: false,
    transparent: true,
    opacity: 0.6,
  })
  const ls = new THREE.LineSegments(geo, mat)
  ls.renderOrder = 998
  return ls
}

/** The dotted tie from an axis-locked snap's source point (`projectedFrom`)
 *  to its projection on the locked line — SketchUp's "projected inference"
 *  visual: it says WHICH midpoint or corner the point on the lock came
 *  from. Same dash rhythm as the guide line, solid-ish so it reads over
 *  geometry. */
function buildTieLine(from: THREE.Vector3, to: THREE.Vector3, color: number): THREE.LineSegments {
  const d = to.clone().sub(from)
  const len = d.length()
  if (len < 1e-9) return new THREE.LineSegments()
  const dashes = Math.max(4, Math.min(40, Math.round(len / TIE_DASH_LENGTH_M) * 2))
  const pts: number[] = []
  for (let i = 0; i < dashes; i += 2) {
    const t0 = i / dashes
    const t1 = (i + 1) / dashes
    pts.push(
      from.x + d.x * t0, from.y + d.y * t0, from.z + d.z * t0,
      from.x + d.x * t1, from.y + d.y * t1, from.z + d.z * t1,
    )
  }
  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pts), 3))
  const mat = new THREE.LineBasicMaterial({ color, depthTest: false, transparent: true, opacity: 0.8 })
  const ls = new THREE.LineSegments(geo, mat)
  ls.renderOrder = 998
  return ls
}

/** Nominal dash length of the projection tie (metres, before the 4..40 dash
 *  clamp) — short enough to read as dotted at drawing scale. */
const TIE_DASH_LENGTH_M = 0.02

/** Below this length (metres) a meeting line has nothing to draw: the
 *  kernel's own point-merge distance, the shortest stretch it offers. */
const MEETING_LINE_MIN_LENGTH_M = 1e-9

/** The virtual edge where two objects' faces meet, drawn solid between its
 *  two ends in the snap's own color. Nothing in the model draws this line
 *  — objects never merge, so it is no stored edge — and without it the
 *  cursor would appear to slide along nothing. Drawn over geometry like
 *  the tie, a little fainter than a real edge's highlight. `null` for a
 *  degenerate line, which draws nothing. */
function buildMeetingLine(a: THREE.Vector3, b: THREE.Vector3, color: number): THREE.LineSegments | null {
  if (a.distanceTo(b) < MEETING_LINE_MIN_LENGTH_M) return null
  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array([a.x, a.y, a.z, b.x, b.y, b.z]), 3))
  const mat = new THREE.LineBasicMaterial({ color, depthTest: false, transparent: true, opacity: 0.85 })
  const ls = new THREE.LineSegments(geo, mat)
  ls.renderOrder = 998
  return ls
}

export class CueLayer {
  readonly group: THREE.Group

  constructor() {
    this.group = new THREE.Group()
    this.group.name = 'CueLayer'
  }

  /**
   * Rebuild the cue layer for the current snap result.
   *
   * `suppressAxisLine` (Shop Mode round-3 playtest finding 2): Shop Mode
   * renders no world axes at all (`showAxes=false`), so the dashed guide
   * line this class draws through an `'on-axis'` snap's own direction would
   * reference geometry that literally isn't on screen — incoherent, not
   * just redundant. Scoped to that one kind: a guide line through an
   * `'on-edge'`/`'on-guide'` direction still references something real and
   * visible, so those are untouched. The snap POINT itself (this class's
   * caller also drives the DOM `SnapDot`/tooltip via `publishSnapCues`) is
   * a separate concern this flag never touches — callers that want the
   * point but not the line (Tape Measure under `readOnly`) get exactly
   * that by passing `true` here while leaving the snap itself unmodified.
   */
  update(snap: Snap | null, suppressAxisLine = false): void {
    // Dispose old geometry
    this.group.traverse((child) => {
      if (child instanceof THREE.LineSegments) {
        child.geometry.dispose()
        if (child.material instanceof THREE.Material) {
          child.material.dispose()
        }
      }
    })
    this.group.clear()

    if (snap === null) return

    const pos = new THREE.Vector3(snap.x, snap.y, snap.z)

    // Determine color
    let color = snapColor(snap.kind)
    // `'from-point'` is LineTool's from-point closing inference (module
    // doc, `_findFromPointCandidate`) — colored exactly like `'on-axis'`,
    // by the dominant component of the axis it rides.
    if ((snap.kind === 'on-axis' || snap.kind === 'from-point') && snap.direction !== undefined) {
      // Infer axis color from dominant direction component (—
      // theme-aware: light/dark axis colors differ, per 01_design_tokens.md).
      const [dx, dy, dz] = snap.direction
      const abs = [Math.abs(dx), Math.abs(dy), Math.abs(dz)]
      const axis = abs.indexOf(Math.max(...abs)) as 0 | 1 | 2
      color = axisColorsForTheme(getResolvedTheme())[axis]
    }

    // The snap POINT itself is now marked by the DOM `SnapDot` overlay
    // (Refinement pass, issue B) — a bright pulsing dot that reads clearly
    // against any geometry, unlike the thin 1px three.js cross this used to
    // draw. CueLayer keeps only the in-scene dashed GUIDE line (which the DOM
    // layer can't do — it needs depth + world extent along the axis/edge).
    if (snap.direction !== undefined && !(suppressAxisLine && snap.kind === 'on-axis')) {
      this.group.add(buildGuideLine(pos, snap.direction, color))
    }
    // A locked snap projected from a real point: tie the two together in
    // the snap's own kind colour (the axis colour above belongs to the
    // guide line, not to the midpoint/corner the tie explains).
    if (snap.projectedFrom !== undefined) {
      const from = new THREE.Vector3(...snap.projectedFrom)
      this.group.add(buildTieLine(from, pos, snapColor(snap.kind)))
    }
    // The line where two objects' faces meet: show the whole stretch the
    // cursor is on, in the kind's color (orange-red for the line, cyan for
    // its midpoint) — a real edge is already drawn by the model, this one
    // is not.
    if (snap.meetingLine !== undefined) {
      const [a, b] = snap.meetingLine
      const line = buildMeetingLine(new THREE.Vector3(...a), new THREE.Vector3(...b), snapColor(snap.kind))
      if (line !== null) this.group.add(line)
    }
  }

  /** Clear without disposing (called on cleanup) */
  clear(): void {
    this.group.clear()
  }
}
