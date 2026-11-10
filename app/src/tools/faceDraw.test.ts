import { afterEach, beforeEach, describe, it, expect, vi } from 'vitest'
import { FacePickCache, PICK_NUDGE_PX, PICK_NUDGE_RAD, cursorOnFrozenPlane, defaultFaceEligible, ineligibleFacePlane, worldFaceNormal, worldFacePlane } from './faceDraw'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import { pickNudgeRad, setPickAperture, setPickRenderMode } from './pickPolicy'
import type { Ray } from '../viewport/math'
import type { V3 } from '../viewport/geoHelpers'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makePick(object: bigint, face: bigint, instance?: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => instance,
    depth: () => 0,
    free: vi.fn(),
  }
}

/** node_parent(0, id) — `parents` maps a grouped object to its group id. */
function makeWasmScene(opts: {
  pick?: ReturnType<typeof makePick>
  parents?: Map<bigint, bigint>
} = {}): WasmScene {
  return {
    pick_face: vi.fn(() => opts.pick),
    node_parent: vi.fn((_kind: number, id: bigint) => opts.parents?.get(id)),
  } as unknown as WasmScene
}

describe('defaultFaceEligible (plain objects are directly drawable)', () => {
  it('top level: a plain, ungrouped object is eligible', () => {
    const scene = makeWasmScene()
    expect(defaultFaceEligible(scene, null, 7n, undefined)).toBe(true)
  })

  it('top level: an object inside a group needs the explicit edit step', () => {
    const scene = makeWasmScene({ parents: new Map([[7n, 3n]]) })
    expect(defaultFaceEligible(scene, null, 7n, undefined)).toBe(false)
  })

  it('top level: instanced (component) geometry needs the explicit edit step', () => {
    const scene = makeWasmScene()
    expect(defaultFaceEligible(scene, null, 7n, 12n)).toBe(false)
  })

  it('inside an entered object context: only that object, never instanced geometry', () => {
    const scene = makeWasmScene()
    expect(defaultFaceEligible(scene, 7n, 7n, undefined)).toBe(true)
    expect(defaultFaceEligible(scene, 7n, 8n, undefined)).toBe(false)
    expect(defaultFaceEligible(scene, 7n, 7n, 12n)).toBe(false)
  })
})

describe('FacePickCache', () => {
  it('memoizes the pick per Ray reference — one raycast for repeated queries on the same event', () => {
    const scene = makeWasmScene({ pick: makePick(7n, 3n) })
    const cache = new FacePickCache()
    const eligible = () => true

    const first = cache.pickFor(scene, RAY, eligible)
    const second = cache.pickFor(scene, RAY, eligible)
    expect(first).toEqual({ object: 7n, face: 3n })
    expect(second).toEqual(first)
    expect(scene.pick_face).toHaveBeenCalledTimes(1)

    // A NEW ray object re-picks (the Viewport builds one Ray per event).
    cache.pickFor(scene, { ...RAY }, eligible)
    expect(scene.pick_face).toHaveBeenCalledTimes(2)
  })

  it('hands the pick instance to the eligibility predicate and caches a rejection as null', () => {
    const scene = makeWasmScene({ pick: makePick(7n, 3n, 12n) })
    const cache = new FacePickCache()
    const isEligible = vi.fn(() => false)

    expect(cache.pickFor(scene, RAY, isEligible)).toBeNull()
    expect(isEligible).toHaveBeenCalledWith(7n, 12n)
    expect(cache.pickFor(scene, RAY, isEligible)).toBeNull()
    expect(scene.pick_face).toHaveBeenCalledTimes(1)
  })

  it('frees the wasm pick handle', () => {
    const pick = makePick(7n, 3n)
    const scene = makeWasmScene({ pick })
    new FacePickCache().pickFor(scene, RAY, () => true)
    expect(pick.free).toHaveBeenCalledTimes(1)
  })
})

describe('worldFaceNormal (component-edit-parity.md phase A2)', () => {
  function normalScene(local: [number, number, number], pose?: Float64Array): WasmScene {
    return {
      face_normal: vi.fn(() => new Float64Array(local)),
      instance_pose: vi.fn(() => pose),
    } as unknown as WasmScene
  }

  it('with activeInstance null, returns face_normal raw — a world object or Group member (local == world already)', () => {
    const scene = normalScene([0, 0, 1])
    expect(worldFaceNormal(scene, 3n, 4n, null)).toEqual([0, 0, 1])
  })

  it('with an activeInstance, poses the LOCAL normal forward through its pose (rotation)', () => {
    // 90° about Z: +X normal becomes +Y.
    const pose = new Float64Array([0, -1, 0, 5, 1, 0, 0, -2, 0, 0, 1, 9])
    const scene = normalScene([1, 0, 0], pose)
    const n = worldFaceNormal(scene, 3n, 4n, 42n)
    expect(n).not.toBeNull()
    expect(n![0]).toBeCloseTo(0)
    expect(n![1]).toBeCloseTo(1)
    expect(n![2]).toBeCloseTo(0)
  })

  it('maps by the inverse-transpose under a non-uniform scale, not the plain linear part', () => {
    const pose = new Float64Array([2, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]) // ×2 in X only
    const local: [number, number, number] = [Math.SQRT1_2, Math.SQRT1_2, 0]
    const scene = normalScene(local, pose)
    const n = worldFaceNormal(scene, 3n, 4n, 42n)
    expect(n).not.toBeNull()
    // The correct inverse-transpose of diag(2,1,1) is diag(0.5,1,1); the
    // WRONG plain-linear-part answer (diag(2,1,1) applied directly) would
    // tilt the other way (toward X, not away from it).
    const correctLen = Math.hypot(0.5, 1, 0)
    expect(n![0]).toBeCloseTo(0.5 / correctLen, 6)
    expect(n![1]).toBeCloseTo(1 / correctLen, 6)
    const wrongLen = Math.hypot(2, 1, 0)
    expect(n![0]).not.toBeCloseTo(2 / wrongLen, 2)
  })

  it('returns null for a stale/unknown instance — never falls back to the raw local normal', () => {
    const scene = normalScene([0, 0, 1], undefined)
    expect(worldFaceNormal(scene, 3n, 4n, 42n)).toBeNull()
  })
})

