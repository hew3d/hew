/**
 * TextBillboard — DPR-aware canvas-texture text quads for in-scene
 * annotation labels (dimensions/leader text — docs/design/dimensions-text.md
 * "Rendering"). In-scene (a `THREE.Mesh` in the model's own coordinate
 * space), not a DOM overlay, so annotation text appears in captured frames
 * (`captureFrame`/export screenshots) and stays depth-consistent with the
 * geometry it labels, unlike `InferenceTooltip`'s cursor-anchored DOM chip.
 *
 * Two independent concerns, deliberately factored apart:
 *
 *  - **Billboard sizing** (`billboardWorldScale`, `glyphContentFraction`):
 *    pure math, no `THREE`/DOM dependency, unit-tested directly. Mirrors
 *    `FollowMeTool.updateGripScale`'s screen-constant grip formula
 *    (`worldSize = px · dist · tan(fov/2) / viewportHeight`) — the same
 *    reasoning applies to text: it should read as the same physical size on
 *    screen at any zoom, like SketchUp's own dimension text. `screenPxHeight`
 *    (the constructor argument) means the visible GLYPH height, not the
 *    padded canvas quad's — `glyphContentFraction` corrects for the halo/
 *    line-height padding baked into the rasterized canvas (see its own doc
 *    comment; this was Finding 1 of the playtest round: a quad held to a
 *    constant screen size still reads small if the glyph only fills half of
 *    it). This correction and the screen-constant formula both run every
 *    frame in `update()` — genuinely continuous, not tiered.
 *  - **`TextBillboard`**: the canvas-rasterize-to-texture mechanics. Content
 *    (text/color) changes rasterize the canvas; `update()` (called every
 *    render frame, like `updateGripScale`) only re-orients/re-scales the
 *    already-built quad — no canvas work per frame, per the design doc's "no
 *    per-frame canvas churn".
 *  - **Inline orientation** (`inlineTextQuaternion`): pure math, no `THREE`
 *    DOM dependency beyond `THREE`'s own vector/matrix types and a camera's
 *    projection matrix (both work without a real DOM). A dimension/radial
 *    label's default orientation is laid flat in the annotation's own plane,
 *    aligned with its dimension/leader line (SketchUp's "aligned" dimension
 *    text), NOT billboarded, and kept reading left-to-right/upright from any
 *    viewpoint via a screen-space readability check (see the function's own
 *    doc comment) — `SceneRenderer.updateAnnotationBillboards` passes the
 *    resulting quaternion as `update()`'s `faceQuaternion` override, and
 *    persists the resulting flip state per-annotation across frames so the
 *    check has continuity to fall back on in its edge-on case. Leader-text
 *    annotations pass nothing and keep the default camera-facing billboard
 *    (an explicit maintainer decision, docs/design/dimensions-text.md).
 *
 * Re-rasterization triggers (`setContent`): text content, fill color (theme
 * change), devicePixelRatio (a window dragged to another display), and a
 * crossed *zoom tier* (`zoomTierFor`) — three coarse camera-distance bands
 * that bump the canvas's internal supersample RESOLUTION (crispness) so
 * close-up glyphs stay sharp without retriggering on every dolly-wheel tick
 * of ordinary zooming. This tiering is a rasterization-quality knob only —
 * it has no bearing on the on-screen SIZE of the label, which `update()`
 * computes continuously (see above).
 *
 * Any of those triggers can also change the rasterized canvas's PIXEL
 * DIMENSIONS (a longer/shorter string, a supersample bump/drop across a
 * zoom-tier crossing) — `_rasterize` recreates the `THREE.CanvasTexture`
 * object itself whenever that happens, rather than reusing one across a
 * canvas resize. A WebGL2 backend allocates a `CanvasTexture`'s GPU storage
 * immutably at whatever size its FIRST upload had; reusing the same texture
 * object against a later, differently-sized canvas only `texSubImage2D`s
 * into that fixed storage's corner, leaving stale pixels from the previous
 * (often larger) rasterization visible around the new content — a second,
 * cropped ghost of the old label sharing the same quad. See `_rasterize`'s
 * own comment for the full mechanism.
 */
import * as THREE from 'three'

