import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import { drawnPointVisible, meshWritesDepth, type DrawnVisibilityDeps } from './drawnVisibility'

type V3 = readonly [number, number, number]
type Cam = THREE.PerspectiveCamera | THREE.OrthographicCamera

const PX_H = 800

/** Camera `distance` in front of the origin looking along +y, scaled with
 *  the scene. */
function perspective(scale: number, center: V3 = [0, 0, 0]): THREE.PerspectiveCamera {
  const c = new THREE.PerspectiveCamera(45, 1, 0.1 * scale, 100 * scale)
  c.position.set(center[0], center[1] - 10 * scale, center[2])
  c.lookAt(center[0], center[1], center[2])
  c.updateMatrixWorld()
  return c
}

/** A parallel camera whose near plane is 10 km behind the model. */
function parallel(scale: number): THREE.OrthographicCamera {
  const h = 4 * scale
  const c = new THREE.OrthographicCamera(-h, h, h, -h, 0.1, 20_000 * scale)
  c.position.set(0, -10_000 * scale, 0)
  c.lookAt(0, 0, 0)
  c.updateMatrixWorld()
  return c
}

function box(
  sx: number, sy: number, sz: number, at: V3,
  mat: THREE.Material | THREE.Material[] = new THREE.MeshBasicMaterial(),
): THREE.Mesh {
  const m = new THREE.Mesh(new THREE.BoxGeometry(sx, sy, sz), mat)
  m.position.set(at[0], at[1], at[2])
  m.updateMatrixWorld(true)
  return m
}

const opaque = (): THREE.MeshBasicMaterial => new THREE.MeshBasicMaterial({ side: THREE.FrontSide })

function deps(camera: Cam, meshes: THREE.Object3D[], over: Partial<DrawnVisibilityDeps> = {}): DrawnVisibilityDeps {
  const rc = new THREE.Raycaster()
  return {
    camera,
    facesOcclude: true,
    worldPerPixelAt: (p) => {
      if (camera instanceof THREE.OrthographicCamera) return (camera.top - camera.bottom) / camera.zoom / PX_H
      const d = camera.position.distanceTo(new THREE.Vector3(...p))
      return (2 * d * Math.tan((camera.fov * Math.PI) / 360)) / PX_H
    },
    // The kernel ignores materials: a mesh at any state counts (sides honoured).
    nearestFaceDepth: (o, d) => {
      rc.set(new THREE.Vector3(...o), new THREE.Vector3(...d))
      const hits = rc.intersectObjects(meshes, false)
      return hits.length === 0 ? null : hits[0].distance
    },
    faceMeshes: () => meshes,
    ...over,
  }
}

