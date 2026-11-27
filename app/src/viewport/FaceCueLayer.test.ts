import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import type { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import { FACE_CUE_OUTLINE, FACE_CUE_TINT_OPACITY, FaceCueLayer, type FaceCueSource } from './FaceCueLayer'
import { FACE_CUE_MAX_VERTICES, loopSegments, mapThroughPose, trianglesInVertexRange, triangulatePlanarLoop } from './faceCueGeometry'
import type { HoverFace } from '../tools/types'
import { DEPTH_BIAS } from './depthPolicy'

const TRI = new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0])
const LOOP = new Float32Array([0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0])

function source(over: Partial<FaceCueSource> = {}): FaceCueSource {
  return {
    faceTriangles: vi.fn(() => TRI),
    regionTriangles: vi.fn(() => TRI),
    faceOutline: vi.fn(() => LOOP),
    regionOutline: vi.fn(() => LOOP),
    ...over,
  }
}

const face = (f: bigint, token: HoverFace['colorToken'] = '--preview-pushpull', instance: bigint | null = null): HoverFace => ({
  target: { kind: 'face', object: 3n, face: f, instance },
  colorToken: token,
})
const region: HoverFace = { target: { kind: 'region', sketch: 9n, region: 7n, instance: null }, colorToken: '--preview-extrude-new' }

describe('FaceCueLayer', () => {
  it('draws a tint and a 1.5 px outline for a face, in the preview color', () => {
    const layer = new FaceCueLayer()
    layer.update(face(4n), source())
    const [tint, outline] = layer.group.children as [THREE.Mesh, LineSegments2]
    const mat = tint.material as THREE.MeshBasicMaterial
    expect(mat.opacity).toBe(FACE_CUE_TINT_OPACITY)
    expect(mat.depthTest).toBe(true)
    expect(mat.depthWrite).toBe(false)
    expect(mat.polygonOffsetFactor).toBe(DEPTH_BIAS.PREVIEW)
    expect(mat.color.getHex()).toBe(0xee8800) // --preview-pushpull fallback (no DOM theme in the unit run)
    expect((outline.material as { linewidth: number }).linewidth).toBe(FACE_CUE_OUTLINE.widthPx)
    expect(layer.current).toMatchObject({ kind: 'face', owner: 3n, id: 4n, instance: null, colorToken: '--preview-pushpull', tinted: true, outlined: true })
  })

  it('a region is blue', () => {
    const layer = new FaceCueLayer()
    layer.update(region, source())
    const tint = layer.group.children[0] as THREE.Mesh
    expect((tint.material as THREE.MeshBasicMaterial).color.getHex()).toBe(0x3d7fe6)
    expect(layer.current).toMatchObject({ kind: 'region', owner: 9n, id: 7n })
  })

  it('rebuilds once across many updates over one face, again when the target or color changes', () => {
    const layer = new FaceCueLayer()
    const src = source()
    for (let i = 0; i < 50; i++) layer.update(face(4n), src)
    expect(layer.buildCount).toBe(1)
    expect(src.faceTriangles).toHaveBeenCalledTimes(1)
    layer.update(face(5n), src)
    expect(layer.buildCount).toBe(2)
    layer.update(face(5n, '--preview-extrude-new'), src)
    expect(layer.buildCount).toBe(3)
    expect(layer.current?.colorToken).toBe('--preview-extrude-new')
    layer.update(face(5n, '--preview-extrude-new', 12n), src)
    expect(layer.buildCount).toBe(4)
  })

  it('a null cue and clear() take it away, and the same face lights again afterwards', () => {
    const layer = new FaceCueLayer()
    const src = source()
    layer.update(face(4n), src)
    layer.update(null, src)
    expect(layer.current).toBeNull()
    expect(layer.group.children).toHaveLength(0)
    layer.update(face(4n), src)
    expect(layer.group.children).toHaveLength(2)
    layer.clear()
    expect(layer.current).toBeNull()
    expect(layer.group.children).toHaveLength(0)
    layer.update(face(4n), src)
    expect(layer.buildCount).toBe(3)
  })

  it('draws the outline alone when there are no triangles (a face over the vertex cap), nothing for a stale face', () => {
    const layer = new FaceCueLayer()
    layer.update(face(4n), source({ faceTriangles: () => null }))
    expect(layer.current).toMatchObject({ tinted: false, outlined: true })
    expect(layer.group.children).toHaveLength(1)
    layer.update(face(5n), source({ faceTriangles: () => { throw new Error('stale') } }))
    expect(layer.group.children).toHaveLength(0)
    expect(layer.current).toMatchObject({ tinted: false, outlined: false })
  })

  it('is never pickable', () => {
    const layer = new FaceCueLayer()
    layer.update(face(4n), source())
    const hits: THREE.Intersection[] = []
    const ray = new THREE.Raycaster(new THREE.Vector3(0.2, 0.2, 5), new THREE.Vector3(0, 0, -1))
    ray.intersectObjects(layer.group.children, true, hits)
    expect(hits).toHaveLength(0)
  })

  it('clear() disposes the tint geometry and material', () => {
    const layer = new FaceCueLayer()
    layer.update(face(4n), source())
    const tint = layer.group.children[0] as THREE.Mesh
    const geo = vi.spyOn(tint.geometry, 'dispose')
    const mat = vi.spyOn(tint.material as THREE.Material, 'dispose')
    layer.clear()
    expect(geo).toHaveBeenCalled()
    expect(mat).toHaveBeenCalled()
  })
})