/** Screen-constant world-space HALF-extent (at `dist` from the camera, along
 * the vertical FOV direction) that projects to `screenPx` on-screen pixels as
 * a FULL size — `FollowMeTool.updateGripScale`'s formula (identical to
 * `viewport/math.ts`'s `screenConstantWorldHalf`, which spells out the
 * "half" in its name). Stable under both FOV change and viewport resize
 * (unlike a naive `k · dist` shorthand). Returns 0 for a non-positive
 * `viewportHeight` (avoids a division blow-up).
 *
 * This is a HALF-extent: correct to use directly as a uniform scale on a
 * radius-1-native geometry (a unit sphere/ring/circle, whose own "1" already
 * means radius — `FollowMeTool`'s grip markers), but a caller scaling a
 * diameter-native geometry (e.g. `THREE.PlaneGeometry(1,1)`, whose "1" means
 * full width/height) must DOUBLE this before using it as that geometry's
 * scale — `TextBillboard.update()` does exactly that. Forgetting the doubling
 * is exactly the kind of latent bug that under-scaled this file's own label
 * quads (Finding 1, docs/design/dimensions-text.md — labels rendered at half
 * their nominal size before this was caught).
 */
export function billboardWorldScale(
  screenPx: number,
  cameraDistance: number,
  fovDegrees: number,
  viewportHeight: number,
): number {
  if (viewportHeight <= 0) return 0
  const tanHalfFov = Math.tan((fovDegrees * Math.PI) / 360)
  return (screenPx * cameraDistance * tanHalfFov) / viewportHeight
}

/** Distance thresholds (meters) separating the three zoom tiers — see the
 * module doc comment. Tier 0 (closest) supersamples the hardest; tier 2
 * (farthest) supersamples the least, since a screen-constant quad occupies
 * the same screen pixels either way and a far billboard is rarely the focus
 * of a close read. */
const ZOOM_TIER_DISTANCES = [5, 20] as const

/** Canvas supersample multiplier per zoom tier (see `zoomTierFor`). */
export const ZOOM_TIER_SUPERSAMPLE = [1.5, 1.0, 0.75] as const

/** Which of the three discrete zoom tiers `distance` (camera-to-billboard,
 * meters) falls into. Coarse buckets, not a continuous function of distance
 * — the point is to avoid re-rasterizing on every frame of an ordinary
 * dolly/zoom (the design's "no per-frame canvas churn"); only crossing a
 * tier boundary triggers a fresh canvas rasterization. */
export function zoomTierFor(distance: number): 0 | 1 | 2 {
  if (distance < ZOOM_TIER_DISTANCES[0]) return 0
  if (distance < ZOOM_TIER_DISTANCES[1]) return 1
  return 2
}

/** Base canvas font size (CSS px, before DPR/supersample) — an internal
 * rasterization detail; the on-screen size is governed entirely by
 * `screenPxHeight` + `update()`'s billboard scaling, not by this constant. */
const BASE_FONT_PX = 40
/** Horizontal/vertical canvas padding around the text, as a fraction of
 * `BASE_FONT_PX`, so the halo stroke never clips at the canvas edge. */
const PAD_X_FRAC = 0.28
const PAD_Y_FRAC = 0.34
/** Halo stroke width, as a fraction of the font size (readability against
 * either theme's background, mirroring `toolIcons.ts`'s cursor-glyph halo). */
const HALO_WIDTH_FRAC = 0.16

function hexToCss(color: number): string {
  return `#${(color & 0xffffff).toString(16).padStart(6, '0')}`
}

/** Fraction of the CSS font size (em) approximating a REFERENCE DIGIT's
 * visible ink height — the same quantity `measureReferenceDigitInkPx` below
 * measures directly (the `actualBoundingBoxAscent`/`Descent` of the glyph
 * `'0'`) — used as a stand-in only when the canvas backend doesn't report
 * usable `TextMetrics.actualBoundingBox*` values at all (a NaN/absent-metrics
 * backend; this rarely fires in practice). Because this constant and the
 * measured path now approximate the SAME quantity — a digit's ink, not the
 * font's em-box (`fontBoundingBoxAscent`/`Descent`, which runs ~1.2-1.33 EM,
 * nearly double a digit's real ~0.72 EM of ink) — a backend without full
 * `TextMetrics` support (older Firefox, a partial-TextMetrics test DOM) lands
 * close to what a backend WITH full support would measure, instead of
 * silently approximating a ~1.7x-larger quantity (Finding 2 of the second
 * playtest round, docs/design/dimensions-text.md). */
