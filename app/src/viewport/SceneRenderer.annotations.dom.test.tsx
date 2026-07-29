/**
 * SceneRenderer annotation-rendering tests — the playtest-round-2 fixes
 * (docs/design/dimensions-playtest2.md): breaking a linear dimension's line
 * around its label (§2), and radial dimensions drawing the geometry they
 * actually measure (§4). `.dom.test.tsx` (not `.test.ts`) because
 * `refreshAnnotations`/`updateAnnotationBillboards` construct `TextBillboard`
 * instances, which need a `document` global — this repo's plain `.test.ts`
 * files run under vitest's `node` environment with no DOM at all (see
 * `vitest.config.ts`'s `environmentMatchGlobs`; mirrors
 * `TextBillboard.dom.test.tsx`'s own split from `TextBillboard.test.ts`).
 *
 * jsdom has no canvas 2D backend (no `canvas`/jest-canvas-mock polyfill in
 * this repo), so `TextBillboard`'s rasterization never actually runs here —
 * `aspect`/`contentFrac` stay at their pre-raster defaults of `1`, which
 * makes `worldSize()` a clean, deterministic function of camera distance
 * alone (independent of the label's actual text) — exactly what these tests
 * exploit to hit each of the three §2 regimes on purpose.
 */
import { describe, expect, it } from 'vitest'
import * as THREE from 'three'
import { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import type { Scene as WasmScene } from '../wasm/loader'
import { SceneRenderer } from './SceneRenderer'

/** A minimal mock WasmScene exposing exactly the `annotation_*` surface
 * `refreshAnnotations`/`_buildAnnotationBase` read. */
function makeAnnotationScene(defs: {
  linear?: Record<
    string,
    { a: number[]; b: number[]; offset: number[]; plane: number[]; detached?: boolean; override?: string }
  >
  radial?: Record<
    string,
    { anchor: number[]; kind: 'radius' | 'diameter'; curve: number[]; leaderDir: number[]; detached?: boolean }
  >
}): WasmScene {
  const linear = defs.linear ?? {}
  const radial = defs.radial ?? {}
  const ids = [...Object.keys(linear), ...Object.keys(radial)].map(BigInt)
  return {
    annotation_ids: () => BigUint64Array.from(ids),
    annotation_kind: (id: bigint) => {
      if (id.toString() in linear) return 'linear'
      if (id.toString() in radial) return 'radial'
      return undefined
    },
    annotation_detached: (id: bigint) => linear[id.toString()]?.detached ?? radial[id.toString()]?.detached ?? false,
    annotation_anchor_point: (id: bigint, which: number) => {
      const l = linear[id.toString()]
      if (l !== undefined) return Float64Array.from(which === 0 ? l.a : l.b)
      const r = radial[id.toString()]
      if (r !== undefined && which === 0) return Float64Array.from(r.anchor)
      return undefined
    },
    annotation_offset: (id: bigint) => {
      const l = linear[id.toString()]
      return l !== undefined ? Float64Array.from(l.offset) : undefined
    },
    annotation_plane: (id: bigint) => {
      const l = linear[id.toString()]
      return l !== undefined ? Float64Array.from(l.plane) : undefined
    },
    annotation_text_override: (id: bigint) => linear[id.toString()]?.override ?? undefined,
    annotation_leader_dir: (id: bigint) => {
      const r = radial[id.toString()]
      return r !== undefined ? Float64Array.from(r.leaderDir) : undefined
    },
    annotation_curve: (id: bigint) => {
      const r = radial[id.toString()]
      return r !== undefined ? Float64Array.from(r.curve) : undefined
    },
    annotation_radial_kind: (id: bigint) => radial[id.toString()]?.kind,
    annotation_text: () => undefined,
  } as unknown as WasmScene
}

/** Every fat-line batch (`annotationLines`/`annotationLinesDetached`/
 * `annotationHighlight`) currently parented under the renderer's (public)
 * `annotationsGroup` — inspecting the group instead of SceneRenderer's
 * private fields, like the other SceneRenderer tests inspect public groups. */
function annotationFatLines(renderer: SceneRenderer): LineSegments2[] {
  const out: LineSegments2[] = []
  renderer.annotationsGroup.traverse((o) => {
    if (o instanceof LineSegments2) out.push(o)
  })
  return out
}

/** Every 2-point segment `[ax,ay,az,bx,by,bz]` across every live fat-line
 * batch, decoded from `LineSegmentsGeometry`'s interleaved `instanceStart`
 * buffer (the same flat array `makeFatSegments`/`setPositions` was given). */
function allSegments(renderer: SceneRenderer): [THREE.Vector3, THREE.Vector3][] {
  const segs: [THREE.Vector3, THREE.Vector3][] = []
  for (const line of annotationFatLines(renderer)) {
    const attr = line.geometry.attributes.instanceStart as THREE.InterleavedBufferAttribute | undefined
    if (attr === undefined) continue
    const arr = attr.data.array as Float32Array
    for (let i = 0; i + 5 < arr.length; i += 6) {
      segs.push([new THREE.Vector3(arr[i], arr[i + 1], arr[i + 2]), new THREE.Vector3(arr[i + 3], arr[i + 4], arr[i + 5])])
    }
  }
  return segs
}

/** Whether any segment spans (within `tol`, absorbing the §1 view-direction
 * bias — which scales with camera distance, e.g. ~0.7 world units at this
 * file's farthest (regime 3) camera, so the default is generous rather than
 * a tight geometric tolerance) from `p` to `q` OR `q` to `p` (fat-line
 * segment endpoints have no fixed order relative to a logical a->b line). */
function hasSegmentSpanning(renderer: SceneRenderer, p: THREE.Vector3, q: THREE.Vector3, tol = 1.0): boolean {
  return allSegments(renderer).some(
    ([a, b]) => (a.distanceTo(p) < tol && b.distanceTo(q) < tol) || (a.distanceTo(q) < tol && b.distanceTo(p) < tol),
  )
}

/** A camera looking straight down -Z at `(midX, midY, 0)` from `dist` units
 * up — `fov`/`viewportHeight` match the values baked into this file's
 * regime-selecting distances (see the module doc comment on why `dist`
 * alone determines the (deterministic, jsdom-no-canvas) label size). */
function topDownCamera(midX: number, midY: number, dist: number): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(FOV_DEG, 1, 0.1, 10000)
  cam.position.set(midX, midY, dist)
  cam.lookAt(midX, midY, 0)
  cam.updateMatrixWorld(true)
  cam.updateProjectionMatrix()
  return cam
}

