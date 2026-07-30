import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import { CameraRig, MIN_FOV_DEG, MAX_FOV_DEG, isBehindCamera } from './cameraRig'
import { tanHalfFovRad, worldPerPixelPerspective } from './math'

describe('CameraRig — construction', () => {
  it('starts in perspective, active === perspective', () => {
    const rig = new CameraRig(16 / 9)
    expect(rig.projection).toBe('perspective')
    expect(rig.active).toBe(rig.perspective)
  })
})

describe('CameraRig — toggleProjection round trip', () => {
  it('perspective -> parallel sizes the ortho frustum to match what is on screen at the target', () => {
    const rig = new CameraRig(2)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    rig.perspective.updateProjectionMatrix()
    const target = new THREE.Vector3(0, 0, 0)

    rig.toggleProjection(target)

    expect(rig.projection).toBe('parallel')
    expect(rig.active).toBe(rig.orthographic)
    const dist = 10
    const expectedHalfH = dist * tanHalfFovRad(45)
    expect(rig.orthographic.top).toBeCloseTo(expectedHalfH, 9)
    expect(rig.orthographic.bottom).toBeCloseTo(-expectedHalfH, 9)
    expect(rig.orthographic.right).toBeCloseTo(expectedHalfH * 2, 9)
    expect(rig.orthographic.zoom).toBe(1)
    // Pose carried over exactly.
    expect(rig.orthographic.position.toArray()).toEqual(rig.perspective.position.toArray())
  })

  it('parallel -> perspective dollies the eye to reproduce the SAME apparent size at the target (round-trip stable)', () => {
    const rig = new CameraRig(2)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    rig.perspective.updateProjectionMatrix()
    const target = new THREE.Vector3(0, 0, 0)
    const vpH = 800

    const beforeWpp = rig.worldPerPixel(rig.perspective.position.distanceTo(target), vpH)

    rig.toggleProjection(target) // -> parallel
    rig.toggleProjection(target) // -> perspective

    expect(rig.projection).toBe('perspective')
    expect(rig.active).toBe(rig.perspective)
    // Distance-to-target reproduced (started at 10).
    expect(rig.perspective.position.distanceTo(target)).toBeCloseTo(10, 9)
    const afterWpp = rig.worldPerPixel(rig.perspective.position.distanceTo(target), vpH)
    expect(afterWpp).toBeCloseTo(beforeWpp, 9)
  })

  it('is visually stable through TWO full round trips (toggle x4 returns to the original pose)', () => {
    const rig = new CameraRig(1.5)
    rig.perspective.position.set(3, -4, 5)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(1, 1, 1)
    rig.perspective.updateProjectionMatrix()
    const target = new THREE.Vector3(1, 1, 1)
    const startPos = rig.perspective.position.clone()

    rig.toggleProjection(target)
    rig.toggleProjection(target)
    rig.toggleProjection(target)
    rig.toggleProjection(target)

    expect(rig.projection).toBe('perspective')
    expect(rig.perspective.position.x).toBeCloseTo(startPos.x, 7)
    expect(rig.perspective.position.y).toBeCloseTo(startPos.y, 7)
    expect(rig.perspective.position.z).toBeCloseTo(startPos.z, 7)
  })

  it('preserves fov across a toggle (fov is a perspective-only property that must survive parallel mode)', () => {
    const rig = new CameraRig(1)
    rig.setFov(70)
    rig.perspective.position.set(0, -5, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    const target = new THREE.Vector3(0, 0, 0)
    rig.toggleProjection(target)
    rig.toggleProjection(target)
    expect(rig.perspective.fov).toBe(70)
  })
})

describe('CameraRig — worldPerPixel parity with legacy fov math (perspective)', () => {
  it('matches worldPerPixelPerspective exactly', () => {
    const rig = new CameraRig(1)
    rig.setFov(52)
    for (const dist of [1, 8, 40]) {
      for (const vpH of [480, 900]) {
        expect(rig.worldPerPixel(dist, vpH)).toBeCloseTo(worldPerPixelPerspective(dist, 52, vpH), 12)
      }
    }
  })
})

describe('CameraRig — worldPerPixel under parallel projection', () => {
  it('is independent of dist (the defining ortho property)', () => {
    const rig = new CameraRig(1)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    rig.toggleProjection(new THREE.Vector3())
    const a = rig.worldPerPixel(1, 800)
    const b = rig.worldPerPixel(1000, 800)
    expect(a).toBe(b)
  })

  it('halves when zoom doubles (zooming in halves world-per-pixel)', () => {
    const rig = new CameraRig(1)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    rig.toggleProjection(new THREE.Vector3())
    const before = rig.worldPerPixel(5, 800)
    rig.orthographic.zoom *= 2
    rig.orthographic.updateProjectionMatrix()
    expect(rig.worldPerPixel(5, 800)).toBeCloseTo(before / 2, 9)
  })
})

describe('CameraRig — setFov clamping', () => {
  it('clamps below MIN_FOV_DEG and above MAX_FOV_DEG', () => {
    const rig = new CameraRig(1)
    rig.setFov(-5)
    expect(rig.perspective.fov).toBe(MIN_FOV_DEG)
    rig.setFov(500)
    expect(rig.perspective.fov).toBe(MAX_FOV_DEG)
    rig.setFov(60)
    expect(rig.perspective.fov).toBe(60)
  })
})

describe('CameraRig — setFov is a true lens change (playtest finding 4a)', () => {
  it('a typed FOV commit changes ONLY fovDeg — eye position, orientation, and target are untouched', () => {
    const rig = new CameraRig(1)
    rig.perspective.position.set(3, -4, 5)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    const positionBefore = rig.perspective.position.clone()
    const quaternionBefore = rig.perspective.quaternion.clone()

    rig.setFov(28)

    expect(rig.perspective.fov).toBe(28)
    expect(rig.perspective.position.equals(positionBefore)).toBe(true)
    expect(rig.perspective.quaternion.equals(quaternionBefore)).toBe(true)
  })

  it('holds regardless of the starting fov or the typed target (no distance compensation hiding in the clamped range)', () => {
    const rig = new CameraRig(1)
    rig.perspective.position.set(10, 0, 2)
    const positionBefore = rig.perspective.position.clone()
    for (const typedDeg of [1, 10, 45, 90, 120]) {
      rig.setFov(typedDeg)
      expect(rig.perspective.fov).toBe(typedDeg)
      expect(rig.perspective.position.equals(positionBefore)).toBe(true)
    }
  })
})

describe('CameraRig — setAspect', () => {
  it('updates perspective.aspect and rescales the ortho half-width, preserving half-height', () => {
    const rig = new CameraRig(1)
    rig.frameOrthoToRadius(10, 1.2, 1)
    const halfH = rig.orthographic.top
    rig.setAspect(2)
    expect(rig.perspective.aspect).toBe(2)
    expect(rig.orthographic.top).toBeCloseTo(halfH, 9) // unchanged
    expect(rig.orthographic.right).toBeCloseTo(halfH * 2, 9) // rescaled
  })
})

describe('CameraRig — ortho framing (frameOrthoToRadius)', () => {
  it('sizes the frustum to radius * margin, resets zoom to 1, and is symmetric', () => {
    const rig = new CameraRig(1.5)
    rig.frameOrthoToRadius(4, 1.2, 1.5)
    expect(rig.orthographic.top).toBeCloseTo(4.8, 9)
    expect(rig.orthographic.bottom).toBeCloseTo(-4.8, 9)
    expect(rig.orthographic.right).toBeCloseTo(4.8 * 1.5, 9)
    expect(rig.orthographic.left).toBeCloseTo(-4.8 * 1.5, 9)
    expect(rig.orthographic.zoom).toBe(1)
  })
})

describe('CameraRig — perspectiveFramingDistance', () => {
  it('matches radius * margin / tanHalfFov', () => {
    const rig = new CameraRig(1)
    rig.setFov(60)
    const d = rig.perspectiveFramingDistance(5, 1.2)
    expect(d).toBeCloseTo((5 * 1.2) / tanHalfFovRad(60), 9)
  })
})

describe('CameraRig — scaleOrthoFrustum (Zoom Window parallel path)', () => {
  it('factor < 1 zooms in (worldPerPixel shrinks by the same factor)', () => {
    const rig = new CameraRig(1)
    rig.frameOrthoToRadius(10, 1.2, 1)
    const before = rig.worldPerPixel(0, 800)
    rig.scaleOrthoFrustum(0.5)
    expect(rig.worldPerPixel(0, 800)).toBeCloseTo(before * 0.5, 9)
  })

  it('factor > 1 zooms out', () => {
    const rig = new CameraRig(1)
    rig.frameOrthoToRadius(10, 1.2, 1)
    const before = rig.worldPerPixel(0, 800)
    rig.scaleOrthoFrustum(2)
    expect(rig.worldPerPixel(0, 800)).toBeCloseTo(before * 2, 9)
  })
})

describe('CameraRig — effectiveDistance', () => {
  it('perspective: returns controlsDistance verbatim AT the reference fov (the default)', () => {
    const rig = new CameraRig(1)
    expect(rig.effectiveDistance(17)).toBe(17)
  })

  it('perspective: at a non-reference fov, normalizes controlsDistance by tanHalf(fov)/tanHalf(referenceFov) rather than returning it verbatim', () => {
    const rig = new CameraRig(1)
    rig.setFov(90)
    const expected = (17 * tanHalfFovRad(90)) / tanHalfFovRad(45)
    expect(rig.effectiveDistance(17)).toBeCloseTo(expected, 9)
    expect(rig.effectiveDistance(17)).not.toBeCloseTo(17, 3)
  })

  it('is CONTINUOUS across toggleProjection at a non-default fov — a toggle alone must not jump the value (guide-dash/grid stability)', () => {
    const rig = new CameraRig(1)
    rig.setFov(90)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    const target = new THREE.Vector3()
    const before = rig.effectiveDistance(rig.perspective.position.distanceTo(target))

    rig.toggleProjection(target) // -> parallel, same pose/fov

    const after = rig.effectiveDistance(0) // arg ignored under parallel
    expect(after).toBeCloseTo(before, 9)
  })

  it('parallel: reacts to zoom the same way a perspective dolly would (halving zoom halves effectiveDistance... doubling zoom halves it)', () => {
    const rig = new CameraRig(1)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    rig.toggleProjection(new THREE.Vector3())
    const before = rig.effectiveDistance(999) // arg ignored under parallel
    rig.orthographic.zoom *= 2
    rig.orthographic.updateProjectionMatrix()
    expect(rig.effectiveDistance(999)).toBeCloseTo(before / 2, 9)
  })

  it('parallel effectiveDistance feeds worldPerPixel(dist, vpH) at the reference 45 deg fov and reproduces the SAME worldPerPixel the ortho frustum actually has', () => {
    const rig = new CameraRig(1)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    rig.toggleProjection(new THREE.Vector3())
    const vpH = 800
    const realWpp = rig.worldPerPixel(0, vpH)
    const effDist = rig.effectiveDistance(0)
    const reconstructed = worldPerPixelPerspective(effDist, 45, vpH)
    expect(reconstructed).toBeCloseTo(realWpp, 9)
  })
})

describe('CameraRig — apertureBasis', () => {
  it('perspective: kind perspective with the current fov', () => {
    const rig = new CameraRig(1)
    rig.setFov(38)
    const basis = rig.apertureBasis(800)
    expect(basis).toEqual({ kind: 'perspective', fovYDeg: 38 })
  })

  it('parallel: kind parallel carrying worldPerPixel, no distance parameter needed', () => {
    const rig = new CameraRig(1)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    rig.toggleProjection(new THREE.Vector3())
    const basis = rig.apertureBasis(800)
    expect(basis.kind).toBe('parallel')
    if (basis.kind === 'parallel') {
      expect(basis.worldPerPixel).toBeCloseTo(rig.worldPerPixel(0, 800), 12)
    }
  })

  it('parallel: worldPerPixel is genuinely distance-independent — matches at any dist', () => {
    const rig = new CameraRig(1)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    rig.toggleProjection(new THREE.Vector3())
    const basis = rig.apertureBasis(800)
    if (basis.kind === 'parallel') {
      expect(basis.worldPerPixel).toBeCloseTo(rig.worldPerPixel(999, 800), 12)
    }
  })
})

describe('CameraRig — effectiveDistance guards a degenerate (<=0) ortho zoom', () => {
  it('returns 0 instead of Infinity/NaN — the same guard worldPerPixelOrtho has', () => {
    const rig = new CameraRig(1)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    rig.toggleProjection(new THREE.Vector3())
    rig.orthographic.zoom = 0
    expect(rig.effectiveDistance(999)).toBe(0)
    rig.orthographic.zoom = -3
    expect(rig.effectiveDistance(999)).toBe(0)
  })
})

describe('CameraRig — syncInactiveCamera', () => {
  it('while active is perspective, RE-poses+resizes a stale (inactive) ortho camera to match the CURRENT perspective pose — not whatever it was left at', () => {
    const rig = new CameraRig(2)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    rig.perspective.updateProjectionMatrix()
    const target = new THREE.Vector3(0, 0, 0)

    // Corrupt the (inactive) ortho camera's pose/frustum, as if left over
    // from a much earlier toggle or the constructor's placeholder.
    rig.orthographic.position.set(999, 999, 999)
    rig.orthographic.top = 1
    rig.orthographic.bottom = -1
    rig.orthographic.zoom = 7

    rig.syncInactiveCamera(target)

    expect(rig.projection).toBe('perspective') // unflipped
    expect(rig.active).toBe(rig.perspective) // unflipped
    const expectedHalfH = 10 * tanHalfFovRad(45)
    expect(rig.orthographic.top).toBeCloseTo(expectedHalfH, 9)
    expect(rig.orthographic.zoom).toBe(1)
    expect(rig.orthographic.position.toArray()).toEqual(rig.perspective.position.toArray())
  })

  it('while active is parallel, RE-poses a stale (inactive) perspective camera to match the CURRENT ortho pose/frustum — not whatever it was left at', () => {
    const rig = new CameraRig(2)
    rig.perspective.position.set(0, -10, 0)
    rig.perspective.up.set(0, 0, 1)
    rig.perspective.lookAt(0, 0, 0)
    rig.perspective.updateProjectionMatrix()
    const target = new THREE.Vector3(0, 0, 0)
    rig.toggleProjection(target) // -> parallel; perspective/ortho consistent

    // Corrupt the (inactive) perspective camera's pose, as if left over
    // from a much earlier toggle.
    rig.perspective.position.set(999, 999, 999)

    rig.syncInactiveCamera(target)

    expect(rig.projection).toBe('parallel') // unflipped
    expect(rig.active).toBe(rig.orthographic) // unflipped
    expect(rig.perspective.position.x).toBeCloseTo(0, 6)
    expect(rig.perspective.position.y).toBeCloseTo(-10, 6)
    expect(rig.perspective.position.z).toBeCloseTo(0, 6)
  })
})

describe('isBehindCamera — projection-agnostic via camera-space z', () => {
  it('perspective: a point in front of the eye reads NOT behind; a point behind the eye reads behind', () => {
    const cam = new THREE.PerspectiveCamera(45, 1, 0.1, 100)
    cam.position.set(0, 0, 10)
    cam.up.set(0, 1, 0)
    cam.lookAt(0, 0, 0)
    cam.updateMatrixWorld()
    expect(isBehindCamera(new THREE.Vector3(0, 0, 0), cam)).toBe(false)
    expect(isBehindCamera(new THREE.Vector3(0, 0, 20), cam)).toBe(true)
  })

  it('orthographic: a point in front of the eye reads NOT behind; a point behind the eye reads behind (the old NDC `v.z > 1` heuristic gets this backwards)', () => {
    const cam = new THREE.OrthographicCamera(-5, 5, 5, -5, 0.1, 100)
    cam.position.set(0, 0, 10)
    cam.up.set(0, 1, 0)
    cam.lookAt(0, 0, 0)
    cam.updateMatrixWorld()
    expect(isBehindCamera(new THREE.Vector3(0, 0, 0), cam)).toBe(false)
    expect(isBehindCamera(new THREE.Vector3(0, 0, 20), cam)).toBe(true)
  })

  it('a point exactly at the eye counts as behind (degenerate — not usable for picking)', () => {
    const cam = new THREE.PerspectiveCamera(45, 1, 0.1, 100)
    cam.position.set(3, 4, 5)
    cam.lookAt(0, 0, 0)
    cam.updateMatrixWorld()
    expect(isBehindCamera(cam.position.clone(), cam)).toBe(true)
  })
})