const FALLBACK_GLYPH_HEIGHT_FRAC = 0.72

/** A `glyphContentFraction` result is clamped to this floor so a
 * pathological (near-zero) ink measurement can't blow up the resulting
 * on-screen scale in `update()`. */
const MIN_GLYPH_CONTENT_FRAC = 0.15

/**
 * Fraction of the rasterized canvas's height (`canvasHeightPx`) that
 * corresponds to a REFERENCE DIGIT's visible ink at `fontPx` —
 * `referenceDigitInkPx`, in the same canvas-px units as `fontPx`/
 * `canvasHeightPx` (see `measureReferenceDigitInkPx`: the
 * `actualBoundingBoxAscent + actualBoundingBoxDescent` of the glyph `'0'`,
 * measured with the SAME font string `_rasterize` rasterizes with, cached
 * per font string). Falls back to `FALLBACK_GLYPH_HEIGHT_FRAC * fontPx` when
 * `referenceDigitInkPx` comes back non-finite or non-positive (a backend
 * that doesn't populate `actualBoundingBox*` at all).
 *
 * Deliberately keyed off a REFERENCE glyph's ink, not the font's em-box
 * (`fontBoundingBoxAscent`/`Descent`) and not the actual label's own ink
 * (`actualBoundingBoxAscent`/`Descent` of the string being drawn). Both
 * alternatives were tried, and each broke a different real property:
 *
 *  - Per-LABEL ink (the drawn string's own `actualBoundingBox*`) varies with
 *    content — a digit-only label like "12" has no descenders and measures
 *    shorter ink than a descender-bearing label like "doorway gap" at the
 *    same `fontPx`, so editing a label's text visibly changed its on-screen
 *    size (Finding 2 of the first playtest round).
 *  - Switching to the font's em-box (`fontBoundingBoxAscent`/`Descent`)
 *    fixed that — content-independent, same denominator for every label at a
 *    given `fontPx` — but the em-box runs ~1.2-1.33 EM, nearly double a
 *    digit's actual ~0.72 EM of ink. Dividing `screenPxHeight` by that
 *    inflated denominator (`update()`'s `quadScreenPx = screenPxHeight /
 *    glyphContentFraction`) shrank every label's real visible ink to well
 *    under the `ANNOTATION_TEXT_SCREEN_PX` contract (~8-9px against a 14px
 *    target), regressing toward the ORIGINAL "unreadably small" playtest
 *    complaint this whole mechanism exists to fix (Finding 1 of the second
 *    playtest round).
 *
 * A REFERENCE digit's ink (measured once, from `'0'`, independent of
 * whatever the label being rasterized actually says) has both properties at
 * once: content-independent — every label at a given `fontPx` gets the same
 * denominator, so "12" and "doorway gap" render at the same apparent glyph
 * size — AND ink-calibrated — a digit label's OWN visible ink lands at
 * exactly `screenPxHeight`, since the denominator IS a digit's ink (see
 * `TextBillboard`'s end-to-end sizing tests). `FALLBACK_GLYPH_HEIGHT_FRAC`
 * is chosen to approximate this SAME quantity, so a backend without
 * `actualBoundingBox*` support lands close to the measured path instead of a
 * different, diverging one (Finding 2 of the second playtest round).
 */
export function glyphContentFraction(referenceDigitInkPx: number, fontPx: number, canvasHeightPx: number): number {
  if (canvasHeightPx <= 0) return 1
  const inkPx = isFinite(referenceDigitInkPx) && referenceDigitInkPx > 0 ? referenceDigitInkPx : fontPx * FALLBACK_GLYPH_HEIGHT_FRAC
  const frac = inkPx / canvasHeightPx
  return Math.min(1, Math.max(MIN_GLYPH_CONTENT_FRAC, frac))
}

/**
 * Cache of a REFERENCE DIGIT's measured ink height (canvas px —
 * `actualBoundingBoxAscent + actualBoundingBoxDescent` for the glyph `'0'`),
 * keyed by the exact CSS font string (`ctx.font`, e.g.
 * `"60px -apple-system, ..."`) used to both MEASURE and RASTERIZE. A given
 * font string (family/weight/size) always measures the same digit ink, so
 * this is a per-FONT-STRING cache, not per-label or per-`TextBillboard`
 * instance — every label at a given zoom tier/DPR shares one entry, and the
 * extra `measureText('0')` call this needs (beyond the one `_rasterize`
 * already does for whatever the label actually says) fires once per distinct
 * font string ever seen, not once per label or per frame. `NaN` is cached
 * too (a backend without `actualBoundingBox*` support won't gain it later in
 * the same session), so a failing lookup doesn't retry every rasterize.
 */
