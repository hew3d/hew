/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/** Status-bar copy for the Fillet tool. */

import { type GripId } from '../filletGrips'

const SEAM_HINT = 'These are two objects. Union them to blend where they meet.'

/** The hint where separate objects meet, naming them when they have names
 *  (a second, overlapping copy of an object is otherwise invisible). */
export function seamHint(names: readonly string[], group?: string): string {
  if (group !== undefined) {
    const of = group === '' ? 'one group' : `the group ${group}`
    return `These are separate pieces of ${of}. Ungroup it and Union the pieces to blend where they meet. If they only touch, move one a little into the other first.`
  }
  if (names.length === 2 && names[0] === names[1]) {
    return `Two objects named ${names[0]} meet here. Union them to blend where they meet.`
  }
  if (names.length === 2) return `${names[0]} and ${names[1]} are separate objects. Union them to blend where they meet.`
  if (names.length > 2) return `${names.length} separate objects meet here. Union them to blend where they meet.`
  return SEAM_HINT
}
export const SMOOTH_HINT = 'That edge is already smooth.'
export const BLEND_CHANGED_HINT = "A later edit changed this fillet, so it can't be resized or removed. Undo back past that edit to change it."

/** The status line with the pointer on a grip. */
export function gripHoverStatus(g: GripId): string {
  switch (g.kind) {
    case 'radius':
      return 'Drag this handle to size it. Click it to finish.'
    case 'stop':
      return 'Click or drag this grip to stop the fillet short of the end.'
    case 'handlePos':
      return 'Click or drag this grip to move the radius point along the edge.'
    default:
      return 'Click or drag this grip to set the radius at this end.'
  }
}

/** The status line while a grip is picked up. */
export function gripStatus(g: GripId): string {
  switch (g.kind) {
    case 'radius':
      return 'Move to size it, then let go. Or type a size.'
    case 'stop':
      return 'Move to place the stop, click to set it, or type its distance from the end.'
    case 'handlePos':
      return 'Move the radius point along the edge, click to place it, or type its distance from the start.'
    default:
      return 'Move to set the radius here, click to keep it, or type a radius.'
  }
}
