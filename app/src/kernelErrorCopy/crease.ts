/* oxlint-disable lingui/no-unlocalized-strings -- Smooth is a Preview feature: the crease refusals are English only until it graduates, and are compiled out of a production build (src/preview). */
/**
 * Plain-language copy for the kernel refusals of the Eraser's crease half:
 * marking edges that stay sharp when smoothed. Spread into `DESCRIPTIONS` by
 * `kernelErrors.ts`.
 *
 * Plain descriptors, not `msg`, so the words stay out of the translation
 * catalogs; empty in a production build, where `hew.object.crease` is
 * refused before it can raise them.
 */

import type { MessageDescriptor } from '@lingui/core'
import { previewBuild } from '../preview'

export const CREASE_ERRORS: Record<string, MessageDescriptor> = previewBuild
  ? {
      CreaseOnSoftEdge: { id: 'kernel.CreaseOnSoftEdge', message: "Soft seams can't be creased." },
      CreaseOnBoundary: { id: 'kernel.CreaseOnBoundary', message: "An edge on an opening can't be creased." },
    }
  : {}