const referenceDigitInkCache = new Map<string, number>()

/**
 * Measures (and caches, see `referenceDigitInkCache`) the reference-digit ink
 * height described above. `ctx.font` MUST already be set to the exact font
 * string `_rasterize` is about to draw with — same `fontPx` (which already
 * bakes in DPR and the zoom-tier supersample factor), same family — before
 * calling this, so the measurement happens at the SAME scale the label is
 * actually rasterized at: measure-scale must equal raster-scale, or this
 * would silently reintroduce a calibration error of its own, the same shape
 * of bug as Finding 1, just relocated. Returns `NaN` — rather than a
 * misleading `0` — when the backend's `TextMetrics` doesn't populate
 * `actualBoundingBox*`, so `glyphContentFraction`'s caller falls through to
 * `FALLBACK_GLYPH_HEIGHT_FRAC`.
 */
function measureReferenceDigitInkPx(ctx: CanvasRenderingContext2D): number {
  const key = ctx.font
  const cached = referenceDigitInkCache.get(key)
  if (cached !== undefined) return cached
  const metrics = ctx.measureText('0')
  const ink = metrics.actualBoundingBoxAscent + metrics.actualBoundingBoxDescent
  const value = isFinite(ink) && ink > 0 ? ink : NaN
  referenceDigitInkCache.set(key, value)
  return value
}

/** Numeric floor below which a vector is treated as degenerate — used by
 * `inlineTextQuaternion`'s in-plane axis fallbacks. */
const MIN_AXIS_LENGTH_SQ = 1e-12

/** Below this NDC-x delta, the probed baseline direction is treated as
 * viewed edge-on (near-zero on-screen extent) — too close to call, so
 * `inlineTextQuaternion` keeps the previous frame's flip state rather than
 * letting float noise flip a decision back and forth. */
const MIN_SCREEN_DX = 1e-6

/**
 * Whether `right` (a direction anchored at `textPosition`) currently projects
 * to a NEGATIVE on-screen x, i.e. would read right-to-left — probed by
 * projecting two actual points (`textPosition` and a point offset along
 * `right`) through `camera`, the same NDC convention (`Vector3.project`)
 * `Viewport.tsx`'s `worldToPixels` and this module's own sizing tests use,
 * rather than a linear dot-product against the camera's basis — that stays
 * correct under perspective distortion for an off-center label, not just one
 * sitting on the view axis. Returns `null` when the probe is degenerate
 * (camera colocated with the label, or the baseline viewed edge-on — its
 * projected screen extent collapses to ~0) so the caller can hold its
 * previous flip state instead of guessing.
 */
function baselineReadsRightToLeft(camera: THREE.Camera, textPosition: THREE.Vector3, right: THREE.Vector3): boolean | null {
  const dist = camera.position.distanceTo(textPosition)
  if (dist < 1e-9) return null
  const eps = dist * 0.01
  const a = textPosition.clone().project(camera)
  const b = textPosition.clone().addScaledVector(right, eps).project(camera)
  const dx = b.x - a.x
  if (Math.abs(dx) < MIN_SCREEN_DX) return null
  return dx < 0
}