describe('faceCueGeometry', () => {
  // Two quads of 4 vertices each: face A = vertices 0..3, face B = 4..7.
  const positions = new Float32Array([
    0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0,
    0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1,
  ])
  const indices = new Uint32Array([0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7])

  it('copies exactly one face\'s triangles out of the shared buffers', () => {
    const a = trianglesInVertexRange(positions, indices, 0, 4)!
    expect(a).toHaveLength(18)
    expect(Array.from(a.slice(0, 3))).toEqual([0, 0, 0])
    const b = trianglesInVertexRange(positions, indices, 4, 4)!
    expect(b).toHaveLength(18)
    for (let i = 2; i < 18; i += 3) expect(b[i]).toBe(1)
  })

  it('refuses a range past the buffer and one over the vertex cap', () => {
    expect(trianglesInVertexRange(positions, indices, 6, 4)).toBeNull()
    expect(trianglesInVertexRange(positions, indices, 0, 0)).toBeNull()
    expect(trianglesInVertexRange(positions, indices, 0, FACE_CUE_MAX_VERTICES + 1)).toBeNull()
  })

  it('maps points through a pose and closes an outline loop', () => {
    const pose = [1, 0, 0, 10, 0, 1, 0, 20, 0, 0, 1, 30]
    expect(Array.from(mapThroughPose(new Float32Array([1, 2, 3]), pose))).toEqual([11, 22, 33])
    const seg = loopSegments(LOOP)
    expect(seg).toHaveLength(24)
    expect(Array.from(seg.slice(18, 24))).toEqual([0, 1, 0, 0, 0, 0]) // the last edge closes back to the first vertex
  })

  it('triangulates a non-convex loop without covering its notch, and drops a degenerate one', () => {
    // An L: area 3.
    const l = [0, 0, 0, 2, 0, 0, 2, 1, 0, 1, 1, 0, 1, 2, 0, 0, 2, 0]
    const tris = triangulatePlanarLoop(l)!
    let area = 0
    for (let i = 0; i < tris.length; i += 9) {
      area += Math.abs((tris[i + 3] - tris[i]) * (tris[i + 7] - tris[i + 1]) - (tris[i + 6] - tris[i]) * (tris[i + 4] - tris[i + 1])) / 2
    }
    expect(area).toBeCloseTo(3)
    expect(triangulatePlanarLoop([0, 0, 0, 1, 0, 0])).toBeNull()
    expect(triangulatePlanarLoop([0, 0, 0, 1, 0, 0, 2, 0, 0])).toBeNull()
  })
})
