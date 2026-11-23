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
import { Trans, useLingui } from '@lingui/react/macro'
import { libraryStore } from '../io/libraryStore'
import { setCheckLibraryUpdates, useCheckLibraryUpdates } from './libraryUpdates'
import { setCheckSources, useCheckSources } from './checkSources'

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
  const { t } = useLingui()
  const store = libraryStore()
  const available = store.available()
  const canChoose = store.capabilities().canChooseFolder
  const [path, setPath] = useState<string | null>(null)
  const [webMode, setWebMode] = useState<'browser' | 'folder' | null>(null)
  const checkUpdates = useCheckLibraryUpdates()
  const checkSources = useCheckSources()

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
      <SettingsRow label={t`Library folder:`} htmlFor="settings-library-folder-path">
        <div style={rowStyle}>
          <input
            id="settings-library-folder-path"
            type="text"
            readOnly
            value={available ? (path ?? '') : ''}
            placeholder={available ? '…' : t`Not available`}
            title={path ?? undefined}
            style={pathInputStyle}
          />
          <button
            type="button"
            style={buttonStyle(!available || !canChoose)}
            disabled={!available || !canChoose}
            onClick={handleChoose}
          >
            <Trans>Change…</Trans>
          </button>
        </div>
      </SettingsRow>

      <SettingsRow label="" alignTop>
        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', fontSize: '13px', fontFamily: 'var(--font-family-ui, system-ui, sans-serif)' }}>
          <input
            type="checkbox"
            data-testid="settings-check-library-updates"
            checked={checkUpdates}
            onChange={(e) => setCheckLibraryUpdates(e.target.checked)}
            style={{ accentColor: 'var(--accent-base, #5b8cff)', margin: 0 }}
          />
          <Trans>Check the Library for newer parts</Trans>
        </label>
      </SettingsRow>
      <SettingsNote>
        <Trans>
          When a part you inserted from the Library has a newer version there, Hew says so under the menu bar. It
          never changes anything until you choose to update. With this off, Hew stops checking by itself and never
          shows that note, but it still checks when you open the Components panel or the Library, or save a part to
          the Library.
        </Trans>
      </SettingsNote>

      <SettingsRow label="" alignTop>
        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', fontSize: '13px', fontFamily: 'var(--font-family-ui, system-ui, sans-serif)' }}>
          <input
            type="checkbox"
            data-testid="settings-check-sources"
            checked={checkSources}
            onChange={(e) => setCheckSources(e.target.checked)}
            style={{ accentColor: 'var(--accent-base, #5b8cff)', margin: 0 }}
          />
          <Trans>Check sources for new and changed items</Trans>
        </label>
      </SettingsRow>
      <SettingsNote>
        <Trans>
          When you open the Library, Hew looks at the sources you added and says when they have new or changed
          items. It never changes anything until you choose to. Turn this off and Hew checks only when you press
          Refresh on a source.
        </Trans>
      </SettingsNote>

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
            <Trans>Use browser storage</Trans>
          </button>
        </SettingsRow>
      )}

      {!available ? (
        <SettingsNote>
          <Trans>The library is not available in this browser.</Trans>
        </SettingsNote>
      ) : webMode === 'browser' ? (
        <SettingsNote>
          {canChoose
            ? t`Items are stored by this browser on this device. Choose a folder to keep them as ordinary .hew files on disk instead — a cloud-synced folder makes the library follow you.`
            : isBrave()
              ? t`Items are stored by this browser on this device. Brave ships with the folder-picker API turned off — enable "File System Access API" at brave://flags to keep items as ordinary .hew files in a folder instead.`
              : t`Items are stored by this browser on this device. This browser doesn’t offer the folder-picker API needed to keep them on disk (Chrome and Edge do).`}
        </SettingsNote>
      ) : webMode === 'folder' ? (
        <SettingsNote>
          <Trans>
            The library is ordinary .hew files in the folder you picked. The browser may ask for
            permission to it again after a restart.
          </Trans>
        </SettingsNote>
      ) : null}
    </SettingsForm>
  )
}
