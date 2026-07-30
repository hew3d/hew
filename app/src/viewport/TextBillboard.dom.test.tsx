/**
 * DOM-dependent TextBillboard tests — construction and `update()`'s
 * billboard math need a `document` global, so this file runs under jsdom
 * (`.test.tsx` per vitest.config.ts's environmentMatchGlobs) even though it
 * renders no React. The canvas rasterization itself still can't be
 * pixel-tested (jsdom has no canvas 2D backend and this repo has no
 * `canvas`/jest-canvas-mock polyfill — TextBillboard.ts's constructor
 * degrades to a no-op rasterize when `getContext('2d')` returns null, which
 * these tests confirm doesn't throw).
 */
import { describe, it, expect, vi, afterEach } from 'vitest'
import * as THREE from 'three'
import { billboardWorldScale, TextBillboard } from './TextBillboard'

describe('TextBillboard', () => {
  it('constructs without a canvas 2D backend and exposes a renderable mesh', () => {
    const billboard = new TextBillboard(26, 1000)
    expect(billboard.mesh).toBeInstanceOf(THREE.Mesh)
    expect(billboard.mesh.renderOrder).toBe(1000)
    // setContent must not throw even when the environment has no real
    // canvas 2D context (this.ctx === null degrades to a no-op rasterize).
    expect(() => billboard.setContent('3.500 m', 0xe6e9ee, 1, 10)).not.toThrow()
    billboard.dispose()
  })

  it('setContent is a no-op (does not throw / mutate state oddly) when called twice with identical inputs', () => {
    const billboard = new TextBillboard(26, 1000)
    billboard.setContent('1.000 m', 0xffffff, 1, 10)
    expect(() => billboard.setContent('1.000 m', 0xffffff, 1, 10)).not.toThrow()
    billboard.dispose()
  })

  it('update() faces the camera and sizes the quad to the screen-constant formula', () => {
    const billboard = new TextBillboard(26, 1000)
    billboard.mesh.position.set(0, 0, 0)
    const camera = new THREE.PerspectiveCamera(50, 1, 0.1, 1000)
    camera.position.set(0, -10, 0)
    camera.lookAt(0, 0, 0)
    camera.updateMatrixWorld()

    billboard.update(camera, 800)

    // Quaternion copied verbatim from the camera (billboarding).
    expect(billboard.mesh.quaternion.equals(camera.quaternion)).toBe(true)
    // Height matches the closed-form scale (aspect defaults to 1 before any
    // successful rasterization, so width === height here).
    const expectedHeight = 2 * billboardWorldScale(26, 10, 50, 800) // full quad height = 2x the half-extent (see TextBillboard.ts's doc comment)
    expect(billboard.mesh.scale.y).toBeCloseTo(expectedHeight, 10)
    billboard.dispose()
  })

  it('update() is a no-op for an orthographic camera', () => {
    const billboard = new TextBillboard(26, 1000)
    const before = billboard.mesh.scale.clone()
    const ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 100)
    billboard.update(ortho, 800)
    expect(billboard.mesh.scale.equals(before)).toBe(true)
    billboard.dispose()
  })

  it('update() applies an explicit faceQuaternion instead of the camera-facing default (inline dimension/radial text, Finding 2)', () => {
    const billboard = new TextBillboard(26, 1000)
    billboard.mesh.position.set(0, 0, 0)
    const camera = new THREE.PerspectiveCamera(50, 1, 0.1, 1000)
    camera.position.set(0, -10, 0)
    camera.lookAt(0, 0, 0)
    camera.updateMatrixWorld()

    const inlineQuat = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(1, 0, 0), Math.PI / 2)
    billboard.update(camera, 800, inlineQuat)

    // The quad takes the explicit orientation, NOT the camera's own
    // quaternion — leader text (no faceQuaternion argument, the other test
    // above) still billboards; a linear/radial dimension label passes its
    // in-plane orientation here instead.
    expect(billboard.mesh.quaternion.equals(inlineQuat)).toBe(true)
    expect(billboard.mesh.quaternion.equals(camera.quaternion)).toBe(false)
    // Sizing is unaffected by the orientation override — still the same
    // screen-constant formula.
    const expectedHeight = 2 * billboardWorldScale(26, 10, 50, 800) // full quad height = 2x the half-extent (see TextBillboard.ts's doc comment)
    expect(billboard.mesh.scale.y).toBeCloseTo(expectedHeight, 10)
    billboard.dispose()
  })

  it('worldSize() reports the same height update() would apply, without mutating the mesh', () => {
    const billboard = new TextBillboard(26, 1000)
    billboard.mesh.position.set(0, 0, 0)
    const camera = new THREE.PerspectiveCamera(50, 1, 0.1, 1000)
    camera.position.set(0, -10, 0)
    camera.lookAt(0, 0, 0)
    camera.updateMatrixWorld()

    const before = billboard.mesh.scale.clone()
    const size = billboard.worldSize(camera, 800)
    expect(size).not.toBeNull()
    const expectedHeight = 2 * billboardWorldScale(26, 10, 50, 800)
    expect(size!.height).toBeCloseTo(expectedHeight, 10)
    expect(size!.width).toBeCloseTo(expectedHeight, 10) // aspect defaults to 1 pre-rasterize
    // A pure query — the mesh itself is untouched.
    expect(billboard.mesh.scale.equals(before)).toBe(true)
    billboard.dispose()
  })

  it('worldSize() returns null for a non-perspective camera', () => {
    const billboard = new TextBillboard(26, 1000)
    const ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 100)
    expect(billboard.worldSize(ortho, 800)).toBeNull()
    billboard.dispose()
  })
})

