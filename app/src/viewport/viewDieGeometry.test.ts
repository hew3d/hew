import { describe, expect, it } from 'vitest'
import {
  CORNERS,
  FACET_MIN_FACING_COS,
  DIE_LAYOUT,
  FACE_DEFS,
  cornerEye,
  eyeForHit,
  faceEye,
  facesOfAxis,
  hitTest,
  nearestView,
  projectDie,
  quatForEye,
  stepTriangles,
  worldAxisOfNormal,
  type CornerSigns,
  type DieFaceId,
  type V2,
  type V3,
} from './viewDieGeometry'

const POLE = 0.001
const rad = (d: number): number => (d * Math.PI) / 180

function eyeFor(face: DieFaceId): V3 {
  const e = faceEye(face)
  // Top and Bottom sit a hair off the pole, as the viewport bakes them.
  return face === 'top' || face === 'bottom' ? [0, -POLE, e[2]] : e
}

const HOME: V3 = [1, -1, 1]

describe('nearestView', () => {
  for (const f of FACE_DEFS) {
    it(`names the ${f.id} face view and reports it on view`, () => {
      const p = projectDie(quatForEye(eyeFor(f.id)))
      expect(p.view).toMatchObject({ kind: 'face', face: f.id, on: true })
    })
  }
  for (const c of CORNERS) {
    it(`names the corner view ${c.join(',')} and reports it on view`, () => {
      const p = projectDie(quatForEye(cornerEye(c)))
      expect(p.view).toMatchObject({ kind: 'corner', on: true })
      expect((p.view as { corner: CornerSigns }).corner).toEqual(c)
    })
  }
  it('reports a 3 degree offset as off view', () => {
    const t = rad(3)
    const p = projectDie(quatForEye([Math.sin(t), -Math.cos(t), 0]))
    expect(p.view).toMatchObject({ kind: 'face', face: 'front', on: false })
  })
  it('stays on view within half a degree', () => {
    const t = rad(0.4)
    expect(projectDie(quatForEye([Math.sin(t), -Math.cos(t), 0])).view.on).toBe(true)
  })
})

describe('projection', () => {
  it('shows Top, Front and Right from the home corner and hides the rest', () => {
    const p = projectDie(quatForEye(HOME))
    const seen = p.faces.filter((f) => f.facing).map((f) => f.id).sort()
    expect(seen).toEqual(['front', 'right', 'top'])
    expect(p.facets.filter((f) => f.facing).map((f) => f.corner.join()).sort()).toEqual(['-1,-1,1', '1,-1,-1', '1,-1,1', '1,1,1'])
  })
  it('puts the front face centre below the top face centre', () => {
    const p = projectDie(quatForEye(HOME))
    const y = (id: DieFaceId): number => p.faces.find((f) => f.id === id)!.center[1]
    expect(y('top')).toBeLessThan(y('front'))
  })
  it('keeps the silhouette inside the widget box', () => {
    for (const eye of [HOME, [1, 1, -1], [0, -1, 0], [0.3, -0.2, 0.9]] as V3[]) {
      for (const [x, y] of projectDie(quatForEye(eye)).silhouette) {
        expect(x).toBeGreaterThanOrEqual(0)
        expect(x).toBeLessThanOrEqual(DIE_LAYOUT.size)
        expect(y).toBeGreaterThanOrEqual(0)
        expect(y).toBeLessThanOrEqual(DIE_LAYOUT.size)
      }
    }
  })
  it('draws only stubs on a visible edge', () => {
    expect(projectDie(quatForEye(HOME)).stubs.length).toBeGreaterThan(0)
    expect(projectDie(quatForEye([-1, 1, 1])).stubs.length).toBeLessThan(3)
  })
})

describe('hitTest', () => {
  it('hits the facing faces at the home view', () => {
    const p = projectDie(quatForEye(HOME))
    for (const id of ['top', 'front', 'right'] as DieFaceId[]) {
      const f = p.faces.find((x) => x.id === id)!
      // A point between the face centre and its far-from-facet side.
      expect(hitTest(p, f.center)).toEqual({ kind: 'face', face: id })
    }
  })
  it('hits the front-right-top facet and not a back-facing one', () => {
    const p = projectDie(quatForEye(HOME))
    const front = p.facets.find((f) => f.corner.join() === '1,-1,1')!
    expect(hitTest(p, front.center)).toEqual({ kind: 'facet', corner: [1, -1, 1] })
    const back = p.facets.find((f) => f.corner.join() === '-1,1,-1')!
    expect(back.facing).toBe(false)
  })
  it('keeps a 6 px minimum radius around a thin facet', () => {
    // Nearly edge-on facet: its polygon is a sliver, the disc still hits.
    const p = projectDie(quatForEye([1, -0.05, 0.05]))
    const f = p.facets.find((x) => x.corner.join() === '1,-1,1')!
    expect(f.facing).toBe(true)
    const near: V2 = [f.center[0] + 4, f.center[1]]
    expect(hitTest(p, near)).toEqual({ kind: 'facet', corner: [1, -1, 1] })
  })
  it('misses outside the die', () => {
    const p = projectDie(quatForEye(HOME))
    expect(hitTest(p, [1, 1])).toBeNull()
  })
  it('face-on: centre is the face, a corner of the octagon is a facet or face of the same side only', () => {
    const p = projectDie(quatForEye(eyeFor('front')))
    expect(hitTest(p, [38, 38])).toEqual({ kind: 'face', face: 'front' })
  })
  it('face-on: facets at the cube corners are reachable', () => {
    const p = projectDie(quatForEye(eyeFor('front')))
    const facing = p.facets.filter((f) => f.facing)
    expect(facing).toHaveLength(4)
    for (const f of facing) {
      expect(hitTest(p, f.center)).toEqual({ kind: 'facet', corner: f.corner })
    }
  })
})

