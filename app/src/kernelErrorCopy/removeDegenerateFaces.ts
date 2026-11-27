/**
 * Plain-language copy for the kernel refusals of one curve feature: Model
 * Health ▸ Remove degenerate faces, and the boolean refusals that point at
 * it.
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const REMOVE_DEGENERATE_FACES_ERRORS: Record<string, MessageDescriptor> = {
  NothingToRemove: msg`No degenerate faces found.`,
  NonPlanarInput: msg`One of these objects has a face that isn't flat enough to combine.`,
  DegenerateInput:
    msg`One of these objects has a face with no area. Use Model Health ▸ Remove degenerate faces first.`,
}
