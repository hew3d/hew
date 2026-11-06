/**
 * Catalog activation — loads a locale's compiled messages and switches the
 * Lingui runtime (and the document's `lang`) to it.
 *
 * `startI18n()` runs once per window before the React root renders (see
 * main.tsx) so the first paint is already in the resolved language, then
 * keeps the runtime following the Language setting and Debug Mode for the
 * life of the window. Switching re-renders through `I18nProvider`; nothing
 * reloads and the model is untouched.
 *
 * The English catalog is imported statically: it is the fallback for
 * everything, production builds strip message text from the code, and a
 * first paint must never depend on a second network fetch succeeding.
 * Every other locale is a `.po` imported on demand through
 * `@lingui/vite-plugin`, one chunk per locale, so a user who picks Japanese
 * downloads only the Japanese catalog.
 */

import { i18n } from '@lingui/core'
import { isTauri } from '../io/fileHost'
import { getDebugMode, subscribe as subscribeDebugMode } from '../settings/debugMode'
import { messages as englishMessages } from '../locales/en/messages.po'
import { SOURCE_LOCALE, type Locale } from './locales'
import { applyDocumentDirection } from './direction'
import { getLanguageSetting, resolveLocale, subscribe as subscribeLanguage, systemLanguages } from './language'

const loaded = new Set<Locale>()

/** Monotonic request id: only the newest activation request may activate,
 * so a slow catalog load that finishes after a later switch cannot leave the
 * runtime in a language the setting no longer names. */
let latestRequest = 0

function ensureEnglish(): void {
  if (loaded.has(SOURCE_LOCALE)) return
  i18n.load(SOURCE_LOCALE, englishMessages)
  loaded.add(SOURCE_LOCALE)
}

async function loadCatalog(locale: Locale): Promise<void> {
  if (locale === SOURCE_LOCALE) {
    ensureEnglish()
    return
  }
  if (loaded.has(locale)) return
  // The extension is mandatory: it is what routes the import through the
  // Lingui Vite plugin.
  const { messages } = await import(`../locales/${locale}/messages.po`)
  i18n.load(locale, messages)
  loaded.add(locale)
}

function apply(locale: Locale): void {
  i18n.activate(locale)
  // `lang` and `dir` move together: an RTL locale (Arabic) mirrors the whole
  // interface at this one point, and switching back un-mirrors it. The 3D
  // session is untouched — nothing reloads (i18n/direction.ts).
  applyDocumentDirection(locale)
  syncNativeMenu(locale)
}

/**
 * Load `locale`'s catalog (once) and make it the active one. English is
 * always available; a non-English catalog that fails to load falls back to
 * English rather than throwing into the render path. If a newer request
 * arrives while this one's catalog is still loading, this one does nothing
 * on completion.
 */
export async function activateLocale(locale: Locale): Promise<void> {
  const request = ++latestRequest
  let target = locale
  try {
    await loadCatalog(locale)
  } catch (err) {
    console.error(`[i18n] could not load catalog for ${locale}; using English`, err)
    ensureEnglish()
    target = SOURCE_LOCALE
  }
  if (request !== latestRequest) return
  apply(target)
}

/** The locale the current settings resolve to. */
export function currentResolvedLocale(): Locale {
  return resolveLocale(getLanguageSetting(), systemLanguages(), { debugMode: getDebugMode() })
}

/**
 * Activate the resolved locale, then follow every later change to the
 * Language setting or Debug Mode (which gates the pseudo-locale). The
 * subscriptions are in place before the first catalog is awaited, so a
 * change that lands during startup (another window's broadcast) is applied
 * too. Returns once an initial catalog is active; English is active at the
 * very least.
 */
export async function startI18n(): Promise<void> {
  const follow = (): void => {
    const next = currentResolvedLocale()
    if (next !== i18n.locale) void activateLocale(next)
  }
  subscribeLanguage(follow)
  subscribeDebugMode(follow)
  await activateLocale(currentResolvedLocale())
  // A change that arrived while the first catalog was loading was queued
  // behind it by the request counter; make sure the newest setting wins.
  follow()
}

/**
 * Desktop shell: relabel the native menu bar in place and remember the
 * locale for the next launch, so the menu is in the right language from
 * its first frame (docs/LOCALIZATION.md "Native menus"). The pseudo-locale
 * has no menu file; the shell falls back to English for it. Best-effort —
 * a failure here never blocks activation.
 */
function syncNativeMenu(locale: Locale): void {
  if (!isTauri) return
  import('@tauri-apps/api/core')
    .then(({ invoke }) => invoke('set_menu_locale', { locale }))
    .catch((err) => console.warn('[i18n] native menu relabel failed', err))
}