describe('drawnPointVisible', () => {
  it('opaque box in front hides a point behind it; a point in front of it is visible', () => {
    const cam = perspective(1)
    const b = box(2, 2, 2, [0, 0, 0], opaque())
    const d = deps(cam, [b])
    expect(drawnPointVisible([0, 3, 0], d)).toBe(false)
    expect(drawnPointVisible([0, -3, 0], d)).toBe(true)
    expect(drawnPointVisible([3, 3, 0], d)).toBe(true)
  })

  it('a depthWrite:false material hides nothing; a material array is decided per group', () => {
    const cam = perspective(1)
    const glass = new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.4, depthWrite: false })
    expect(drawnPointVisible([0, 3, 0], deps(cam, [box(2, 2, 2, [0, 0, 0], glass)]))).toBe(true)
    // BoxGeometry groups 0..5 = +x,-x,+y,-y,+z,-z. The front face toward -y is group 3.
    const mats = [opaque(), opaque(), opaque(), opaque(), opaque(), opaque()]
    const arr = box(2, 2, 2, [0, 0, 0], mats)
    expect(drawnPointVisible([0, 3, 0], deps(cam, [arr]))).toBe(false)
    const mats2 = mats.map(() => opaque())
    mats2[3] = glass.clone()
    // Front (-y) face is see-through, but the +y back face is culled (FrontSide): visible.
    expect(drawnPointVisible([0, 3, 0], deps(cam, [box(2, 2, 2, [0, 0, 0], mats2)]))).toBe(true)
    // Glass only on an unrelated face: still hidden through the opaque front face.
    const mats3 = mats.map(() => opaque())
    mats3[0] = glass.clone()
    expect(drawnPointVisible([0, 3, 0], deps(cam, [box(2, 2, 2, [0, 0, 0], mats3)]))).toBe(false)
  })

  it('an invisible mesh or ancestor hides nothing; an invisible material hides nothing', () => {
    const cam = perspective(1)
    const b = box(2, 2, 2, [0, 0, 0], opaque())
    b.visible = false
    expect(drawnPointVisible([0, 3, 0], deps(cam, [b]))).toBe(true)
    const parent = new THREE.Group()
    const c = box(2, 2, 2, [0, 0, 0], opaque())
    parent.add(c)
    parent.visible = false
    parent.updateMatrixWorld(true)
    expect(drawnPointVisible([0, 3, 0], deps(cam, [c]))).toBe(true)
    const m = opaque()
    m.visible = false
    expect(drawnPointVisible([0, 3, 0], deps(cam, [box(2, 2, 2, [0, 0, 0], m)]))).toBe(true)
  })

  it('InstancedMesh: a real slot hides, a zero-scale slot does not', () => {
    const cam = perspective(1)
    const im = new THREE.InstancedMesh(new THREE.BoxGeometry(2, 2, 2), opaque(), 2)
    const m = new THREE.Matrix4()
    im.setMatrixAt(0, m.makeTranslation(0, 0, 0))
    im.setMatrixAt(1, m.makeScale(0, 0, 0).setPosition(5, 0, 0))
    im.computeBoundingSphere()
    im.updateMatrixWorld(true)
    const d = deps(cam, [im])
    expect(drawnPointVisible([0, 3, 0], d)).toBe(false)
    expect(drawnPointVisible([5, 3, 0], d)).toBe(true)
  })

  describe('section plane', () => {
    // Keep x <= 0: the plane normal points to the kept side.
    const plane = new THREE.Plane(new THREE.Vector3(-1, 0, 0), 0)
    const sectioned = (): THREE.Mesh => {
      const m = new THREE.MeshBasicMaterial({ side: THREE.FrontSide })
      m.clippingPlanes = [plane]
      return box(4, 2, 2, [0, 0, 0], m)
    }
    it('the cut-away part hides nothing; the kept part hides', () => {
      const cam = perspective(1)
      const d = deps(cam, [sectioned()])
      expect(drawnPointVisible([1, 3, 0], d)).toBe(true) // behind the cut-away half
      expect(drawnPointVisible([-1, 3, 0], d)).toBe(false) // behind the kept half
    })
    it('the double-sided far wall hides what is behind it', () => {
      const cam = perspective(1)
      const m = new THREE.MeshBasicMaterial({ side: THREE.DoubleSide })
      m.clippingPlanes = [plane]
      const b = box(4, 2, 2, [0, 0, 0], m)
      // Behind the cut-away half: the far +y wall (cut only where x > 0) is clipped too,
      // so a point behind the kept half is hidden by the near face, and one past the
      // far wall of the kept half is hidden as well.
      expect(drawnPointVisible([-1, 3, 0], deps(cam, [b]))).toBe(false)
      expect(drawnPointVisible([1, 3, 0], deps(cam, [b]))).toBe(true)
    })
  })

  for (const scale of [1e-3, 1, 1e3]) {
    it(`thin wall (0.05 px) hides, at scale ${scale}`, () => {
      const cam = perspective(scale)
      const wpp = (2 * 10 * scale * Math.tan((45 * Math.PI) / 360)) / PX_H
      const thick = 0.05 * wpp
      const wall = box(4 * scale, thick, 4 * scale, [0, 0, 0], opaque())
      const d = deps(cam, [wall])
      expect(drawnPointVisible([0, thick * 0.5 + 0.02 * wpp, 0], d)).toBe(false)
    })

    it(`a point on a face is visible, head-on and 0.4 degrees off, at scale ${scale}`, () => {
      const wall = box(4 * scale, 2 * scale, 4 * scale, [0, 0, 0], opaque())
      const cam = perspective(scale)
      expect(drawnPointVisible([0.3 * scale, -1 * scale, 0.2 * scale], deps(cam, [wall]))).toBe(true)
      // Camera 0.4 degrees off the face plane (grazing).
      const g = new THREE.PerspectiveCamera(45, 1, 0.1 * scale, 100 * scale)
      const a = (0.4 * Math.PI) / 180
      g.position.set(-10 * scale, -1 * scale - 10 * scale * Math.tan(a), 0.2 * scale)
      g.lookAt(0.3 * scale, -1 * scale, 0.2 * scale)
      g.updateMatrixWorld()
      // The camera now views the y=-1 face almost edge-on from the -x side.
      expect(drawnPointVisible([0.3 * scale, -1 * scale, 0.2 * scale], deps(g, [wall]))).toBe(true)
    })

    it(`parallel camera with the near plane 10 km behind: thin wall hides, coplanar is visible, at scale ${scale}`, () => {
      const cam = parallel(scale)
      const wpp = (8 * scale) / PX_H
      const thick = 0.05 * wpp
      const thin = box(4 * scale, thick, 4 * scale, [0, 0, 0], opaque())
      expect(drawnPointVisible([0, thick * 0.5 + 0.02 * wpp, 0], deps(cam, [thin]))).toBe(false)
      const wall = box(4 * scale, 2 * scale, 4 * scale, [0, 0, 0], opaque())
      expect(drawnPointVisible([0.3 * scale, -1 * scale, 0.2 * scale], deps(cam, [wall]))).toBe(true)
    })
  }

  it('a coplanar point seen through a glass pane, 1000 m from the origin, is visible', () => {
    const c: V3 = [1000, 1000, 1000]
    const cam = perspective(1, c)
    const glass = new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.3, depthWrite: false })
    const pane = box(2, 0.02, 2, [c[0], c[1] - 1, c[2]], glass)
    const wall = box(4, 1, 4, [c[0], c[1], c[2]], opaque())
    // Point on the wall's near face (y = c.y - 0.5), behind the glass pane.
    const p: V3 = [c[0] + 0.2, c[1] - 0.5, c[2] + 0.1]
    expect(drawnPointVisible(p, deps(cam, [pane, wall]))).toBe(true)
  })

  it('facesOcclude false: visible; outside the depth range: not visible', () => {
    const cam = perspective(1)
    const b = box(2, 2, 2, [0, 0, 0], opaque())
    expect(drawnPointVisible([0, 3, 0], deps(cam, [b], { facesOcclude: false }))).toBe(true)
    expect(drawnPointVisible([0, -10.05, 0], deps(cam, [b], { facesOcclude: false }))).toBe(false) // nearer than near
    expect(drawnPointVisible([0, -15, 0], deps(cam, [b], { facesOcclude: false }))).toBe(false) // behind the camera
    expect(drawnPointVisible([0, 500, 0], deps(cam, [b], { facesOcclude: false }))).toBe(false) // beyond far
  })

  it('a single-sided box with the camera inside it hides nothing', () => {
    const cam = new THREE.PerspectiveCamera(45, 1, 0.01, 100)
    cam.position.set(0, 0, 0)
    cam.lookAt(0, 5, 0)
    cam.updateMatrixWorld()
    const b = box(10, 10, 10, [0, 0, 0], opaque())
    expect(drawnPointVisible([0, 3, 0], deps(cam, [b]))).toBe(true)
  })
})

