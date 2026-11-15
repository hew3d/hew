/**
 * Plain-language copy for the kernel refusals of one curve feature: Array
 * along Path.
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const PATH_ARRAY_ERRORS: Record<string, MessageDescriptor> = {
  PathTooShortForCount: msg`The path is too short for that many copies.`,
}
