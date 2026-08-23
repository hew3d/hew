/**
 * FoldersPane — the "Folders" settings pane: the Library folder location.
 *
 * A single row: a read-only text input holding the configured library
 * folder's real absolute path (selectable, monospace, exactly what the
 * backend reports — no truncation trick that could visually corrupt it)
 * plus a "Change…" button that opens a folder picker via the storage seam
 * (io/libraryStore.ts): the native dialog on desktop
 * (tauriLibraryStore.ts -> `library_choose_dir`), `showDirectoryPicker` on
 * Chromium-family browsers (webLibraryStore.ts). On the web the "path" is
 * a display label — "Browser storage", or the bound folder's name — and a
 * bound folder adds a "Use browser storage" way back. Browsers without a
 * directory picker (Firefox, Safari) show browser storage with the picker
 * disabled; a browser with no storage backend at all gets the explanatory
 * note instead of a broken control.
 *
 * Laid out on the shared macOS-HIG form grid (SettingsForm.tsx). Reusable
 * from both the standalone Settings window (Tauri) and the in-app modal
 * fallback (web). Changes apply instantly — no OK/Cancel.
 */

import { useEffect, useState, type CSSProperties } from 'react'
import { libraryStore } from '../io/libraryStore'

/** Brave with File System Access disabled (its default) simply doesn't
 * define showDirectoryPicker, but it does identify itself — so the note
 * can name the exact flag instead of a generic "not supported". */
function isBrave(): boolean {
  return typeof (navigator as { brave?: unknown }).brave !== 'undefined'
}
import { SettingsForm, SettingsRow, SettingsNote } from './SettingsForm'

const rowStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '10px',
  width: '340px',
  minWidth: 0,
}

const pathInputStyle: CSSProperties = {
  flex: 1,
  minWidth: 0,
  padding: '4px 8px',
  fontFamily: 'var(--font-family-mono, ui-monospace, SFMono-Regular, monospace)',
  fontSize: '12px',
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '6px',
}

function buttonStyle(disabled: boolean): CSSProperties {
  return {
    padding: '4px 12px',
    fontSize: '13px',
    fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
    background: 'var(--surface-input, #2a2a2a)',
    color: disabled ? 'var(--text-faint, #666)' : 'var(--text-primary, #eee)',
    border: '1px solid var(--border-strong, #444)',
    borderRadius: '6px',
    cursor: disabled ? 'default' : 'pointer',
    flexShrink: 0,
  }
}

export function FoldersPane() {
  const store = libraryStore()
  const available = store.available()
  const canChoose = store.capabilities().canChooseFolder
  const [path, setPath] = useState<string | null>(null)
  const [webMode, setWebMode] = useState<'browser' | 'folder' | null>(null)

  useEffect(() => {
    if (!available) return
    let cancelled = false
    const refresh = (): void => {
      store
        .folderInfo()
        .then(({ path: p }) => {
          if (!cancelled) setPath(p)
        })
        .catch(() => {
          /* best-effort — leave the last known path displayed */
        })
      void store
        .webStorage?.()
        .then(({ mode }) => {
          if (!cancelled) setWebMode(mode)
        })
        .catch(() => {})
    }
    refresh()
    const unsubscribe = store.subscribe(refresh)
    return () => {
      cancelled = true
      unsubscribe()
    }
    // `store` is the module-level singleton libraryStore() always returns —
    // its identity is stable, so it's deliberately not in the dep array.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [available])

  async function handleChoose(): Promise<void> {
    if (!available) return
    try {
      const next = await store.chooseFolder()
      if (next !== null) setPath(next)
    } catch {
      /* cancelled or unsupported — leave the current path displayed */
    }
  }

  return (
    <SettingsForm>
      <SettingsRow label="Library folder:" htmlFor="settings-library-folder-path">
        <div style={rowStyle}>
          <input
            id="settings-library-folder-path"
            type="text"
            readOnly
            value={available ? (path ?? '') : ''}
            placeholder={available ? '…' : 'Not available'}
            title={path ?? undefined}
            style={pathInputStyle}
          />
          <button
            type="button"
            style={buttonStyle(!available || !canChoose)}
            disabled={!available || !canChoose}
            onClick={handleChoose}
          >
            Change…
          </button>
        </div>
      </SettingsRow>

      {/* Web only, on its OWN row — inline it crowded the path out of its
          box (playtest). Always visible so the way back is discoverable;
          disabled while browser storage is already active. */}
      {webMode !== null && (
        <SettingsRow label="">
          <button
            type="button"
            style={{ ...buttonStyle(webMode === 'browser'), alignSelf: 'flex-start' }}
            disabled={webMode === 'browser'}
            onClick={() => {
              void store.useBrowserStorage?.()
              setWebMode('browser')
            }}
          >
            Use browser storage
          </button>
        </SettingsRow>
      )}

      {!available ? (
        <SettingsNote>The library is not available in this browser.</SettingsNote>
      ) : webMode === 'browser' ? (
        <SettingsNote>
          {canChoose
            ? 'Items are stored by this browser on this device. Choose a folder to keep them as ordinary .hew files on disk instead — a cloud-synced folder makes the library follow you.'
            : isBrave()
              ? 'Items are stored by this browser on this device. Brave ships with the folder-picker API turned off — enable "File System Access API" at brave://flags to keep items as ordinary .hew files in a folder instead.'
              : 'Items are stored by this browser on this device. This browser doesn\u2019t offer the folder-picker API needed to keep them on disk (Chrome and Edge do).'}
        </SettingsNote>
      ) : webMode === 'folder' ? (
        <SettingsNote>
          The library is ordinary .hew files in the folder you picked. The browser may ask for
          permission to it again after a restart.
        </SettingsNote>
      ) : null}
    </SettingsForm>
  )
}
