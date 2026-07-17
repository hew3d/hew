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
 * |  -3  | origin axes                             | fat lines (`Line2`)      |
 * |  -2  | tool rubber-bands, sketch region fills  | fat lines / triangles    |
 * |  -1  | committed sketch lines                  | fat lines (`LineSegments2`) |
 * |   0  | object edge overlays, guide lines       | native `GL_LINES` (fixed) |
 * |  +1  | face fills                              | triangles                |
 *
 * Native `GL_LINES` primitives are the fixed reference at 0: `glPolygonOffset`
 * only applies to polygon primitives, so plain `LineSegments` can never be
 * biased — every other layer is placed around them. Fat lines (three's
 * `Line2`/`LineSegments2`) render as camera-facing triangle strips whose
 * fragment depth is the underlying segment's depth, so the offset applies to
 * them like any polygon.
 *
 * The ordering itself reproduces what the old world-space lifts showed in a
 * normal (from-above) view: axes read over rubber-bands and committed sketch
 * work, rubber-bands and fills over committed lines, sketch lines and edges
 * over face fills. One-integer gaps are deliberate — each layer clears its
 * neighbour by a full resolvable depth unit, which is exactly what a
 * deterministic tie-break needs; wider gaps would only grow the (harmless but
 * nonzero) epsilon by which a biased layer can poke through genuinely nearer
 * geometry.
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
 * Regression net: `app/e2e/edge-stability.spec.ts` measures repaint stability
 * for both the face-vs-edge case (bias +1 vs 0) and the axis-vs-edge /
 * axis-vs-sketch cases (-3 vs 0 / -1) via `__hew_test.frameStability`.
 */

/** polygonOffset (factor and units alike) per layer — see the ladder above. */
export const DEPTH_BIAS = {
  /** Origin axes (`Line2` fat lines) — front of all coincident linework. */
  AXES: -3,
  /** Active-gesture rubber-bands (`PREVIEW_LINE_STYLE` fat lines). */
  PREVIEW: -2,
  /** Sketch region fills (translucent, no depth write; biased so their
   * depth *test* against coincident native edges is deterministic). */
  REGION_FILL: -2,
  /** Committed sketch lines (fat) — in front of coincident object edges. */
  SKETCH_LINE: -1,
  /** Face fills — behind their own edge overlay (the shipped shimmer fix). */
  FACE: 1,
} as const
