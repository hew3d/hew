import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import { guideLinePickPx, guideLinePickSamples, segmentPickPx, segmentPickSamples } from './screenPick'

const SIZE = 100

/** Canvas pixel of a world point, for a square SIZE × SIZE canvas. */
function pixelOf(cam: THREE.Camera, x: number, y: number, z: number): { x: number; y: number } {
  const v = new THREE.Vector3(x, y, z).project(cam)
  return { x: (v.x * 0.5 + 0.5) * SIZE, y: (0.5 - v.y * 0.5) * SIZE }
}

/** An infinite guide along +x through the origin, as `guide_geometry` gives it. */
const X_GUIDE = [0, 0, 0, 1, 0, 0, -Infinity, Infinity]

describe('guideLinePickPx', () => {
  /** Zoomed in on a small part: 2 cm above the x-axis guide, looking along
   *  it, with the eye a little past the guide's origin. */
  function zoomedInAlongGuide(): THREE.PerspectiveCamera {
    const cam = new THREE.PerspectiveCamera(45, 1, 0.01, 100)
    cam.position.set(0.3, 0.02, 0)
    cam.lookAt(0.5, 0, 0)
    cam.updateMatrixWorld()
    return cam
  }

  it('picks a guide running into depth right next to the camera', () => {
    // Regression: the guide used to be sampled every ~1.56 m and any sample
    // pair reaching behind the camera was dropped — here that pair holds the
    // whole stretch between the eye and 0.78 m ahead, so a click on the
    // guide 10 cm in front of the eye was never a hit.
    const cam = zoomedInAlongGuide()
    const click = pixelOf(cam, 0.4, 0, 0)
    expect(guideLinePickPx(cam, SIZE, SIZE, X_GUIDE, click.x, click.y)).toBeLessThan(0.5)
  })

  it('does not pick a click away from the guide', () => {
    const cam = zoomedInAlongGuide()
    const click = pixelOf(cam, 0.4, 0, 0)
    expect(guideLinePickPx(cam, SIZE, SIZE, X_GUIDE, click.x + 30, click.y)).toBeGreaterThan(20)
  })

  it('honours a finite guide span: a segment wholly behind the camera is not pickable', () => {
    const cam = zoomedInAlongGuide()
    const behind = [0, 0, 0, 1, 0, 0, -2, 0.2]
    const click = pixelOf(cam, 0.4, 0, 0)
    expect(guideLinePickPx(cam, SIZE, SIZE, behind, click.x, click.y)).toBe(Infinity)
  })

  it('a degenerate direction is not pickable', () => {
    const cam = zoomedInAlongGuide()
    expect(guideLinePickPx(cam, SIZE, SIZE, [0, 0, 0, 0, 0, 0], 50, 50)).toBe(Infinity)
  })

  it('orthographic: picks a guide that passes through the eye position', () => {
    const cam = new THREE.OrthographicCamera(-0.1, 0.1, 0.1, -0.1, 0.01, 100)
    cam.position.set(0.3, 0.02, 0)
    cam.lookAt(0.5, 0, 0)
    cam.updateMatrixWorld()
    const click = pixelOf(cam, 0.4, 0, 0)
    expect(guideLinePickPx(cam, SIZE, SIZE, X_GUIDE, click.x, click.y)).toBeLessThan(0.5)
  })
})

describe('segmentPickPx', () => {
  /** Camera at +10z looking at the origin, y-up. */
  function camera(): THREE.PerspectiveCamera {
    const cam = new THREE.PerspectiveCamera(90, 1, 0.1, 100)
    cam.position.set(0, 0, 10)
    cam.lookAt(0, 0, 0)
    cam.updateMatrixWorld()
    return cam
  }

  it('measures a segment wholly in front as its projection', () => {
    // fov 90 at distance 10: the canvas spans ±10 at z = 0.
    expect(segmentPickPx(camera(), SIZE, SIZE, [-5, 0, 0], [5, 0, 0], 50, 50)).toBeCloseTo(0)
    expect(segmentPickPx(camera(), SIZE, SIZE, [-5, 0, 0], [5, 0, 0], 50, 40)).toBeCloseTo(10)
    expect(segmentPickPx(camera(), SIZE, SIZE, [-5, 0, 0], [5, 0, 0], 85, 50)).toBeCloseTo(10)
  })

  it('a point segment measures to the point', () => {
    expect(segmentPickPx(camera(), SIZE, SIZE, [0, 0, 0], [0, 0, 0], 53, 54)).toBeCloseTo(5)
  })

  it('keeps the in-front part of a segment that reaches behind the camera', () => {
    const cam = camera()
    const click = pixelOf(cam, 1, 0, 5)
    expect(segmentPickPx(cam, SIZE, SIZE, [1, 0, 5], [1, 0, 20], click.x, click.y)).toBeCloseTo(0)
    // The behind endpoint does not drag the segment across the canvas: the
    // clipped stretch runs from (1, 0, 5) off the right edge, never left.
    expect(segmentPickPx(cam, SIZE, SIZE, [1, 0, 5], [1, 0, 20], 20, 50)).toBeGreaterThan(30)
  })

  it('a segment wholly behind the near plane is not pickable', () => {
    expect(segmentPickPx(camera(), SIZE, SIZE, [0, 0, 9.95], [0, 0, 20], 50, 50)).toBe(Infinity)
  })
})

