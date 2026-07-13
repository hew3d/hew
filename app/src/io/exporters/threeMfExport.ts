/**
 * 3MF export.
 *
 * Follows `stlExport.ts`'s architecture: runs entirely in TypeScript over the
 * live three.js scene via `SceneRenderer.buildExportScene()` (per-object
 * tessellated face meshes + instance nodes — never edges, sketches, guides,
 * or preview overlays). Where STL flattens everything into one anonymous
 * triangle soup, 3MF keeps structure: one `<object>` per world object and per
 * placed instance member, carrying its display name and per-triangle colors,
 * inside an OPC (zip) container with an explicit `unit="millimeter"` — the
 * roadmap's "better suited to multi-part prints" points.
 *
 * Deliberate choices, mirroring the STL path where they overlap:
 *  - **Z-up, millimeters.** 3MF is natively Z-up (Hew's world orientation),
 *    so the Y-up rotation `buildExportScene` bakes for glTF is reset; kernel
 *    meters are scaled ×1000 and the unit is declared on `<model>`.
 *  - **World-space baking.** Instance poses are baked into each emitted
 *    mesh (build items carry no transform). 3MF forbids mirroring transforms
 *    on components/items, so referencing shared definition geometry would
 *    need a reflected-pose special case anyway; baking sidesteps it and a
 *    mesh whose pose has negative determinant gets its winding flipped so
 *    triangles stay outward-facing, exactly like the STL path.
 *  - **Colors via core-spec `<basematerials>`.** Face colors are baked into
 *    vertex colors by the renderer (uniform per face), so each triangle has
 *    one color; distinct colors dedupe into one document-wide basematerials
 *    group, each object declares its first triangle's color as its default
 *    (`pid`/`pindex`), and only triangles that differ carry a `p1` override.
 *    Textured faces export as their material's tint color — core 3MF has no
 *    texture support without the materials extension.
 *  - **Vertices weld exactly.** Within one object, vertices dedupe on their
 *    formatted coordinate string — bit-identical positions only, never a
 *    tolerance (rule 4 in spirit): shared face edges from the tessellator
 *    weld back together, nothing else moves. Triangles whose indices
 *    collapse (fewer than 3 distinct vertices — forbidden by the 3MF spec)
 *    are skipped and counted, never repaired.
 *  - **Deterministic bytes.** Fixed zip timestamps, insertion-ordered
 *    palettes, and stable number formatting make the same scene export to
 *    the same bytes.
 */
import * as THREE from 'three'
import { zipSync, strToU8 } from 'three/examples/jsm/libs/fflate.module.js'
import type { SceneRenderer } from '../../viewport/SceneRenderer'

/** Kernel lengths are f64 meters; 3MF ships at millimeter scale. */
const METERS_TO_MM = 1000

/** The 3MF core-spec model namespace. */
const MODEL_XMLNS = 'http://schemas.microsoft.com/3dmanufacturing/core/2015/02'

/** Result of encoding the scene as a 3MF container. */
export interface ThreeMfBuildResult {
  bytes: Uint8Array
  /** `<object>` resources actually written. */
  objectCount: number
  /** Triangles actually written (after degenerate-index skipping). */
  triangleCount: number
  /** Triangles dropped because welding collapsed their vertex indices. */
  skippedDegenerate: number
}

/** Injected in tests; real builds get the Vite-defined app version. */
declare const __HEW_VERSION__: string | undefined

function hewVersion(): string {
  return typeof __HEW_VERSION__ !== 'undefined' ? __HEW_VERSION__ : '0.0.0'
}

/**
 * One exportable part: a named world-space triangle soup in METERS (9 numbers
 * per triangle, counter-clockwise = outward) with one sRGB color per triangle
 * (`#RRGGBB` or `#RRGGBBAA`, uppercase).
 */
export interface ThreeMfPart {
  name: string
  triangles: ArrayLike<number>
  colors: string[]
}

/**
 * The minimal slice of the wasm `Scene` surface the part-naming walk needs —
 * structural, so tests can pass a plain mock (same pattern as
 * `stlExport.ts`'s `SolidQueryScene`).
 */
export interface PartNameScene {
  object_name(object: bigint): string | undefined
  instance_name(instance: bigint): string | undefined
}

