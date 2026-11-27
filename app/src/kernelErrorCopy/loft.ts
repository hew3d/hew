/**
 * Plain-language copy for the kernel refusals of one curve feature: Loft:
 * shapes, holes, and rails.
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const LOFT_ERRORS: Record<string, MessageDescriptor> = {
  OpenProfile: msg`Loft needs closed shapes. Close this one first.`,
  LoftProfileHasHole: msg`Loft needs shapes without holes. Shell the lofted solid to hollow it.`,
  LoftNeedsTwoProfiles: msg`Loft needs at least two shapes, or one shape and a point.`,
  LoftTooManyRails: msg`Loft takes at most two rails.`,
  RailMissesProfile: msg`Each rail must run through a matching corner or point on every shape you loft between, and two rails must line the shapes up the same way.`,
}
