/**
 * guidePreview — the Move tool's ghost of a construction guide, the guide
 * counterpart of `buildSelectionPreview`: a solid line (or a point guide's
 * cross) in the selection color, drawn on top like the selected guide's
 * highlight, that the tool slides with the cursor until the commit.
 */
import * as THREE from 'three'
import type { Scene as WasmScene } from '../wasm/loader'
import { guideOverlayPositions } from '../viewport/guideWindow'
import { EDGE_COLOR_SELECTED } from '../viewport/SceneRenderer'

export function buildGuidePreview(
  scene: Pick<WasmScene, 'guide_kind' | 'guide_geometry'>,
  guide: bigint,
): THREE.LineSegments | null {
  const kind = scene.guide_kind(guide)
  const geometry = scene.guide_geometry(guide)
  if (kind === undefined || geometry === undefined) return null
  const pts = guideOverlayPositions(kind, geometry)
  if (pts === null) return null
  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pts), 3))
  const mat = new THREE.LineBasicMaterial({ color: EDGE_COLOR_SELECTED, depthTest: false })
  const line = new THREE.LineSegments(geo, mat)
  line.renderOrder = 999
  // The tool moves it through `position`; its bounding sphere stays where
  // it was built, so culling would hide a ghost dragged out of view.
  line.frustumCulled = false
  return line
}
