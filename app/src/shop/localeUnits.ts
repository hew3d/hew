/**
 * Shop-boot locale seed for the length-unit format (design_handoff_shop_mode/
 * README.md "Decisions": "Default from locale (US/LR/MM -> Architectural,
 * else Meters), overridable, persisted per device").
 *
 * `settings/units.ts` is shared with the full desktop editor and its own
 * default is (and must stay) plain Meters regardless of locale — that
 * module's behavior is byte-identical whether Shop Mode exists or not. This
 * file is the ONE place that reaches in and, ONCE, ON SHOP MODE'S OWN BOOT
 * ONLY, writes an Architectural seed for a phone whose locale implies
 * imperial and that has never had a unit format persisted before (checked
 * via `hasPersistedLengthUnit()` — never overwrites an explicit choice, the
 * editor's own default, OR an earlier seed). Every other locale is left
 * alone: units.ts's own 'm' default already matches what the design wants
 * for them, so there's nothing to seed.
 */
import { hasPersistedLengthUnit, setLengthUnit } from '../settings/units'

/** ISO 3166-1 alpha-2 regions that use imperial/US customary units for
 *  everyday measurement — the design's explicit "US/LR/MM" list (United
 *  States, Liberia, Myanmar; every other country is metric-first). */
const IMPERIAL_REGIONS = new Set(['US', 'LR', 'MM'])

/**
 * Extract the region subtag from a BCP-47 locale string ("en-US" -> "US"),
 * or `null` if none is present/parseable. `Intl.Locale` (widely supported
 * in the browsers Shop Mode targets) does the real parsing; a manual
 * fallback covers environments where the constructor throws (malformed
 * input) or isn't available at all, so a locale string missing a region
 * ("en") or carrying no locale at all never seeds imperial by accident —
 * only an explicit, well-formed US/LR/MM region does.
 */
export function regionFromLocale(locale: string | undefined | null): string | null {
  if (locale === undefined || locale === null || locale === '') return null
  if (typeof Intl !== 'undefined' && typeof Intl.Locale === 'function') {
    try {
      // Deliberately NOT `.maximize()`: that fills in a CLDR "likely
      // subtag" region for a bare language tag (`new
      // Intl.Locale('en').maximize().region` is 'US', not undefined) — a
      // guess this function must never make. Only an EXPLICIT region
      // subtag in the input locale string should ever resolve here.
      const region = new Intl.Locale(locale).region
      return region ?? null
    } catch {
      // Fall through to the manual parse below.
    }
  }
  // Manual BCP-47 fallback: the first 2-letter, all-caps subtag after the
  // primary language ("en-US" -> "US"). Region subtags are the only ones
  // that are exactly 2 letters and conventionally uppercase (script
  // subtags are 4 letters — "Hans" — and variant/extension subtags are
  // never bare 2-letter uppercase), so this is unambiguous without a full
  // BCP-47 grammar.
  const subtags = locale.split('-')
  for (const tag of subtags.slice(1)) {
    if (/^[A-Z]{2}$/.test(tag)) return tag
  }
  return null
}

/**
 * Seed the Architectural format once, at Shop Mode boot, if (a) this device
 * has never had a length format persisted, and (b) `locale` resolves to a
 * US/LR/MM region. A no-op in every other case — including every later
 * call, once the first seed (or any explicit user choice) has persisted
 * something. `locale` defaults to `navigator.language`; parameterized so
 * this stays a pure, unit-testable function rather than reading `navigator`
 * directly in the common case.
 */
export function seedLocaleLengthUnit(locale: string | undefined = typeof navigator === 'undefined' ? undefined : navigator.language): void {
  if (hasPersistedLengthUnit()) return
  const region = regionFromLocale(locale)
  if (region !== null && IMPERIAL_REGIONS.has(region)) {
    setLengthUnit('arch')
  }
}
