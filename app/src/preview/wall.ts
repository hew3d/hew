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
 *  - A feature's switch is on unless the user turned it off, except a feature
 *    marked `defaultOn: false`, which is off unless the user turned it on.
 *  - The loader kinds (`rootAllowed`, `paneAllowed`, `loaderEnabled`) are the
 *    other way round from the rest: they name code that exists only for a
 *    Preview feature, so an id the registry does NOT list is refused too.
 *    Nothing they guard is public, in any build.
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
  /** Whether the hash-selected window root `id` may render: a loader root of
   *  a feature that is on. False for an id no entry lists. */
  rootAllowed(id: string): boolean
  /** Whether the Settings pane `id` may be shown; as `rootAllowed`. */
  paneAllowed(id: string): boolean
  /** Whether code-loading feature `featureId` may start: it has a loader, the
   *  build is a preview build, and its switch is on. */
  loaderEnabled(featureId: string): boolean
}

/** The stored switches: features turned off, and default-off features turned on. */
export interface Switches {
  readonly off: ReadonlySet<string>
  readonly on: ReadonlySet<string>
}

export function createWall(
  build: boolean,
  features: readonly PreviewFeature[],
  switches: Switches,
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
  const rootOwner = owner((f) => f.loader?.roots ?? [])
  const paneOwner = owner((f) => f.loader?.panes ?? [])
  const known = new Map(features.map((f) => [f.id, f]))

  const switchOn = (f: PreviewFeature): boolean =>
    f.defaultOn === false ? switches.on.has(f.id) && !switches.off.has(f.id) : !switches.off.has(f.id)
  const featureEnabled = (id: string): boolean => {
    const f = known.get(id)
    return f === undefined || (build && switchOn(f))
  }
  const loaderEnabled = (id: string): boolean => {
    const f = known.get(id)
    return build && f !== undefined && f.loader !== undefined && switchOn(f)
  }
  const loaderAllowed = (map: Map<string, string>, id: string): boolean => {
    const f = map.get(id)
    return f !== undefined && loaderEnabled(f)
  }
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
    rootAllowed: (id) => loaderAllowed(rootOwner, id),
    paneAllowed: (id) => loaderAllowed(paneOwner, id),
    loaderEnabled,
  }
}
