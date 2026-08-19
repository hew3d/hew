/**
 * Print pass — renders the live viewport scene into offscreen page bitmaps
 * (docs/design/printing.md §7). Runs inside the Viewport mount effect
 * (`ViewportApi.renderPrintPages`) because the one `WebGLRenderer` and every
 * uploaded buffer live there; this module keeps the logic out of
 * `Viewport.tsx`.
 *
 * Per job it (1) reshapes the scene through `SceneRenderer.beginPrintPass`
 * (fat edge overlay in place of 1-px native edges, no highlights, optional
 * flat-white "line art"), hides chrome layers (grid, axes, cues, previews,
 * section widget), swaps in the light-theme light rig on a white background,
 * scales every fat line by dpi/96 so linework keeps its physical size, and
 * (2) renders each requested page with its own camera into a multisampled
 * render target, reads the pixels back, flips them, and encodes a Blob —
 * yielding a frame between pages so progress stays live. Everything is
 * restored afterwards; the on-screen scene is unchanged.
 *
 * Colour space: three's shaders write LINEAR values into an ordinary render
 * target (the sRGB output transform only applies to the canvas). Giving the
 * target's texture `SRGBColorSpace` makes three allocate an SRGB8_ALPHA8
 * texture (and a matching multisample renderbuffer), and WebGL2 encodes
 * linear→sRGB on write to such an attachment — so `readRenderTargetPixels`
 * hands back the same sRGB bytes the canvas would show. (Do NOT also flag the
 * target `isXRRenderTarget`: that forces a linear renderbuffer against the
 * sRGB texture and the multisample resolve blit fails with a format
 * mismatch — a blank page.)
 */
import * as THREE from 'three'
import type { CameraRig } from './cameraRig'
import type { SceneRenderer } from './SceneRenderer'
import { getFatLineResolution, makeFatSegments, disposeFatSegments, setFatLineWidthScale, updateFatLineResolutions } from './fatLine'
import { GUIDE_COLOR } from './guideColors'

/** A rectangle in the view plane, meters, y up, relative to the print
 * camera's centre (see `computeViewPlaneExtent`). */
export interface ViewRect {
  x: number
  y: number
  w: number
  h: number
}

export type PrintCameraSpec =
  /** The live viewport camera, aspect matched to the page image (Standard
   * mode). `fit` keeps the view direction but re-frames the visible model
   * to fill the page (Standard's Zoom: Fit). */
  | { kind: 'live'; fit?: boolean }
  /** A saved pose (a Scene's camera) — Standard mode printing each Scene. */
  | {
      kind: 'pose'
      projection: 'perspective' | 'parallel'
      eye: [number, number, number]
      target: [number, number, number]
      up: [number, number, number]
      fovDeg: number
      /** Re-frame the visible model to fill the page (Standard's Zoom: Fit). */
      fit?: boolean
    }
  /** An orthographic camera looking along `dir` through `center`, showing
   * exactly `rect` (Scaled mode; one per tile). */
  | {
      kind: 'ortho'
      center: [number, number, number]
      dir: [number, number, number]
      up: [number, number, number]
      rect: ViewRect
      /** Depth range of the geometry along `dir` relative to `center`. */
      depth: { min: number; max: number }
    }

export interface PrintPageRequest {
  camera: PrintCameraSpec
  widthPx: number
  heightPx: number
}

export interface PrintRenderOptions {
  /** A Scene's resolved hidden sets, replacing the live ones for the pass. */
  hiddenOverride?: { objects: bigint[]; instances: bigint[] } | null
  /** A Scene's section plane for the pass (`null` = no cut); undefined = live. */
  sectionOverride?: { origin: [number, number, number]; normal: [number, number, number]; active: boolean } | null
  style: 'shaded' | 'lineart'
  includeGuides: boolean
  /** Standard mode only: keep the ground grid + origin axes (as on screen). */
  includeGridAxes: boolean
  includeAnnotations: boolean
  restrictTo: { objects: Set<bigint>; instances: Set<bigint> } | null
  dpi: number
  format: 'image/png' | 'image/jpeg'
  quality?: number
}