/**
 * Minimal fake `CanvasRenderingContext2D` — jsdom has no real canvas 2D
 * backend (see this file's top doc comment), so `TextBillboard`'s own `ctx`
 * is always `null` there and `_rasterize` never actually runs; every test
 * above only exercises `update()`'s math against the pre-raster defaults
 * (`aspect`/`contentFrac` both 1). Installing this fake via
 * `HTMLCanvasElement.prototype.getContext` lets the FULL `setContent` ->
 * `_rasterize` -> `update()` path run end-to-end, with `measureText`
 * returning realistic-but-controlled `TextMetrics` — closing the gap the
 * delta review flagged: the pure-math tests on `glyphContentFraction` in
 * isolation could not have caught `_rasterize` wiring the wrong measurement
 * into it (the actual ~40% size regression).
 */
class FakeCanvas2DContext {
  font = ''
  textAlign = ''
  textBaseline = ''
  lineJoin = ''
  lineWidth = 0
  strokeStyle = ''
  fillStyle = ''

  /** `null` simulates a backend that doesn't populate `actualBoundingBox*`
   * at all (older Firefox / a partial-TextMetrics test DOM) — forces
   * `glyphContentFraction`'s fallback branch. Otherwise, the fraction of
   * `fontPx` treated as the reference digit `'0'`'s ink (a realistic value
   * is ~0.72, matching `FALLBACK_GLYPH_HEIGHT_FRAC`). */
  constructor(private readonly digitInkFrac: number | null) {}

  private currentFontPx(): number {
    const match = /([0-9.]+)px/.exec(this.font)
    return match ? parseFloat(match[1]) : 0
  }

