/**
 * Language setting — module-level singleton (docs/LOCALIZATION.md).
 *
 * Owns ONLY the persisted preference ('system' or a locale tag) and its
 * cross-window sync, plus the pure resolution from preference to the locale
 * the app runs in. Activating a catalog is `i18n/activate.ts`'s job, which
 * subscribes here.
 *
 * Persistence + cross-window sync mirrors settings/units.ts exactly:
 *   - localStorage under `hew.settings.language`.
 *   - Under Tauri, the main and Settings webviews do not share a 'storage'
 *     event, so changes are ALSO broadcast on the shared 'settings-changed'
 *     Tauri global event with their own payload key (`language`).
 *   - The browser 'storage' event covers same-origin web tabs.
 *
 * The default is 'system': the OS/browser language list resolved against
 * the SHIPPED locales, English otherwise. An explicit choice is never
 * overridden by the system locale.
 */

import { isTauri } from '../io/fileHost'
import { PSEUDO_LOCALE, SHIPPED_LOCALES, SOURCE_LOCALE, isLocale, type Locale } from './locales'

export const SYSTEM_LANGUAGE = 'system'

/** The persisted preference: follow the system, or one specific locale. */
export type LanguageSetting = typeof SYSTEM_LANGUAGE | Locale

const STORAGE_KEY = 'hew.settings.language'
const DEFAULT_SETTING: LanguageSetting = SYSTEM_LANGUAGE

function isLanguageSetting(v: unknown): v is LanguageSetting {
  return v === SYSTEM_LANGUAGE || isLocale(v)
}

function loadInitial(): LanguageSetting {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (isLanguageSetting(raw)) return raw
  } catch {
    /* ignore — privacy mode / unavailable storage */
  }
  return DEFAULT_SETTING
}

let currentSetting: LanguageSetting = loadInitial()
const subscribers = new Set<(setting: LanguageSetting) => void>()

function notify(): void {
  for (const cb of subscribers) cb(currentSetting)
}

/** Read the persisted preference ('system' or a locale tag). */
export function getLanguageSetting(): LanguageSetting {
  return currentSetting
}

/**
 * Set the preference. Persists to localStorage, notifies local subscribers,
 * and broadcasts to other windows. Storage failures are best-effort: the
 * in-memory selection still applies for this session.
 */
export function setLanguageSetting(setting: LanguageSetting): void {
  currentSetting = setting
  try {
    localStorage.setItem(STORAGE_KEY, setting)
  } catch {
    /* ignore quota / privacy-mode errors */
  }
  notify()
  broadcastTauri(setting)
}

/** Subscribe to preference changes (local + cross-window). Returns an unsubscribe fn. */
export function subscribe(cb: (setting: LanguageSetting) => void): () => void {
  subscribers.add(cb)
  return () => subscribers.delete(cb)
}

// ---------------------------------------------------------------------------
// Resolution: preference → the locale the app runs in
// ---------------------------------------------------------------------------

/** The primary language subtag of a BCP 47 tag: "ja-JP" → "ja", "zh-Hant-TW"
 * → "zh". Case-insensitive, as tags are. */
function primaryLanguage(tag: string): string {
  return tag.split('-')[0]?.toLowerCase() ?? ''
}

/**
 * Pure: pick the locale to run in.
 *
 * - An explicit shipped locale wins outright.
 * - The pseudo-locale is honored only while Debug Mode is on; otherwise it
 *   resolves as 'system' so a stale persisted choice never leaves a user in
 *   a bracketed, padded UI.
 * - An explicit but unshipped locale (a catalog that exists on disk but is
 *   not offered yet, or one removed in an update) resolves as 'system'.
 * - 'system' walks the preferred-language list in order and returns the
 *   first shipped locale that matches exactly or by primary language
 *   ("ja-JP" matches "ja"), then falls back to the source locale.
 */
export function resolveLocale(
  setting: LanguageSetting,
  preferred: readonly string[],
  opts: { debugMode: boolean; shipped?: readonly string[] } = { debugMode: false },
): Locale {
  // `shipped` is injectable for tests; every entry must be a real Locale.
  const shipped: readonly Locale[] = (opts.shipped ?? SHIPPED_LOCALES).filter(isLocale)
  if (setting === PSEUDO_LOCALE) {
    if (opts.debugMode) return PSEUDO_LOCALE
  } else if (setting !== SYSTEM_LANGUAGE && shipped.includes(setting)) {
    return setting
  }
  for (const tag of preferred) {
    const lower = tag.toLowerCase()
    const exact = shipped.find((l) => l.toLowerCase() === lower)
    if (exact !== undefined) return exact
    const lang = primaryLanguage(tag)
    const byLanguage = shipped.find((l) => primaryLanguage(l) === lang)
    if (byLanguage !== undefined) return byLanguage
  }
  return SOURCE_LOCALE
}

/** The browser's/OS's preferred languages, most preferred first. */
export function systemLanguages(): readonly string[] {
  if (typeof navigator === 'undefined') return []
  const list = navigator.languages
  if (list !== undefined && list.length > 0) return list
  return navigator.language ? [navigator.language] : []
}

// ---------------------------------------------------------------------------
// Cross-window sync
// ---------------------------------------------------------------------------

let tauriEmit: ((event: string, payload?: unknown) => Promise<void>) | null = null

function broadcastTauri(setting: LanguageSetting): void {
  if (!isTauri) return
  if (tauriEmit !== null) {
    tauriEmit('settings-changed', { language: setting }).catch(() => { /* ignore */ })
    return
  }
  import('@tauri-apps/api/event').then(({ emit }) => {
    tauriEmit = emit
    return emit('settings-changed', { language: setting })
  }).catch(() => { /* ignore */ })
}

function applyExternal(next: unknown): void {
  if (!isLanguageSetting(next) || next === currentSetting) return
  currentSetting = next
  notify()
}

if (typeof window !== 'undefined') {
  window.addEventListener('storage', (ev) => {
    if (ev.key !== STORAGE_KEY) return
    applyExternal(ev.newValue)
  })

  if (isTauri) {
    import('@tauri-apps/api/event').then(({ listen }) => {
      return listen<{ language?: unknown }>('settings-changed', (event) => {
        applyExternal(event.payload?.language)
      })
    }).catch(() => { /* ignore */ })
  }
}
