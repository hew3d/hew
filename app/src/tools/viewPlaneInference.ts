/**
 * Tool-side glue for view-plane inference (docs/design/view-plane-inference.md):
 * the camera feed the draw tools share, and the status-bar line that names the
 * plane an idle draw tool inferred from the view. The picking rule itself
 * lives in `drawPlane.ts` (`inferredAxisPlane`); this file only carries what
 * the tools add around it.
 */
import * as THREE from 'three'
import type { CameraView } from './drawPlane'

const scratch = new THREE.Vector3()

/** The camera as the inference reads it: its unit forward direction in world
 *  space and its eye (`camera.position`), both copied, so later camera moves
 *  never alias into a held view. */
export function cameraView(camera: THREE.Camera): CameraView {
  camera.getWorldDirection(scratch)
  return {
    dir: [scratch.x, scratch.y, scratch.z],
    eye: [camera.position.x, camera.position.y, camera.position.z],
  }
}
