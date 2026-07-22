/**
 * InfiniteGrid — a shader-based ground grid that reads as "effectively
 * infinite" and adapts its cell size to the camera's distance.
 *
 * Technique: a single large flat plane at world Z=0, with a fragment shader
 * that procedurally draws anti-aliased grid lines (`fwidth`-based — the
 * standard "pristine grid" trick) at whichever decade cell size (…, 0.1, 1,
 * 10, …) suits the fragment's distance from the camera, smoothly blending
 * between adjacent decades. This needs no JS-side geometry rebuilding as the
 * camera moves — only a per-frame camera-position uniform update (see
 * `update()`, called from `Viewport.tsx`'s render loop) — and reads as
 * "infinite" simply because the plane (600x600) is comfortably larger than
 * the camera's far-clip distance (100), so its edges are never visible.
 *
 * The plane paints an opaque `uGroundColor` base everywhere (not just at
 * grid lines) — testing: "ground and sky are the same color, the only
 * difference is the grid" — with grid lines blended on top, so the ground
 * reads as a distinct surface from the sky (the renderer's clear color,
 * always set lighter than `uGroundColor` — see `Viewport.tsx`) even between
 * lines.
 *
 * Colors are set once at construction and again on every theme change via
 * `setColors()` — cheap uniform writes, no geometry/shader rebuild needed
 * (unlike the origin-axis lines, which do rebuild on theme change, in
 * `Viewport.tsx`).
 */
import * as THREE from 'three'

const VERTEX_SHADER = `
  varying vec3 vWorldPos;
  void main() {
    vec4 worldPosition = modelMatrix * vec4(position, 1.0);
    vWorldPos = worldPosition.xyz;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
`

const FRAGMENT_SHADER = `
  precision highp float;
  varying vec3 vWorldPos;
  uniform vec3 uCameraPos;
  uniform vec3 uGroundColor;
  uniform vec3 uMinorColor;
  uniform vec3 uMajorColor;
  uniform float uAxesVisible;

  // Anti-aliased grid-line intensity at world-space cell size \`cellSize\`
  // (the standard fwidth-based "pristine grid" technique — the derivative
  // scales the line width to stay ~1px regardless of distance/angle).
  // fwidth is clamped away from 0 as a defensive guard against a driver
  // returning a degenerate derivative (which would otherwise divide-by-zero
  // into Inf/NaN and silently vanish the whole grid).
  float gridFactor(vec2 coord, float cellSize) {
    vec2 c = coord / cellSize;
    // Derive the line's screen width from fwidth of the CONTINUOUS world coord,
    // then scale by cellSize — NOT fwidth(coord/cellSize). cellSize is
    // piecewise-constant across the LOD decades (it steps at each power-of-ten
    // distance from the camera), so fwidth(coord/cellSize) SPIKES on the
    // one-pixel ring where two adjacent fragments straddle a tier boundary —
    // that spike drew a phantom dashed circle on the ground (the "dashed
    // circle" bug). fwidth(coord) is smooth everywhere, so dividing it by the
    // constant cellSize gives the same line width without the boundary spike.
    vec2 fw = max(fwidth(coord) / cellSize, vec2(1e-4));
    vec2 g = abs(fract(c - 0.5) - 0.5) / fw;
    return 1.0 - clamp(min(g.x, g.y), 0.0, 1.0);
  }

  // Anti-aliased intensity of the two THROUGH-ORIGIN grid lines (world x=0
  // and world y=0) — same fwidth-based technique as gridFactor above, but
  // against a fixed coordinate (0) instead of the nearest multiple of a
  // decade cell size, and combined with max (not min): either line alone
  // should suppress, not just their intersection at the origin point.
  // AXIS_SUPPRESS_HALF_WIDTH widens the antialiased band a few pixels past
  // gridFactor's native ~1px line so it fully covers the fat origin-axis
  // line's own width (2.6px solid / 1.8px dashed, Viewport.tsx) plus its own
  // antialiased fringe, with a little margin.
  float originLineFactor(vec2 coord) {
    const float AXIS_SUPPRESS_HALF_WIDTH = 2.5;
    vec2 fw = max(fwidth(coord), vec2(1e-4));
    vec2 d = clamp(abs(coord) / (fw * AXIS_SUPPRESS_HALF_WIDTH), 0.0, 1.0);
    return max(1.0 - d.x, 1.0 - d.y);
  }

  void main() {
    float dist = length(uCameraPos - vWorldPos);

    // Map camera distance to a cell size on a log10 scale, clamped to Hew's
    // supported range [1mm, 10m] — the low end reaches millimetre cells so a
    // camera framed for a small-scale model (cm/mm/inch display units start
    // the empty scene zoomed in) still sees a usable grid instead of one
    // giant 10cm cell. The 0.12 constant is a tuned-by-eye mapping from
    // camera distance to cell size, not derived from a spec value.
    float raw = clamp(dist * 0.12, 0.001, 10.0);
    float logRaw = log(raw) / log(10.0);
    float lo = pow(10.0, floor(logRaw));
    float hi = lo * 10.0;
    float t = smoothstep(0.0, 1.0, (raw - lo) / (hi - lo));

    float gMinor = mix(gridFactor(vWorldPos.xy, lo), gridFactor(vWorldPos.xy, hi), t);
    // Major lines: one decade up from whichever tier is currently dominant.
    float gMajor = mix(gridFactor(vWorldPos.xy, hi), gridFactor(vWorldPos.xy, hi * 10.0), t);
    float gLine = clamp(max(gMinor, gMajor), 0.0, 1.0);

    // NO distance fade. Any fade keyed to radial camera distance draws a
    // circle on the ground (the fade boundary is a ring where it crosses the
    // axis-aligned grid lines) — that was the "dashed circle" bug in both its
    // original alpha-fade form and the first line-fade fix. It's unnecessary
    // here: the grid is zoom-ADAPTIVE (cell size ∝ camera distance, above), so
    // on-screen line density stays roughly constant at every zoom, which is
    // what keeps distant lines from moiré-ing — not a fade. The plane is opaque
    // and its only boundary is the frustum far-plane: a natural straight
    // horizon, no ring at any camera angle.
    vec3 color = mix(uGroundColor, mix(uMinorColor, uMajorColor, gMajor), gLine);

    // Suppress the grid's own through-origin lines when the origin axes are
    // visible — they're geometrically coincident with the red/green axes
    // (Viewport.tsx draws X/Y through world x=0/y=0 on this same ground
    // plane) and, drawn underneath, visually crowd the axis line instead of
    // reading as one clean line. Restored (mix factor 0) the moment the axes
    // are hidden — "one or the other, never both stacked". Every OTHER grid
    // line (not through the origin) is untouched either way.
    if (uAxesVisible > 0.5) {
      color = mix(color, uGroundColor, originLineFactor(vWorldPos.xy));
    }

    gl_FragColor = vec4(color, 1.0);
  }
`

