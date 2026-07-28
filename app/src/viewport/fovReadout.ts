/**
 * fovReadout — what the Zoom tool's VCB should show (docs/design/camera.md
 * §2; the standalone Camera ▸ Field of View menu item this section used to
 * describe is gone — see camera-playtest2.md §2), at rest (not actively
 * being typed into).
 *
 * Playtest finding 4b: the readout used to appear ONLY once the user typed
 * a digit — a fresh Zoom activation, or a cursor drag with no typing yet,
 * left the VCB blank. It must instead PERSISTENTLY show the current fovDeg
 * the whole time the Zoom camera mode is active in perspective, updating
 * whenever fovDeg changes; blank the rest of the time (no camera tool
 * active, or under parallel projection — no lens to report).
 */
import { fovDegToFocalMm } from './fovUnits'

/** Formats a fovDeg value in BOTH units (camera-playtest2.md §1): degrees
 * first — matching every other angle VCB in this app
 * (`RotateTool`/`ProtractorTool`: `${deg.toFixed(1)}°`) and staying the
 * primary/resting unit — then the equivalent 35mm-equivalent focal length,
 * so a user thinking in either vocabulary sees their number. Always shows
 * the value fov ACTUALLY holds (post-clamp), never the raw typed number —
 * see `parseFovEntry`'s doc (fovUnits.ts) for why clamping happens before
 * this ever runs. */
export function formatFov(fovDeg: number): string {
  return `${fovDeg.toFixed(1)}° · ${fovDegToFocalMm(fovDeg).toFixed(1)} mm`
}

/** Pure decision: given the active camera tool and projection, what should
 * the VCB show at rest? Empty string means "nothing" (`MeasurementBox`
 * renders null for an empty value). */
export function fovReadoutText(
  activeCameraTool: string | null,
  projection: 'perspective' | 'parallel',
  fovDeg: number,
): string {
  return activeCameraTool === 'Zoom' && projection === 'perspective' ? formatFov(fovDeg) : ''
}

/** The `activeCameraTool` value for whatever tool name switchToolRef is
 * about to switch TO — null for every name that isn't one of the four
 * camera modes (every drawing/modify tool, Position Camera/Look
 * Around/Walk, and Select).
 *
 * Pulled out as its own pure lookup (camera-fov-fixes, finding C) so
 * Viewport's switchToolRef can compute `activeCameraTool` in ONE place that
 * runs for EVERY switch, rather than the tool having previously set it only
 * inside the Orbit/Pan/Zoom/Zoom Window/default cases — which left it
 * stale (still showing the outgoing camera tool) whenever the switch
 * target was some OTHER named tool that fell through its own case without
 * ever touching `activeCameraTool`, e.g. Zoom → Rectangle. */
export function activeCameraToolForName(
  toolName: string,
): 'Orbit' | 'Pan' | 'Zoom' | 'ZoomWindow' | null {
  switch (toolName) {
    case 'Orbit': return 'Orbit'
    case 'Pan': return 'Pan'
    case 'Zoom': return 'Zoom'
    case 'Zoom Window': return 'ZoomWindow'
    default: return null
  }
}
