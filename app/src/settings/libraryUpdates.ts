/**
 * "Check the Library for newer parts" (Settings ▸ Folders): whether the
 * model looks at the Library for newer versions of the parts it uses, and
 * says so. On by default. Persisted to localStorage like the rest of
 * settings/*; the Settings window and the document window share the origin,
 * so a change reaches an open document through the `storage` event.
 */

import { useEffect, useState } from 'react'

const STORAGE_KEY = 'hew.settings.checkLibraryUpdates'
const CHANGE_EVENT = 'hew:check-library-updates'

/** Whether the check runs (default true). */
export function getCheckLibraryUpdates(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) !== 'false'
  } catch {
    return true
  }
}

export function setCheckLibraryUpdates(on: boolean): void {
  try {
    localStorage.setItem(STORAGE_KEY, on ? 'true' : 'false')
  } catch {
    /* storage unavailable: the choice lasts until the window closes */
  }
  window.dispatchEvent(new Event(CHANGE_EVENT))
}

/** The setting, kept current across this window and the Settings window. */
export function useCheckLibraryUpdates(): boolean {
  const [on, setOn] = useState(getCheckLibraryUpdates)
  useEffect(() => {
    const read = (): void => setOn(getCheckLibraryUpdates())
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