  measureText(text: string): TextMetrics {
    const fontPx = this.currentFontPx()
    const width = fontPx * 0.6 * Math.max(1, text.length)
    // fontBoundingBox* is supplied for realism (a real backend always
    // populates both alongside actualBoundingBox*) but `_rasterize` no
    // longer consults it for sizing — only the reference digit '0'`'s
    // actualBoundingBox* feeds `glyphContentFraction` now (Findings 1 & 2).
    // Deliberately a much larger fraction than digit ink (~1.2 em, matching
    // real fontBoundingBox behavior) so a test that accidentally regresses
    // to reading this instead would produce a visibly different, wrong size.
    const fontBoundingBoxAscent = fontPx * 0.9
    const fontBoundingBoxDescent = fontPx * 0.3
    if (this.digitInkFrac === null) {
      return {
        width,
        actualBoundingBoxAscent: NaN,
        actualBoundingBoxDescent: NaN,
        fontBoundingBoxAscent,
        fontBoundingBoxDescent,
      } as unknown as TextMetrics
    }
    // A label's OWN measured ink depends on its content shape: digits sit at
    // the reference fraction, descender-bearing strings measure noticeably
    // more, and other strings a little more than a bare digit. The spread
    // must differ BETWEEN the two labels the uniformity test compares —
    // that's what lets it discriminate: under a regression of `_rasterize`
    // back to per-label ink, '12' and 'doorway gap' get different
    // contentFrac (different scale.y) and the same-quad-scale assertion
    // fails; a flat non-'0' multiplier would let that regression pass.
    const hasDescender = /[gjpqy]/.test(text)
    const inkFrac =
      text === '0'
        ? this.digitInkFrac
        : this.digitInkFrac * (hasDescender ? 1.35 : 1.08)
    return {
      width,
      actualBoundingBoxAscent: fontPx * inkFrac * 0.8,
      actualBoundingBoxDescent: fontPx * inkFrac * 0.2,
      fontBoundingBoxAscent,
      fontBoundingBoxDescent,
    } as unknown as TextMetrics
  }

  clearRect(): void {}
  fillText(): void {}
  strokeText(): void {}
}

function installFakeCanvas2D(digitInkFrac: number | null) {
  return vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation((contextId: any): any => {
    if (contextId !== '2d') return null
    return new FakeCanvas2DContext(digitInkFrac) as unknown as CanvasRenderingContext2D
  })
}