/**
 * The world-space quaternion for an INLINE dimension/radial label: lying
 * flat in the plane whose unit normal is `planeNormal`, its baseline aligned
 * with `alignDir` (a linear dimension's a→b direction, or a radial
 * dimension's leader direction) — SketchUp's "aligned" dimension-text
 * default (docs/design/dimensions-text.md "Rendering").
 *
 * Two independent corrections are layered on top of that base orientation,
 * in order:
 *
 *  1. **Facing** — rotate 180° about the in-plane `up` axis whenever the
 *     camera sits on the plane's far side (`towardCamera · normal < 0`), so
 *     the SAME physical face of the canvas quad — the one with the correctly
 *     drawn (non-mirrored) glyphs — always points at the camera. This is
 *     NOT a readability nicety: `TextBillboard`'s material is `DoubleSide`,
 *     so without this correction, viewing the quad from its back would show
 *     the canvas texture's true mirrored backface (reading through the back
 *     of a printed page) — no rotation *about the label's own normal* can
 *     fix that, only re-presenting the drawn face by rotating about an axis
 *     that lies IN the plane. Kept from the original implementation; not
 *     something the screen-space rule below can absorb.
 *  2. **Readability** (the actual fix for the playtest round's Finding 1) —
 *     after the facing correction above, rotate 180° about the (now facing-
 *     corrected) plane NORMAL — negate right AND up, keep normal — whenever
 *     the baseline (`baselineReadsRightToLeft`) currently projects to
 *     negative screen-x, i.e. reads right-to-left on screen. This is the
 *     SketchUp-style screen-space rule: it fires from the actual on-screen
 *     reading direction, so it self-corrects for BOTH a dimension whose two
 *     endpoints were clicked in the "other" order (there is no canonical
 *     `alignDir` sign to rely on) AND a camera that orbits past the label
 *     without ever crossing its plane (the common case for a ground-plane
 *     dimension in this Z-up app: azimuth can flip the on-screen reading
 *     direction while the camera stays on the same side of the plane the
 *     whole time, which the old plane-crossing check could never detect).
 *     `previousFlipped` is the flip state computed for this same annotation
 *     on the prior frame; when the baseline is viewed edge-on (its projected
 *     screen extent is ~0 — `baselineReadsRightToLeft` returns `null`), that
 *     prior state is reused rather than guessed, so the label doesn't
 *     oscillate as the view passes through the degenerate angle. The very
 *     first frame for a given annotation has no prior state; callers default
 *     `previousFlipped` to `false` (un-flipped) then.
 *
 * Degenerate inputs (an `alignDir` of ~zero length, or one parallel to
 * `planeNormal`) fall back to an arbitrary but stable in-plane axis rather
 * than producing a NaN quaternion — callers should treat that case as "this
 * annotation's geometry is degenerate," not rely on the fallback's exact
 * direction. `alignDir`/`planeNormal` need not be pre-normalized.
 */
export function inlineTextQuaternion(
  alignDir: THREE.Vector3,
  planeNormal: THREE.Vector3,
  camera: THREE.Camera,
  textPosition: THREE.Vector3,
  previousFlipped: boolean,
): { quaternion: THREE.Quaternion; flipped: boolean } {
  const normal = planeNormal.lengthSq() > MIN_AXIS_LENGTH_SQ ? planeNormal.clone().normalize() : new THREE.Vector3(0, 0, 1)

  let right = alignDir.clone().projectOnPlane(normal)
  if (right.lengthSq() < MIN_AXIS_LENGTH_SQ) {
    // alignDir is ~parallel to the plane normal (degenerate dimension
    // geometry) — pick whichever world axis is least aligned with normal so
    // projecting it in-plane is well-conditioned.
    const fallbackAxis = Math.abs(normal.x) < 0.9 ? new THREE.Vector3(1, 0, 0) : new THREE.Vector3(0, 1, 0)
    right = fallbackAxis.projectOnPlane(normal)
  }
  right.normalize()

  const up = new THREE.Vector3().crossVectors(normal, right).normalize()
  let effRight = right
  let effUp = up
  let effNormal = normal.clone()

  // 1. Facing — see doc comment above.
  const towardCamera = camera.position.clone().sub(textPosition)
  if (towardCamera.dot(effNormal) < 0) {
    effRight = effRight.clone().multiplyScalar(-1)
    effNormal = effNormal.clone().multiplyScalar(-1)
  }

  // 2. Readability — see doc comment above.
  const rtl = baselineReadsRightToLeft(camera, textPosition, effRight)
  const flipped = rtl ?? previousFlipped
  if (flipped) {
    effRight = effRight.clone().multiplyScalar(-1)
    effUp = effUp.clone().multiplyScalar(-1)
  }

  const basis = new THREE.Matrix4().makeBasis(effRight, effUp, effNormal)
  return { quaternion: new THREE.Quaternion().setFromRotationMatrix(basis), flipped }
}

