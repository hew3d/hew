import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import { guideLinePickPx, segmentPickPx } from './screenPick'

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