describe('end-to-end label sizing through _rasterize + update (closes the pure-math-only gap)', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  const targetPx = 14 // matches SceneRenderer.ts's ANNOTATION_TEXT_SCREEN_PX
  const viewportHeight = 800
  const fov = 50
  // Mirrors TextBillboard.ts's private BASE_FONT_PX/PAD_Y_FRAC (like the
  // existing pure-math tests already duplicate FALLBACK_GLYPH_HEIGHT_FRAC as
  // a literal 0.72) so this test can compute an expected canvas height
  // without reaching into TextBillboard's private internals.
  const BASE_FONT_PX = 40
  const PAD_Y_FRAC = 0.34

  function expectedCanvasHeightPx(fontPx: number): number {
    return Math.ceil(fontPx * 1.2 + 2 * fontPx * PAD_Y_FRAC)
  }

  function makeCamera(): THREE.PerspectiveCamera {
    const camera = new THREE.PerspectiveCamera(fov, 1, 0.1, 1000)
    camera.position.set(0, 0, 0)
    camera.lookAt(0, 0, -1)
    camera.updateMatrixWorld()
    return camera
  }

  /** Ground truth for "how tall does the rendered quad actually project to
   * on screen," independent of TextBillboard's own formulas — the same
   * `Vector3.project`/NDC technique TextBillboard.test.ts's continuous-sizing
   * test uses, applied here to the REAL mesh a full `setContent` + `update()`
   * pass produced (not a value hand-computed from `billboardWorldScale`). */
  function projectedScreenPxHeight(worldHeight: number, camera: THREE.PerspectiveCamera, distance: number): number {
    const center = new THREE.Vector3(0, 0, -distance)
    const top = center.clone().add(new THREE.Vector3(0, worldHeight / 2, 0))
    const bottom = center.clone().add(new THREE.Vector3(0, -worldHeight / 2, 0))
    const topNdc = top.project(camera)
    const bottomNdc = bottom.project(camera)
    return (Math.abs(topNdc.y - bottomNdc.y) / 2) * viewportHeight
  }

  it("a digit label's visible ink projects to screenPxHeight within 2% (Finding 1: em-box vs. digit-ink calibration)", () => {
    installFakeCanvas2D(0.72) // a realistic measured digit ink (~0.72em)
    const distance = 10 // zoom tier 1 -> supersample 1.0
    const dpr = 1
    const fontPx = BASE_FONT_PX * dpr * 1.0
    const h = expectedCanvasHeightPx(fontPx)
    const expectedInkPx = fontPx * 0.72 // this mock's digit ink

    const billboard = new TextBillboard(targetPx, 1000)
    billboard.mesh.position.set(0, 0, -distance)
    const camera = makeCamera()

    billboard.setContent('12', 0xffffff, dpr, distance)
    billboard.update(camera, viewportHeight)

    const quadScreenPx = projectedScreenPxHeight(billboard.mesh.scale.y, camera, distance)
    const inkScreenPx = quadScreenPx * (expectedInkPx / h)

    expect(Math.abs(inkScreenPx - targetPx) / targetPx).toBeLessThan(0.02)
    billboard.dispose()
  })

  it("'doorway gap' gets the SAME quad scale as '12' — uniformity preserved (Finding 2)", () => {
    installFakeCanvas2D(0.72)
    const distance = 10
    const dpr = 1
    const camera = makeCamera()

    const digitBillboard = new TextBillboard(targetPx, 1000)
    digitBillboard.mesh.position.set(0, 0, -distance)
    digitBillboard.setContent('12', 0xffffff, dpr, distance)
    digitBillboard.update(camera, viewportHeight)

    const descenderBillboard = new TextBillboard(targetPx, 1000)
    descenderBillboard.mesh.position.set(0, 0, -distance)
    descenderBillboard.setContent('doorway gap', 0xffffff, dpr, distance)
    descenderBillboard.update(camera, viewportHeight)

    // Height (not width — the strings have different widths/aspect, which is
    // expected) must match exactly: both rasterizations share the same
    // fontPx/canvas-height formula and the same cached reference-digit ink,
    // regardless of which glyphs either label actually contains.
    expect(descenderBillboard.mesh.scale.y).toBeCloseTo(digitBillboard.mesh.scale.y, 10)
    expect(descenderBillboard.mesh.scale.x).not.toBeCloseTo(digitBillboard.mesh.scale.x, 2)

    digitBillboard.dispose()
    descenderBillboard.dispose()
  })

  it('the fallback path (no actualBoundingBox* metrics) lands within ~5% of the measured path', () => {
    const distance = 10
    const camera = makeCamera()

    // Distinct DPRs so each billboard rasterizes at a font string never used
    // by the other test in this suite — measureReferenceDigitInkPx caches by
    // exact font string, so reusing one here would just read back a cached
    // value from a different mock instead of exercising this one.
    installFakeCanvas2D(0.72)
    const measuredDpr = 5
    const measuredFontPx = BASE_FONT_PX * measuredDpr
    const measuredBillboard = new TextBillboard(targetPx, 1000)
    measuredBillboard.mesh.position.set(0, 0, -distance)
    measuredBillboard.setContent('12', 0xffffff, measuredDpr, distance)
    measuredBillboard.update(camera, viewportHeight)
    const measuredInkScreenPx =
      projectedScreenPxHeight(measuredBillboard.mesh.scale.y, camera, distance) * ((measuredFontPx * 0.72) / expectedCanvasHeightPx(measuredFontPx))

    installFakeCanvas2D(null) // no actualBoundingBox* support at all
    const fallbackDpr = 6
    const fallbackFontPx = BASE_FONT_PX * fallbackDpr
    const fallbackBillboard = new TextBillboard(targetPx, 1000)
    fallbackBillboard.mesh.position.set(0, 0, -distance)
    fallbackBillboard.setContent('12', 0xffffff, fallbackDpr, distance)
    fallbackBillboard.update(camera, viewportHeight)
    // FALLBACK_GLYPH_HEIGHT_FRAC (0.72) mirrored as a literal, same
    // convention as the pure-math tests.
    const fallbackInkScreenPx =
      projectedScreenPxHeight(fallbackBillboard.mesh.scale.y, camera, distance) * ((fallbackFontPx * 0.72) / expectedCanvasHeightPx(fallbackFontPx))

    expect(Math.abs(fallbackInkScreenPx - measuredInkScreenPx) / measuredInkScreenPx).toBeLessThan(0.05)

    measuredBillboard.dispose()
    fallbackBillboard.dispose()
  })
})