/** Everything the pass needs from the mount effect. */
export interface PrintPassHandles {
  renderer: THREE.WebGLRenderer
  scene: THREE.Scene
  rig: CameraRig
  sceneRenderer: SceneRenderer
  ambient: THREE.AmbientLight
  dirLight: THREE.DirectionalLight
  /** Light-theme rig intensities (prints are always on white paper). */
  lightRig: { ambient: number; directional: number }
  gridAxes: THREE.Object3D[]
  hideAlways: THREE.Object3D[]
  /** Live viewport CSS size, to restore fat-line resolution / billboards. */
  viewportCssSize: () => { width: number; height: number }
  /** The live applied theme, to re-lay-out annotations after the pass. */
  liveTheme: () => 'light' | 'dark'
  /** The live section plane (to restore after a Scene override). */
  liveSection: () => { origin: [number, number, number]; normal: [number, number, number]; active: boolean } | null
}

/** Physical edge weights (mm) per style — §7 / D7. */
export const EDGE_WEIGHT_MM = { shaded: 0.2, lineart: 0.35 } as const
const EDGE_COLOR = { shaded: 0x1a1a1a, lineart: 0x000000 } as const
const GUIDE_WEIGHT_MM = 0.18
const MAX_TEXTURE_FALLBACK = 4096

export function mmToDevicePx(mm: number, dpi: number): number {
  // Never below one device pixel — a 0.35 mm line at preview dpi would
  // otherwise vanish.
  return Math.max(1, (mm / 25.4) * dpi)
}

/** Basis vectors of the view plane for a view direction + up: `right` and
 * `upV` as three's `lookAt` would build them (x = up × −dir, y = z × x). */
export function viewPlaneBasis(dir: THREE.Vector3, up: THREE.Vector3): { right: THREE.Vector3; upV: THREE.Vector3; back: THREE.Vector3 } {
  const back = dir.clone().negate().normalize()
  let right = new THREE.Vector3().crossVectors(up, back)
  if (right.lengthSq() < 1e-12) {
    // Looking straight along `up` (a plan or an underside): world +X is
    // "right" (else +Y when the view runs along X), orthogonalized against
    // the direction — the same rule `hlr::Frame` uses, so vector and raster
    // pages agree.
    const d = dir.clone().normalize()
    const seed = Math.abs(d.x) < 0.9 ? new THREE.Vector3(1, 0, 0) : new THREE.Vector3(0, 1, 0)
    right = seed.sub(d.clone().multiplyScalar(seed.dot(d)))
  }
  right.normalize()
  const upV = new THREE.Vector3().crossVectors(back, right).normalize()
  return { right, upV, back }
}

export interface ViewPlaneExtent {
  /** World point at the view-plane origin (the print camera's centre). */
  center: [number, number, number]
  /** Extent relative to `center`, meters, y up. */
  rect: ViewRect
  depth: { min: number; max: number }
  /** True when nothing visible contributed (an empty document). */
  empty: boolean
}

/**
 * Project every visible vertex onto the view plane and return the
 * enclosing rectangle, centred (docs/design/printing.md §7 "Extent math").
 */
export function computeViewPlaneExtent(
  sceneRenderer: SceneRenderer,
  dirIn: [number, number, number],
  upIn: [number, number, number],
  opts: { includeSketches: boolean; includeAnnotations: boolean },
): ViewPlaneExtent {
  const dir = new THREE.Vector3(...dirIn).normalize()
  const up = new THREE.Vector3(...upIn).normalize()
  const { right, upV } = viewPlaneBasis(dir, up)
  let minX = Infinity
  let maxX = -Infinity
  let minY = Infinity
  let maxY = -Infinity
  let minD = Infinity
  let maxD = -Infinity
  // Every visible vertex (strided for huge meshes), not bounding boxes: an
  // oblique view of a box's AABB is much bigger than the model, and a fit
  // must reach the margins.
  sceneRenderer.forEachVisibleWorldPoint((p) => {
    const x = p.dot(right)
    const y = p.dot(upV)
    const d = p.dot(dir)
    if (x < minX) minX = x
    if (x > maxX) maxX = x
    if (y < minY) minY = y
    if (y > maxY) maxY = y
    if (d < minD) minD = d
    if (d > maxD) maxD = d
  }, opts)
  if (!isFinite(minX)) {
    return { center: [0, 0, 0], rect: { x: -0.5, y: -0.5, w: 1, h: 1 }, depth: { min: -0.5, max: 0.5 }, empty: true }
  }
  const cx = (minX + maxX) / 2
  const cy = (minY + maxY) / 2
  const cd = (minD + maxD) / 2
  const center = new THREE.Vector3().addScaledVector(right, cx).addScaledVector(upV, cy).addScaledVector(dir, cd)
  const w = Math.max(maxX - minX, 1e-6)
  const h = Math.max(maxY - minY, 1e-6)
  return {
    center: [center.x, center.y, center.z],
    rect: { x: -w / 2, y: -h / 2, w, h },
    depth: { min: minD - cd, max: maxD - cd },
    empty: false,
  }
}

