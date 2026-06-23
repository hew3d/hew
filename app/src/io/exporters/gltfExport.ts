/**
 * glTF/GLB export.
 *
 * Architecture (DESIGN decision): export runs entirely in TypeScript over the
 * live three.js scene, rather than as a Rust crate mirroring `dae-import`.
 * The world-space scene graph only exists on the TS side — instance poses are
 * applied at render time and never baked into the kernel — and the
 * renderer already holds tessellated geometry, PBR-able materials, and decoded
 * textures. three's `GLTFExporter` then gives us node hierarchy, per-instance
 * transforms, embedded textures, and the GLB binary container for free, which
 * directly serves the Blender round-trip goal. Import stays a
 * Rust crate because it must heal/rebuild into editable kernel Objects.
 *
 * OBJ export was intentionally cut from this slice.
 */
import { GLTFExporter } from 'three/examples/jsm/exporters/GLTFExporter.js'
import type { SceneRenderer } from '../../viewport/SceneRenderer'

/**
 * Serialize the current solid geometry (objects + instances, faces only) to a
 * binary glTF (`.glb`) buffer. Returns the GLB bytes, or `null` when there is
 * nothing solid to export.
 */
export async function exportSceneToGlb(
  renderer: SceneRenderer,
): Promise<Uint8Array | null> {
  if (!renderer.hasExportableGeometry()) return null

  const root = renderer.buildExportScene()
  try {
    const exporter = new GLTFExporter()
    // binary: true → the result is an ArrayBuffer (the .glb container).
    const result = await exporter.parseAsync(root, { binary: true })
    return new Uint8Array(result as ArrayBuffer)
  } finally {
    renderer.disposeExportScene(root)
  }
}