describe('worldFaceNormal / worldFacePlane on a stale handle (the documented null, not a throw)', () => {
  /** A scene whose face queries refuse like the kernel does for a handle
   *  that names nothing — the state a memoized face pick is left in once a
   *  retype has undone and recommitted the imprint under the cursor. */
  function staleScene(code: string): WasmScene {
    const refuse = () => {
      throw new Error(`${code}: stale or unknown ${code === 'UnknownObject' ? 'object' : 'face'} handle`)
    }
    return { face_normal: vi.fn(refuse), face_plane: vi.fn(refuse), instance_pose: vi.fn(() => undefined) } as unknown as WasmScene
  }

  it.each(['UnknownFace', 'UnknownObject'])('worldFaceNormal answers null when face_normal throws %s', (code) => {
    expect(worldFaceNormal(staleScene(code), 3n, 4n, null)).toBeNull()
  })

  it.each(['UnknownFace', 'UnknownObject'])('worldFacePlane answers null when face_plane throws %s', (code) => {
    const scene = {
      face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
      face_plane: vi.fn(() => { throw new Error(`${code}: stale or unknown handle`) }),
      instance_pose: vi.fn(() => undefined),
    } as unknown as WasmScene
    expect(worldFacePlane(scene, 3n, 4n, null)).toBeNull()
  })

  it('still propagates any other kernel error — only a gone handle is a documented miss', () => {
    const scene = {
      face_normal: vi.fn(() => { throw new Error('Internal: something else entirely') }),
      face_plane: vi.fn(),
      instance_pose: vi.fn(() => undefined),
    } as unknown as WasmScene
    expect(() => worldFaceNormal(scene, 3n, 4n, null)).toThrow('Internal')
    expect(() => worldFacePlane(scene, 3n, 4n, null)).toThrow('Internal')
  })
})

describe('FacePickCache boundary probe', () => {
  const RAY = { origin: [0, 0, 5] as [number, number, number], direction: [0, 0, -1] as [number, number, number] }
  const eligible = () => true
  const sceneMissingOnTheRay = () => {
    const pick_face = vi.fn((_ox: number, _oy: number, _oz: number, dx: number, dy: number, dz: number) =>
      dx === 0 && dy === 0 && dz === -1
        ? undefined // exactly on the edge: a strict miss
        : { object: () => 7n, instance: () => undefined, face: () => 3n, free: vi.fn() },
    )
    return { scene: { pick_face } as unknown as Parameters<FacePickCache['pickFor']>[0], pick_face }
  }

  it('does not probe unless asked, and a miss costs one raycast', () => {
    const { scene, pick_face } = sceneMissingOnTheRay()
    const cache = new FacePickCache()
    expect(cache.pickFor(scene, RAY, eligible)).toBeNull()
    expect(pick_face).toHaveBeenCalledTimes(1)
  })
})

