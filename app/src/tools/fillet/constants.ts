/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/** Fillet tool limits, colors and thresholds. */

import { EDGE_COLOR_SELECTED } from '../../viewport/SceneRenderer'

/**
 * Objects with more triangles than this skip the face mask and draw the
 * preview patch over the unmasked object with a depth offset instead.
 *
 * Measured (Node, one linear pass over the index buffer with 40 masked
 * faces; rebuilt only when the masked set changes): 10,000 triangles
 * 1-2 ms; 100,000 triangles 2-4 ms; 500,000 triangles 13-17 ms (one frame is
 * 16 ms); 1,000,000 triangles 23-24 ms. The cost crosses a frame between
 * 500k and 600k triangles; the limit sits below that so the rebuild plus the
 * buffer upload stay inside one frame.
 */
export const FACE_MASK_MAX_TRIANGLES = 400_000

/** A click that commits from the pointer must follow a move of at least this
 *  many CSS pixels since arming, so the second click of a double-click never
 *  commits a zero-size blend. */
export const MIN_COMMIT_MOVE_PX = 4
/** A grip pressed and released this many screen pixels apart was dragged,
 *  not clicked. Matches the commit move: touch, trackpad, and high-DPI
 *  clicks jitter a pixel or two in CSS pixels. */
export const GRIP_DRAG_PX = MIN_COMMIT_MOVE_PX
/** A click this close (screen px) to the armed edge adds a radius point;
 *  this close to where the handle leaves the edge, it finishes. */
export const HANDLE_PICK_PX = 12
export const HANDLE_PICK_FALLBACK_M = 0.03

/** The preview patch and rail colors. */
export const PATCH_COLOR = 0xdcdcdc
export const RAIL_COLOR = EDGE_COLOR_SELECTED
/** Rails turn this color while the size is clamped. */
export const CLAMPED_RAIL_COLOR = 0xcc2200
export const HIGHLIGHT_STYLE = { color: EDGE_COLOR_SELECTED, widthPx: 3.5, depthTest: false, renderOrder: 998 } as const

/** A preview of an object that takes this long (a dense imported scan)
 *  turns that object's live preview off: every pointer move would wait as
 *  long again. Typed sizes and clicks still round it. */
export const SLOW_PREVIEW_MS = 1000

/** Refusals that come after the kernel searched for the size that fits,
 *  so take longer than a preview would. */
export const SEARCHING_REFUSALS = new Set(['RadiusTooLarge', 'Obstructed', 'ProfileTooWide'])

/** The status while an object's live preview is off. */
export const SLOW_PREVIEW_HINT =
  'This object is large, so there is no live preview. Type a size and press Enter, or click to use the size shown.'