/** The rectangle the live parallel frustum shows, in the same frame as
 * `computeViewPlaneExtent` (Scaled mode's "Current view" extent). */
export function liveOrthoExtent(rig: CameraRig, target: THREE.Vector3, dir: [number, number, number], up: [number, number, number], depthHint: { min: number; max: number }): ViewPlaneExtent {
  const o = rig.orthographic
  const w = (o.right - o.left) / o.zoom
  const h = (o.top - o.bottom) / o.zoom
  void dir
  void up
  return { center: [target.x, target.y, target.z], rect: { x: -w / 2, y: -h / 2, w, h }, depth: depthHint, empty: false }
}

/** Orient `cam` at `eye` looking along `dir` with the print basis —
 * `viewPlaneBasis`, NOT `lookAt`: three's lookAt nudges a degenerate
 * (plan / underside) view by 0.0001 and lands on right = +Y, up = −X, a
 * quarter turn away from the basis the extent, the vector line drawing, and
 * the headless renderer all use. */
function orientCamera(cam: THREE.Camera, eye: THREE.Vector3, dir: THREE.Vector3, up: THREE.Vector3): void {
  const { right, upV, back } = viewPlaneBasis(dir, up)
  cam.position.copy(eye)
  cam.quaternion.setFromRotationMatrix(new THREE.Matrix4().makeBasis(right, upV, back))
  cam.updateMatrixWorld(true)
}

/** Zoom: Fit — keep the camera's orientation, re-frame every visible thing
 * (objects, instances, sketches, annotations) so it fills the page image
 * with a small margin: the tightest frustum around the projected bounds
 * corners, not a bounding sphere. */
function fitLiveCamera(h: PrintPassHandles, cam: THREE.PerspectiveCamera | THREE.OrthographicCamera, aspect: number): void {
  // Every visible vertex (within a budget; box corners past it), flat-packed
  // — so the fit reaches the margins for any shape from any angle; a box's
  // corners alone would leave a gap around anything that isn't a box.
  const flat: number[] = []
  const box = new THREE.Box3()
  h.sceneRenderer.forEachVisibleWorldPoint(
    (p) => {
      flat.push(p.x, p.y, p.z)
      box.expandByPoint(p)
    },
    { includeSketches: true, includeAnnotations: true, maxPoints: 400_000 },
  )
  const count = flat.length / 3
  if (count === 0) return
  cam.updateMatrixWorld(true)
  const right = new THREE.Vector3().setFromMatrixColumn(cam.matrixWorld, 0).normalize()
  const up = new THREE.Vector3().setFromMatrixColumn(cam.matrixWorld, 1).normalize()
  const back = new THREE.Vector3().setFromMatrixColumn(cam.matrixWorld, 2).normalize()
  const center = box.getCenter(new THREE.Vector3())
  const radius = Math.max(box.getBoundingSphere(new THREE.Sphere()).radius, 1e-6)
  // Right up to the margins, with a hair of breathing room for line widths.
  const MARGIN = 1.01
  // Point i relative to `origin`, dotted with a unit vector.
  const rel = (i: number, origin: THREE.Vector3, axis: THREE.Vector3): number =>
    (flat[i * 3] - origin.x) * axis.x + (flat[i * 3 + 1] - origin.y) * axis.y + (flat[i * 3 + 2] - origin.z) * axis.z
  if (cam instanceof THREE.OrthographicCamera) {
    let uMin = Infinity
    let uMax = -Infinity
    let vMin = Infinity
    let vMax = -Infinity
    for (let i = 0; i < count; i++) {
      const u = rel(i, center, right)
      const w = rel(i, center, up)
      if (u < uMin) uMin = u
      if (u > uMax) uMax = u
      if (w < vMin) vMin = w
      if (w > vMax) vMax = w
    }
    // Aim at the projected extents' centre (not the box centre).
    const aim = center.clone().addScaledVector(right, (uMin + uMax) / 2).addScaledVector(up, (vMin + vMax) / 2)
    const hw = (uMax - uMin) / 2
    const hh = (vMax - vMin) / 2
    const halfH = Math.max(hh, hw / aspect, 1e-6) * MARGIN
    const halfW = halfH * aspect
    cam.left = -halfW
    cam.right = halfW
    cam.top = halfH
    cam.bottom = -halfH
    const dist = radius * 2
    cam.position.copy(aim).addScaledVector(back, dist)
    cam.near = dist * 1e-3
    cam.far = dist + radius * 4
    return
  }
  // Perspective: the eye distance from the aim point (along the view
  // direction) at which every point is inside both half-angles. A
  // perspective projection is asymmetric (near geometry looms), so the aim
  // point is re-centred on the projected extents and the distance
  // recomputed, a few rounds — a tighter fit than framing the box centre.
  const tanV = Math.tan((cam.fov * Math.PI) / 360)
  const tanH = tanV * aspect
  const aim = center.clone()
  let dist = 0
  for (let round = 0; round < 4; round++) {
    dist = 0
    for (let i = 0; i < count; i++) {
      const depth = -rel(i, aim, back)
      const need = Math.max(Math.abs(rel(i, aim, up)) / tanV - depth, Math.abs(rel(i, aim, right)) / tanH - depth)
      if (need > dist) dist = need
    }
    dist = Math.max(dist * MARGIN, radius * 1e-3)
    // Projected extents (in tangent units) at this eye; shift the aim so
    // they centre, scaled back to world by the mean viewing depth.
    let uMin = Infinity
    let uMax = -Infinity
    let vMin = Infinity
    let vMax = -Infinity
    let meanDepth = 0
    for (let i = 0; i < count; i++) {
      const depth = dist - rel(i, aim, back)
      meanDepth += depth / count
      const u = rel(i, aim, right) / depth
      const w = rel(i, aim, up) / depth
      if (u < uMin) uMin = u
      if (u > uMax) uMax = u
      if (w < vMin) vMin = w
      if (w > vMax) vMax = w
    }
    aim.addScaledVector(right, ((uMin + uMax) / 2) * meanDepth).addScaledVector(up, ((vMin + vMax) / 2) * meanDepth)
  }
  cam.position.copy(aim).addScaledVector(back, dist)
  // Scale-relative planes: an absolute floor could sit past a tiny model.
  cam.near = Math.max(dist * 1e-3, 1e-9)
  cam.far = dist + radius * 4
}

