/**
 * Plain-language copy for the kernel refusals of one curve feature:
 * Refusals shared by the shape tools (a result that crosses itself, a non-
 * solid input, an out-of-range amount, an invalid result).
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const SHAPE_COMMON_ERRORS: Record<string, MessageDescriptor> = {
  SelfIntersecting: msg`The result would pass through itself.`,
  ShapeNeedsSolid: msg`This needs a solid. This object has gaps.`,
  AmountOutOfRange: msg`That amount is out of range.`,
  ShapeDegenerate: msg`That change didn't produce a valid solid. The model is unchanged.`,
  ReshapeRedoDiverged: msg`Couldn't redo this change. The model is unchanged.`,
}
