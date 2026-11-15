/**
 * gripStyle — the on-screen look shared by every tool that draws grips
 * (Scale's box handles, the Fillet tool's stops and radius handles).
 *
 * A grip is a small square marker held at a constant size in CSS pixels
 * whatever the camera distance or model size; the pick tolerance is a
 * multiple of its rendered half-size so a handle is easy to grab.
 */

/**
 * Target on-screen size of a grip marker — full edge length, in CSS pixels —
 * held CONSTANT regardless of camera distance or model scale. Grips used to
 * be sized as a fraction of the box diagonal, clamped to 2–15 cm of WORLD
 * space: the 2 cm floor dwarfed small models, and a compact rounded shape's
 * 26 always-on-top grips piled into what playtesting called "an explosion of
 * multi-colored cubes that make no logical sense." A small fixed pixel size
 * reads as a handle, not a block, at any zoom or model size.
 */
export const GRIP_SCREEN_PX = 9

/** Floor on a grip's rendered WORLD half-size, in meters — only guards
 * against a literal zero/negative scale at a degenerate (zero-height)
 * viewport; far below anything a user would ever perceive. */
export const MIN_GRIP_WORLD_HALF = 1e-5

/** Placeholder half-size (meters) a newly-drawn grip renders at for the one
 * frame before the Viewport's render loop first calls `updateGripScale`.
 * Also the fallback used for pick tolerance before that first tick — e.g. in
 * unit tests, which never drive a real render loop. */
export const FALLBACK_GRIP_HALF_M = 0.02

/** Pick tolerance around a grip, as a multiple of its rendered on-screen
 * half-size — forgiving enough to grab a small handle without pixel-precise
 * aiming. */
export const GRIP_PICK_MULTIPLIER = 3

/** Neutral color for grips that stand for no axis. */
export const GRIP_NEUTRAL_COLOR = 0xdddddd
