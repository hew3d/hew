/**
 * drawnVisibility — one rule for "is this world point, as the camera draws it
 * right now, covered by a drawn face fragment that writes depth?".
 *
 * Guides, dimension lines and leader text are drawn depth-tested, so a solid
 * in front of one hides it; their pickers ask this before accepting a click.
 * The rule reads the RENDER meshes themselves (a ray cast over the solid face
 * meshes), so every state that decides whether a face hides what is behind it
 * is honoured with no second copy of the logic: the material's `depthWrite`
 * (glass, the edit-session dim, isolation and shop-mode fades, X-ray),
 * `visible` flags, section-plane `clippingPlanes`, and `material.side` (a
 * single-sided solid's back faces never hide). The kernel's face pick is only
 * an exact fast path for "nothing is in front at all".
 *
 * Pure: depends on `three` only, everything else is injected.
 */
import * as THREE from 'three'

type V3 = readonly [number, number, number]

/** Fraction of one screen pixel's world size, at the tested point, by which a
 *  face must be nearer than the point to hide it. Scale- and projection-
 *  independent: a thin wall hides what is behind it at any model scale, and a
 *  point lying ON a face (same depth up to rounding) is not hidden by it. */
export const VISIBILITY_PIXEL_FRACTION = 1e-3

export interface DrawnVisibilityDeps {
  camera: THREE.PerspectiveCamera | THREE.OrthographicCamera
  /** False in X-ray and Wireframe: no face writes depth. */
  facesOcclude: boolean
  /** World size of one screen pixel at a world point (CameraRig.worldPerPixel). */
  worldPerPixelAt(p: V3): number
  /** Radial depth of the nearest solid face of any owner along a ray,
   *  ignoring render mode and editing scope (wasm `pick_face_geometry`), or
   *  null. */
  nearestFaceDepth(origin: V3, dir: V3): number | null
  /** Every solid face mesh the renderer draws (objects, materialized
   *  instances, instanced batches). */
  faceMeshes(): THREE.Object3D[]
  /** Dense-mesh fast path (optional; absent = always cast). The nearest face
   *  along the ray with the facts the shortcut needs. */
  nearestFace?(origin: V3, dir: V3): NearestFace | null
  /** True while a section plane is active (any clipped material). */
  sectionActive?(): boolean
  /** Whether EVERY material of the face meshes drawing this owner is visible
   *  and writes depth (`meshWritesDepth`); false when the owner's meshes
   *  cannot be resolved. */
  ownerWritesDepth?(owner: FaceOwner): boolean
}

export interface FaceOwner {
  object: bigint
  instance: bigint | undefined
}

export interface NearestFace {
  depth: number
  /** World-space outward unit normal of the face hit. */
  normal: V3
  owner: FaceOwner
}

const raycaster = new THREE.Raycaster()
raycaster.layers.enableAll()

/** Float32 floor for step 6: vertex buffers and instance matrices are
 *  Float32, so a face can sit a few ulps off its true plane far from the
 *  origin. */
function f32Floor(p: THREE.Vector3): number {
  return 4 * 2 ** -23 * Math.max(Math.abs(p.x), Math.abs(p.y), Math.abs(p.z))
}

function ancestorsVisible(o: THREE.Object3D): boolean {
  for (let n: THREE.Object3D | null = o; n !== null; n = n.parent) if (!n.visible) return false
  return true
}

/** Whether every material of `mesh` is visible, writes depth, carries no
 *  clipping plane and is FrontSide or DoubleSide, and the mesh and its
 *  ancestors are visible: any ray that meets one of its faces from the
 *  outside is hidden by it. A BackSide material (the reflected-instance
 *  bucket) draws only faces turned away from the ray, so "met from its outward
 *  side" no longer implies "drawn": such a mesh is left to the full cast. */
