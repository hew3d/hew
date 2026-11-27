/**
 * Plain-language copy for the kernel refusals of one curve feature:
 * Revolve: the axis and its profile plane.
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const REVOLVE_ERRORS: Record<string, MessageDescriptor> = {
  AxisNotInProfilePlane: msg`The axis has to lie in the plane of the shape. Move the axis onto that plane.`,
}