const FOV_DEG = 50
const VIEWPORT = 800

describe('SceneRenderer — dimension-line/label gap (dimensions-playtest2.md §2)', () => {
  // RED-CHECK (all three regimes): the unfixed `_buildAnnotationLines`
  // always emits the SINGLE, unbroken `a1 -> b1` segment — `hasSegmentSpanning`
  // for the full a1/b1 span would be TRUE in every regime, including regime
  // 1 (label fits with room), where the fixed expectation is that it is
  // FALSE (broken around the label). The unfixed code also has no
  // `annotationTextWorldPosition` deviation for regime 3 — the label always
  // sits at the exact midpoint, never pushed outside.

  // a1=(0,5,0), b1=(10,5,0): a 10-unit dimension line, offset 5 units off
  // the (0,0,0)-(10,0,0) baseline, drawn in the z=0 plane.
  const scene = makeAnnotationScene({
    linear: { '1': { a: [0, 0, 0], b: [10, 0, 0], offset: [0, 5, 0], plane: [0, 0, 0, 0, 0, 1] } },
  })
  const A1 = new THREE.Vector3(0, 5, 0)
  const B1 = new THREE.Vector3(10, 5, 0)
  const MID = new THREE.Vector3(5, 5, 0)

  it('regime 1: label fits with room to spare -> the line is broken (no segment spans the whole a1-b1 run)', () => {
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refreshAnnotations()
    // dist=50 -> worldHeight ~0.82 world units -> gap ~1.5 units, well under
    // the 10-unit line (see this file's module doc comment for the formula).
    const camera = topDownCamera(5, 5, 50)
    renderer.updateAnnotationBillboards(camera, VIEWPORT, VIEWPORT, 'dark')

    expect(hasSegmentSpanning(renderer, A1, B1)).toBe(false)
    // The label stays centered — this is the "fits comfortably" case, not
    // the "pushed outside" one.
    const labelPos = renderer.annotationTextWorldPosition(1n)
    expect(labelPos).not.toBeNull()
    expect(labelPos![0]).toBeCloseTo(MID.x, 1)
  })

  it('regime 2: label nearly fills the line -> still broken, but the gap consumes most of it', () => {
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refreshAnnotations()
    // dist=300 -> worldHeight ~4.9 -> gap ~8.8 units of the 10-unit line —
    // comfortably inside "broken" but with only a sliver of line on each side.
    const camera = topDownCamera(5, 5, 300)
    renderer.updateAnnotationBillboards(camera, VIEWPORT, VIEWPORT, 'dark')

    expect(hasSegmentSpanning(renderer, A1, B1)).toBe(false)
    // A short stub must still remain on EACH side of the gap (not degenerate
    // to nothing) — some segment starts at (or very near) a1, and some
    // segment ends at (or very near) b1.
    const segs = allSegments(renderer)
    const nearA1 = segs.some(([a, b]) => a.distanceTo(A1) < 1.0 || b.distanceTo(A1) < 1.0)
    const nearB1 = segs.some(([a, b]) => a.distanceTo(B1) < 1.0 || b.distanceTo(B1) < 1.0)
    expect(nearA1).toBe(true)
    expect(nearB1).toBe(true)
    const labelPos = renderer.annotationTextWorldPosition(1n)
    expect(labelPos![0]).toBeCloseTo(MID.x, 1)
  })

  it('regime 3: label wider than the line -> the line is drawn WHOLE and the label moves outside', () => {
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refreshAnnotations()
    // dist=800 -> worldHeight ~13.1 -> gap ~23.5 units, wider than the
    // 10-unit line itself.
    const camera = topDownCamera(5, 5, 800)
    renderer.updateAnnotationBillboards(camera, VIEWPORT, VIEWPORT, 'dark')

    expect(hasSegmentSpanning(renderer, A1, B1)).toBe(true)
    // The label is no longer at the midpoint — it moved past one end.
    const labelPos = renderer.annotationTextWorldPosition(1n)
    expect(labelPos).not.toBeNull()
    expect(Math.abs(labelPos![0] - MID.x)).toBeGreaterThan(3)
    // And it landed roughly on the line's own axis extended (y ~ 5), not
    // off in some unrelated direction.
    expect(labelPos![1]).toBeCloseTo(5, 0)
  })
})