describe('FacePickCache.faceThrough (boundary-click face resolution)', () => {
  const RAY = { origin: [0, 0, 5] as [number, number, number], direction: [0, 0, -1] as [number, number, number] }
  const eligible = () => true

  /** A fake scene whose `pick_face` misses on the exact ray direction (a
   *  boundary click — the strict polygon test misses both faces meeting
   *  there) but returns `hit` for every nudged ring probe direction, and
   *  whose `face_plane`/`face_normal` answer per the given `planes` map
   *  (keyed `object:face`). */
  function makeScene(
    hit: { object: bigint; face: bigint } | ((callIndex: number) => { object: bigint; face: bigint } | undefined),
    planes: Map<string, { point: V3; normal: V3 }>,
  ) {
    let callIndex = 0
    const pick_face = vi.fn((_ox: number, _oy: number, _oz: number, dx: number, dy: number, dz: number) => {
      const isBaseRay = dx === 0 && dy === 0 && dz === -1
      const idx = callIndex++
      if (isBaseRay) return undefined // exactly on the edge: a strict miss
      const picked = typeof hit === 'function' ? hit(idx) : hit
      if (picked === undefined) return undefined
      return { object: () => picked.object, instance: () => undefined, face: () => picked.face, depth: () => 0, free: vi.fn() }
    })
    const face_normal = vi.fn((object: bigint, face: bigint) => {
      const p = planes.get(`${object}:${face}`)
      if (p === undefined) throw new Error(`no plane for ${object}:${face}`)
      return new Float64Array(p.normal)
    })
    const face_plane = vi.fn((object: bigint, face: bigint) => {
      const p = planes.get(`${object}:${face}`)
      if (p === undefined) throw new Error(`no plane for ${object}:${face}`)
      return new Float64Array([...p.point, ...p.normal])
    })
    return { pick_face, face_normal, face_plane } as unknown as WasmScene
  }

  it('a miss on the exact ray, a ring hit whose plane contains the through point, is returned', () => {
    const scene = makeScene(
      { object: 7n, face: 3n },
      new Map([['7:3', { point: [0, 0, 0], normal: [0, 0, 1] }]]),
    )
    const cache = new FacePickCache()
    const result = cache.faceThrough(scene, RAY, eligible, null, [[0, 0, 0]])
    expect(result).toEqual({ object: 7n, face: 3n, point: [0, 0, 0], normal: [0, 0, 1] })
  })

  it('a ring hit whose plane does NOT contain the through point is rejected', () => {
    const scene = makeScene(
      { object: 7n, face: 3n },
      new Map([['7:3', { point: [0, 0, 0], normal: [0, 0, 1] }]]),
    )
    const cache = new FacePickCache()
    // The through point sits 5m off the z=0 plane the only candidate lives on.
    const result = cache.faceThrough(scene, RAY, eligible, null, [[0, 0, 5]])
    expect(result).toBeNull()
  })

  it('two candidates through the point: the more camera-facing one wins', () => {
    // Object 10/face 1: a vertical plane through the origin (normal +X) —
    // facing the ray [0,0,-1] edge-on (facing score 0). Object 20/face 2: a
    // horizontal plane through the origin (normal +Z) — facing the camera
    // dead-on (facing score 1, since -ray.direction is +Z). Both planes pass
    // through the origin, so the through point [0,0,0] lies on both.
    const planes = new Map([
      ['10:1', { point: [0, 0, 0] as V3, normal: [1, 0, 0] as V3 }],
      ['20:2', { point: [0, 0, 0] as V3, normal: [0, 0, 1] as V3 }],
    ])
    const scene = makeScene((idx) => (idx === 1 ? { object: 10n, face: 1n } : { object: 20n, face: 2n }), planes)
    const cache = new FacePickCache()
    const result = cache.faceThrough(scene, RAY, eligible, null, [[0, 0, 0]])
    expect(result).toEqual({ object: 20n, face: 2n, point: [0, 0, 0], normal: [0, 0, 1] })
  })

  it('X-ray: a point hidden behind a nearer face still finds the face it lies on; Wireframe adopts no face', () => {
    // `pick_face` sees only the face in front (z = 1, which does not hold
    // the point at z = 0) — or nothing at all in Wireframe. In X-ray the
    // geometry pick, started just short of the point, reaches the face under
    // it; in Wireframe faces are no input, so nothing is adopted.
    const planes = new Map([
      ['1:1', { point: [0, 0, 1] as V3, normal: [0, 0, 1] as V3 }],
      ['2:2', { point: [0, 0, 0] as V3, normal: [0, 0, 1] as V3 }],
    ])
    const sceneWith = (front: { object: bigint; face: bigint } | undefined) => {
      const scene = makeScene(() => front, planes) as unknown as Record<string, unknown>
      const origins: number[] = []
      scene.pick_face_geometry = vi.fn((_ox: number, _oy: number, oz: number) => {
        origins.push(oz)
        // Started past the front face: the face holding the point.
        return oz < 1
          ? { object: () => 2n, instance: () => undefined, face: () => 2n, depth: () => 0, free: vi.fn() }
          : { object: () => 1n, instance: () => undefined, face: () => 1n, depth: () => 0, free: vi.fn() }
      })
      return { scene: scene as unknown as WasmScene, origins }
    }
    try {
      setPickRenderMode('xray')
      const { scene, origins } = sceneWith({ object: 1n, face: 1n })
      const result = new FacePickCache().faceThrough(scene, RAY, eligible, null, [[0, 0.001, 0]])
      expect(result).toMatchObject({ object: 2n, face: 2n, normal: [0, 0, 1] })
      // Every fallback probe starts 2% short of the point, past the z = 1 face.
      expect(origins.length).toBeGreaterThan(0)
      for (const oz of origins) expect(oz).toBeCloseTo(0.1, 2)

      setPickRenderMode('wireframe')
      const wire = sceneWith(undefined)
      expect(new FacePickCache().faceThrough(wire.scene, RAY, eligible, null, [[0, 0.001, 0]])).toBeNull()
      expect(wire.origins).toHaveLength(0)
    } finally {
      setPickRenderMode('shaded')
    }
  })

  it('a pick memoized for a ray is not served after a render-mode switch', () => {
    const planes = new Map([['7:3', { point: [0, 0, 0] as V3, normal: [0, 0, 1] as V3 }]])
    let hit: { object: bigint; face: bigint } | undefined = { object: 7n, face: 3n }
    const scene = makeScene(() => hit, planes) as unknown as Record<string, unknown>
    // The plain pick answers the exact ray too here.
    scene.pick_face = vi.fn(() =>
      hit && { object: () => hit!.object, instance: () => undefined, face: () => hit!.face, depth: () => 1, free: vi.fn() },
    )
    const cache = new FacePickCache()
    try {
      expect(cache.pickFor(scene as unknown as WasmScene, RAY, eligible)).toEqual({ object: 7n, face: 3n })
      // Wireframe: the kernel picks no face for the same held ray.
      hit = undefined
      expect(cache.pickFor(scene as unknown as WasmScene, RAY, eligible), 'same mode: memoized').toEqual({ object: 7n, face: 3n })
      setPickRenderMode('wireframe')
      expect(cache.pickFor(scene as unknown as WasmScene, RAY, eligible), 'after the switch: picked afresh').toBeNull()
      expect(cache.faceThrough(scene as unknown as WasmScene, RAY, eligible, null, [[0, 0, 0]])).toBeNull()
    } finally {
      setPickRenderMode('shaded')
    }
  })

  it('a visible point never takes the geometry fallback', () => {
    const planes = new Map([['7:3', { point: [0, 0, 0] as V3, normal: [0, 0, 1] as V3 }]])
    const scene = makeScene({ object: 7n, face: 3n }, planes) as unknown as Record<string, unknown>
    scene.pick_face_geometry = vi.fn()
    new FacePickCache().faceThrough(scene as unknown as WasmScene, RAY, eligible, null, [[0, 0, 0]])
    expect(scene.pick_face_geometry).not.toHaveBeenCalled()
  })
})