export function meshWritesDepth(mesh: THREE.Object3D): boolean {
  if (!ancestorsVisible(mesh)) return false
  const mats = (mesh as THREE.Mesh).material
  const list = Array.isArray(mats) ? mats : [mats]
  if (list.length === 0) return false
  return list.every(
    (m) =>
      m !== undefined &&
      m.visible &&
      m.depthWrite &&
      (m.side === THREE.FrontSide || m.side === THREE.DoubleSide) &&
      (m.clippingPlanes === null || m.clippingPlanes === undefined || m.clippingPlanes.length === 0),
  )
}

/** Whether a hit on `mesh` is a drawn depth-writing fragment. */
function hitHides(hit: THREE.Intersection): boolean {
  const mesh = hit.object as THREE.Mesh
  if (!ancestorsVisible(mesh)) return false
  const mats = mesh.material
  const mat = Array.isArray(mats) ? mats[hit.face?.materialIndex ?? 0] : mats
  if (mat === undefined || !mat.visible || !mat.depthWrite) return false
  const planes = mat.clippingPlanes
  if (planes !== null && planes !== undefined) {
    for (const pl of planes) if (pl.distanceToPoint(hit.point) < 0) return false
  }
  return true
}

/** Whether the world point `p` is drawn unhidden by the current frame's faces. */
export function drawnPointVisible(p: V3, deps: DrawnVisibilityDeps): boolean {
  const { camera } = deps
  const world = new THREE.Vector3(p[0], p[1], p[2])
  // 1. In the depth range (view-space z: right in both projections).
  const viewDepth = -world.clone().applyMatrix4(camera.matrixWorldInverse).z
  if (viewDepth < camera.near || viewDepth > camera.far) return false
  // 2. Render mode.
  if (!deps.facesOcclude) return true
  // 3. The ray through the tested point, from its near-plane point.
  const ndc = world.clone().project(camera)
  const n = new THREE.Vector3(ndc.x, ndc.y, -1).unproject(camera)
  const toP = world.clone().sub(n)
  const dist = toP.length()
  if (dist === 0) return true
  const dir = toP.divideScalar(dist)
  // 4. Tolerance.
  const tol = VISIBILITY_PIXEL_FRACTION * deps.worldPerPixelAt(p)
  const limit = dist - tol
  if (limit <= 0) return true
  // 5. Fast path: nothing of any kind in front.
  const nA: V3 = [n.x, n.y, n.z]
  const dirA: V3 = [dir.x, dir.y, dir.z]
  if (deps.nearestFace !== undefined && deps.ownerWritesDepth !== undefined && deps.sectionActive !== undefined) {
    const hit = deps.nearestFace(nA, dirA)
    if (hit === null || hit.depth >= limit) return true
    // 5b. Dense-mesh fast path: the nearest kernel face is in front, met from
    // its outward side, and everything that draws its owner writes depth, so
    // the full mesh cast could only agree. No section plane may be active.
    // The depth gate is the cast's own (`tol` or the Float32 floor, whichever
    // is larger): a face nearer than the point by less than that is within
    // vertex-buffer rounding of it, and the cast would not count it, so the
    // two agree there and differ only by what the GPU could not tell apart.
    const facing = hit.normal[0] * dir.x + hit.normal[1] * dir.y + hit.normal[2] * dir.z
    if (
      facing < 0 &&
      hit.depth < dist - Math.max(tol, f32Floor(world)) &&
      !deps.sectionActive() &&
      deps.ownerWritesDepth(hit.owner)
    ) {
      return false
    }
  } else {
    const d = deps.nearestFaceDepth(nA, dirA)
    if (d === null || d >= limit) return true
  }
  // 6. Drawn occluders.
  raycaster.set(n, dir)
  raycaster.near = 0
  raycaster.far = dist - Math.max(tol, f32Floor(world))
  if (raycaster.far <= 0) return true
  const hits = raycaster.intersectObjects(deps.faceMeshes(), false)
  for (const hit of hits) if (hitHides(hit)) return false
  return true
}
