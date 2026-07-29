/**
 * depthPolicy — the single depth-bias ladder for everything that renders at
 * (or on) the same surface: faces, their edge overlays, sketch lines, region
 * fills, tool rubber-bands, and the origin axes.
 *
 * ## Why a ladder exists
 *
 * Several layers are *geometrically coincident by design*: the edge overlay
 * traces the exact vertices of the faces it outlines, sketch lines lie in the
 * plane of the face (or ground) they were drawn on, and the origin axes run
 * along z=0 where ground-sketch lines and object edges also live. Coincident
 * fragments land within one depth-buffer quantum of each other, and the depth
 * test then resolves the tie by floating-point rounding noise. That noise is
 * a function of the camera matrices, so the damping tail after an orbit
 * (dozens of repaints at sub-pixel camera deltas) re-rolls it every frame —
 * visible as shimmer wherever two coincident layers disagree in color
 * (dark edge vs. face fill, blue axis vs. dark cube edge, …).
 *
 * ## Policy intent: model work beats reference geometry
 *
 * The origin axes are reference geometry — a drawing aid, like the grid.
 * Anything the user actually draws on top of an axis (a sketch line traced
 * along it, or the edge of a solid built at the origin) is the thing they
 * came to see, and must read in front of the axis every time, at every
 * zoom. The axes still belong in front of *faces* — they are a drawing aid
 * for placing geometry, not a backdrop to be hidden by the first face that
 * happens to pass through the origin. Concretely: axes render behind every
 * class of linework (previews, region fills, sketch lines, and native model/
 * instance edges) and in front of face fills only.
 *
 * The wrong fix is a world-space lift (moving a layer up by a millimetre or
 * two): it is invisible at metre scale but *geometrically false*, and at
 * centimetre scale the parallax is glaring — the viewport once lifted the
 * axes to z=+0.002 and region fills / ground rubber-bands by +0.001, and a
 * 5 cm square drawn at the origin visibly floated below the axes it should
 * have been coplanar with.
 *
 * The right fix is `glPolygonOffset`: a *depth-only* bias applied at
 * rasterization, scaled to the depth-buffer's own resolution at whatever
 * depth the fragment lands (`offset = factor·m + units·r`). Geometry stays
 * exactly where it belongs at every zoom; only depth-test ties resolve
 * deterministically. One precondition: a bias of a few quanta can only
 * out-vote *interpolation* noise when the rasterizer's depth interpolation
 * is itself tight, which for fat lines means no endpoint may project to
 * extreme off-screen coordinates — `clampOriginAxes` (Viewport.tsx) is the
 * companion that guarantees this for the 150 m axes.
 *
 * ## The ladder (front → back; entries are polygonOffset factor & units)
 *
 * | bias | layer                                   | primitive               |
 * |------|-----------------------------------------|--------------------------|
 * |  -2  | tool rubber-bands, sketch region fills  | fat lines / triangles    |
 * |  -1  | committed sketch lines                  | fat lines (`LineSegments2`) |
 * |   0  | object edge overlays, guide lines, dim/leader annotations | native `GL_LINES` (fixed), fat lines, quads |
 * |  +1  | origin axes                             | fat lines (`Line2`)      |
 * |  +2  | face fills                              | triangles                |
 *
 * Native `GL_LINES` primitives are the fixed reference at 0: `glPolygonOffset`
 * only applies to polygon primitives, so plain `LineSegments` can never be
 * biased — every other layer is placed around them. Fat lines (three's
 * `Line2`/`LineSegments2`) render as camera-facing triangle strips whose
 * fragment depth is the underlying segment's depth, so the offset applies to
 * them like any polygon. Dimension/leader-text annotations join this
 * reference rung too, deliberately carrying NO polygonOffset of their own —
 * see the "Dimension/leader-text annotations" note below for why that,
 * rather than a nonzero rung, is the fix.
 *
 * The ordering puts every class of model linework — previews, region fills,
 * sketch lines, and native edges — in front of the axes, and the axes in
 * front of faces: rubber-bands and fills over committed lines, sketch lines
 * and edges over the axes, axes over face fills. One-integer gaps are
 * deliberate — each layer clears its neighbour by a full resolvable depth
 * unit, which is exactly what a deterministic tie-break needs; wider gaps
 * would only grow the (harmless but nonzero) epsilon by which a biased layer
 * can poke through genuinely nearer geometry.
 *
 * Dimension/leader-text annotations (fat-line geometry + label quads,
 * `SceneRenderer.ts`'s `_installAnnotationLineBatch`/`TextBillboard`) sit at
 * rung 0 — meaning, concretely, they carry NO `glPolygonOffset` of their
 * own. That is the second design tried for this problem, after two other
 * approaches were measured and rejected:
 *
 *  - A first attempt applied `glPolygonOffset` to the annotation geometry
 *    itself, uncapped, sized to out-rank EVERY rung on this ladder (i.e.
 *    more negative than -2) so an annotation would win against literally
 *    anything it might be coincident with. Wrong target: `glPolygonOffset`'s
 *    bias is bounded by local depth-buffer precision, which grows with
 *    distance, so an offset large enough to beat the whole ladder at typical
 *    "see the whole model" distances leaked several millimetres past a
 *    genuinely nearer occluder — reintroducing the "annotations read through
 *    the model" bug the depth-tested-ink change existed to fix.
 *  - The fix at the time swapped to a world-space nudge instead: a per-vertex
 *    push toward the camera along the view direction, capped at a fixed
 *    0.3mm world-space ceiling regardless of camera distance
 *    (`ANNOTATION_BIAS_MAX_WORLD`/`annotationViewBiasVector`, both since
 *    removed from `annotationLayout.ts`). A DELTA review measured this
 *    broken too, on BOTH sides of the tradeoff it was meant to hold:
 *     - Occlusion: re-measured with a real frame yield between the camera
 *       move and the pixel sample (the shipped regression test lacked one —
 *       `pixelColorAt`'s underlying `captureFrame` does NOT itself call the
 *       per-frame annotation-billboard update, so it was sampling a bias
 *       baked for a STALE camera pose), the 0.3mm nudge already leaked past
 *       a 2mm-separated occluder by the reviewer's own "ordinary see-the-
 *       whole-model" distance (~11.6 world units) — the exact failure this
 *       mechanism was built to prevent.
 *     - The coincident-face tie itself was re-examined for a DIFFERENT
 *       structural reason: a VIEW-DIRECTION nudge has almost no component
 *       perpendicular to a surface viewed near edge-on, however large its
 *       cap, because the nudge direction (toward the camera) is itself
 *       nearly TANGENT to the surface at grazing incidence. No cap size
 *       fixes a direction problem. (This is the same failure mode the
 *       original design already rejected a plane-normal bias for — just
 *       reached by a different vector, since "toward the camera" degrades
 *       the same way "along the surface normal" does when the two are
 *       nearly parallel.)
 *  - `glPolygonOffset` on the annotation's own geometry was tried again, this
 *    time MODEST — rung -1, shared with `SKETCH_LINE`, a comfortable
 *    3-quantum margin under FACE rather than sized to beat the whole ladder.
 *    Measured against the review's own 2mm-occluder-gap reproduction across
 *    a distance sweep (before/after-same-point pixel comparison, real frame
 *    yields), this still leaked: correct at 6.6 world units, but already
 *    leaking past the occluder by 11.6 — the review's own target distance.
 *    Smaller magnitude than the uncapped first attempt's leak (which the
 *    original round measured leaking several millimetres at that same
 *    distance), but the same underlying mechanism: ANY nonzero offset on the
 *    annotation's own geometry pulls it toward the camera by an amount that
 *    grows with distance, eventually exceeding a small enough real gap.
 *
 * The fix that actually holds: give the annotation's own geometry NO offset
 * at all (rung 0, the same reference level as native edges). Annotations
 * only need to beat ONE thing — a coincident FACE (+2) — and FACE already
 * recedes on its own account (it has carried +2 since before annotations
 * existed, to lose against its own edge overlay). An annotation drawn
 * exactly on a face is closer, at its true unmodified depth, than that
 * face's own artificially-receded fragment — no annotation-side push
 * required, and so no annotation-side leak possible: an annotation's
 * rendered depth is always its GENUINE geometric depth, so it can never
 * appear closer than a real, non-coincident object actually is. (Losing a
 * tie against `PREVIEW`/`REGION_FILL` at -2, or `SKETCH_LINE` at -1, in the
 * rare case an annotation is coincident with one of those instead of a face,
 * is accepted — those are overlay layers, and reading under one during an
 * active gesture is fine.)
 *
 * Verified across a full grazing/edge-on-angle distance sweep (5-50 world
 * units, both a fixed camera-height sweep and one holding the shipped
 * edgeOn pose's exact ~0.4°-off-level angle constant, with a real frame
 * yield before every sample) that the coincident-face tie is won throughout,
 * and against the review's 2mm-occluder-gap reproduction (before/after-
 * same-point pixel comparison) that occlusion holds correctly at 6.6 AND
 * 11.6 world units — the review's own target distance — by
 * `app/e2e/dimensions-depth.spec.ts`. The residual, measured directly rather
 * than assumed: a 2mm gap between an annotation and a genuinely nearer
 * occluder starts failing to occlude correctly somewhere between 11.6 and
 * 15 world units, because FACE's own +2 recession (not the annotation's,
 * which is zero) grows in world-space terms at long range, same as it would
 * for any other rung on this ladder — an existing, orthogonal property of
 * `glPolygonOffset` shared by every layer here, not something specific to
 * annotations. A 30mm gap (the review's own control case) still occludes
 * correctly at 20 world units in the same measurement. Accepted as the
 * documented tradeoff per the design priority: correct occlusion at
 * ordinary working distances (and for anything but a sub-few-millimetre
 * gap) matters more than a coincident-tie win at a camera distance and gap
 * size combination far outside normal modeling precision.
 *
 * Rubber-bands and region fills sharing rung -2 is deliberate, and their one
 * same-bias encounter — an active rubber-band drawn coplanar over an
 * existing fill (re-drawing across a closed region) — is measured stable:
 * 0 hard / 0 differing pixel flips per sub-pixel repaint at near and far
 * poses (the fill never writes depth and only blends a low-alpha tint).
 * The pairing is pinned by the shared-rung spec in
 * `app/e2e/edge-stability.spec.ts`; anyone splitting these rungs or making
 * fills write depth must keep that spec green.
 *
 * Layers *not* on the ladder, and why:
 *  - the ground grid (`InfiniteGrid`) is an opaque backdrop that neither
 *    writes depth nor needs to win one — it draws first (renderOrder -1) and
 *    everything else paints over it, so it is geometrically at z=0 with no
 *    bias at all;
 *  - overlays with `depthTest: false` (selection highlights, inference cues,
 *    protractor/rotate widgets) opt out of depth entirely;
 *  - two *native-line* layers (an object edge coincident with another
 *    object's edge, or with a guide) cannot be separated — unbiasable — but
 *    coincident edge overlays share one color, so the tie is invisible; a
 *    guide coincident with an edge remains a known, cosmetically minor
 *    residual.
 *
 * Known residual pre-dating this ladder: a region fill's *interior* (not its
 * boundary, which is separate `SKETCH_LINE` geometry and fully covered by
 * the ladder) can still show the **dashed** axis half through it, because
 * both are `transparent: true` with `depthWrite: false` — three.js sorts the
 * transparent queue by `renderOrder` before distance, the axis line sets
 * `renderOrder = 1` (to clear the grid) while the fill leaves the default 0,
 * so the axis paints after (on top of) the fill regardless of
 * `polygonOffset`, which only arbitrates a *depth test* neither side's
 * fragment here can lose (the fill never writes depth to fail one against).
 * The solid axis half is unaffected (it is opaque, drawn in the depth-tested
 * opaque pass entirely before the fill). Narrow in practice — it takes a
 * region fill whose interior a *negative* axis half crosses — and
 * unaffected by AXES moving from -3 to +1 (`renderOrder` predates this
 * change); noted here rather than fixed, since closing it would mean the
 * fill writing depth or an explicit per-pair render-order rule, either a
 * real design change to a currently-deliberate choice
 * (`SceneRenderer.ts`'s region-fill material comment), not a ladder tweak.
 *

 * Regression net: `app/e2e/edge-stability.spec.ts` measures repaint stability
 * for both the face-vs-edge case (bias +2 vs 0) and the axis-vs-edge /
 * axis-vs-sketch cases (+1 vs 0 / -1) via `__hew_test.frameStability`.
 */