/** Plane half-extent well beyond the camera's far-clip (100) — see module doc. */
const PLANE_SIZE = 600

export class InfiniteGrid {
  readonly mesh: THREE.Mesh
  private readonly material: THREE.ShaderMaterial

  constructor(groundColor: number, minorColor: number, majorColor: number, axesVisible = true) {
    const geometry = new THREE.PlaneGeometry(PLANE_SIZE, PLANE_SIZE)
    this.material = new THREE.ShaderMaterial({
      vertexShader: VERTEX_SHADER,
      fragmentShader: FRAGMENT_SHADER,
      uniforms: {
        uCameraPos: { value: new THREE.Vector3() },
        uGroundColor: { value: new THREE.Color(groundColor) },
        uMinorColor: { value: new THREE.Color(minorColor) },
        uMajorColor: { value: new THREE.Color(majorColor) },
        // Matches the origin axes' own default visibility (Viewport.tsx's
        // `originAxes` group is visible=true at construction) — a caller
        // that constructs the axes hidden should pass `axesVisible: false`
        // here too, or the grid will suppress lines under axes nobody sees
        // until the next real `setAxesVisible` call corrects it.
        uAxesVisible: { value: axesVisible ? 1 : 0 },
      },
      // A BACKDROP, not an occluder: opaque-pass (transparent: false) so it
      // draws before the model, at renderOrder -1 so it's first within that
      // pass, and with no depth write so everything drawn after paints over
      // it regardless of depth. The ground is virtual — it must never hide
      // the model: with `transparent: true` it rendered in the transparent
      // pass AFTER the opaque model and depth-tested against it, so a Bottom
      // view showed only ground (grid nearer → test passed → opaque fill
      // over the model) and a face lying exactly at z=0 flickered as the
      // depth tie broke differently pixel to pixel. As a backdrop the model
      // always wins the sightline; geometry below z=0 likewise stays visible
      // through the ground instead of being buried.
      transparent: false,
      depthWrite: false,
      // Defensive: don't rely on the plane's winding order matching what a
      // default front-face culling test expects.
      side: THREE.DoubleSide,
    })
    this.mesh = new THREE.Mesh(geometry, this.material)
    this.mesh.name = 'InfiniteGrid'
    // First in the opaque pass (see the material comment above): the model
    // overdraws it, and the ground-plane overlays (sketch region fills,
    // sketch edges, axes, tool previews — all geometrically AT z=0, ordered
    // by the depth-bias ladder in depthPolicy.ts) draw over it later.
    this.mesh.renderOrder = -1
    // Defensive: a 600x600 plane's bounding sphere should always intersect
    // the frustum near the origin, but skip the computed-bounds culling test
    // entirely rather than risk it being wrong.
    this.mesh.frustumCulled = false
  }

  /** Call every frame the scene re-renders (camera moved) — see `Viewport.tsx`'s render loop. */
  update(cameraPos: THREE.Vector3): void {
    (this.material.uniforms.uCameraPos.value as THREE.Vector3).copy(cameraPos)
  }

  /** Call on every theme change — cheap uniform write, no rebuild. */
  setColors(groundColor: number, minorColor: number, majorColor: number): void {
    (this.material.uniforms.uGroundColor.value as THREE.Color).set(groundColor)
    ;(this.material.uniforms.uMinorColor.value as THREE.Color).set(minorColor)
    ;(this.material.uniforms.uMajorColor.value as THREE.Color).set(majorColor)
  }

  /** Call whenever the origin axes' own visibility changes (View ▸ Axes) —
   * cheap uniform write, no rebuild. When `visible` the grid stops drawing
   * its through-origin lines (they'd sit directly under the axis lines and
   * visually crowd them); when not, the grid draws its full pattern again.
   * One or the other, never both stacked. */
  setAxesVisible(visible: boolean): void {
    this.material.uniforms.uAxesVisible.value = visible ? 1 : 0
  }

  dispose(): void {
    this.mesh.geometry.dispose()
    this.material.dispose()
  }
}
