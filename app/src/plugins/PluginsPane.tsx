/* oxlint-disable lingui/no-unlocalized-strings -- plugin install copy is English only until the feature graduates, and stays out of the catalogs. */
/**
 * PluginsPane — Settings ▸ Plugins: install add-ons made by other people,
 * turn them on and off, go back to the version before, and remove them.
 * Reached only while the Plugins preview switch is on (`paneAllowed`).
 *
 * Every change goes through the installer (`installer.ts`), which is the
 * desktop shell's commands or the browser store; the pane never touches
 * either. Installing always shows the consent sheet first. Anything a plugin
 * wrote (its name, its path) is cleaned before it is shown.
 */

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import { ConsentSheet } from './ConsentSheet'
import { pluginInstaller } from './installer'
import { errorText, toPluginError } from './pluginError'
import { permissionText } from './permissionCopy'
import { cleanText, displayName } from './pluginText'
import { buttonStyle } from './styles'
import type { InstallSource, Listed, PendingView, PluginInstaller } from './types'

const noteStyle: CSSProperties = { fontSize: '11px', lineHeight: 1.5, color: 'var(--text-faint, #888)', maxWidth: '36em' }

const cardStyle: CSSProperties = {
  display: 'flex',
  gap: '12px',
  alignItems: 'flex-start',
  padding: '10px 12px',
  border: '1px solid var(--border-hairline, rgba(128,128,128,0.25))',
  borderRadius: '8px',
  background: 'var(--surface-bar, #202020)',
}

/** The stand-in when a plugin has no icon. */
function GenericGlyph() {
  return (
    <svg
      width={32}
      height={32}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      style={{ color: 'var(--text-faint, #888)', flexShrink: 0 }}
    >
      <path d="M9.5 4.5a2 2 0 0 1 4 0V6h3.5v3.5h1.5a2 2 0 0 1 0 4H17V17h-3.5v-1.5a2 2 0 0 0-4 0V17H6v-3.5H4.5a2 2 0 0 1 0-4H6V6h3.5z" />
    </svg>
  )
}

function sourceLine(source: InstallSource): string {
  if (source.kind === 'file') return 'From file'
  if (source.kind === 'dir') return `Unpacked: ${cleanText(source.path, 512)}`
  return `From ${source.kind}`
}

