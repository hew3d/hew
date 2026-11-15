/**
 * The Preview wall's app-side entry (docs/design/preview-channel.md section
 * 3). Every filter at a single source (tool registry, registrations, menus,
 * palette, keymap, dock, dispatch, panels) asks these functions; nothing else
 * reads the registry or the build flag.
 *
 * The wall is fixed when the app loads: `previewBuild` is a build-time
 * literal, and the per-feature switches are read once (see `./switches.ts`).
 */

import { previewBuild } from './build'
import { PREVIEW_FEATURES } from './registry'
import { switchedOffNow } from './switches'
import { createWall } from './wall'

export { previewBuild }
export type { PreviewFeature } from './registry'

const wall = createWall(previewBuild, PREVIEW_FEATURES, switchedOffNow())

/** The features in the registry (the Settings list); empty in production. */
export const previewFeatures = wall.features
export const featureEnabled = wall.featureEnabled
export const toolAllowed = wall.toolAllowed
export const actionAllowed = wall.actionAllowed
export const chipAllowed = wall.chipAllowed
export const behaviorEnabled = wall.behaviorEnabled
export const isPreviewTool = wall.isPreviewTool
export const isPreviewAction = wall.isPreviewAction

/** Ids of the features that are on in this window: what Report Bug stamps. */
export function enabledFeatureIds(): string[] {
  return wall.features().filter((f) => wall.featureEnabled(f.id)).map((f) => f.id)
}