function buildCamera(h: PrintPassHandles, spec: PrintCameraSpec, W: number, H: number): THREE.PerspectiveCamera | THREE.OrthographicCamera {
  if (spec.kind === 'pose') {
    const eye = new THREE.Vector3(...spec.eye)
    const target = new THREE.Vector3(...spec.target)
    const up = new THREE.Vector3(...spec.up).normalize()
    const dist = Math.max(eye.distanceTo(target), 1e-6)
    // Near/far bracket the eye–target distance AND everything visible: a
    // close-up Scene pose of a big model must not clip its far wall.
    let farthest = dist
    const dir = target.clone().sub(eye).normalize()
    const p = new THREE.Vector3()
    h.sceneRenderer.forEachVisibleWorldBox((box) => {
      for (let i = 0; i < 8; i++) {
        p.set(i & 1 ? box.max.x : box.min.x, i & 2 ? box.max.y : box.min.y, i & 4 ? box.max.z : box.min.z)
        farthest = Math.max(farthest, p.sub(eye).dot(dir))
      }
    }, { includeSketches: true, includeAnnotations: true })
    const near = Math.max(dist * 1e-3, 1e-4)
    const far = Math.max(dist * 100, farthest * 1.5)
    if (spec.projection === 'perspective') {
      const cam = new THREE.PerspectiveCamera(spec.fovDeg, W / H, near, far)
      orientCamera(cam, eye, dir, up)
      if (spec.fit === true) fitLiveCamera(h, cam, W / H)
      cam.updateProjectionMatrix()
      cam.updateMatrixWorld(true)
      return cam
    }
    // Parallel from a pose: the rig's own rule — half-height = dist·tan(fov/2).
    const halfH = dist * Math.tan((spec.fovDeg * Math.PI) / 360)
    const halfW = halfH * (W / H)
    const cam = new THREE.OrthographicCamera(-halfW, halfW, halfH, -halfH, near, far)
    orientCamera(cam, eye, dir, up)
    if (spec.fit === true) fitLiveCamera(h, cam, W / H)
    cam.updateProjectionMatrix()
    cam.updateMatrixWorld(true)
    return cam
  }
  if (spec.kind === 'live') {
    const cam = h.rig.active.clone() as THREE.PerspectiveCamera | THREE.OrthographicCamera
    if (cam instanceof THREE.PerspectiveCamera) {
      cam.aspect = W / H
    } else {
      // Keep the vertical framing, match the page's aspect (the rig's own
      // `setAspect` rule) — normally identical to the viewport's.
      const halfH = (cam.top - cam.bottom) / 2
      const halfW = halfH * (W / H)
      cam.left = -halfW
      cam.right = halfW
    }
    if (spec.fit === true) fitLiveCamera(h, cam, W / H)
    cam.updateProjectionMatrix()
    cam.updateMatrixWorld(true)
    return cam
  }
  const center = new THREE.Vector3(...spec.center)
  const dir = new THREE.Vector3(...spec.dir).normalize()
  const up = new THREE.Vector3(...spec.up).normalize()
  const range = Math.max(spec.depth.max - spec.depth.min, 1e-3)
  const pad = Math.max(range * 0.1, 0.05)
  const eye = center.clone().addScaledVector(dir, spec.depth.min - pad)
  const cam = new THREE.OrthographicCamera(spec.rect.x, spec.rect.x + spec.rect.w, spec.rect.y + spec.rect.h, spec.rect.y, pad * 0.5, range + pad * 2)
  orientCamera(cam, eye, dir, up)
  cam.updateProjectionMatrix()
  return cam
}