describe('cursorOnFrozenPlane (a frozen draw plane meeting an off-plane snap)', () => {
  const ray = { origin: [0.2, 0.3, 5] as V3, direction: [0, 0, -1] as V3 }
  const snap = (x: number, y: number, z: number) => ({ x, y, z, kind: 'midpoint' }) as unknown as Parameters<typeof cursorOnFrozenPlane>[0]

  it('takes an on-plane snap exactly, not projected', () => {
    expect(cursorOnFrozenPlane(snap(1, 2, 1), ray, [0, 0, 1], [0, 0, 1])).toEqual({ point: [1, 2, 1], projected: false })
  })

  it('drops an off-plane snap (hidden geometry offered in X-ray) straight onto the plane, disclosed', () => {
    const at = cursorOnFrozenPlane(snap(1, 0.7, 0.6), ray, [0, 0, 1], [0, 0, 1])
    expect(at?.projected).toBe(true)
    expect(at?.point[0]).toBeCloseTo(1)
    expect(at?.point[1]).toBeCloseTo(0.7)
    expect(at?.point[2]).toBeCloseTo(1)
    // A tilted plane: the drop runs along its normal.
    const n: V3 = [0, Math.SQRT1_2, Math.SQRT1_2]
    const tilted = cursorOnFrozenPlane(snap(0, 1, 1), ray, [0, 0, 0], n)
    expect(tilted?.projected).toBe(true)
    expect(Math.abs(tilted!.point[1] * n[1] + tilted!.point[2] * n[2])).toBeLessThan(1e-12)
  })

  it('with no snap, the ray hit; parallel rays miss', () => {
    expect(cursorOnFrozenPlane(null, ray, [0, 0, 1], [0, 0, 1])).toEqual({ point: [0.2, 0.3, 1], projected: false })
    expect(cursorOnFrozenPlane(null, ray, [0, 0, 0], [1, 0, 0])).toBeNull()
  })
})

// ---------------------------------------------------------------------------
// The face the cursor leans toward (view-plane design §4, OW-10).
//
// These run `faceThrough` against a small real ray caster — convex planar
// faces with the kernel's strict boundary test — so the ring probes, the
// cursor's own ray, and the facing rule all meet genuine geometry. Leans are
// built from the CAMERA's screen basis (right = forward × Z, up = right ×
// forward), never from faceDraw's ring frame, so a ring/lean basis that
// disagreed in handedness or orientation would fail them (§12 lens 4).
// ---------------------------------------------------------------------------

type Vec = [number, number, number]
const vsub = (a: Vec, b: Vec): Vec => [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
const vadd = (a: Vec, b: Vec): Vec => [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
const vscale = (a: Vec, k: number): Vec => [a[0] * k, a[1] * k, a[2] * k]
const vdot = (a: Vec, b: Vec): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
const vcross = (a: Vec, b: Vec): Vec => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
const vnorm = (a: Vec): Vec => vscale(a, 1 / Math.hypot(a[0], a[1], a[2]))

interface GeoFace { object: bigint; face: bigint; verts: Vec[] }

/** A scene double that really casts: `pick_face` returns the nearest face
 *  whose polygon the ray crosses STRICTLY inside (a ray through an edge or
 *  corner misses, as the kernel's does), and `face_plane`/`face_normal`
 *  answer from the polygons (normal from the vertex winding). */
function castingScene(faces: GeoFace[]) {
  const normalOf = (f: GeoFace): Vec => vnorm(vcross(vsub(f.verts[1], f.verts[0]), vsub(f.verts[2], f.verts[0])))
  const byId = new Map(faces.map((f) => [`${f.object}:${f.face}`, f]))
  const pick_face = vi.fn((ox: number, oy: number, oz: number, dx: number, dy: number, dz: number) => {
    const o: Vec = [ox, oy, oz]
    const d: Vec = vnorm([dx, dy, dz])
    let best: { f: GeoFace; t: number } | null = null
    for (const f of faces) {
      const n = normalOf(f)
      const den = vdot(d, n)
      if (Math.abs(den) < 1e-12) continue
      const t = vdot(vsub(f.verts[0], o), n) / den
      if (t <= 0) continue
      const hit = vadd(o, vscale(d, t))
      let inside = true
      for (let i = 0; i < f.verts.length; i++) {
        const a = f.verts[i]
        const b = f.verts[(i + 1) % f.verts.length]
        if (vdot(vcross(vsub(b, a), vsub(hit, a)), n) <= 1e-12) { inside = false; break }
      }
      if (inside && (best === null || t < best.t)) best = { f, t }
    }
    if (best === null) return undefined
    const hitFace = best
    return { object: () => hitFace.f.object, face: () => hitFace.f.face, instance: () => undefined, depth: () => hitFace.t, free: vi.fn() }
  })
  const face_normal = vi.fn((object: bigint, face: bigint) => new Float64Array(normalOf(byId.get(`${object}:${face}`)!)))
  const face_plane = vi.fn((object: bigint, face: bigint) => {
    const f = byId.get(`${object}:${face}`)!
    return new Float64Array([...f.verts[0], ...normalOf(f)])
  })
  return { pick_face, face_normal, face_plane, node_parent: vi.fn(() => undefined) } as unknown as WasmScene
}

const CUBE = 7n
const FRONT = 1n
const BACK = 2n
const LEFT = 3n
const RIGHT = 4n
const BOTTOM = 5n
const TOP_FACE = 6n
/** The unit cube [0,1]³, each face wound counter-clockwise about its outward normal. */
const CUBE_FACES: GeoFace[] = [
  { object: CUBE, face: FRONT, verts: [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]] }, // y = 0, −Y
  { object: CUBE, face: BACK, verts: [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]] }, // y = 1, +Y
  { object: CUBE, face: LEFT, verts: [[0, 0, 0], [0, 0, 1], [0, 1, 1], [0, 1, 0]] }, // x = 0, −X
  { object: CUBE, face: RIGHT, verts: [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]] }, // x = 1, +X
  { object: CUBE, face: BOTTOM, verts: [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]] }, // z = 0, −Z
  { object: CUBE, face: TOP_FACE, verts: [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]] }, // z = 1, +Z
]

