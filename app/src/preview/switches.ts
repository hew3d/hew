/**
 * Per-feature switches (Settings > Preview), preview builds only.
 *
 * Stored with the other settings under `hew.settings.preview` as two lists:
 * `off`, the features switched OFF, so a feature is on by default and one
 * added later is on for everyone; and `on`, the features switched ON among
 * those the registry marks `defaultOn: false` (a feature that runs other
 * people's code stays off until asked for). A production build never creates
 * the setting: it does not read the key, and a stored value there does
 * nothing.
 *
 * A switch applies the next time Hew starts. The wall is computed once, when
 * the app loads, because every surface it filters (rail, menus, palette,
 * keymap, dock, tool table) is built from it at that point; rebuilding all of
 * them live would be a second code path for the same filter.
 */

import { makeSetting, type ViewSetting } from '../settings/view'
import { previewBuild } from './build'
import type { Switches } from './wall'

/** The stored value. Both lists are sorted and free of duplicates. */
export interface StoredSwitches {
  readonly off: readonly string[]
  readonly on: readonly string[]
}

const NONE: StoredSwitches = { off: [], on: [] }

function idList(raw: unknown): readonly string[] | undefined {
  if (raw === undefined) return []
  if (!Array.isArray(raw) || !raw.every((id) => typeof id === 'string')) return undefined
  return [...new Set(raw as string[])].sort()
}

function parseStored(raw: unknown): StoredSwitches | undefined {
  let value: unknown = raw
  if (typeof raw === 'string') {
    try {
      value = JSON.parse(raw)
    } catch {
      return undefined
    }
  }
  if (value === null || typeof value !== 'object') return undefined
  const { off, on } = value as { off?: unknown; on?: unknown }
  const offList = idList(off)
  const onList = idList(on)
  if (offList === undefined || onList === undefined) return undefined
  return { off: offList, on: onList }
}

/* oxlint-disable lingui/no-unlocalized-strings -- storage and payload keys */
export const previewSwitches: ViewSetting<StoredSwitches> | null = previewBuild
  ? makeSetting<StoredSwitches>(
      'hew.settings.preview',
      'preview',
      NONE,
      parseStored,
      (stored) => JSON.stringify(stored.on.length > 0 ? stored : { off: stored.off }),
    )
  : null
/* oxlint-enable lingui/no-unlocalized-strings */

/** The switches as stored right now; nothing in production. */
export function switchesNow(): Switches {
  const stored = previewSwitches?.get() ?? NONE
  return { off: new Set(stored.off), on: new Set(stored.on) }
}

/** Whether `id`'s switch reads on in `stored`, given the feature's default. */
export function switchReadsOn(stored: StoredSwitches, id: string, defaultOn: boolean): boolean {
  return defaultOn ? !stored.off.includes(id) : stored.on.includes(id) && !stored.off.includes(id)
}

/** `stored` with `id`'s switch set to `on`. */
export function withSwitch(stored: StoredSwitches, id: string, on: boolean, defaultOn: boolean): StoredSwitches {
  const off = stored.off.filter((x) => x !== id)
  const onList = stored.on.filter((x) => x !== id)
  if (defaultOn) return { off: on ? off : [...off, id].sort(), on: onList }
  return { off, on: on ? [...onList, id].sort() : onList }
}
