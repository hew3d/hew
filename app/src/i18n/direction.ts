/**
 * Text direction — which locales read right-to-left, and the one place that
 * puts that on the document (docs/LOCALIZATION.md "Right-to-left").
 *
 * Direction is a property of the active *locale*, not of the user: Arabic
 * (`ar`) mirrors the whole interface, English keeps it left-to-right. The
 * document's `dir` and `lang` attributes are set together by
 * `applyDocumentDirection`, which `i18n/activate.ts` calls for every
 * activation, so a language switch mirrors (or un-mirrors) the UI in the
 * same commit that swaps the catalog — nothing reloads and the 3D session is
 * untouched.
 *
 * `dir` on `<html>` is what mirrors the layout: flex rows reverse, `start`
 * and `end` swap, scrollbars move. Physical CSS is what would break, which
 * is why the app's own styles use logical properties (`inset-inline-start`,
 * `margin-inline`, …) and the few unavoidable physical cases carry a
 * `[dir='rtl']` override.
 *
 * The module-level current direction (rather than reading the DOM on every
 * render) is what `useTextDirection()` subscribes to, so a component that
 * flips an icon or inverts a drag delta re-renders on the switch. Layout
 * itself never needs it — CSS handles that.
 */

import { useSyncExternalStore } from 'react'

export type TextDirection = 'ltr' | 'rtl'

/**
 * Languages whose native script runs right-to-left. Keyed by primary
 * subtag, so `ar-EG`, `ar-SA`, and a bare `ar` all resolve. Persian,
 * Hebrew, and Urdu are listed so adding one of their catalogs needs no
 * change here; the shipped set is `SHIPPED_LOCALES` in `./locales`.
 */
const RTL_LANGUAGES: ReadonlySet<string> = new Set(['ar', 'fa', 'he', 'ur'])

/** True when `locale`'s script is written right-to-left. */
export function isRtlLocale(locale: string): boolean {
  const primary = locale.split(/[-_]/)[0]?.toLowerCase() ?? ''
  return RTL_LANGUAGES.has(primary)
}

/** The direction `locale` lays its interface out in. */
export function directionForLocale(locale: string): TextDirection {
  return isRtlLocale(locale) ? 'rtl' : 'ltr'
}

let current: TextDirection = 'ltr'
const listeners = new Set<() => void>()

/**
 * Put `locale`'s language and direction on the document: `lang` drives
 * hyphenation, font selection, and screen-reader pronunciation; `dir`
 * mirrors the layout. No DOM → no-op (the catalog tests run under jsdom,
 * the scripts under Node).
 */
export function applyDocumentDirection(locale: string): void {
  const next = directionForLocale(locale)
  if (typeof document !== 'undefined') {
    const root = document.documentElement
    root.lang = locale
    root.dir = next
  }
  if (next === current) return
  current = next
  for (const listener of listeners) listener()
}

/** The direction the app is currently laid out in. */
export function textDirection(): TextDirection {
  return current
}

/** True when the app is currently mirrored (an RTL locale is active). */
export function isRtl(): boolean {
  return current === 'rtl'
}

/**
 * Signed multiplier that turns a physical horizontal delta (a pointer's
 * `clientX` movement, a bounding rect's `left`) into a leading-edge delta —
 * `1` reading left-to-right, `-1` mirrored. Drag handles that resize a
 * region from its physical edge (the tray) multiply by this instead of
 * branching on direction.
 */
export function horizontalSign(): 1 | -1 {
  return current === 'rtl' ? -1 : 1
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

function getSnapshot(): TextDirection {
  return current
}

/** React binding for the current direction. Re-renders on a language switch. */
export function useTextDirection(): TextDirection {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot)
}

/** True while the app is mirrored; the React reading of `isRtl()`. */
export function useIsRtl(): boolean {
  return useTextDirection() === 'rtl'
}
