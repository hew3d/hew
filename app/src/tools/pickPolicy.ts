/**
 * The window's render mode as the tools' own picking helpers see it.
 *
 * The kernel applies the mode's face policy to every snap and face pick
 * (`Scene.set_render_mode`); a few app-side helpers decide things the kernel
 * cannot (`FacePickCache.faceThrough`'s hidden-point fallback is for X-ray
 * only — Wireframe has no faces to adopt). They read the mode here, set by
 * the Viewport alongside the kernel switch.
 *
 * `pickPolicyGeneration` moves on every change, so a pick memoized for a ray
 * before the switch (the per-event `FacePickCache` / `SketchPickCache`, keyed
 * by the Ray object a held pointer's re-resolve reuses) is never served
 * after it.
 *
 * The pick aperture (`setPickAperture`) is the camera's projection as the
 * same helpers need it: `FacePickCache.faceThrough`'s boundary ring and lean
 * dead zone are a pixel and a half wide on screen under either projection
 * (`pickNudgeRad`).
 */
import type { RenderMode } from '../viewport/SceneRenderer'
import type { ApertureBasis } from '../viewport/math'
import { apertureForPixelRadius } from '../viewport/math'
import { PICK_NUDGE_PX, PICK_NUDGE_RAD } from './faceDraw'

let mode: RenderMode = 'shaded'
let generation = 0

/** Record the window's render mode (the Viewport calls this when it pushes
 *  the mode to the kernel). A change starts a new pick generation. */
export function setPickRenderMode(next: RenderMode): void {
  if (next === mode) return
  mode = next
  generation += 1
}

/** The render mode the tools' picks follow. */
export function pickRenderMode(): RenderMode {
  return mode
}

/** A counter that moves whenever the render mode does: memoized picks keyed
 *  to an older value are stale. */
export function pickPolicyGeneration(): number {
  return generation
}

/** The editing scope changed (`pushPickScope`): memoized picks keyed to an
 *  older generation are stale even though no pointer moved and the render
 *  mode is unchanged. */
export function bumpPickScopeGeneration(): void {
  generation += 1
}

let aperture: { basis: ApertureBasis; viewportHeightPx: number } = {
  basis: { kind: 'perspective', fovYDeg: 45 },
  viewportHeightPx: 720,
}

/** Record the camera's pick aperture (the Viewport calls this wherever it
 *  builds an `ApertureBasis`, and once per frame). Until the first call the
 *  aperture is a perspective one, so a caller that never sets it keeps the
 *  perspective constant. */
export function setPickAperture(basis: ApertureBasis, viewportHeightPx: number): void {
  aperture = { basis, viewportHeightPx }
}

/**
 * The boundary ring's angular radius, and the lean dead zone, for a ray to a
 * point `distanceToPoint` meters from the eye: `PICK_NUDGE_PX` on screen in
 * either projection. Perspective is the constant `PICK_NUDGE_RAD` (about
 * 1.5 px of a 45° view on a 720 px canvas), whatever the distance. Under
 * parallel projection a pixel is a fixed world width, so the angle that
 * spans it at the point shrinks with distance: `PICK_NUDGE_PX` world-per-
 * pixel widths over the distance (floored, so a point at the eye stays
 * finite).
 */
export function pickNudgeRad(distanceToPoint: number): number {
  if (aperture.basis.kind === 'perspective') return PICK_NUDGE_RAD
  const radius = apertureForPixelRadius(aperture.basis, PICK_NUDGE_PX, aperture.viewportHeightPx)
  return radius / Math.max(distanceToPoint, 1e-6)
}
