/**
 * The Preview wall, as pure logic: given the build flag, the registry and the
 * features switched off, which tools, commands, chips and behaviors are
 * reachable. No module state and no build flag in here, so a test can build a
 * wall for either configuration with an explicit argument.
 *
 * Rules:
 *  - An id the registry does not list is public (a graduated feature is just
 *    its entry deleted).
 *  - An id the registry lists is reachable only in a preview build, and only
 *    while its feature's switch is on.
 */

import type { PreviewFeature } from './registry'

export interface Wall {
  /** Whether this wall is for a preview build. */
  readonly build: boolean
  /** The registry entries, for the Settings section. Empty outside a preview build. */
  features(): readonly PreviewFeature[]
  featureEnabled(id: string): boolean
  toolAllowed(name: string): boolean
  actionAllowed(id: string): boolean
  chipAllowed(id: string): boolean
  behaviorEnabled(id: string): boolean
  /** Whether the tool is a Preview feature's, in a preview build (for the
   *  "Preview" tag). Always false in production. */
  isPreviewTool(name: string): boolean
  /** Whether the command is a Preview feature's, in a preview build. */
  isPreviewAction(id: string): boolean
}

export function createWall(
  build: boolean,
  features: readonly PreviewFeature[],
  switchedOff: ReadonlySet<string>,
): Wall {
  const owner = (pick: (f: PreviewFeature) => readonly string[]): Map<string, string> => {
    const out = new Map<string, string>()
    for (const f of features) for (const id of pick(f)) out.set(id, f.id)
    return out
  }
  const toolOwner = owner((f) => f.tools)
  const actionOwner = owner((f) => f.actions)
  const chipOwner = owner((f) => f.dockChips)
  const behaviorOwner = owner((f) => f.behaviors)
  const known = new Set(features.map((f) => f.id))

  const featureEnabled = (id: string): boolean => !known.has(id) || (build && !switchedOff.has(id))
  const allowed = (map: Map<string, string>, id: string): boolean => {
    const f = map.get(id)
    return f === undefined || featureEnabled(f)
  }
  return {
    build,
    features: () => (build ? features : []),
    featureEnabled,
    toolAllowed: (name) => allowed(toolOwner, name),
    actionAllowed: (id) => allowed(actionOwner, id),
    chipAllowed: (id) => allowed(chipOwner, id),
    behaviorEnabled: (id) => allowed(behaviorOwner, id),
    isPreviewTool: (name) => build && toolOwner.has(name),
    isPreviewAction: (id) => build && actionOwner.has(id),
  }
}
