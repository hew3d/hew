/**
 * Per-feature switches (Settings > Preview), preview builds only.
 *
 * Stored with the other settings under `hew.settings.preview` as the list of
 * features switched OFF, so every feature is on by default and a feature
 * added later is on for everyone. A production build never creates the
 * setting: it does not read the key, and a stored value there does nothing.
 *
 * A switch applies the next time Hew starts. The wall is computed once, when
 * the app loads, because every surface it filters (rail, menus, palette,
 * keymap, dock, tool table) is built from it at that point; rebuilding all of
 * them live would be a second code path for the same filter.
 */

import { makeSetting, type ViewSetting } from '../settings/view'
import { previewBuild } from './build'

function parseOff(raw: unknown): readonly string[] | undefined {
  let value: unknown = raw
  if (typeof raw === 'string') {
    try {
      value = JSON.parse(raw)
    } catch {
      return undefined
    }
  }
  if (value === null || typeof value !== 'object') return undefined
  const off = (value as { off?: unknown }).off
  if (!Array.isArray(off) || !off.every((id) => typeof id === 'string')) return undefined
  return off as string[]
}

/* oxlint-disable lingui/no-unlocalized-strings -- storage and payload keys */
export const previewSwitches: ViewSetting<readonly string[]> | null = previewBuild
  ? makeSetting<readonly string[]>(
      'hew.settings.preview',
      'preview',
      [],
      parseOff,
      (off) => JSON.stringify({ off }),
    )
  : null
/* oxlint-enable lingui/no-unlocalized-strings */

/** The features switched off right now (stored value); empty in production. */
export function switchedOffNow(): ReadonlySet<string> {
  return new Set(previewSwitches?.get() ?? [])
}