/** A camera at `eye` looking at `target`, with the screen basis three.js
 *  gives it for an up of world Z (Y when looking straight up or down). */
function camera(eye: Vec, target: Vec) {
  const f = vnorm(vsub(target, eye))
  const up: Vec = Math.abs(f[2]) > 0.999999 ? [0, 1, 0] : [0, 0, 1]
  const right = vnorm(vcross(f, up))
  const screenUp = vcross(right, f)
  /** The cursor ray `size` radians from the snapped point `p`, toward
   *  screen angle `deg` (0 = screen-right, 90 = screen-up). `size` 0 puts
   *  the cursor exactly on `p`. */
  const rayNear = (p: Vec, deg: number, size: number): Ray => {
    const a = (deg * Math.PI) / 180
    const dirP = vnorm(vsub(p, eye))
    const offset = vadd(vscale(right, Math.cos(a) * size), vscale(screenUp, Math.sin(a) * size))
    return { origin: [...eye], direction: vnorm(vadd(dirP, offset)) }
  }
  return { eye, f, right, screenUp, rayNear }
}

/** The home view (eye offset [3.5, −3, 2.5] from the target, math.ts),
 *  framing the unit cube from twice the home distance. */
const HOME = camera(vadd([0.5, 0.5, 0.5], vscale([3.5, -3.0, 2.5], 2)), [0.5, 0.5, 0.5])
/** About 4.5 px of lean at this distance on a 720 px canvas — well inside
 *  the 8 px snap radius, well outside the 1.5 px dead zone. */
const LEAN = 0.006
const ALL = () => true

const facePicked = (pick: { object: bigint; face: bigint } | null) => (pick === null ? null : pick.face)