/**
 * One billboarded text quad. `screenPxHeight` is the on-screen height (CSS
 * px) a REFERENCE DIGIT's visible ink is held to via `update()` — analogous
 * to `FollowMeTool`'s `screenPx` marker parameter. Because the correction
 * (`glyphContentFraction`) is calibrated from a reference digit's ink, not
 * the label's own, a label with no digits at all (pure Latin leader text)
 * reads at the same apparent glyph size as a digit-only dimension label —
 * see that function's doc comment.
 */
export class TextBillboard {
  readonly mesh: THREE.Mesh<THREE.PlaneGeometry, THREE.MeshBasicMaterial>

  private readonly canvas: HTMLCanvasElement
  private readonly ctx: CanvasRenderingContext2D | null
  // NOT readonly — `_rasterize` reassigns this to a brand-new `CanvasTexture`
  // whenever the canvas's pixel dimensions change (see `_rasterize`'s "stale
  // GPU texture storage" comment). Reusing one `CanvasTexture` instance
  // across a canvas resize is exactly the bug this dance avoids.
  private texture: THREE.CanvasTexture
  private readonly screenPxHeight: number

  private text = ''
  private color = 0xffffff
  private dpr = 1
  private tier: 0 | 1 | 2 = 1
  /** Canvas aspect (width/height) of the last rasterization — drives the
   * quad's non-uniform scale so glyphs aren't stretched. */
  private aspect = 1
  /** Fraction of the last rasterization's canvas height that is a REFERENCE
   * DIGIT's stable ink band (`glyphContentFraction`) — defaults to 1 (no
   * correction) before any successful rasterization, matching `aspect`'s
   * pre-raster default. `update()` divides its target screen height by this
   * so `screenPxHeight` means a digit's visible ink height, not the padded
   * canvas's (Finding 1, docs/design/dimensions-text.md) — and, because the
   * denominator comes from a reference digit rather than each label's own
   * ink, every label (digits or descender-bearing leader text alike) gets
   * the same correction, so `screenPxHeight` reads as the same apparent
   * glyph size across content (Finding 2). */
  private contentFrac = 1

  /**
   * `depthBias`, when given, is a `glPolygonOffset` factor/units applied to
   * the label quad — the same mechanism (and, for annotation callers, the
   * same `DEPTH_BIAS.ANNOTATION` value — deliberately zero, see
   * `depthPolicy.ts`'s "Dimension/leader-text annotations" note)
   * `SceneRenderer.ts` applies to annotation fat lines, so a label anchored
   * exactly on a coplanar face (a leader-text anchor with no offset, or a
   * broken dimension's label point) is affected by the SAME depth-bias
   * policy the line is, kept explicit here rather than silently defaulting,
   * so a future retune of `DEPTH_BIAS.ANNOTATION` covers the label quad too
   * without a separate edit. Omitted → no offset (plain depth test, the
   * pre-existing default for any other `TextBillboard` caller).
   */
  constructor(screenPxHeight: number, renderOrder: number, depthBias?: number) {
    this.screenPxHeight = screenPxHeight
    this.canvas = document.createElement('canvas')
    // Some environments (older/headless test DOMs without a canvas 2D
    // polyfill) return null here — degrade to an empty texture rather than
    // throw; `update()` (pure THREE math) still works so billboard
    // positioning stays testable without a real canvas backend.
    this.ctx = this.canvas.getContext('2d')
    this.canvas.width = 2
    this.canvas.height = 2
    this.texture = new THREE.CanvasTexture(this.canvas)
    this.texture.minFilter = THREE.LinearFilter
    this.texture.magFilter = THREE.LinearFilter
    const geo = new THREE.PlaneGeometry(1, 1)
    const mat = new THREE.MeshBasicMaterial({
      map: this.texture,
      transparent: true,
      // Depth-tested like any other document ink (docs/design/
      // dimensions-playtest2.md §1 — findings 2/3: annotations used to read
      // through the model on a permanent overlay via depthTest:false + a
      // high renderOrder; SketchUp's dimensions/leader text are ordinary
      // depth-tested ink that geometry in front hides). `depthWrite: false`
      // keeps the usual transparent-quad convention — a label doesn't need
      // to occlude anything ELSE drawn after it, only be occluded BY solid
      // geometry already in the depth buffer.
      depthTest: true,
      depthWrite: false,
      side: THREE.DoubleSide,
      polygonOffset: depthBias !== undefined,
      polygonOffsetFactor: depthBias ?? 0,
      polygonOffsetUnits: depthBias ?? 0,
    })
    this.mesh = new THREE.Mesh(geo, mat)
    this.mesh.renderOrder = renderOrder
  }