/** polygonOffset (factor and units alike) per layer — see the ladder above. */
export const DEPTH_BIAS = {
  /** Origin axes (`Line2` fat lines) — reference geometry: behind every
   * class of model linework (previews, region fills, sketch lines, native
   * edges) so a line drawn on an axis always wins, but still in front of
   * face fills so the axis reads over a coplanar face. */
  AXES: 1,
  /** Active-gesture rubber-bands (`PREVIEW_LINE_STYLE` fat lines). */
  PREVIEW: -2,
  /** Sketch region fills (translucent, no depth write; biased so their
   * depth *test* against coincident native edges is deterministic). */
  REGION_FILL: -2,
  /** Committed sketch lines (fat) — in front of coincident object edges. */
  SKETCH_LINE: -1,
  /** Dimension/leader-text annotations (fat-line geometry + label quads) —
   * deliberately ZERO, not omitted: see this file's "Dimension/leader-text
   * annotations" note above for why relying on FACE's own recession, rather
   * than adding any bias here (even a small one), is what makes annotations
   * win a coincident-face tie WITHOUT the annotation-side leak that sized
   * every prior attempt (uncapped polygonOffset, a capped world-space nudge,
   * a modest polygonOffset) too aggressively for one requirement or the
   * other. */
  ANNOTATION: 0,
  /** Face fills — behind their own edge overlay (the shipped shimmer fix)
   * and behind the origin axes. */
  FACE: 2,
} as const
