import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import { BoxGizmo, gripsFromBox, distSqToRay } from './boxGizmo'
import type { Ray } from '../viewport/math'

const MIN: [number, number, number] = [0, 0, 0]
const MAX: [number, number, number] = [1, 2, 3]

function ray(origin: [number, number, number], direction: [number, number, number]): Ray {
  return { origin, direction }
}

describe('gripsFromBox', () => {
  it('makes 6 face, 8 corner, and optionally 12 edge grips', () => {
    expect(gripsFromBox(MIN, MAX, false)).toHaveLength(14)
    expect(gripsFromBox(MIN, MAX, true)).toHaveLength(26)
  })

  it('a face grip sits on its face centre and drives one axis', () => {
    const faces = gripsFromBox(MIN, MAX, false).slice(0, 6)
    const top = faces.find((g) => g.axisIndex === 2 && g.pos[2] === 3)
    expect(top?.pos).toEqual([0.5, 1, 3])
    expect(top?.opposite).toEqual([0.5, 1, 0])
    expect(top?.axisIndex2).toBeNull()
  })

  it('a corner grip is uniform and anchors on the opposite corner', () => {
    const corners = gripsFromBox(MIN, MAX, false).slice(6)
    expect(corners).toHaveLength(8)
    for (const g of corners) {
      expect(g.axisIndex).toBeNull()
      expect(g.opposite).toEqual([1 - g.pos[0], 2 - g.pos[1], 3 - g.pos[2]])
    }
  })
})

describe('distSqToRay', () => {
  it('measures the perpendicular distance for a point ahead of the ray', () => {
    expect(distSqToRay(ray([0, 0, 10], [0, 0, -1]), [3, 4, 0])).toBeCloseTo(25, 9)
  })

  it('a point behind the origin is measured from the origin, never the line', () => {
    expect(distSqToRay(ray([0, 0, 10], [0, 0, -1]), [0, 0, 20])).toBeCloseTo(100, 9)
  })
})

describe('BoxGizmo', () => {
  it('draws the outline and grips into its parent, and hides them again', () => {
    const parent = new THREE.Group()
    const gizmo = new BoxGizmo()
    gizmo.draw(parent, MIN, MAX, gripsFromBox(MIN, MAX, false), 'Test')
    expect(gizmo.group).not.toBeNull()
    expect(parent.children).toHaveLength(1)
    expect(parent.children[0].name).toBe('Test')
    // One outline plus one marker per grip.
    expect(gizmo.meshes).toHaveLength(14)
    expect(parent.children[0].children).toHaveLength(15)
    gizmo.hide(parent)
    expect(gizmo.group).toBeNull()
    expect(gizmo.meshes).toBeNull()
    expect(parent.children).toHaveLength(0)
  })

  it('a grip-less draw is an outline only', () => {
    const parent = new THREE.Group()
    const gizmo = new BoxGizmo()
    gizmo.draw(parent, MIN, MAX, [], 'Outline')
    const group = parent.children[0] as THREE.Group
    expect(group.children.some((c) => c instanceof THREE.Mesh)).toBe(false)
  })

  it('hide on a gizmo that is not showing does nothing', () => {
    const parent = new THREE.Group()
    expect(() => new BoxGizmo().hide(parent)).not.toThrow()
  })

  it('picks the grip the ray points at, and nothing from far away', () => {
    const grips = gripsFromBox(MIN, MAX, false)
    const gizmo = new BoxGizmo()
    const top = grips.find((g) => g.axisIndex === 2 && g.pos[2] === 3)!
    // Oblique, so the ray meets the top grip alone: the bottom grip sits straight below it.
    const on = ray([0.5, -6, 10], [0, 7, -7])
    expect(gizmo.pick(on, grips)).toBe(top)
    expect(gizmo.pick(ray([50, 50, 10], [0, 0, -1]), grips)).toBeNull()
  })
})