describe('meshWritesDepth', () => {
  it('takes FrontSide and DoubleSide; refuses BackSide, clipped, depthWrite:false and hidden materials', () => {
    const make = (init: Partial<THREE.MeshBasicMaterial>): THREE.Mesh =>
      box(1, 1, 1, [0, 0, 0], Object.assign(new THREE.MeshBasicMaterial(), init))
    expect(meshWritesDepth(make({ side: THREE.FrontSide }))).toBe(true)
    expect(meshWritesDepth(make({ side: THREE.DoubleSide }))).toBe(true)
    expect(meshWritesDepth(make({ side: THREE.BackSide }))).toBe(false)
    expect(meshWritesDepth(make({ depthWrite: false }))).toBe(false)
    expect(meshWritesDepth(make({ clippingPlanes: [new THREE.Plane(new THREE.Vector3(1, 0, 0), 0)] }))).toBe(false)
    expect(meshWritesDepth(make({ visible: false }))).toBe(false)
    const mixed = box(1, 1, 1, [0, 0, 0], [opaque(), opaque(), opaque(), opaque(), opaque(), new THREE.MeshBasicMaterial({ side: THREE.BackSide })])
    expect(meshWritesDepth(mixed)).toBe(false)
  })
})

describe('dense-mesh fast path agrees with the full cast', () => {
  it('on random boxes, instanced owners, mirrored back-side buckets, clipping, depthWrite and section state', () => {
    let seed = 12345
    const rnd = (): number => {
      seed = (seed * 1664525 + 1013904223) >>> 0
      return seed / 2 ** 32
    }
    const cam = perspective(1)
    const rc = new THREE.Raycaster()
    const sectionPlane = new THREE.Plane(new THREE.Vector3(-1, 0, 0), 0.5)
    let fastHidden = 0
    let fastTaken = 0
    let backSideHits = 0
    for (let round = 0; round < 120; round++) {
      const section = round % 4 === 3
      const meshes: THREE.Mesh[] = []
      for (let i = 0; i < 4; i++) {
        const r = rnd()
        const side = r < 0.5 ? THREE.FrontSide : r < 0.75 ? THREE.BackSide : THREE.DoubleSide
        const mat = new THREE.MeshBasicMaterial({ side })
        if (rnd() < 0.2) mat.depthWrite = false
        if (section || rnd() < 0.15) mat.clippingPlanes = [sectionPlane]
        const size: V3 = [0.5 + rnd() * 2, 0.5 + rnd() * 2, 0.5 + rnd() * 2]
        const at: V3 = [rnd() * 4 - 2, rnd() * 4 - 2, rnd() * 4 - 2]
        let m: THREE.Mesh
        if (rnd() < 0.4) {
          // An instanced owner; a BackSide one is the reflected bucket, so
          // its slots carry a mirror.
          const im = new THREE.InstancedMesh(new THREE.BoxGeometry(size[0], size[1], size[2]), mat, 2)
          const mirror = side === THREE.BackSide || rnd() < 0.3
          const mx = new THREE.Matrix4().makeScale(mirror ? -1 : 1, 1, 1).setPosition(at[0], at[1], at[2])
          im.setMatrixAt(0, mx)
          im.setMatrixAt(1, new THREE.Matrix4().makeScale(0, 0, 0).setPosition(9, 9, 9))
          im.computeBoundingSphere()
          im.updateMatrixWorld(true)
          m = im
        } else {
          m = box(size[0], size[1], size[2], at, mat)
        }
        if (rnd() < 0.1) m.visible = false
        meshes.push(m)
      }
      // The kernel ignores materials, visibility and winding: its stand-ins
      // are double-sided copies of every mesh that always draw.
      const solids: THREE.Mesh[] = meshes.map((m) => {
        const mat = new THREE.MeshBasicMaterial({ side: THREE.DoubleSide })
        const s: THREE.Mesh =
          m instanceof THREE.InstancedMesh
            ? new THREE.InstancedMesh(m.geometry, mat, m.count)
            : new THREE.Mesh(m.geometry, mat)
        if (m instanceof THREE.InstancedMesh && s instanceof THREE.InstancedMesh) {
          s.instanceMatrix = m.instanceMatrix
          s.computeBoundingSphere()
        }
        s.matrix.copy(m.matrix)
        s.matrixWorld.copy(m.matrixWorld)
        return s
      })
      const full = deps(cam, meshes)
      let ownerAsked = 0
      const fast = deps(cam, meshes, {
        nearestFace: (o, d) => {
          rc.set(new THREE.Vector3(...o), new THREE.Vector3(...d))
          const hit = rc.intersectObjects(solids, false)[0]
          if (hit === undefined || hit.face === null || hit.face === undefined) return null
          const world = hit.object.matrixWorld.clone()
          if (hit.instanceId !== undefined && hit.object instanceof THREE.InstancedMesh) {
            const im = new THREE.Matrix4()
            hit.object.getMatrixAt(hit.instanceId, im)
            world.multiply(im)
          }
          // The face's outward normal: a box's own normal, carried through the
          // slot's matrix (a mirror keeps a solid's outward side outward).
          const n = hit.face.normal.clone().transformDirection(world)
          const owner = solids.indexOf(hit.object as THREE.Mesh)
          return { depth: hit.distance, normal: [n.x, n.y, n.z], owner: { object: BigInt(owner), instance: undefined } }
        },
        sectionActive: () => section,
        ownerWritesDepth: (ow) => {
          ownerAsked++
          return meshWritesDepth(meshes[Number(ow.object)])
        },
      })
      for (let k = 0; k < 40; k++) {
        const p: V3 = [rnd() * 8 - 4, rnd() * 8 - 4, rnd() * 8 - 4]
        const a = drawnPointVisible(p, full)
        const b = drawnPointVisible(p, fast)
        expect(b).toBe(a)
        if (!a) fastHidden++
      }
      if (section) expect(ownerAsked, 'an active section plane never takes the fast path').toBe(0)
      else fastTaken += ownerAsked
      if (meshes.some((m) => (m.material as THREE.Material).side === THREE.BackSide)) backSideHits++
    }
    expect(fastHidden).toBeGreaterThan(20)
    expect(fastTaken).toBeGreaterThan(20)
    expect(backSideHits).toBeGreaterThan(20)
  })

  it('a BackSide owner met from the front by the kernel is left to the cast', () => {
    // A mirrored batch drawn BackSide: the kernel's front-facing nearest hit
    // must not decide "hidden" on its own.
    const cam = perspective(1)
    const mat = new THREE.MeshBasicMaterial({ side: THREE.BackSide })
    const b = box(2, 2, 2, [0, 0, 0], mat)
    const base = deps(cam, [b])
    const p: V3 = [0, 3, 0]
    const fast = deps(cam, [b], {
      nearestFace: () => ({ depth: 9, normal: [0, -1, 0], owner: { object: 1n, instance: undefined } }),
      sectionActive: () => false,
      ownerWritesDepth: () => meshWritesDepth(b),
    })
    expect(drawnPointVisible(p, fast)).toBe(drawnPointVisible(p, base))
  })
})
