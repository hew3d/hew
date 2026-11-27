/**
 * Plain-language copy for the kernel refusals of one curve feature: Smooth
 * (subdivision levels).
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const SMOOTH_ERRORS: Record<string, MessageDescriptor> = {
  SmoothTooDense: msg`This shape is too dense for that Smooth level. Try a lower level, or simplify the shape first.`,
}