  /**
   * Rasterize `text` in `color` (a `0xRRGGBB` hex, theme-aware fill) at
   * `dpr` and the zoom tier for `cameraDistance`. A no-op when none of
   * text/color/dpr/tier actually changed since the last call — the "no
   * per-frame canvas churn" guarantee; callers may call this every frame
   * without cost as long as inputs are stable.
   */
  setContent(text: string, color: number, dpr: number, cameraDistance: number): void {
    const tier = zoomTierFor(cameraDistance)
    if (text === this.text && color === this.color && dpr === this.dpr && tier === this.tier) {
      return
    }
    this.text = text
    this.color = color
    this.dpr = dpr
    this.tier = tier
    this._rasterize()
  }

  private _rasterize(): void {
    if (this.ctx === null) return
    const supersample = ZOOM_TIER_SUPERSAMPLE[this.tier]
    const fontPx = BASE_FONT_PX * this.dpr * supersample
    const ctx = this.ctx
    // Measure at the target font BEFORE resizing the canvas — a canvas
    // resize clears its 2D state, so font is re-applied after. This also
    // measures the reference-digit ink (below) at the SAME scale, per
    // `measureReferenceDigitInkPx`'s "measure-scale must equal raster-scale"
    // note.
    ctx.font = `${fontPx}px ${UI_FONT_STACK}`
    const label = this.text === '' ? ' ' : this.text
    const textW = Math.max(1, ctx.measureText(label).width)
    const referenceDigitInkPx = measureReferenceDigitInkPx(ctx)
    const padX = fontPx * PAD_X_FRAC
    const padY = fontPx * PAD_Y_FRAC
    const w = Math.ceil(textW + padX * 2)
    const h = Math.ceil(fontPx * 1.2 + padY * 2)
    // A canvas resize (right below) is exactly what a REUSED `CanvasTexture`
    // cannot tolerate on a WebGL2 backend: the renderer allocates that
    // texture's GPU storage immutably (`texStorage2D`), sized to whatever the
    // FIRST upload was, and every later upload — even to a differently-sized
    // canvas — only `texSubImage2D`s into that fixed storage's top-left
    // corner. Shrinking (this app's supersample zoom tiers shrink the canvas
    // when the camera moves far away) leaves the REST of the old, larger
    // texture holding stale pixels from the previous, bigger rasterization —
    // rendered as a second, larger, horizontally-cropped ghost of the old
    // label sharing the same quad. Growing can silently fail to upload past
    // the original bounds. Recreating the texture object (a fresh `Source`,
    // so the renderer allocates fresh GPU storage sized to the new canvas)
    // whenever the pixel dimensions actually change avoids both failure
    // modes; a same-size rasterization (a text-only edit at an unchanged
    // width, or a `color`/`dpr` change alone) keeps reusing the existing
    // texture/GPU storage via the ordinary, cheap `texSubImage2D` path below.
    const dimsChanged = w !== this.canvas.width || h !== this.canvas.height
    this.canvas.width = w
    this.canvas.height = h
    ctx.font = `${fontPx}px ${UI_FONT_STACK}`
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.clearRect(0, 0, w, h)
    const cx = w / 2
    const cy = h / 2
    ctx.lineJoin = 'round'
    ctx.lineWidth = fontPx * HALO_WIDTH_FRAC
    ctx.strokeStyle = haloColorFor(this.color)
    ctx.strokeText(label, cx, cy)
    ctx.fillStyle = hexToCss(this.color)
    ctx.fillText(label, cx, cy)
    if (dimsChanged) {
      this.texture.dispose()
      this.texture = new THREE.CanvasTexture(this.canvas)
      this.texture.minFilter = THREE.LinearFilter
      this.texture.magFilter = THREE.LinearFilter
      this.mesh.material.map = this.texture
      this.mesh.material.needsUpdate = true
    } else {
      this.texture.needsUpdate = true
    }
    this.aspect = w / h
    // Reference-digit ink (`'0'`, cached per font string) — not this label's
    // own `actualBoundingBox*`, and not the font's `fontBoundingBox*`
    // em-box — see glyphContentFraction's doc comment, Findings 1 & 2.
    this.contentFrac = glyphContentFraction(referenceDigitInkPx, fontPx, h)
  }

