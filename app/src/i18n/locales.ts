/**
 * The locale inventory — the one place that knows which languages exist.
 *
 * Dependency-free on purpose: `lingui.config.ts` (the extraction/compile
 * config, run by the Lingui CLI under Node) and `scripts/i18n-check.mjs`
 * (the verify-gate completeness check) import this file as well as the app,
 * so it must stay plain data with no DOM, React, or Lingui imports.
 *
 * Three tiers of locale:
 *   - SOURCE_LOCALE: the language the source code is written in. Every
 *     message's key IS its English text (docs/LOCALIZATION.md).
 *   - SHIPPED_LOCALES: offered in Settings ▸ Language. A locale moves here
 *     only when its catalog is complete — `scripts/i18n-check.mjs` fails
 *     the gate on any empty translation in a shipped catalog, so a user
 *     who picks a language never sees a mixed interface.
 *   - ALL_LOCALES: everything with a catalog on disk, shipped or not. A
 *     translator works on a catalog here before it is promoted.
 *
 * The pseudo-locale is generated from the English catalog at compile time
 * (`pseudoLocale` in lingui.config.ts) and is selectable only when Debug
 * Mode is on (`i18n/language.ts`); it is a test aid, never a shipped
 * language.
 */

export const SOURCE_LOCALE = 'en'

export const PSEUDO_LOCALE = 'pseudo-LOCALE'

/** Locales offered to users. Add a locale here only once its catalog is
 * complete (docs/LOCALIZATION.md "Adding a language"). */
export const SHIPPED_LOCALES = ['en', 'ar'] as const

/** Every locale with a catalog under `app/src/locales/`. */
export const ALL_LOCALES = ['en', 'ar', 'ja', PSEUDO_LOCALE] as const

export type ShippedLocale = (typeof SHIPPED_LOCALES)[number]
export type Locale = (typeof ALL_LOCALES)[number]

/**
 * Each language's name in its own script — the Settings list shows every
 * language the way its speakers write it, so a user who cannot read the
 * current UI language can still find their own. Never translated.
 */
/* oxlint-disable lingui/no-unlocalized-strings -- language names stay in their own script */
export const LOCALE_NAMES: Record<Locale, string> = {
  en: 'English',
  ar: 'العربية',
  ja: '日本語',
  [PSEUDO_LOCALE]: 'Pseudo (debug)',
}
/* oxlint-enable lingui/no-unlocalized-strings */

export function isLocale(v: unknown): v is Locale {
  return typeof v === 'string' && (ALL_LOCALES as readonly string[]).includes(v)
}

export function isShippedLocale(v: unknown): v is ShippedLocale {
  return typeof v === 'string' && (SHIPPED_LOCALES as readonly string[]).includes(v)
}
