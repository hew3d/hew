/**
 * "Check sources for new and changed items" (Settings ▸ Folders): whether the
 * Library looks at the sources you added when it opens, and says what is new
 * or changed there (docs/design/shared-library-sources.md 7.4, 7.5). On by
 * default. Stored like `libraryUpdates.ts`: localStorage, with a change
 * reaching every open window through the `storage` event.
 *
 * With this off nothing is checked by itself; pressing Refresh on a source
 * still checks it.
 */

import { useEffect, useState } from 'react'

const STORAGE_KEY = 'hew.settings.checkSources'
const CHANGE_EVENT = 'hew:check-sources'

/** Whether the Library checks sources when it opens (default true). */
export function getCheckSources(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) !== 'false'
  } catch {
    return true
  }
}

export function setCheckSources(on: boolean): void {
  try {
    localStorage.setItem(STORAGE_KEY, on ? 'true' : 'false')
  } catch {
    /* storage unavailable: the choice lasts until the window closes */
  }
  window.dispatchEvent(new Event(CHANGE_EVENT))
}

/** The setting, kept current across this window and the Settings window. */
export function useCheckSources(): boolean {
  const [on, setOn] = useState(getCheckSources)
  useEffect(() => {
    const read = (): void => setOn(getCheckSources())
    const onStorage = (e: StorageEvent): void => {
      if (e.key === null || e.key === STORAGE_KEY) read()
    }
    window.addEventListener(CHANGE_EVENT, read)
    window.addEventListener('storage', onStorage)
    return () => {
      window.removeEventListener(CHANGE_EVENT, read)
      window.removeEventListener('storage', onStorage)
    }
  }, [])
  return on
}
