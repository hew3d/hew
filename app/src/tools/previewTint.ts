/**
 * Tool-preview tints read from the theme tokens, so a ghost keeps its
 * contrast in both light and dark. Falls back to the given hex outside a DOM
 * (unit tests) or when the token is missing or unparsable.
 */
import * as THREE from 'three'

export function readPreviewTint(token: string, fallback: number): number {
  if (typeof window === 'undefined' || typeof window.getComputedStyle !== 'function') return fallback
  const raw = window.getComputedStyle(document.documentElement).getPropertyValue(token).trim()
  if (raw === '') return fallback
  try {
    return new THREE.Color(raw).getHex()
  } catch {
    return fallback
  }
}

/** Plain push/pull preview (the existing orange look). */
export const PREVIEW_PUSHPULL_TOKEN = '--preview-pushpull'
export const PREVIEW_PUSHPULL_FALLBACK = 0xee8800
/** Extrude-as-new-object preview (a blue, clearly apart from the orange). */
export const PREVIEW_EXTRUDE_NEW_TOKEN = '--preview-extrude-new'
export const PREVIEW_EXTRUDE_NEW_FALLBACK = 0x3d7fe6
/** Detached-shape draw preview (the same blue as extrude-as-new: "new, apart"). */
export const PREVIEW_DETACHED_TOKEN = '--preview-detached'
export const PREVIEW_DETACHED_FALLBACK = 0x3d7fe6
/** Draw-tool preview while the shape will JOIN the drawing under it (the
 *  default): the same orange as a plain pull. Reverting to the plain
 *  preview blue is a token change (set it to #4d90ff / the light value). */
export const PREVIEW_DRAW_JOINED_TOKEN = '--preview-draw-joined'
export const PREVIEW_DRAW_JOINED_FALLBACK = PREVIEW_PUSHPULL_FALLBACK