  /**
   * Orient the quad and hold a REFERENCE DIGIT's visible ink at a constant
   * on-screen height (`screenPxHeight`, corrected for canvas padding via
   * `contentFrac`, which is calibrated from a reference digit's ink, not the
   * label's own — see `contentFrac`'s and `glyphContentFraction`'s doc
   * comments) regardless of camera distance/FOV/viewport size — call once
   * per render frame, mirroring `FollowMeTool.updateGripScale`.
   * A no-op for a non-positive `viewportHeight` or a camera that is neither
   * perspective nor orthographic.
   *
   * `faceQuaternion`, when given, replaces the default camera-facing
   * billboard orientation — the INLINE dimension/radial text case
   * (`inlineTextQuaternion`, Finding 2): the quad still sizes itself the
   * same continuous, screen-constant way, it just doesn't face the camera.
   * Omit it (leader-text annotations) to keep the original billboard
   * behavior.
   */
  update(camera: THREE.Camera, viewportHeight: number, faceQuaternion?: THREE.Quaternion): void {
    const size = this.worldSize(camera, viewportHeight)
    if (size === null) return
    this.mesh.quaternion.copy(faceQuaternion ?? camera.quaternion)
    this.mesh.scale.set(size.width, size.height, 1)
  }

  /**
   * The world-space (width, height) this billboard's quad currently occupies
   * at `viewportHeight`/`camera` — the SAME screen-constant sizing `update()`
   * applies to `mesh.scale`, exposed as a query rather than a mutation so a
   * caller (the dimension-line/label gap layout, docs/design/
   * dimensions-playtest2.md §2) can measure the label BEFORE deciding where
   * to draw the line around it, without first having to move/orient the
   * mesh. `null` for an unsupported camera type or a non-positive
   * `viewportHeight` (mirrors `update()`'s own no-op guard).
   */
  worldSize(camera: THREE.Camera, viewportHeight: number): { width: number; height: number } | null {
    if (viewportHeight <= 0) return null
    const quadScreenPx = this.screenPxHeight / this.contentFrac
    let worldHeight: number
    if (camera instanceof THREE.PerspectiveCamera) {
      const dist = camera.position.distanceTo(this.mesh.position)
      // `billboardWorldScale` returns a HALF-extent (see its doc comment);
      // this mesh's `PlaneGeometry(1,1)` is diameter-native (its "1" is a
      // full width/height), so the FULL on-screen height needs double that.
      worldHeight = 2 * billboardWorldScale(quadScreenPx, dist, camera.fov, viewportHeight)
    } else if (camera instanceof THREE.OrthographicCamera) {
      // Parallel projection: world-per-pixel is uniform — the visible frustum
      // height over the viewport height (`worldPerPixelOrtho`'s formula).
      // Before this branch an orthographic camera was a silent no-op, which
      // left dimension labels frozen at their last perspective size (and
      // their line gaps un-laid-out) under View ▸ Parallel Projection.
      const frustumHeight = (camera.top - camera.bottom) / camera.zoom
      worldHeight = (quadScreenPx * frustumHeight) / viewportHeight
    } else {
      return null
    }
    return { width: worldHeight * this.aspect, height: worldHeight }
  }

  dispose(): void {
    this.texture.dispose()
    this.mesh.geometry.dispose()
    this.mesh.material.dispose()
  }
}

/** Matches `--font-family-ui` (theme/tokens.css) — canvas text has no
 * webfont-loading race since every listed face is a system stack. */
const UI_FONT_STACK = '-apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif'

/** A halo/stroke color contrasting with `fill` — dark fill gets a light
 * halo and vice versa, so the label reads over either theme's background
 * (mirrors `toolIcons.ts`'s cursor-glyph halo technique). Judged by the
 * fill's own luminance rather than the app theme, so it stays correct even
 * if a caller ever passes an unusual fill color (e.g. the detached-warning
 * color) in either theme. */
function haloColorFor(fill: number): string {
  const r = (fill >> 16) & 0xff
  const g = (fill >> 8) & 0xff
  const b = fill & 0xff
  const luminance = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255
  return luminance > 0.5 ? 'rgba(0,0,0,0.65)' : 'rgba(255,255,255,0.65)'
}