/** Yield to the event loop between pages. Prefers a frame (keeps the progress
 * line painting) but never waits on one: an occluded or backgrounded window
 * (WKWebView with the display asleep, a minimized tab) stops delivering
 * animation frames entirely, and a print must still finish. */
function nextFrame(): Promise<void> {
  return new Promise((resolve) => {
    let done = false
    const finish = (): void => {
      if (!done) {
        done = true
        resolve()
      }
    }
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(finish)
    setTimeout(finish, 40)
  })
}

function encode(pixels: Uint8Array, W: number, H: number, format: string, quality?: number): Promise<Blob> {
  const canvas = document.createElement('canvas')
  canvas.width = W
  canvas.height = H
  const ctx = canvas.getContext('2d')
  if (ctx === null) return Promise.reject(new Error('2D canvas unavailable'))
  // GL readback is bottom-row-first; flip while copying (one row at a time —
  // the same idiom as frameThumbnail.ts, without a second canvas).
  const flipped = new Uint8ClampedArray(W * H * 4)
  const rowBytes = W * 4
  for (let y = 0; y < H; y++) {
    const src = (H - 1 - y) * rowBytes
    flipped.set(pixels.subarray(src, src + rowBytes), y * rowBytes)
  }
  ctx.putImageData(new ImageData(flipped, W, H), 0, 0)
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (b) => {
        canvas.width = 0
        canvas.height = 0
        if (b === null) reject(new Error('encode failed'))
        else resolve(b)
      },
      format,
      quality,
    )
  })
}

/**
 * Line-art silhouettes (raster pages). Hard edges are real geometry (fat
 * lines), but the outline of a curved wall — a table leg's sides, a rim —
 * is a view-dependent silhouette no edge list holds. This renders a
 * faces-only normal + depth pass and inks every pixel where depth or normal
 * jumps against a neighbour `radius` px away, over the colour page: an
 * outline about `2·radius` px wide, in the same ink as the hard edges.
 */
const FACES_LAYER = 30