describe('texture reallocation across a canvas-resizing rasterization (WebGL2 texStorage2D immutable-allocation gotcha)', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('swaps in a fresh CanvasTexture instance when a re-rasterization changes the canvas pixel dimensions, instead of reusing the old one', () => {
    // Regression test for a real playtest defect: a camera teleport far past
    // a zoom-tier boundary (`zoomTierFor`) made every live annotation label
    // render TWICE — the correct label at the new scale, plus a larger
    // sprite showing only the horizontally-cropped TAIL of the previous
    // (larger) rasterization, sharing the same quad.
    //
    // Root cause: a `THREE.CanvasTexture` reused across a canvas resize has
    // its GPU storage allocated IMMUTABLY (`gl.texStorage2D`) at the size of
    // its FIRST upload, on the WebGL2 backend `WebGLRenderer` uses by default
    // (`Viewport.tsx`'s renderer requests no explicit context, so `three`
    // picks WebGL2 when available — this app's real desktop/browser target).
    // Every later upload to a differently-sized canvas then only
    // `texSubImage2D`s into that fixed storage's top-left corner, leaving
    // stale pixels from the earlier (often larger) rasterization visible
    // around the new content instead of being replaced.
    //
    // jsdom has no real WebGL backend to observe the visual artifact
    // directly (this repo has no headless-GL harness), so this asserts the
    // fix at the level that actually matters to a real renderer: whenever a
    // rasterization changes the canvas's pixel dimensions, the billboard
    // must swap in a NEW texture object (a fresh `Source`, so a real
    // `WebGLRenderer` allocates fresh GPU storage sized to fit) rather than
    // reuse the same texture object with its GPU-side storage still sized
    // for the old, mismatched dimensions.
    installFakeCanvas2D(0.72)
    const camera = new THREE.PerspectiveCamera(50, 1, 0.1, 10000)
    camera.position.set(0, 0, 0)
    camera.lookAt(0, 0, -1)
    camera.updateMatrixWorld()

    const billboard = new TextBillboard(14, 1000)
    billboard.mesh.position.set(0, 0, -10)

    // Tier 0 (near, distance 1 < 5m) — highest supersample, largest canvas.
    billboard.setContent('inspect this edge', 0xffffff, 1, 1)
    billboard.update(camera, 800)
    const textureNear = billboard.mesh.material.map as THREE.CanvasTexture
    expect(textureNear).not.toBeNull()
    const wNear = (textureNear.image as HTMLCanvasElement).width
    const hNear = (textureNear.image as HTMLCanvasElement).height

    // Teleport far out — tier 2 (distance 40 > 20m) — lowest supersample,
    // smaller canvas. Same text, so ONLY the zoom tier changed, mirroring
    // the reported repro (a camera teleport, not a content edit).
    billboard.setContent('inspect this edge', 0xffffff, 1, 40)
    billboard.update(camera, 800)
    const textureFar = billboard.mesh.material.map as THREE.CanvasTexture
    const wFar = (textureFar.image as HTMLCanvasElement).width
    const hFar = (textureFar.image as HTMLCanvasElement).height

    // Sanity: the tiers really did produce different pixel dimensions —
    // otherwise this test would vacuously pass without exercising the bug.
    expect(wFar).not.toBe(wNear)
    expect(hFar).not.toBe(hNear)

    // The actual regression assertion.
    expect(textureFar).not.toBe(textureNear)

    billboard.dispose()
  })

  it('reuses the same texture instance when a re-rasterization does NOT change the canvas pixel dimensions (no unnecessary GPU texture churn)', () => {
    installFakeCanvas2D(0.72)
    const billboard = new TextBillboard(14, 1000)
    billboard.setContent('1.000 m', 0xffffff, 1, 10) // tier 1
    const textureA = billboard.mesh.material.map
    // Color-only change (a theme swap) at the same text/dpr/tier — the
    // canvas's pixel dimensions don't depend on fill color, so this must NOT
    // reallocate the texture.
    billboard.setContent('1.000 m', 0xe6e9ee, 1, 10)
    const textureB = billboard.mesh.material.map
    expect(textureB).toBe(textureA)
    billboard.dispose()
  })
})
