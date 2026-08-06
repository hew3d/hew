/**
 * FoldersPane — the "Folders" settings pane: the Library folder location.
 *
 * A single row: a read-only text input holding the configured library
 * folder's real absolute path (selectable, monospace, exactly what the
 * backend reports — no truncation trick that could visually corrupt it)
 * plus a "Change…" button that opens a native folder picker via the
 * storage seam (io/libraryStore.ts -> tauriLibraryStore.ts -> the
 * `library_choose_dir` Tauri command). The web build has no folder backend
 * yet (`libraryStore().available()` is false there) — the row renders with
 * an explanatory note instead of a broken control.
 *
 * Laid out on the shared macOS-HIG form grid (SettingsForm.tsx). Reusable
 * from both the standalone Settings window (Tauri) and the in-app modal
 * fallback (web). Changes apply instantly — no OK/Cancel.
 */

import { useEffect, useState, type CSSProperties } from 'react'
import { libraryStore } from '../io/libraryStore'
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
  const [path, setPath] = useState<string | null>(null)

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
            style={buttonStyle(!available)}
            disabled={!available}
            onClick={handleChoose}
          >
            Change…
          </button>
        </div>
      </SettingsRow>

      {!available ? <SettingsNote>Not available in the browser build yet.</SettingsNote> : null}
    </SettingsForm>
  )
}
