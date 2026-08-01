/**
 * guideColors — single source of truth for the neutral/uncommitted
 * construction-guide color, shared between the committed-guide renderer
 * (`SceneRenderer.refreshGuides`) and any tool's live guide preview (e.g.
 * `TapeMeasureTool`'s guide-line preview) so the two never drift apart.
 */

/** Muted grey for construction guides — distinct from edges/axes/sketch
 *  lines, and the fallback color for a guide-preview segment whose direction
 *  doesn't read as any particular drawing axis. */
export const GUIDE_COLOR = 0x555555
