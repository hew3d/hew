/**
 * Plain-language copy for the kernel refusals of one curve feature: Eraser
 * creases: marking edges that stay sharp when smoothed.
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const CREASE_ERRORS: Record<string, MessageDescriptor> = {
  CreaseOnSoftEdge: msg`Soft seams can't be creased.`,
  CreaseOnBoundary: msg`An edge on an opening can't be creased.`,
}