describe('FacePickCache.faceThrough — the face the cursor leans toward (view-plane §4)', () => {
  // Screen angles of the cube's edges from the home view, measured from the
  // camera basis:
  //   the upper-front-LEFT corner L = (0,0,1), on the silhouette:
  //     front face (y = 0) wedge −88°…−29°, top face −29°…+20°, the rest
  //     (+20° round through 180° to −88°) is off the cube;
  //   the near upper-front-right corner N = (1,0,1), inside the silhouette:
  //     front 151°…270°, right (x = 1) −90°…+22°, top +22°…151°.
  const L: Vec = [0, 0, 1]
  const N: Vec = [1, 0, 1]

  function through(cache: FacePickCache, scene: WasmScene, ray: Ray, p: Vec) {
    // The Viewport's order per event: `snapConstraint` picks the plain ray
    // first, then the tool asks for the boundary face at the snapped point.
    cache.pickFor(scene, ray, ALL)
    return cache.faceThrough(scene, ray, ALL, null, [p])
  }

  it('leaning into each face at the near corner picks that face', () => {
    const scene = castingScene(CUBE_FACES)
    expect(facePicked(through(new FacePickCache(), scene, HOME.rayNear(N, 210, LEAN), N))).toBe(FRONT)
    expect(facePicked(through(new FacePickCache(), scene, HOME.rayNear(N, 85, LEAN), N))).toBe(TOP_FACE)
    expect(facePicked(through(new FacePickCache(), scene, HOME.rayNear(N, -35, LEAN), N))).toBe(RIGHT)
  })

  it('at the silhouette corner: down-into-front gives front, up-into-top gives top', () => {
    const scene = castingScene(CUBE_FACES)
    expect(facePicked(through(new FacePickCache(), scene, HOME.rayNear(L, -60, LEAN), L))).toBe(FRONT)
    expect(facePicked(through(new FacePickCache(), scene, HOME.rayNear(L, -5, LEAN), L))).toBe(TOP_FACE)
  })

  it('off the cube, the nearest face\'s plane extended: left of the corner takes the front (side) face, above it the top', () => {
    const scene = castingScene(CUBE_FACES)
    const left = HOME.rayNear(L, 180, LEAN)
    const above = HOME.rayNear(L, 90, LEAN)
    // Neither cursor ray touches the cube: only the ring at the corner can answer.
    expect(new FacePickCache().pickFor(scene, left, ALL)).toBeNull()
    expect(new FacePickCache().pickFor(scene, above, ALL)).toBeNull()
    const leftPick = through(new FacePickCache(), scene, left, L)
    expect(facePicked(leftPick)).toBe(FRONT)
    expect(leftPick?.normal).toEqual([0, -1, 0])
    expect(facePicked(through(new FacePickCache(), scene, above, L))).toBe(TOP_FACE)
    // Down and off to the lower left, still nearer the front face's wedge.
    expect(facePicked(through(new FacePickCache(), scene, HOME.rayNear(L, 250, LEAN), L))).toBe(FRONT)
  })

  it('the face under the cursor wins even where the nearest ring probe struck its neighbour', () => {
    // −25° is inside the top face's wedge (−29°…+20°) but nearer the −45°
    // probe, which struck the front face, than the 0° probe on the top.
    const scene = castingScene(CUBE_FACES)
    const ray = HOME.rayNear(L, -25, LEAN)
    expect(new FacePickCache().pickFor(scene, ray, ALL)).toEqual({ object: CUBE, face: TOP_FACE })
    expect(facePicked(through(new FacePickCache(), scene, ray, L))).toBe(TOP_FACE)
  })

  it('zero offset: the face hovered before the snap acquired (came from), on both sides', () => {
    const scene = castingScene(CUBE_FACES)
    const atCorner = () => HOME.rayNear(N, 0, 0)
    for (const [deg, face] of [[210, FRONT], [85, TOP_FACE], [-35, RIGHT]] as const) {
      const cache = new FacePickCache()
      // A plain hover over the face, well away from the corner (not a boundary snap).
      expect(cache.pickFor(scene, HOME.rayNear(N, deg, 0.05), ALL)?.face).toBe(face)
      expect(facePicked(through(cache, scene, atCorner(), N)), `came from ${face}`).toBe(face)
    }
  })

  it('zero offset with no hover memory: the facing rule', () => {
    const scene = castingScene(CUBE_FACES)
    // From the home view the right face (+X) faces the camera most at N
    // (|d·x| ≈ 0.66 beats front ≈ 0.57 and top ≈ 0.47).
    expect(facePicked(through(new FacePickCache(), scene, HOME.rayNear(N, 0, 0), N))).toBe(RIGHT)
    // Inside the dead zone the lean is ignored even though it is nonzero.
    expect(facePicked(through(new FacePickCache(), scene, HOME.rayNear(N, 85, PICK_NUDGE_RAD * 0.5), N))).toBe(RIGHT)
  })

  it('a hover over empty space forgets the face hovered before it', () => {
    const scene = castingScene(CUBE_FACES)
    const cache = new FacePickCache()
    cache.pickFor(scene, HOME.rayNear(N, 85, 0.05), ALL) // over the top face
    const empty = HOME.rayNear(L, 160, 0.2) // well off the cube
    expect(cache.pickFor(scene, empty, ALL)).toBeNull()
    expect(facePicked(through(cache, scene, HOME.rayNear(N, 0, 0), N))).toBe(RIGHT) // facing, not top
  })

  it('a rounding hit on a boundary event never overwrites where the cursor came from', () => {
    const scene = castingScene(CUBE_FACES)
    const cache = new FacePickCache()
    cache.pickFor(scene, HOME.rayNear(N, 210, 0.05), ALL) // came from the front face
    // Two boundary events inside the dead zone whose plain rays land on the top face.
    for (let i = 0; i < 2; i++) {
      const ray = HOME.rayNear(N, 85, PICK_NUDGE_RAD * 0.5)
      expect(cache.pickFor(scene, ray, ALL)?.face).toBe(TOP_FACE)
      expect(facePicked(cache.faceThrough(scene, ray, ALL, null, [N]))).toBe(FRONT)
    }
  })

  it('…nor when the boundary face is asked for before the plain pick of the same event', () => {
    // `ineligibleFacePlane` and a tool's `_facePickAt` can ask `faceThrough`
    // first; the plain pick that follows on the same ray is still a
    // boundary event's and must not become the memory.
    const scene = castingScene(CUBE_FACES)
    const cache = new FacePickCache()
    cache.pickFor(scene, HOME.rayNear(N, 210, 0.05), ALL) // came from the front face
    for (let i = 0; i < 2; i++) {
      const ray = HOME.rayNear(N, 85, PICK_NUDGE_RAD * 0.5)
      expect(facePicked(cache.faceThrough(scene, ray, ALL, null, [N]))).toBe(FRONT)
      expect(cache.pickFor(scene, ray, ALL)?.face).toBe(TOP_FACE)
    }
  })

  it('the memo never serves the old face once the hover memory has changed under a reused ray (§12 lens 2)', () => {
    const scene = castingScene(CUBE_FACES)
    const cache = new FacePickCache()
    cache.pickFor(scene, HOME.rayNear(N, 210, 0.05), ALL) // front
    const held = HOME.rayNear(N, 0, 0)
    expect(facePicked(through(cache, scene, held, N))).toBe(FRONT)
    // The cursor moves over the top face for two events, then a key press
    // re-resolves against the held ray.
    cache.pickFor(scene, HOME.rayNear(N, 85, 0.05), ALL)
    cache.pickFor(scene, HOME.rayNear(N, 80, 0.05), ALL)
    expect(facePicked(cache.faceThrough(scene, held, ALL, null, [N]))).toBe(TOP_FACE)
    // clear() forgets it: the facing rule.
    cache.clear()
    expect(facePicked(cache.faceThrough(scene, held, ALL, null, [N]))).toBe(RIGHT)
  })

  describe('on an edge (two faces)', () => {
    const M: Vec = [0.5, 0, 1] // the front-top edge's midpoint

    it('leans into either face, both sides of the edge', () => {
      const scene = castingScene(CUBE_FACES)
      expect(facePicked(through(new FacePickCache(), scene, HOME.rayNear(M, -90, LEAN), M))).toBe(FRONT)
      expect(facePicked(through(new FacePickCache(), scene, HOME.rayNear(M, 90, LEAN), M))).toBe(TOP_FACE)
    })

    it('zero offset: came from, else facing', () => {
      const scene = castingScene(CUBE_FACES)
      const cache = new FacePickCache()
      // Over the top face (it spans only ~0.045 rad on screen from here).
      expect(cache.pickFor(scene, HOME.rayNear(M, 90, 0.02), ALL)?.face).toBe(TOP_FACE)
      expect(facePicked(through(cache, scene, HOME.rayNear(M, 0, 0), M))).toBe(TOP_FACE)
      // And from the front face, the other side of the edge.
      const fromFront = new FacePickCache()
      expect(fromFront.pickFor(scene, HOME.rayNear(M, -90, 0.02), ALL)?.face).toBe(FRONT)
      expect(facePicked(through(fromFront, scene, HOME.rayNear(M, 0, 0), M))).toBe(FRONT)
      // No memory: the front face faces the home camera more than the top does.
      expect(facePicked(through(new FacePickCache(), scene, HOME.rayNear(M, 0, 0), M))).toBe(FRONT)
    })
  })

  describe('ring frame vs lean agree on every camera (§12 lens 4)', () => {
    // A roof: two faces meeting along a ridge on x ∈ [0, 1] at y = 0.5,
    // z = 1, falling to z = 0.5 at y = 0 (face A) and y = 1 (face B). The
    // snapped point is the ridge's end, P = (1, 0.5, 1); a cursor off the
    // end of the roof toward +Y is nearest B's wedge, toward −Y A's.
    // Mirroring the lean against the ring would swap them.
    const ROOF = 9n
    const A = 1n
    const B = 2n
    const ROOF_FACES: GeoFace[] = [
      { object: ROOF, face: A, verts: [[0, 0, 0.5], [1, 0, 0.5], [1, 0.5, 1], [0, 0.5, 1]] },
      { object: ROOF, face: B, verts: [[0, 0.5, 1], [1, 0.5, 1], [1, 1, 0.5], [0, 1, 0.5]] },
    ]
    const P: Vec = [1, 0.5, 1]
    /** The cursor ray from `eye` leaning `size` rad from P toward the world
     *  point `toward` — a lean defined in the world, not on any screen. */
    const leanToward = (eye: Vec, toward: Vec, size: number): Ray => {
      const dirP = vnorm(vsub(P, eye))
      const dirT = vnorm(vsub(toward, eye))
      const perp = vnorm(vsub(dirT, vscale(dirP, vdot(dirT, dirP))))
      return { origin: [...eye], direction: vnorm(vadd(dirP, vscale(perp, size))) }
    }
    const offRoofPlusY: Vec = [1.3, 0.8, 1.15]
    const offRoofMinusY: Vec = [1.3, 0.2, 1.15]

    const cameras: [string, Vec][] = [
      ['Top', [0.5, 0.5 - 0.001 * 10, 11]],
      ['Bottom', [0.5, 0.5 - 0.001 * 10, -9]],
      ['just inside the Z-up branch (|d_z| ≈ 0.989)', [1 - 1.5, 0.5, 1 + 10]],
      ['just inside the Y-up branch (|d_z| ≈ 0.991)', [1 - 1.3, 0.5, 1 + 10]],
      // Low and off the gable end, where the ridge's end is a silhouette
      // corner: up-and-sideways from it is off the roof.
      ['a low view off the gable end', [4, 1.5, 1.4]],
    ]

    it.each(cameras)('%s', (_name, eye) => {
      const scene = castingScene(ROOF_FACES)
      const plus = leanToward(eye, offRoofPlusY, LEAN)
      const minus = leanToward(eye, offRoofMinusY, LEAN)
      // Off the roof: the plain cursor rays miss, so the ring decides.
      expect(new FacePickCache().pickFor(scene, plus, ALL)).toBeNull()
      expect(new FacePickCache().pickFor(scene, minus, ALL)).toBeNull()
      expect(facePicked(through(new FacePickCache(), scene, plus, P))).toBe(B)
      expect(facePicked(through(new FacePickCache(), scene, minus, P))).toBe(A)
    })

    it('the two branch-edge cameras really straddle the up-reference switch', () => {
      const dz = (eye: Vec) => Math.abs(vnorm(vsub(P, eye))[2])
      expect(dz(cameras[2][1])).toBeLessThan(0.99)
      expect(dz(cameras[3][1])).toBeGreaterThan(0.99)
    })
  })

  it('the ineligible variant on a grouped cube answers the same way (ineligibleFacePlane)', () => {
    const scene = castingScene(CUBE_FACES)
    const grouped = () => false
    const snapAt = (p: Vec) => ({ x: p[0], y: p[1], z: p[2], kind: 'endpoint', object: CUBE, elementKind: 'vertex' }) as unknown as Snap
    const plane = (ray: Ray, p: Vec, cache = new FacePickCache()) => {
      cache.pickFor(scene, ray, grouped)
      return ineligibleFacePlane(scene, cache, ray, grouped, snapAt(p))
    }
    expect(plane(HOME.rayNear(N, 210, LEAN), N)?.normal).toEqual([0, -1, 0])
    expect(plane(HOME.rayNear(N, 85, LEAN), N)?.normal).toEqual([0, 0, 1])
    expect(plane(HOME.rayNear(L, 180, LEAN), L)?.normal).toEqual([0, -1, 0]) // off the cube: front extended
    expect(plane(HOME.rayNear(L, 90, LEAN), L)?.normal).toEqual([0, 0, 1])
    // Came from, on a grouped face.
    const cache = new FacePickCache()
    cache.pickFor(scene, HOME.rayNear(N, 85, 0.05), grouped)
    expect(plane(HOME.rayNear(N, 0, 0), N, cache)?.normal).toEqual([0, 0, 1])
    expect(plane(HOME.rayNear(N, 0, 0), N)?.normal).toEqual([1, 0, 0]) // no memory: facing
  })
})