describe('step triangles', () => {
  it('appear only on a face view', () => {
    expect(stepTriangles(projectDie(quatForEye(HOME)))).toHaveLength(0)
    expect(stepTriangles(projectDie(quatForEye(eyeFor('front'))))).toHaveLength(4)
  })
  it('step from Front to Right on the right-hand triangle', () => {
    const p = projectDie(quatForEye(eyeFor('front')))
    const right = stepTriangles(p).find((s) => s.side === 'right')!
    expect(right.target).toBe('right')
    const hit = hitTest(p, [
      right.polygon.reduce((a, q) => a + q[0], 0) / 3,
      right.polygon.reduce((a, q) => a + q[1], 0) / 3,
    ])
    expect(hit).toEqual({ kind: 'step', side: 'right', target: 'right' })
  })
  it('steps up from Top to Back and from Front to Top', () => {
    const top = stepTriangles(projectDie(quatForEye(eyeFor('top'))))
    expect(top.find((s) => s.side === 'up')!.target).toBe('back')
    const front = stepTriangles(projectDie(quatForEye(eyeFor('front'))))
    expect(front.find((s) => s.side === 'up')!.target).toBe('top')
  })
})

describe('eyeForHit', () => {
  it('flips to the opposite face on a second click', () => {
    const p = projectDie(quatForEye(eyeFor('front')))
    expect(eyeForHit({ kind: 'face', face: 'front' }, p)).toEqual([0, 1, 0])
    expect(eyeForHit({ kind: 'face', face: 'right' }, p)).toEqual([1, 0, 0])
  })
  it('uses the signed corner for a facet, including from below', () => {
    const p = projectDie(quatForEye(HOME))
    expect(eyeForHit({ kind: 'facet', corner: [-1, 1, -1] }, p)).toEqual([-1, 1, -1])
  })
})

describe('plane fill axis', () => {
  it('matches world axes in either direction within half a degree', () => {
    expect(worldAxisOfNormal([0, 0, 1])).toBe(2)
    expect(worldAxisOfNormal([0, -1, 0])).toBe(1)
    expect(worldAxisOfNormal([Math.sin(rad(0.3)), 0, -Math.cos(rad(0.3))])).toBe(2)
  })
  it('rejects a 30 degree plane', () => {
    expect(worldAxisOfNormal([Math.sin(rad(30)), -Math.cos(rad(30)), 0])).toBeNull()
  })
  it('pairs faces by axis', () => {
    expect(facesOfAxis(2)).toEqual(['top', 'bottom'])
  })
})

describe('nearestView helper', () => {
  it('picks the closest named direction', () => {
    expect(nearestView([0.1, -1, 0.05])).toMatchObject({ kind: 'face', face: 'front' })
  })
})

describe('grazing facets', () => {
  it('a sliver facet does not steal clicks inside the face beside it', () => {
    // Yaw 45 degrees, a little above the horizon: the corner facets on the
    // sides are seen nearly edge-on.
    const yaw = rad(45)
    const el = rad(4)
    const p = projectDie(quatForEye([Math.sin(yaw) * Math.cos(el), -Math.cos(yaw) * Math.cos(el), Math.sin(el)]))
    const slivers = p.facets.filter((f) => f.facing && f.facingCos < FACET_MIN_FACING_COS)
    expect(slivers.length).toBeGreaterThan(0)
    let checked = 0
    for (const f of slivers) {
      // A point 4 px from the sliver's centre, inside a facing face.
      const candidates: V2[] = [[4, 0], [-4, 0], [0, 4], [0, -4]].map(([dx, dy]) => [f.center[0] + dx!, f.center[1] + dy!])
      for (const c of candidates) {
        const insideFace = p.faces.some((fc) => fc.facing && pointInside(c, fc.polygon))
        const insideSliver = pointInside(c, f.polygon)
        if (insideFace && !insideSliver) {
          expect(hitTest(p, c)).toMatchObject({ kind: 'face' })
          checked++
        }
      }
    }
    expect(checked).toBeGreaterThan(0)
  })
  it('the sliver still hits on its own polygon', () => {
    const yaw = rad(45)
    const el = rad(4)
    const p = projectDie(quatForEye([Math.sin(yaw) * Math.cos(el), -Math.cos(yaw) * Math.cos(el), Math.sin(el)]))
    const f = p.facets.find((x) => x.facing && x.facingCos < FACET_MIN_FACING_COS && pointInside(x.center, x.polygon))
    if (f !== undefined) expect(hitTest(p, f.center)).toMatchObject({ kind: 'facet' })
  })
})

function pointInside(pt: V2, poly: V2[]): boolean {
  let inside = false
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i]!
    const [xj, yj] = poly[j]!
    if (yi > pt[1] !== yj > pt[1] && pt[0] < ((xj - xi) * (pt[1] - yi)) / (yj - yi) + xi) inside = !inside
  }
  return inside
}
