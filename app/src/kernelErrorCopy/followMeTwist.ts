/**
 * Plain-language copy for the kernel refusals of one curve feature: Follow
 * Me twist and end scale along a path.
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const FOLLOW_ME_TWIST_ERRORS: Record<string, MessageDescriptor> = {
  TwistNeedsOpenPath: msg`Twist and end scale need an open path.`,
}