const PERSPECTIVE_APERTURE = { kind: 'perspective', fovYDeg: 45 } as const

describe('pickNudgeRad — 1.5 px on screen in either projection', () => {
  afterEach(() => setPickAperture(PERSPECTIVE_APERTURE, 720))

  it('perspective is the constant, bit for bit, at any distance', () => {
    setPickAperture({ kind: 'perspective', fovYDeg: 30 }, 1400)
    for (const d of [0, 1e-9, 0.5, 10, 1e6]) expect(pickNudgeRad(d)).toBe(PICK_NUDGE_RAD)
    expect(PICK_NUDGE_RAD).toBe(0.002)
  })

  it('parallel spans PICK_NUDGE_PX world-per-pixel widths at the point, so it shrinks with distance', () => {
    setPickAperture({ kind: 'parallel', worldPerPixel: 0.0014 }, 900)
    expect(PICK_NUDGE_PX).toBe(1.5)
    expect(pickNudgeRad(10)).toBeCloseTo((1.5 * 0.0014) / 10, 15)
    expect(pickNudgeRad(2)).toBeCloseTo((1.5 * 0.0014) / 2, 15)
    // A point at the eye stays finite (the distance is floored).
    expect(pickNudgeRad(0)).toBeCloseTo((1.5 * 0.0014) / 1e-6, 6)
    expect(Number.isFinite(pickNudgeRad(0))).toBe(true)
  })

  it('a later aperture replaces the earlier one', () => {
    setPickAperture({ kind: 'parallel', worldPerPixel: 0.0014 }, 900)
    setPickAperture(PERSPECTIVE_APERTURE, 900)
    expect(pickNudgeRad(10)).toBe(PICK_NUDGE_RAD)
  })
})

