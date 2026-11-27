/**
 * Which ink a sketch's drawing wears.
 *
 * - `free` (blue): drawing that belongs to no solid.
 * - `attached` (orange): drawing that is part of a solid's face.
 * - `plain`: geometry that came in with a foreign file and has not been
 *   drawn on since. It draws like an ordinary edge (the neutral edge
 *   colour, native width); only drawing made in Hew gets ink. The kernel
 *   clears the imported mark when the user draws on or edits the item, so
 *   the ink comes back by itself on the next refresh.
 */
export type DrawKind = 'free' | 'attached' | 'plain'

/** Imported wins over ownership: an imported sketch is plain whether or not
 *  an Object owns its edges. */
export function drawKindFor(imported: boolean, attached: boolean): DrawKind {
  if (imported) return 'plain'
  return attached ? 'attached' : 'free'
}
