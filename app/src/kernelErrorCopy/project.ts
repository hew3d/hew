/**
 * Plain-language copy for the kernel refusals of one curve feature:
 * Project: engrave or emboss a closed shape onto a solid.
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const PROJECT_ERRORS: Record<string, MessageDescriptor> = {
  ProjectMisses: msg`The shape doesn't land on this object. Aim it at a face.`,
  ProjectNeedsClosedShape: msg`Project needs a closed shape. Close the drawing first.`,
  WouldSever: msg`That depth would cut the object apart. Use Push/Pull or Subtract to split it.`,
}