class SilhouetteCompositor {
  private normalRt: THREE.WebGLRenderTarget | null = null
  private outRt: THREE.WebGLRenderTarget | null = null
  private readonly normalMat = new THREE.MeshNormalMaterial({ side: THREE.DoubleSide })
  private readonly quadScene = new THREE.Scene()
  private readonly quadCam = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1)
  private readonly mat: THREE.ShaderMaterial
  private readonly layerCam = new THREE.Layers()

  constructor() {
    this.mat = new THREE.ShaderMaterial({
      uniforms: {
        tColor: { value: null },
        tNormal: { value: null },
        tDepth: { value: null },
        texel: { value: new THREE.Vector2(1, 1) },
        radius: { value: 1 },
        near: { value: 0.1 },
        far: { value: 100 },
        perspective: { value: 1 },
        // Pure black, like the hard-edge lines (invariant under the target's
        // sRGB encode, so it lands as (0,0,0) whatever the pipeline does).
        ink: { value: new THREE.Vector3(0, 0, 0) },
      },
      vertexShader: 'varying vec2 vUv; void main(){ vUv = uv; gl_Position = vec4(position.xy, 0.0, 1.0); }',
      fragmentShader: `
        varying vec2 vUv;
        uniform sampler2D tColor, tNormal, tDepth;
        uniform vec2 texel; uniform float radius, near, far; uniform int perspective; uniform vec3 ink;
        float lin(vec2 uv) {
          float d = texture2D(tDepth, uv).x;
          if (perspective == 1) { float z = d * 2.0 - 1.0; return (2.0 * near * far) / (far + near - z * (far - near)); }
          return near + d * (far - near);
        }
        void main() {
          vec4 c = texture2D(tColor, vUv);
          vec4 n0s = texture2D(tNormal, vUv);
          float d0 = lin(vUv);
          bool geo0 = n0s.a > 0.5;
          vec3 n0 = normalize(n0s.xyz * 2.0 - 1.0);
          float edge = 0.0;
          vec2 offs[4];
          offs[0] = vec2(texel.x * radius, 0.0); offs[1] = vec2(-texel.x * radius, 0.0);
          offs[2] = vec2(0.0, texel.y * radius); offs[3] = vec2(0.0, -texel.y * radius);
          for (int i = 0; i < 4; i++) {
            vec2 uv = vUv + offs[i];
            vec4 ns = texture2D(tNormal, uv);
            bool geo = ns.a > 0.5;
            if (geo != geo0) { edge = 1.0; }
            if (geo && geo0) {
              float d = lin(uv);
              float dz = abs(d - d0);
              float rel = perspective == 1 ? dz / max(min(d, d0), 1e-6) : dz / max(far - near, 1e-6);
              if (rel > (perspective == 1 ? 0.01 : 0.004)) { edge = 1.0; }
              vec3 n = normalize(ns.xyz * 2.0 - 1.0);
              if (dot(n, n0) < 0.5) { edge = 1.0; }
            }
          }
          gl_FragColor = edge > 0.5 ? vec4(ink, 1.0) : c;
        }`,
      depthTest: false,
      depthWrite: false,
    })
    const quad = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), this.mat)
    quad.frustumCulled = false
    this.quadScene.add(quad)
    this.layerCam.set(FACES_LAYER)
  }

  /** Composite silhouettes over `colorRt` and return the target holding the result. */
  render(renderer: THREE.WebGLRenderer, scene: THREE.Scene, sceneRenderer: SceneRenderer, cam: THREE.PerspectiveCamera | THREE.OrthographicCamera, colorRt: THREE.WebGLRenderTarget, W: number, H: number, edgePx: number): THREE.WebGLRenderTarget {
    if (this.normalRt === null || this.normalRt.width !== W || this.normalRt.height !== H) {
      this.normalRt?.dispose()
      this.outRt?.dispose()
      const depthTexture = new THREE.DepthTexture(W, H, THREE.UnsignedIntType)
      this.normalRt = new THREE.WebGLRenderTarget(W, H, { depthBuffer: true, depthTexture, stencilBuffer: false, samples: 0 })
      this.outRt = new THREE.WebGLRenderTarget(W, H, { depthBuffer: false, stencilBuffer: false, samples: 0, colorSpace: THREE.SRGBColorSpace })
    }
    // Faces only, through the faces layer; the section cut applies too.
    this.normalMat.clippingPlanes = sceneRenderer.getSectionClipPlanes()
    const savedLayers = cam.layers.mask
    const savedOverride = scene.overrideMaterial
    const savedClear = renderer.getClearColor(new THREE.Color())
    const savedAlpha = renderer.getClearAlpha()
    sceneRenderer.setFacesLayer(FACES_LAYER, true)
    cam.layers.mask = this.layerCam.mask
    scene.overrideMaterial = this.normalMat
    renderer.setClearColor(0x000000, 0)
    renderer.setRenderTarget(this.normalRt)
    renderer.clear()
    renderer.render(scene, cam)
    scene.overrideMaterial = savedOverride
    cam.layers.mask = savedLayers
    sceneRenderer.setFacesLayer(FACES_LAYER, false)
    renderer.setClearColor(savedClear, savedAlpha)
    const u = this.mat.uniforms
    u.tColor.value = colorRt.texture
    u.tNormal.value = this.normalRt.texture
    u.tDepth.value = this.normalRt.depthTexture
    ;(u.texel.value as THREE.Vector2).set(1 / W, 1 / H)
    u.radius.value = Math.max(0.75, edgePx / 2)
    u.near.value = cam.near
    u.far.value = cam.far
    u.perspective.value = cam instanceof THREE.PerspectiveCamera ? 1 : 0
    renderer.setRenderTarget(this.outRt)
    renderer.render(this.quadScene, this.quadCam)
    return this.outRt!
  }

  dispose(): void {
    this.normalRt?.dispose()
    this.outRt?.dispose()
    this.normalRt = null
    this.outRt = null
    this.normalMat.dispose()
    this.mat.dispose()
    this.quadScene.traverse((o) => {
      const g = (o as THREE.Mesh).geometry as THREE.BufferGeometry | undefined
      g?.dispose?.()
    })
  }
}

