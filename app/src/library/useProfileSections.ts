/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * useProfileSections — what the Fillet tool's profile chip lists below its
 * four basic kinds: the built-in profiles, the user's Library profiles, and
 * the "Save profile…" action. Loaded when the Fillet tool becomes active and
 * kept fresh by the library store's own change subscription.
 */

import { useEffect, useMemo, useState } from 'react'
import { libraryStore } from '../io/libraryStore'
import type { ProfileOptionItem, ProfileOptionSection } from '../viewport/FilletOptions'
import { readItemSummary } from './itemFiles'
import { buildLibraryItem } from './libraryModel'
import {
  bundledProfiles,
  profileShapeOfBytes,
  profileThumbnailUrl,
  shapeProfile,
  type BundledProfile,
} from './profiles'
import type { ProfileShapeJson } from '../tools/filletMath'

interface LibraryProfileRow {
  id: string
  name: string
  shape: ProfileShapeJson
}

/** The user's Library profile items (the `Profiles/` folder), by display
 *  name. A file that does not read as a profile is skipped. */
export async function listLibraryProfiles(): Promise<LibraryProfileRow[]> {
  const store = libraryStore()
  if (!store.available()) return []
  const rows: LibraryProfileRow[] = []
  for (const file of await store.list()) {
    // Only the Profiles folder is read: the chip must not open every item.
    if (!file.fileName.startsWith('Profiles/')) continue
    try {
      const bytes = await store.read(file.fileName)
      const parsed = await profileShapeOfBytes(bytes)
      if (parsed === null) continue
      const item = buildLibraryItem(file, await readItemSummary(bytes), '')
      rows.push({ id: item.meta.id ?? file.fileName, name: item.displayName, shape: parsed.shape })
    } catch {
      /* an unreadable file is the browser's concern, not the chip's */
    }
  }
  return rows.sort((a, b) => a.name.localeCompare(b.name))
}

/** The tooltip on a disabled "Save profile…": what makes it available. */
export const SAVE_PROFILE_HINT = 'Select one open chain of lines and arcs, or pick a built-in profile, to save it.'

export interface ProfileSectionsOptions {
  /** Load only while the Fillet tool is active. */
  active: boolean
  /** "Save profile…" is enabled (a drawn chain is selected, or the current
   *  profile is a built-in whose copy can be saved). */
  canSave: boolean
  onSave(): void
}

export function useProfileSections({ active, canSave, onSave }: ProfileSectionsOptions): ProfileOptionSection[] {
  const [builtIns, setBuiltIns] = useState<BundledProfile[]>([])
  const [mine, setMine] = useState<LibraryProfileRow[]>([])
  const [thumbs, setThumbs] = useState<Record<string, string>>({})

  useEffect(() => {
    if (!active) return
    let cancelled = false
    let latest = 0
    const load = async (): Promise<void> => {
      const mine = ++latest
      try {
        const [b, m] = await Promise.all([bundledProfiles(), listLibraryProfiles()])
        // A newer reload supersedes this one (the store notifies several
        // times during one write).
        if (cancelled || mine !== latest) return
        setBuiltIns(b)
        setMine(m)
      } catch {
        /* the chip still offers the four basic kinds */
      }
    }
    void load()
    const unsubscribe = libraryStore().subscribe(() => {
      void load()
    })
    return () => {
      cancelled = true
      unsubscribe()
    }
  }, [active])

  useEffect(() => {
    let cancelled = false
    const want: Array<{ key: string; shape: ProfileShapeJson }> = [
      ...builtIns.map((p) => ({ key: p.id, shape: p.shape })),
      ...mine.map((p) => ({ key: p.id, shape: p.shape })),
    ]
    for (const w of want) {
      // Keyed by shape too: a replaced profile keeps its id but not its picture.
      const key = `${w.key}|${JSON.stringify(w.shape)}`
      if (thumbs[key] !== undefined) continue
      void profileThumbnailUrl(w.shape, 64)
        .then((url) => {
          if (!cancelled) setThumbs((t) => (t[key] === undefined ? { ...t, [key]: url } : t))
        })
        .catch(() => {
          /* an odd shape just has no thumbnail */
        })
    }
    return () => {
      cancelled = true
    }
  }, [builtIns, mine, thumbs])

  return useMemo((): ProfileOptionSection[] => {
    const row = (p: { id: string; name: string; shape: ProfileShapeJson }, badge?: string): ProfileOptionItem => ({
      id: p.id,
      label: p.name,
      thumbnail: thumbs[`${p.id}|${JSON.stringify(p.shape)}`],
      badge,
      profile: shapeProfile(p.id, p.name, p.shape),
    })
    return [
      { id: 'builtin', heading: 'Built-in', items: builtIns.map((p) => row(p, 'Built-in')) },
      { id: 'library', heading: 'My profiles', items: mine.map((p) => row(p)), emptyHint: 'No saved profiles yet.' },
      {
        id: 'actions',
        items: [
          {
            id: 'save-profile',
            label: 'Save profile…',
            disabled: !canSave,
            title: canSave ? undefined : SAVE_PROFILE_HINT,
            onSelect: onSave,
          },
        ],
      },
    ]
  }, [builtIns, mine, thumbs, canSave, onSave])
}
