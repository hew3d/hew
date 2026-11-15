/**
 * Plain-language copy for the kernel refusals of one curve feature: Deform
 * (twist, bend, taper, bend to path).
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const DEFORM_ERRORS: Record<string, MessageDescriptor> = {
  TooThinToDeform: msg`Part of this solid is too thin to deform at this size. Deform a shorter piece, or make the thin part thicker.`,
  PreviewTooLarge: msg`This object is too large to preview. Click or press Enter to apply it anyway.`,
}