describe('SceneRenderer — radial dimension geometry shows the measurement (dimensions-playtest2.md §4)', () => {
  // RED-CHECK: the unfixed `_buildAnnotationLines`'s 'radial' branch draws
  // ONLY `anchor -> anchor+leaderDir` — `hasSegmentSpanning(center, anchor)`
  // and the diameter chord-through-centre check both fail against it (the
  // centre point never appears in the geometry at all).

  it('Radius: a segment runs from the true centre to the rim anchor', () => {
    const scene = makeAnnotationScene({
      radial: {
        '1': {
          anchor: [4, 2, 0], // radius 3 along +X from center (1,2,0)
          kind: 'radius',
          curve: [1, 2, 0, 3, 1, 2, 0, 0, 0, 1],
          leaderDir: [1, 0, 0],
        },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refreshAnnotations()
    const camera = topDownCamera(1, 2, 50)
    renderer.updateAnnotationBillboards(camera, VIEWPORT, VIEWPORT, 'dark')

    const center = new THREE.Vector3(1, 2, 0)
    const anchor = new THREE.Vector3(4, 2, 0)
    expect(hasSegmentSpanning(renderer, center, anchor)).toBe(true)
    expect(renderer.annotationLabelText(1n)).toContain('R')
  })

  it('Diameter: the drawn chord spans rim to rim, straight through the centre', () => {
    const scene = makeAnnotationScene({
      radial: {
        '1': {
          anchor: [5, 0, 0], // radius 5 along +X from center (0,0,0)
          kind: 'diameter',
          curve: [0, 0, 0, 5, 0, 0, 0, 0, 0, 1],
          leaderDir: [1, 0, 0],
        },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refreshAnnotations()
    const camera = topDownCamera(0, 0, 50)
    renderer.updateAnnotationBillboards(camera, VIEWPORT, VIEWPORT, 'dark')

    const rim = new THREE.Vector3(5, 0, 0)
    const antipode = new THREE.Vector3(-5, 0, 0)
    expect(hasSegmentSpanning(renderer, rim, antipode)).toBe(true)
    expect(renderer.annotationLabelText(1n)).toContain('Ø')
  })
})