/** Print passes run one at a time: a preview pass still in flight when the
 * user clicks Print… must finish before the print pass reshapes the scene. */
let passChain: Promise<unknown> = Promise.resolve()

/**
 * Render every requested page and return one encoded Blob per page (in
 * order). Throws if the WebGL context is lost or a page cannot be allocated.
 * Calls are serialized (see `passChain`); the returned promise settles when
 * this call's own pass has finished.
 */
export function renderPrintPages(
  h: PrintPassHandles,
  pages: PrintPageRequest[],
  opts: PrintRenderOptions,
  onProgress?: (done: number, total: number) => void,
): Promise<Blob[]> {
  const run = passChain.then(() => renderPrintPagesNow(h, pages, opts, onProgress))
  passChain = run.catch(() => undefined)
  return run
}

async function renderPrintPagesNow(
  h: PrintPassHandles,
  pages: PrintPageRequest[],
  opts: PrintRenderOptions,
  onProgress?: (done: number, total: number) => void,
): Promise<Blob[]> {
  const { renderer, scene, sceneRenderer } = h
  const k = opts.dpi / 96
  const gl = renderer.getContext()
  const maxSide = (gl.getParameter(gl.MAX_RENDERBUFFER_SIZE) as number) || MAX_TEXTURE_FALLBACK

  // ---- setup (everything undone in `restore`)
  const undo: (() => void)[] = []
  const savedClear = renderer.getClearColor(new THREE.Color())
  const savedClearAlpha = renderer.getClearAlpha()
  const savedBackground = scene.background
  const savedAmbient = h.ambient.intensity
  const savedDir = h.dirLight.intensity
  const savedRes = getFatLineResolution()
  const savedTarget = renderer.getRenderTarget()
  const savedViewport = renderer.getViewport(new THREE.Vector4())
  renderer.setClearColor(0xffffff, 1)
  scene.background = null
  h.ambient.intensity = h.lightRig.ambient
  h.dirLight.intensity = h.lightRig.directional
  undo.push(() => {
    renderer.setClearColor(savedClear, savedClearAlpha)
    scene.background = savedBackground
    h.ambient.intensity = savedAmbient
    h.dirLight.intensity = savedDir
    renderer.setRenderTarget(savedTarget)
    renderer.setViewport(savedViewport)
    updateFatLineResolutions(savedRes.width, savedRes.height)
    setFatLineWidthScale(1)
  })
  const hide = (o: THREE.Object3D): void => {
    const was = o.visible
    o.visible = false
    undo.push(() => {
      o.visible = was
    })
  }
  for (const o of h.hideAlways) hide(o)
  if (!opts.includeGridAxes) for (const o of h.gridAxes) hide(o)
  if (!opts.includeAnnotations) hide(sceneRenderer.annotationsGroup)

  const edgeWidthPx = mmToDevicePx(EDGE_WEIGHT_MM[opts.style], opts.dpi)
  sceneRenderer.beginPrintPass({ style: opts.style, restrictTo: opts.restrictTo, includeGuides: opts.includeGuides, sketchLineWidthPx: edgeWidthPx, hiddenOverride: opts.hiddenOverride ?? null })
  undo.push(() => sceneRenderer.endPrintPass())
  if (opts.sectionOverride !== undefined) {
    const saved = h.liveSection()
    sceneRenderer.setSectionPlane(opts.sectionOverride)
    undo.push(() => sceneRenderer.setSectionPlane(saved))
  }

  // Fat edge overlay in place of the hidden native edges.
  const overlay = new THREE.Group()
  overlay.name = 'PrintEdgeOverlay'
  const edgePositions = sceneRenderer.collectPrintEdgeSegments()
  if (edgePositions.length >= 6) {
    overlay.add(
      makeFatSegments(edgePositions, {
        color: EDGE_COLOR[opts.style],
        widthPx: edgeWidthPx,
        absoluteWidth: true,
      }),
    )
  }
  if (opts.includeGuides) {
    const g = sceneRenderer.collectGuideSegments()
    const guideColor = GUIDE_COLOR
    if (g.lines.length >= 6) {
      overlay.add(
        makeFatSegments(g.lines, {
          color: guideColor,
          widthPx: mmToDevicePx(GUIDE_WEIGHT_MM, opts.dpi),
          dashed: true,
          dashSize: mmToDevicePx(2, opts.dpi) / 1000,
          gapSize: mmToDevicePx(1.5, opts.dpi) / 1000,
          absoluteWidth: true,
        }),
      )
    }
    if (g.markers.length >= 6) {
      overlay.add(makeFatSegments(g.markers, { color: guideColor, widthPx: mmToDevicePx(GUIDE_WEIGHT_MM, opts.dpi), absoluteWidth: true }))
    }
    // The native guide lines are still visible; the overlay draws over them
    // in the same colour, so the hairlines only reinforce the fat lines.
  }
  scene.add(overlay)
  undo.push(() => {
    scene.remove(overlay)
    overlay.traverse((o) => disposeFatSegments(o))
  })
  setFatLineWidthScale(k)

  let rt: THREE.WebGLRenderTarget | null = null
  undo.push(() => {
    if (rt !== null) rt.dispose()
  })
  const silhouettes = opts.style === 'lineart' ? new SilhouetteCompositor() : null
  undo.push(() => silhouettes?.dispose())

  const restore = (): void => {
    for (let i = undo.length - 1; i >= 0; i--) undo[i]()
  }

  const blobs: Blob[] = []
  try {
    for (let i = 0; i < pages.length; i++) {
      const page = pages[i]
      const W = Math.max(1, Math.min(page.widthPx, maxSide))
      const H = Math.max(1, Math.min(page.heightPx, maxSide))
      if (gl.isContextLost()) throw new Error('WebGL context lost')
      if (rt === null || rt.width !== W || rt.height !== H) {
        if (rt !== null) rt.dispose()
        rt = new THREE.WebGLRenderTarget(W, H, {
          samples: 4,
          depthBuffer: true,
          stencilBuffer: false,
          colorSpace: THREE.SRGBColorSpace,
        })
      }
      const cam = buildCamera(h, page.camera, W, H)
      updateFatLineResolutions(W, H)
      // Labels: hold their CSS-px size relative to a virtual viewport of
      // H/k px, so on paper they are k× taller in device px = the same
      // physical size as on a 96-dpi screen; rasterize at k for crispness.
      sceneRenderer.updateAnnotationBillboards(cam, W / k, H / k, 'light', { dpr: Math.max(1, k) })
      // three's `LineSegments2.onBeforeRender` overwrites every fat line's
      // `resolution` uniform with `renderer.getViewport()` — the CANVAS
      // viewport, not the render target's size — so without this the fat
      // lines would render (canvasW/W)× too wide. Point the renderer's
      // viewport at the page size for the draw (before binding the target,
      // which takes its own viewport from the target), then restore.
      renderer.setViewport(0, 0, W, H)
      renderer.setRenderTarget(rt)
      renderer.render(scene, cam)
      // Line art: ink the silhouettes of curved walls (a view-dependent
      // outline no edge list holds) over the page, in the edge weight.
      const readFrom = silhouettes !== null ? silhouettes.render(renderer, scene, sceneRenderer, cam, rt, W, H, edgeWidthPx) : rt
      renderer.setRenderTarget(null)
      renderer.setViewport(savedViewport)
      const pixels = new Uint8Array(W * H * 4)
      renderer.readRenderTargetPixels(readFrom, 0, 0, W, H, pixels)
      blobs.push(await encode(pixels, W, H, opts.format, opts.quality))
      onProgress?.(i + 1, pages.length)
      await nextFrame()
    }
  } finally {
    restore()
    // Re-lay-out annotations for the live camera/viewport right away rather
    // than one frame later.
    const css = h.viewportCssSize()
    sceneRenderer.updateAnnotationBillboards(h.rig.active, css.width, css.height, h.liveTheme())
  }
  return blobs
}