describe('segmentPickSamples', () => {
  const cameras: Array<[string, () => THREE.PerspectiveCamera | THREE.OrthographicCamera]> = [
    ['perspective', () => {
      const c = new THREE.PerspectiveCamera(45, 1, 0.1, 100)
      c.position.set(2, -6, 3)
      c.lookAt(0, 0, 0)
      c.updateMatrixWorld()
      return c
    }],
    ['parallel', () => {
      const c = new THREE.OrthographicCamera(-4, 4, 4, -4, 0.1, 100)
      c.position.set(2, -6, 3)
      c.lookAt(0, 0, 0)
      c.updateMatrixWorld()
      return c
    }],
  ]
  const A: [number, number, number] = [-2, 1, 0]
  const B: [number, number, number] = [2, 3, 1]

  for (const [name, make] of cameras) {
    it(`${name}: nearest first, within the radius, first is segmentPickPx`, () => {
      const cam = make()
      const mid = pixelOf(cam, 0, 2, 0.5)
      const samples = segmentPickSamples(cam, SIZE, SIZE, A, B, mid.x + 2, mid.y + 1, 8)
      expect(samples.length).toBeGreaterThan(1)
      for (let i = 1; i < samples.length; i++) expect(samples[i].px).toBeGreaterThanOrEqual(samples[i - 1].px)
      for (const s of samples) expect(s.px).toBeLessThanOrEqual(8)
      expect(samples[0].px).toBeCloseTo(segmentPickPx(cam, SIZE, SIZE, A, B, mid.x + 2, mid.y + 1), 6)
    })

    it(`${name}: a sample's world point projects back to its screen distance and lies on the segment`, () => {
      const cam = make()
      const mid = pixelOf(cam, 0, 2, 0.5)
      const click = { x: mid.x + 1, y: mid.y }
      for (const s of segmentPickSamples(cam, SIZE, SIZE, A, B, click.x, click.y, 9)) {
        const p = pixelOf(cam, s.point[0], s.point[1], s.point[2])
        expect(Math.hypot(p.x - click.x, p.y - click.y)).toBeCloseTo(s.px, 4)
        // on the segment: cross product with the direction is ~0
        const u = new THREE.Vector3(B[0] - A[0], B[1] - A[1], B[2] - A[2])
        const w = new THREE.Vector3(s.point[0] - A[0], s.point[1] - A[1], s.point[2] - A[2])
        expect(u.clone().cross(w).length()).toBeLessThan(1e-4)
      }
    })

    it(`${name}: clamped to the segment's ends`, () => {
      const cam = make()
      const end = pixelOf(cam, B[0], B[1], B[2])
      const samples = segmentPickSamples(cam, SIZE, SIZE, A, B, end.x, end.y, 8)
      for (const s of samples) {
        const t = new THREE.Vector3(s.point[0] - A[0], s.point[1] - A[1], s.point[2] - A[2])
          .dot(new THREE.Vector3(B[0] - A[0], B[1] - A[1], B[2] - A[2])) /
          new THREE.Vector3(B[0] - A[0], B[1] - A[1], B[2] - A[2]).lengthSq()
        expect(t).toBeLessThanOrEqual(1 + 1e-6)
        expect(t).toBeGreaterThanOrEqual(-1e-6)
      }
    })

    it(`${name}: a click far from the segment has no samples`, () => {
      const cam = make()
      expect(segmentPickSamples(cam, SIZE, SIZE, A, B, 0, 0, 8)).toEqual([])
    })
  }

  it('a segment with one end behind the camera: every sample is in front of the near plane', () => {
    const cam = new THREE.PerspectiveCamera(45, 1, 0.01, 100)
    cam.position.set(0.3, 0.02, 0)
    cam.lookAt(0.5, 0, 0)
    cam.updateMatrixWorld()
    const click = pixelOf(cam, 0.4, 0, 0)
    const samples = guideLinePickSamples(cam, SIZE, SIZE, X_GUIDE, click.x, click.y, 8)
    expect(samples.length).toBeGreaterThan(0)
    for (const s of samples) {
      const v = new THREE.Vector3(...s.point).applyMatrix4(cam.matrixWorldInverse)
      expect(-v.z).toBeGreaterThanOrEqual(cam.near - 1e-9)
    }
    expect(samples[0].px).toBeCloseTo(guideLinePickPx(cam, SIZE, SIZE, X_GUIDE, click.x, click.y), 6)
  })

  it('wholly behind the camera or degenerate: no samples', () => {
    const cam = new THREE.PerspectiveCamera(45, 1, 0.01, 100)
    cam.position.set(0.3, 0.02, 0)
    cam.lookAt(0.5, 0, 0)
    cam.updateMatrixWorld()
    expect(guideLinePickSamples(cam, SIZE, SIZE, [0, 0, 0, 1, 0, 0, -2, 0.2], 50, 50, 8)).toEqual([])
    expect(guideLinePickSamples(cam, SIZE, SIZE, [0, 0, 0, 0, 0, 0], 50, 50, 8)).toEqual([])
  })
})
