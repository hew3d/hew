import { describe, expect, it } from 'vitest'
import * as THREE from 'three'
import {
  MarqueeProjector,
  normalizedRect,
  pointInRect,
  pointInTriangle,
  segmentIntersectsRect,
  segmentsIntersect,
  triangleIntersectsRect,
} from './marquee'

const rect = normalizedRect(10, 10, 50, 50)

describe('normalizedRect', () => {
  it('orders min/max regardless of drag direction', () => {
    expect(normalizedRect(50, 50, 10, 10)).toEqual({ minX: 10, minY: 10, maxX: 50, maxY: 50 })
  })
})

describe('2D predicates', () => {
  it('pointInRect includes the boundary', () => {
    expect(pointInRect(10, 10, rect)).toBe(true)
    expect(pointInRect(30, 30, rect)).toBe(true)
    expect(pointInRect(9.99, 30, rect)).toBe(false)
  })

  it('segmentsIntersect handles crossing, touching, and disjoint pairs', () => {
    expect(segmentsIntersect(0, 0, 10, 10, 0, 10, 10, 0)).toBe(true)
    expect(segmentsIntersect(0, 0, 10, 0, 5, 0, 15, 0)).toBe(true) // collinear overlap
    expect(segmentsIntersect(0, 0, 10, 0, 0, 1, 10, 1)).toBe(false)
  })

  it('segmentIntersectsRect: endpoint inside, pass-through, and miss', () => {
    expect(segmentIntersectsRect(30, 30, 100, 100, rect)).toBe(true) // endpoint in
    expect(segmentIntersectsRect(0, 30, 100, 30, rect)).toBe(true) // crosses both edges
    expect(segmentIntersectsRect(0, 0, 100, 5, rect)).toBe(false) // passes above
  })

  it('pointInTriangle works for both windings', () => {
    expect(pointInTriangle(1, 1, 0, 0, 4, 0, 0, 4)).toBe(true)
    expect(pointInTriangle(1, 1, 0, 0, 0, 4, 4, 0)).toBe(true)
    expect(pointInTriangle(5, 5, 0, 0, 4, 0, 0, 4)).toBe(false)
  })

  it('triangleIntersectsRect: overlap, containment both ways, and miss', () => {
    expect(triangleIntersectsRect(0, 0, 30, 30, 0, 30, rect)).toBe(true) // overlaps
    expect(triangleIntersectsRect(20, 20, 30, 20, 20, 30, rect)).toBe(true) // tri inside rect
    expect(triangleIntersectsRect(-100, -100, 200, -100, 30, 300, rect)).toBe(true) // rect inside tri
    expect(triangleIntersectsRect(60, 60, 70, 60, 60, 70, rect)).toBe(false)
  })
})

describe('MarqueeProjector', () => {
  /** Camera at +10z looking at the origin, y-up, square 100×100 viewport. */
  function projector(): MarqueeProjector {
    const cam = new THREE.PerspectiveCamera(90, 1, 0.1, 100)
    cam.position.set(0, 0, 10)
    cam.lookAt(0, 0, 0)
    cam.updateMatrixWorld()
    return new MarqueeProjector(cam, 100, 100)
  }

  const identity = new THREE.Matrix4()
  // With fov 90 and distance 10, the visible extent at z=0 is ±10 in x/y,
  // so a point at the origin projects to the canvas center (50, 50).
  const centerRect = normalizedRect(40, 40, 60, 60)

  it('window test: contained, straddling, and empty geometry', () => {
    const inside = new Float32Array([0, 0, 0, 1, 1, 0]) // near center
    const straddling = new Float32Array([0, 0, 0, 9, 9, 0]) // second point near the edge
    expect(projector().allVerticesInRect(inside, identity, centerRect)).toBe(true)
    expect(projector().allVerticesInRect(straddling, identity, centerRect)).toBe(false)
    expect(projector().allVerticesInRect(new Float32Array(0), identity, centerRect)).toBe(false)
  })

  it('window test rejects geometry behind the camera', () => {
    const behind = new Float32Array([0, 0, 20]) // behind the camera at +10z
    expect(projector().allVerticesInRect(behind, identity, centerRect)).toBe(false)
  })

  it('crossing test: a triangle overlapping the rect corner hits', () => {
    // Triangle spanning x∈[6,10] at y≈0 → px∈[80,100]: outside the center
    // rect; one spanning x∈[0,4] → px∈[50,70]: overlaps its right edge.
    const missTri = new Float32Array([6, 0, 0, 10, 0, 0, 8, 2, 0])
    const hitTri = new Float32Array([0, 0, 0, 4, 0, 0, 2, 2, 0])
    const p = projector()
    expect(p.meshTouchesRect(missTri, null, identity, centerRect)).toBe(false)
    expect(p.meshTouchesRect(hitTri, null, identity, centerRect)).toBe(true)
  })

  it('crossing test clips segments that pierce the near plane', () => {
    // A segment from behind the camera through the view center must still hit.
    const pierce = new Float32Array([0, 0, 20, 0, 0, 0])
    expect(projector().segmentsTouchRect(pierce, identity, centerRect)).toBe(true)
    // Fully behind the camera never hits.
    const behind = new Float32Array([0, 0, 20, 0, 0, 30])
    expect(projector().segmentsTouchRect(behind, identity, centerRect)).toBe(false)
  })

  it('respects the object matrixWorld', () => {
    const at = new THREE.Matrix4().makeTranslation(9, 0, 0) // pushes px to ~95
    const vert = new Float32Array([0, 0, 0])
    expect(projector().allVerticesInRect(vert, at, centerRect)).toBe(false)
    expect(projector().allVerticesInRect(vert, identity, centerRect)).toBe(true)
  })
})
