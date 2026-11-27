/**
 * Plain-language copy for the kernel refusals of one curve feature: Fillet
 * and Chamfer (the blend feature): edge and corner blends, profiles, and
 * the 2D rounded corner.
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const BLEND_ERRORS: Record<string, MessageDescriptor> = {
  EmptySpec: msg`Select the edges to fillet first.`,
  BlendNeedsSolid: msg`Fillet needs a solid. This object has gaps.`,
  EdgeIsSmooth: msg`That edge is already smooth.`,
  UnsupportedEdge: msg`This edge can't be filleted yet.`,
  UnsupportedVertex: msg`Can't blend this corner. Try these edges one at a time.`,
  RadiusTooLarge: msg`That radius doesn't fit here. Try a smaller one.`,
  ProfileTooWide: msg`This profile is wider than the face it has to fit on. Try a smaller profile or a smaller size.`,
  ProfileNeedsConvexEdge: msg`A cove needs an outside edge.`,
  BullnoseNeedsParallelSides: msg`A bullnose needs two parallel edges of one face.`,
  InvalidProfile: msg`This profile shape isn't valid. Redraw it as one open chain.`,
  UnknownProfile: msg`That profile isn't built in. Pick one from the profile list.`,
  RenderFailed: msg`Couldn't draw the preview picture. Try again.`,
  StopOutOfRange: msg`A stop or radius point has to sit on the edge, farther from its end than the radius.`,
  Obstructed: msg`The fillet would cut into other geometry.`,
  SegmentsOutOfRange: msg`Use between 2 and 96 segments.`,
  NonUniformInstance: msg`Make this component unique, or reset its scale, to fillet it.`,
  BlendModified: msg`This fillet has been changed and can't be edited. It's ordinary geometry now.`,
  UnknownBlend: msg`That face isn't part of a fillet.`,
  BlendDegenerate: msg`That fillet didn't produce a valid solid. The model is unchanged.`,
  CornerNotBlendable: msg`Only a corner where two straight lines meet can be rounded.`,
  CornerIsStraight: msg`Those lines are in a straight line; there is no corner to round.`,
  CornerRadiusTooLarge: msg`The radius is too large for this corner. Try a smaller radius.`,
  CornerLeavesFace: msg`The rounded corner would run off the face. Draw the shape farther inside the face, or use a smaller radius.`,
  DependentBlend: msg`A fillet that runs into this one can't be rebuilt after that change. Change or remove that fillet first.`,
  BadSpec: msg`Hew's own request to the model kernel carried a malformed shape or path. If this keeps happening, use Report Bug.`,
  BlendFacesFlush: msg`The two surfaces meet smoothly here, with no edge between them, as two pipes of the same size do. There's nothing to round, chamfer, or cove there. Make one part a little larger or smaller.`,
}
