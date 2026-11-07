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
 */
import type { RenderMode } from '../viewport/SceneRenderer'

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