describe('FacePickCache.faceThrough under parallel projection (the lean in pixels, not radians)', () => {
  const WPP = 0.0014
  const D = 10
  const N: Vec = [1, 0, 1] // the near upper-front-right corner from the home direction
  beforeEach(() => setPickAperture({ kind: 'parallel', worldPerPixel: WPP }, 900))
  afterEach(() => setPickAperture(PERSPECTIVE_APERTURE, 720))

  /** A parallel-projection cursor ray: every ray runs along the view
   *  direction, from an origin `D` meters back, offset on screen from the
   *  snapped point `p` by `px` pixels toward screen angle `deg`. */
  const orthoRayNear = (p: Vec, deg: number, px: number): Ray => {
    const a = (deg * Math.PI) / 180
    const offset = vadd(vscale(HOME.right, Math.cos(a) * px * WPP), vscale(HOME.screenUp, Math.sin(a) * px * WPP))
    return { origin: vadd(vsub(p, vscale(HOME.f, D)), offset), direction: [...HOME.f] }
  }
  const through = (scene: WasmScene, ray: Ray, p: Vec, cache = new FacePickCache()) => {
    cache.pickFor(scene, ray, ALL)
    return cache.faceThrough(scene, ray, ALL, null, [p])
  }

  it('an 8 px lean into the front face is a lean (rule 1), though it is far under the perspective dead zone in radians', () => {
    const scene = castingScene(CUBE_FACES)
    const ray = orthoRayNear(N, 210, 8)
    // The lean's angle at the point is about 0.0011 rad: inside 0.002.
    const dirP = vnorm(vsub(N, ray.origin))
    expect(Math.hypot(...vsub(HOME.f, dirP))).toBeLessThan(PICK_NUDGE_RAD)
    expect(facePicked(through(scene, ray, N))).toBe(FRONT)
    expect(facePicked(through(scene, orthoRayNear(N, 85, 8), N))).toBe(TOP_FACE)
  })

  it('a 1 px offset is inside the 1.5 px dead zone: no memory, so the facing rule', () => {
    const scene = castingScene(CUBE_FACES)
    expect(facePicked(through(scene, orthoRayNear(N, 210, 1), N))).toBe(RIGHT)
  })

  it('the ring probes land 1.5 px (in world widths) from the snapped point at its depth', () => {
    const scene = castingScene(CUBE_FACES)
    const ray = orthoRayNear(N, 210, 8)
    const cache = new FacePickCache()
    cache.pickFor(scene, ray, ALL)
    const pick = scene.pick_face as unknown as ReturnType<typeof vi.fn>
    pick.mockClear()
    cache.faceThrough(scene, ray, ALL, null, [N])
    const probes = pick.mock.calls.map((c) => c as number[])
    expect(probes.length).toBe(8) // the plain ray's pick was memoized
    const depth = Math.hypot(...vsub(N, ray.origin))
    for (const [ox, oy, oz, dx, dy, dz] of probes) {
      expect([ox, oy, oz]).toEqual(ray.origin)
      const d = vnorm([dx, dy, dz])
      // The probe's point abreast of N (its foot on the plane through N
      // square to the direction to N).
      const dirP = vnorm(vsub(N, ray.origin))
      const along = depth / vdot(d, dirP)
      const at = vadd(ray.origin, vscale(d, along))
      const off = Math.hypot(...vsub(at, N))
      expect(off).toBeGreaterThan(PICK_NUDGE_PX * WPP * 0.999)
      expect(off).toBeLessThan(PICK_NUDGE_PX * WPP * 1.001)
    }
  })
})
