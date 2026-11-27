/**
 * Desktop: the shell keeps the set of Library item names open for editing
 * (`library_edit_register` / `_unregister`; the shell drops a window's names
 * itself when the window is destroyed). A document window keeps it in step
 * with the item it edits, so a source update reports "open for editing"
 * (docs/design/shared-library-sources.md 7.3). The web has no `registerEdit`
 * (it uses a Web Lock), so there this does nothing.
 */
import { useEffect } from 'react'
import { libraryStore, type LibraryStore } from '../io/libraryStore'

type EditRegistry = Pick<LibraryStore, 'registerEdit' | 'unregisterEdit'>

/** The editing binding moved from item `prev` to item `next` (null: none).
 * Drops the old name, then registers the new one. Never throws: a shell
 * without the commands just does not report the edit. */
export async function syncEditRegistration(store: EditRegistry, prev: string | null, next: string | null): Promise<void> {
  if (prev === next) return
  try {
    if (prev !== null) await store.unregisterEdit?.(prev)
    if (next !== null) await store.registerEdit?.(next)
  } catch {
    /* an older shell, or the command failed: the edit still works */
  }
}

/** Keeps the shell's registry in step with the Library item this window edits
 * (`fileName` of its editing binding, null when none): registers it when the
 * binding appears, drops it when the binding changes or clears, and on unmount.
 * Every route that edits an item (Edit, Open on its path, crash recovery) ends
 * in that one binding, so this is the single registration point. */
export function useLibraryEditRegistration(fileName: string | null): void {
  useEffect(() => {
    if (fileName === null) return
    const store = libraryStore()
    void syncEditRegistration(store, null, fileName)
    return () => {
      void syncEditRegistration(libraryStore(), fileName, null)
    }
  }, [fileName])
}
