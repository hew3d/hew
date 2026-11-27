/**
 * Plain-language copy for the kernel refusals of one curve feature: Shell:
 * wall thickness and corners.
 * Spread into `DESCRIPTIONS` by `kernelErrors.ts`.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

export const SHELL_ERRORS: Record<string, MessageDescriptor> = {
  WallTooThick: msg`The wall is too thick for this shape. Try a thinner wall.`,
  UnsupportedShellCorner: msg`Can't thicken this corner: its faces don't meet in one point.`,
}
