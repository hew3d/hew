import type { ViewStyleColors } from './Viewport'

/**
 * Whether a document's own background, sky and ground colors may paint the
 * viewport. Off for now: SketchUp is always light, so the colors people set
 * there look wrong in Hew's dark theme, and Hew cannot edit them yet. The
 * kernel still stores and saves them, so a save back to `.skp` keeps the
 * file's own look. Turning this back on once Hew can edit them is a one-line
 * change: set the flag to `true`.
 */
export const DOCUMENT_BACKDROP_COLORS_ENABLED = false

/**
 * The one gate every consumer of the document's (or a scene's) backdrop
 * colors goes through. Returns the style to paint with, or `null` for Hew's
 * own theme backdrop and grid colors.
 */
export function documentBackdropColors(style: ViewStyleColors | null): ViewStyleColors | null {
  return DOCUMENT_BACKDROP_COLORS_ENABLED ? style : null
}