function xmlEscape(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

/**
 * Format one coordinate for XML: plain decimal, never scientific notation
 * (spec-safe), `-0` normalized to `0`. Welding keys on this string, so the
 * formatting IS the identity — bit-identical inputs always weld, nothing
 * else does.
 */
export function formatCoord(n: number): string {
  if (Object.is(n, -0) || n === 0) return '0'
  const s = String(n)
  if (!s.includes('e') && !s.includes('E')) return s
  // Expand scientific notation — reachable only for |mm| < 1e-6 or ≥ 1e21.
  // Every f64 at or above 1e21 is integral (2^53 < 1e21), so BigInt renders
  // it exactly; toFixed handles the sub-nanometer side.
  if (Math.abs(n) >= 1e21) return BigInt(n).toString()
  const fixed = n.toFixed(20)
  return fixed.includes('.') ? fixed.replace(/0+$/, '').replace(/\.$/, '') : fixed
}

/**
 * Pure 3MF writer over pre-collected parts. Exported separately from the
 * scene walk so the container layout and model XML are unit-testable without
 * three.js scene setup. Returns `null` when no part contributes a triangle.
 */
export function writeThreeMf(
  parts: ThreeMfPart[],
  version: string = hewVersion(),
): ThreeMfBuildResult | null {
  // Document-wide color palette, insertion-ordered for determinism.
  const paletteIndex = new Map<string, number>()
  const colorOf = (hex: string): number => {
    let i = paletteIndex.get(hex)
    if (i === undefined) {
      i = paletteIndex.size
      paletteIndex.set(hex, i)
    }
    return i
  }

  interface BuiltObject {
    id: number
    name: string
    defaultColor: number
    vertexLines: string[]
    triangleLines: string[]
  }

  const objects: BuiltObject[] = []
  let triangleCount = 0
  let skippedDegenerate = 0
  // Resource ids: 1 is the basematerials group, objects follow.
  let nextId = 2

  for (const part of parts) {
    const inCount = Math.floor(part.triangles.length / 9)
    if (inCount === 0) continue

    const vertexIndex = new Map<string, number>()
    const vertexLines: string[] = []
    const triangleLines: string[] = []
    let defaultColor = -1

    const vertexAt = (o: number): number => {
      const x = formatCoord(part.triangles[o] * METERS_TO_MM)
      const y = formatCoord(part.triangles[o + 1] * METERS_TO_MM)
      const z = formatCoord(part.triangles[o + 2] * METERS_TO_MM)
      const key = `${x} ${y} ${z}`
      let i = vertexIndex.get(key)
      if (i === undefined) {
        i = vertexIndex.size
        vertexIndex.set(key, i)
        vertexLines.push(`<vertex x="${x}" y="${y}" z="${z}"/>`)
      }
      return i
    }

    for (let t = 0; t < inCount; t++) {
      const o = t * 9
      const v1 = vertexAt(o)
      const v2 = vertexAt(o + 3)
      const v3 = vertexAt(o + 6)
      if (v1 === v2 || v2 === v3 || v1 === v3) {
        // The spec requires three distinct vertices; a collapsed triangle
        // carries no geometry. Skipped and counted — never repaired.
        skippedDegenerate++
        continue
      }
      const color = colorOf(part.colors[t])
      if (defaultColor < 0) defaultColor = color
      const p1 = color === defaultColor ? '' : ` p1="${color}"`
      triangleLines.push(`<triangle v1="${v1}" v2="${v2}" v3="${v3}"${p1}/>`)
    }

    if (triangleLines.length === 0) continue
    triangleCount += triangleLines.length
    objects.push({
      id: nextId++,
      name: part.name,
      defaultColor,
      vertexLines,
      triangleLines,
    })
  }

  if (objects.length === 0) return null

  const lines: string[] = []
  lines.push('<?xml version="1.0" encoding="UTF-8"?>')
  lines.push(`<model unit="millimeter" xml:lang="und" xmlns="${MODEL_XMLNS}">`)
  lines.push(` <metadata name="Application">Hew ${xmlEscape(version)}</metadata>`)
  lines.push(' <resources>')
  lines.push('  <basematerials id="1">')
  for (const hex of paletteIndex.keys()) {
    lines.push(`   <base name="${hex}" displaycolor="${hex}"/>`)
  }
  lines.push('  </basematerials>')
  for (const obj of objects) {
    lines.push(
      `  <object id="${obj.id}" type="model" name="${xmlEscape(obj.name)}"` +
        ` pid="1" pindex="${obj.defaultColor}">`,
    )
    lines.push('   <mesh>')
    lines.push('    <vertices>')
    for (const v of obj.vertexLines) lines.push(`     ${v}`)
    lines.push('    </vertices>')
    lines.push('    <triangles>')
    for (const t of obj.triangleLines) lines.push(`     ${t}`)
    lines.push('    </triangles>')
    lines.push('   </mesh>')
    lines.push('  </object>')
  }
  lines.push(' </resources>')
  lines.push(' <build>')
  for (const obj of objects) {
    lines.push(`  <item objectid="${obj.id}"/>`)
  }
  lines.push(' </build>')
  lines.push('</model>')
  const model = lines.join('\n') + '\n'

  const contentTypes =
    '<?xml version="1.0" encoding="UTF-8"?>\n' +
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">\n' +
    ' <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>\n' +
    ' <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>\n' +
    '</Types>\n'

  const rels =
    '<?xml version="1.0" encoding="UTF-8"?>\n' +
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">\n' +
    ' <Relationship Target="/3D/3dmodel.model" Id="rel-1"' +
    ' Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>\n' +
    '</Relationships>\n'

  // Fixed mtime keeps the container byte-stable across identical exports.
  // Zip stores DOS local-time fields (1980 minimum), so a local-components
  // date encodes identically in every timezone.
  const stamp = { mtime: new Date(2000, 0, 1) }
  const bytes = zipSync(
    {
      '[Content_Types].xml': [strToU8(contentTypes), stamp],
      '_rels/.rels': [strToU8(rels), stamp],
      '3D/3dmodel.model': [strToU8(model), stamp],
    },
    { level: 6 },
  )

  return { bytes, objectCount: objects.length, triangleCount, skippedDegenerate }
}

/**
 * Extract one mesh's world-space triangles (meters) and per-triangle colors.
 * Colors come from the geometry's baked per-vertex colors when the group's
 * material uses them (the renderer bakes each face's color uniformly, so the
 * first vertex speaks for the triangle) and from the material's base color
 * otherwise (textured groups); alpha is the material's palette opacity.
 * Winding flips under a negative-determinant world matrix, mirroring
 * `collectWorldTriangles` in `stlExport.ts`.
 */
function collectMeshTriangles(
  mesh: THREE.Mesh,
  triangles: number[],
  colors: string[],
): void {
  const geo = mesh.geometry
  const pos = geo.getAttribute('position')
  if (pos === undefined) return
  const colorAttr = geo.getAttribute('color')
  const index = geo.getIndex()
  const totalTris = (index !== null ? index.count : pos.count) / 3
  const flip = mesh.matrixWorld.determinant() < 0

  const materialAt = (i: number): THREE.Material => {
    const mat = mesh.material
    return Array.isArray(mat) ? mat[i] : mat
  }
  // A geometry without explicit groups renders wholly with material 0.
  const groups =
    geo.groups.length > 0
      ? geo.groups
      : [{ start: 0, count: totalTris * 3, materialIndex: 0 }]

  const a = new THREE.Vector3()
  const b = new THREE.Vector3()
  const c = new THREE.Vector3()
  const col = new THREE.Color()

  const alphaHex = (m: THREE.Material): string => {
    const alpha = Math.max(0, Math.min(1, m.opacity))
    if (alpha >= 1) return ''
    return Math.round(alpha * 255)
      .toString(16)
      .toUpperCase()
      .padStart(2, '0')
  }

  for (const group of groups) {
    const m = materialAt(group.materialIndex ?? 0) as THREE.MeshStandardMaterial
    const aa = alphaHex(m)
    const useVertexColors = m.vertexColors === true && colorAttr !== undefined
    // Group base color (textured / non-vertex-color groups): the material's
    // linear-space base color re-encoded as sRGB hex.
    const groupHex = useVertexColors
      ? ''
      : `#${m.color.getHexString(THREE.SRGBColorSpace).toUpperCase()}${aa}`

    const triStart = group.start / 3
    const triEnd = (group.start + group.count) / 3
    for (let t = triStart; t < triEnd; t++) {
      const i0 = index !== null ? index.getX(t * 3) : t * 3
      let i1 = index !== null ? index.getX(t * 3 + 1) : t * 3 + 1
      let i2 = index !== null ? index.getX(t * 3 + 2) : t * 3 + 2
      if (flip) {
        const tmp = i1
        i1 = i2
        i2 = tmp
      }
      a.fromBufferAttribute(pos, i0).applyMatrix4(mesh.matrixWorld)
      b.fromBufferAttribute(pos, i1).applyMatrix4(mesh.matrixWorld)
      c.fromBufferAttribute(pos, i2).applyMatrix4(mesh.matrixWorld)
      triangles.push(a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z)
      if (useVertexColors) {
        // Vertex colors are stored linear; re-encode as sRGB for display.
        col.fromBufferAttribute(colorAttr, i0)
        colors.push(`#${col.getHexString(THREE.SRGBColorSpace).toUpperCase()}${aa}`)
      } else {
        colors.push(groupHex)
      }
    }
  }
}

/**
 * Walk `buildExportScene`'s output into named parts: one per world object,
 * one per placed instance member (pose baked to world space). Display names
 * come from the document (falling back to stable `Object N` labels), with a
 * multi-member instance's parts qualified by their member names.
 */
export function collectExportParts(
  root: THREE.Object3D,
  names: PartNameScene,
): ThreeMfPart[] {
  const parts: ThreeMfPart[] = []

  const pushMesh = (mesh: THREE.Mesh, name: string): void => {
    const triangles: number[] = []
    const colors: string[] = []
    collectMeshTriangles(mesh, triangles, colors)
    if (triangles.length > 0) parts.push({ name, triangles, colors })
  }

  for (const child of root.children) {
    const objectIdStr = child.userData.hewObjectId as string | undefined
    const instanceIdStr = child.userData.hewInstanceId as string | undefined
    if (objectIdStr !== undefined && (child as THREE.Mesh).isMesh === true) {
      const name = names.object_name(BigInt(objectIdStr)) ?? `Object ${objectIdStr}`
      pushMesh(child as THREE.Mesh, name)
    } else if (instanceIdStr !== undefined) {
      const instName = names.instance_name(BigInt(instanceIdStr))
      const memberMeshes = child.children.filter(
        (m): m is THREE.Mesh => (m as THREE.Mesh).isMesh === true,
      )
      for (const mesh of memberMeshes) {
        const memberIdStr = mesh.userData.hewObjectId as string | undefined
        const memberName =
          memberIdStr !== undefined ? names.object_name(BigInt(memberIdStr)) : undefined
        // Fully anonymous parts qualify with the INSTANCE id — the member id
        // is shared by every placement of the component, so falling back to
        // it alone would name fifty unnamed bolts identically. A human-given
        // member name is kept as-is: duplicate "Bolt"s read naturally.
        const anonymous = `Object ${memberIdStr ?? '?'} (Instance ${instanceIdStr})`
        const name =
          instName === undefined
            ? (memberName ?? anonymous)
            : memberMeshes.length > 1
              ? `${instName} · ${memberName ?? `Object ${memberIdStr ?? '?'}`}`
              : instName
        pushMesh(mesh, name)
      }
    }
  }

  return parts
}

/**
 * Serialize the current solid geometry (objects + instances, faces only) to
 * a 3MF container — millimeter unit, Z-up, one named colored mesh per part.
 * Returns `null` when there is nothing solid to export.
 */
export function exportSceneTo3mf(
  renderer: SceneRenderer,
  names: PartNameScene,
): ThreeMfBuildResult | null {
  if (!renderer.hasExportableGeometry()) return null

  const root = renderer.buildExportScene()
  try {
    // buildExportScene bakes glTF's −90°-about-X Y-up rotation into the root;
    // 3MF is natively Z-up (Hew's world frame), so undo it.
    root.matrix.identity()
    root.matrixWorldNeedsUpdate = true
    root.updateMatrixWorld(true)
    return writeThreeMf(collectExportParts(root, names))
  } finally {
    renderer.disposeExportScene(root)
  }
}
