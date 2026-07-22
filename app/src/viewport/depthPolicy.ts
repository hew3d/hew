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
 * |   0  | object edge overlays, guide lines       | native `GL_LINES` (fixed) |
 * |  +1  | origin axes                             | fat lines (`Line2`)      |
 * |  +2  | face fills                              | triangles                |
 *
 * Native `GL_LINES` primitives are the fixed reference at 0: `glPolygonOffset`
 * only applies to polygon primitives, so plain `LineSegments` can never be
 * biased — every other layer is placed around them. Fat lines (three's
 * `Line2`/`LineSegments2`) render as camera-facing triangle strips whose
 * fragment depth is the underlying segment's depth, so the offset applies to
 * them like any polygon.
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
  /** Face fills — behind their own edge overlay (the shipped shimmer fix)
   * and behind the origin axes. */
  FACE: 2,
} as const