export function PluginsPane({ installer }: { installer?: PluginInstaller }) {
  const inst = useMemo(() => installer ?? pluginInstaller(), [installer])
  const [items, setItems] = useState<Listed[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [menuOpen, setMenuOpen] = useState(false)
  const [pending, setPending] = useState<PendingView | null>(null)
  const [sheetBusy, setSheetBusy] = useState(false)
  const [sheetError, setSheetError] = useState<string | null>(null)
  const [confirming, setConfirming] = useState<string | null>(null)
  const [rollingBack, setRollingBack] = useState<{ id: string; version: string; lines: string[] } | null>(null)
  const [icons, setIcons] = useState<Record<string, string>>({})
  const iconsRef = useRef<Record<string, string>>({})
  const fileInput = useRef<HTMLInputElement>(null)

  const refresh = useCallback(async (): Promise<void> => {
    try {
      setItems(await inst.list())
    } catch (e) {
      setError(errorText(e))
      setItems((cur) => cur ?? [])
    }
  }, [inst])

  useEffect(() => {
    void refresh()
    return inst.onChanged(() => void refresh())
  }, [inst, refresh])

  // Icons are blob URLs the pane makes and revokes; they go only into <img>.
  useEffect(() => {
    if (items === null) return
    let cancelled = false
    void (async () => {
      const next: Record<string, string> = {}
      for (const l of items) {
        if (l.kind !== 'installed' || typeof l.manifest.icon !== 'string' || l.manifest.icon === '') continue
        const url = await inst.iconUrl(l.installed.id, l.manifest.icon)
        if (url !== null) next[l.installed.id] = url
      }
      if (cancelled) {
        Object.values(next).forEach((u) => URL.revokeObjectURL(u))
        return
      }
      Object.values(iconsRef.current).forEach((u) => URL.revokeObjectURL(u))
      iconsRef.current = next
      setIcons(next)
    })()
    return () => {
      cancelled = true
    }
  }, [items, inst])
  useEffect(
    () => () => {
      Object.values(iconsRef.current).forEach((u) => URL.revokeObjectURL(u))
    },
    [],
  )

  const nameOf = (id: string): string => {
    const found = items?.find((l) => l.kind === 'installed' && l.installed.id === id)
    return found?.kind === 'installed' ? displayName(found.manifest) : id
  }

  const stage = async (run: () => Promise<PendingView | null>): Promise<void> => {
    setError(null)
    setNotice(null)
    setMenuOpen(false)
    setBusy(true)
    try {
      const view = await run()
      if (view !== null) {
        setSheetError(null)
        setPending(view)
      }
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const install = async (): Promise<void> => {
    if (pending === null) return
    setSheetBusy(true)
    setSheetError(null)
    try {
      const record = await inst.commit(pending.token, true)
      setPending(null)
      // The desktop asks once more in a native dialog; a "no" there installs nothing.
      if (record === null) setNotice('Nothing was installed.')
      await refresh()
    } catch (e) {
      setSheetError(errorText(e))
    } finally {
      setSheetBusy(false)
    }
  }

  const cancel = (): void => {
    const token = pending?.token
    setPending(null)
    setSheetError(null)
    if (token !== undefined) inst.commit(token, false).catch(() => undefined)
  }

  const run = async (work: () => Promise<void>): Promise<void> => {
    setError(null)
    setNotice(null)
    try {
      await work()
    } catch (e) {
      // A "no" in the shell's own confirmation is an answer, not a failure.
      if (toPluginError(e).refusal !== 'rollback_declined') setError(errorText(e))
    }
    await refresh()
  }

  const toggle = (id: string, enabled: boolean): Promise<void> =>
    run(async () => {
      const also = await inst.setEnabled(id, enabled)
      if (also.length > 0) {
        setNotice(`Turning off ${nameOf(id)} also turned off ${also.map(nameOf).join(', ')}, because they need it.`)
      }
    })

  // On the web the pane asks first and lists what the earlier version can do;
  // on the desktop the shell asks in its own dialog.
  const askRollback = (id: string): Promise<void> =>
    run(async () => {
      if (inst.previous === undefined) {
        await inst.rollback(id)
        return
      }
      const prev = await inst.previous(id)
      setRollingBack({
        id,
        version: prev.installed.version,
        lines: (prev.manifest.permissions ?? []).map((p) => permissionText(p)),
      })
    })

  const remove = (id: string): Promise<void> =>
    run(async () => {
      setConfirming(null)
      await inst.uninstall(id)
    })

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '12px', maxWidth: '44em' }}>
      <div style={noteStyle}>
        Plugins are add-ons made by other people. You can install and manage them here, but they do not run yet in
        this build.
      </div>

      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', position: 'relative' }}>
        {inst.tier === 'web' ? (
          <>
            <button type="button" disabled={busy} style={buttonStyle(busy)} onClick={() => fileInput.current?.click()}>
              Install…
            </button>
            <input
              ref={fileInput}
              type="file"
              accept=".hewplugin"
              aria-label="Plugin package"
              style={{ display: 'none' }}
              onChange={(e) => {
                const file = e.target.files?.[0]
                e.target.value = ''
                if (file !== undefined) void stage(() => inst.stageFile(file))
              }}
            />
          </>
        ) : (
          <>
            <button
              type="button"
              disabled={busy}
              aria-haspopup="true"
              aria-expanded={menuOpen}
              style={buttonStyle(busy)}
              onClick={() => setMenuOpen((o) => !o)}
            >
              Install…
            </button>
            {menuOpen && (
              <>
                <button type="button" style={buttonStyle()} onClick={() => void stage(() => inst.pickAndStage('file'))}>
                  From file…
                </button>
                <button type="button" style={buttonStyle()} onClick={() => void stage(() => inst.pickAndStage('dir'))}>
                  Unpacked folder…
                </button>
              </>
            )}
          </>
        )}
      </div>

      {error !== null && (
        <div role="alert" style={{ color: 'var(--danger-base, #e0574f)' }}>
          {error}
        </div>
      )}
      {notice !== null && <div role="status">{notice}</div>}

      {items === null ? (
        <div style={noteStyle}>Loading…</div>
      ) : items.length === 0 ? (
        <div style={noteStyle}>No plugins are installed.</div>
      ) : (
        <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: '8px' }}>
          {items.map((l) =>
            l.kind === 'unreadable' ? (
              <li key={l.id} data-plugin-row={l.id} style={cardStyle}>
                <GenericGlyph />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ color: 'var(--text-primary, #eee)', overflowWrap: 'anywhere' }}>{l.id}</div>
                  <div style={noteStyle}>
                    Hew cannot read this plugin ({cleanText(l.reason, 200)}). It is left as it is. You can remove it.
                  </div>
                </div>
                <RemoveControls
                  id={l.id}
                  label={l.id}
                  confirming={confirming === l.id}
                  onAsk={() => setConfirming(l.id)}
                  onKeep={() => setConfirming(null)}
                  onRemove={() => void remove(l.id)}
                />
              </li>
            ) : (
              <li key={l.installed.id} data-plugin-row={l.installed.id} style={cardStyle}>
                {icons[l.installed.id] !== undefined ? (
                  <img src={icons[l.installed.id]} alt="" width={32} height={32} style={{ flexShrink: 0, objectFit: 'contain' }} />
                ) : (
                  <GenericGlyph />
                )}
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', alignItems: 'baseline', gap: '8px', flexWrap: 'wrap' }}>
                    <span style={{ color: 'var(--text-primary, #eee)', fontWeight: 600, overflowWrap: 'anywhere' }}>
                      {displayName(l.manifest)}
                    </span>
                    <span style={noteStyle}>{l.installed.version}</span>
                  </div>
                  <div style={noteStyle}>{sourceLine(l.installed.source)}</div>
                  <details style={{ marginTop: '6px' }}>
                    <summary style={{ cursor: 'pointer' }}>What it can do</summary>
                    <ul style={{ margin: '4px 0 0', paddingLeft: '18px', lineHeight: 1.5 }}>
                      {(l.manifest.permissions ?? []).map((p) => (
                        <li key={p}>{permissionText(p)}</li>
                      ))}
                    </ul>
                    {(l.manifest.permissions ?? []).length === 0 && <div style={noteStyle}>It asks for no permissions.</div>}
                    {(l.manifest.optionalPermissions ?? []).length > 0 && (
                      <>
                        <div style={{ ...noteStyle, marginTop: '4px' }}>May ask for later:</div>
                        <ul style={{ margin: 0, paddingLeft: '18px', lineHeight: 1.5 }}>
                          {(l.manifest.optionalPermissions ?? []).map((p) => (
                            <li key={p}>{permissionText(p)}</li>
                          ))}
                        </ul>
                      </>
                    )}
                  </details>
                  {rollingBack !== null && rollingBack.id === l.installed.id && (
                    <div role="group" aria-label={`Go back to ${rollingBack.version}?`} style={{ marginTop: '8px' }}>
                      <div>Go back to {rollingBack.version}? It will be able to:</div>
                      <ul style={{ margin: '4px 0', paddingLeft: '18px', lineHeight: 1.5 }}>
                        {rollingBack.lines.length === 0 ? <li>Nothing. It asks for no permissions.</li> : rollingBack.lines.map((t) => <li key={t}>{t}</li>)}
                      </ul>
                      <div style={{ display: 'flex', gap: '6px' }}>
                        <button type="button" style={buttonStyle()} onClick={() => setRollingBack(null)}>
                          Keep this version
                        </button>
                        <button
                          type="button"
                          style={buttonStyle(false, true)}
                          onClick={() => {
                            const id = rollingBack.id
                            setRollingBack(null)
                            void run(async () => void (await inst.rollback(id)))
                          }}
                        >
                          Roll back
                        </button>
                      </div>
                    </div>
                  )}
                  <div style={{ display: 'flex', gap: '8px', marginTop: '8px', flexWrap: 'wrap' }}>
                    {l.previous_version !== null && (
                      <button
                        type="button"
                        style={buttonStyle()}
                        onClick={() => void askRollback(l.installed.id)}
                      >
                        Roll back to {l.previous_version}
                      </button>
                    )}
                  </div>
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: '8px', alignItems: 'flex-end' }}>
                  <label style={{ display: 'flex', alignItems: 'center', gap: '6px', cursor: 'pointer' }}>
                    <input
                      type="checkbox"
                      role="switch"
                      aria-label={`Turn ${displayName(l.manifest)} on or off`}
                      checked={l.installed.enabled}
                      onChange={(e) => void toggle(l.installed.id, e.target.checked)}
                    />
                    <span>{l.installed.enabled ? 'On' : 'Off'}</span>
                  </label>
                  <RemoveControls
                    id={l.installed.id}
                    label={displayName(l.manifest)}
                    confirming={confirming === l.installed.id}
                    onAsk={() => setConfirming(l.installed.id)}
                    onKeep={() => setConfirming(null)}
                    onRemove={() => void remove(l.installed.id)}
                  />
                </div>
              </li>
            ),
          )}
        </ul>
      )}

      {pending !== null && (
        <ConsentSheet consent={pending.consent} onInstall={() => void install()} onCancel={cancel} busy={sheetBusy} error={sheetError} />
      )}
    </div>
  )
}

/** "Uninstall", which asks before it does anything. */
function RemoveControls({
  id,
  label,
  confirming,
  onAsk,
  onKeep,
  onRemove,
}: {
  id: string
  label: string
  confirming: boolean
  onAsk: () => void
  onKeep: () => void
  onRemove: () => void
}) {
  if (!confirming) {
    return (
      <button type="button" data-uninstall={id} style={buttonStyle()} onClick={onAsk}>
        Uninstall
      </button>
    )
  }
  return (
    <div role="group" aria-label={`Uninstall ${label}?`} style={{ display: 'flex', flexDirection: 'column', gap: '6px', alignItems: 'flex-end' }}>
      <div style={noteStyle}>Remove {label} and its settings?</div>
      <div style={{ display: 'flex', gap: '6px' }}>
        <button type="button" style={buttonStyle()} onClick={onKeep}>
          Keep it
        </button>
        <button type="button" data-uninstall-confirm={id} style={buttonStyle(false, true)} onClick={onRemove}>
          Uninstall
        </button>
      </div>
    </div>
  )
}
