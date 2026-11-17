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
import { createPortal } from 'react-dom'
import { ConsentSheet } from './ConsentSheet'
import { pluginInstaller } from './installer'
import { errorText, toPluginError } from './pluginError'
import { permissionText } from './permissionCopy'
import { cleanText, displayName } from './pluginText'
import { buttonStyle } from './styles'
import type { Installed, InstallSource, Listed, PendingView, PluginInstaller } from './types'
import { recordFingerprint, rollbackFingerprint } from './webPluginStore'

/** The desktop shell's refusals for a "no" in its own confirmation. */
const DECLINED: ReadonlySet<string> = new Set(['rollback_declined', 'enable_declined', 'uninstall_declined'])

/** Refusals that are advice, not failures: shown as a plain notice. */
const NOTICES: Readonly<Record<string, string>> = {
  confirmation_open: 'Another confirmation is already open. Answer it first, then try again.',
  plugin_changed: 'This plugin changed while you were deciding. Check it, then try again.',
}

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
  const [turningOn, setTurningOn] = useState<{ id: string; name: string; lines: string[]; shown: string } | null>(null)
  const [rollingBack, setRollingBack] = useState<{ id: string; version: string; lines: string[]; shown: string } | null>(null)
  const [icons, setIcons] = useState<Record<string, string>>({})
  const iconsRef = useRef<Record<string, string>>({})
  const fileInput = useRef<HTMLInputElement>(null)

  // Lists can finish out of order; only the answer to the latest request is
  // applied, so an earlier, slower one never lands over a newer one.
  const latest = useRef(0)
  const refresh = useCallback(async (): Promise<void> => {
    const mine = ++latest.current
    try {
      const listed = await inst.list()
      if (mine === latest.current) setItems(listed)
    } catch (e) {
      if (mine !== latest.current) return
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
    // Once the install has started it cannot be taken back from here: the
    // answer is the install's own.
    if (sheetBusy) return
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
      const refusal = toPluginError(e).refusal
      if (NOTICES[refusal] !== undefined) setNotice(NOTICES[refusal])
      else if (!DECLINED.has(refusal)) setError(errorText(e))
    }
    await refresh()
  }

  // On the web the pane asks before turning a plugin on, as the desktop shell
  // does in its own dialog; turning one off asks nothing.
  const askToggle = (l: Extract<Listed, { kind: 'installed' }>, enabled: boolean): Promise<void> => {
    if (enabled && inst.tier === 'web') {
      setConfirming(null)
      setRollingBack(null)
      setTurningOn({
        id: l.installed.id,
        name: displayName(l.manifest),
        lines: (l.manifest.permissions ?? []).map((p) => permissionText(p)),
        shown: recordFingerprint(l.installed),
      })
      return Promise.resolve()
    }
    return toggle(l.installed.id, enabled)
  }

  const toggle = (id: string, enabled: boolean, shown?: string): Promise<void> =>
    run(async () => {
      const also = shown === undefined ? await inst.setEnabled(id, enabled) : await inst.setEnabled(id, enabled, shown)
      if (also.length > 0) {
        setNotice(`Turning off ${nameOf(id)} also turned off ${also.map(nameOf).join(', ')}, because they need it.`)
      }
    })

  // On the web the pane asks first and lists what the earlier version can do;
  // on the desktop the shell asks in its own dialog.
  const askRollback = (current: Installed): Promise<void> =>
    run(async () => {
      const id = current.id
      if (inst.previous === undefined) {
        await inst.rollback(id)
        return
      }
      const prev = await inst.previous(id)
      setConfirming(null)
      setTurningOn(null)
      setRollingBack({
        id,
        version: prev.installed.version,
        lines: (prev.manifest.permissions ?? []).map((p) => permissionText(p)),
        shown: rollbackFingerprint(current, prev.installed),
      })
    })

  const askRemove = (id: string): void => {
    setTurningOn(null)
    setRollingBack(null)
    setConfirming(id)
  }

  const remove = (id: string): Promise<void> =>
    run(async () => {
      setConfirming(null)
      await inst.uninstall(id)
    })

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '12px', maxWidth: '44em' }}>
      <div style={noteStyle}>
        Plugins are add-ons made by other people. A plugin that is turned on runs in a sandbox, and can do only what
        you allowed when you installed it.
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
                  onAsk={() => (inst.tier === 'web' ? askRemove(l.id) : void remove(l.id))}
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
                    <RollbackConfirm
                      version={rollingBack.version}
                      lines={rollingBack.lines}
                      onKeep={() => setRollingBack(null)}
                      onRollBack={() => {
                        const { id, shown } = rollingBack
                        setRollingBack(null)
                        void run(async () => void (await inst.rollback(id, shown)))
                      }}
                    />
                  )}
                  <div style={{ display: 'flex', gap: '8px', marginTop: '8px', flexWrap: 'wrap' }}>
                    {l.previous_version !== null && (
                      <button
                        type="button"
                        style={buttonStyle()}
                        onClick={() => void askRollback(l.installed)}
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
                      onChange={(e) => void askToggle(l, e.target.checked)}
                    />
                    <span>{l.installed.enabled ? 'On' : 'Off'}</span>
                  </label>
                  <RemoveControls
                    id={l.installed.id}
                    label={displayName(l.manifest)}
                    confirming={confirming === l.installed.id}
                    onAsk={() => (inst.tier === 'web' ? askRemove(l.installed.id) : void remove(l.installed.id))}
                    onKeep={() => setConfirming(null)}
                    onRemove={() => void remove(l.installed.id)}
                  />
                </div>
              </li>
            ),
          )}
        </ul>
      )}

      {turningOn !== null && (
        <TurnOnSheet
          name={turningOn.name}
          lines={turningOn.lines}
          onCancel={() => setTurningOn(null)}
          onAccept={() => {
            const { id, shown } = turningOn
            setTurningOn(null)
            void toggle(id, true, shown)
          }}
        />
      )}

      {pending !== null && (
        <ConsentSheet consent={pending.consent} onInstall={() => void install()} onCancel={cancel} busy={sheetBusy} error={sheetError} />
      )}
    </div>
  )
}

/** The keys of a confirmation: focus starts on its safe button (Cancel),
 *  Escape cancels, Enter does nothing so a stray key never accepts, Tab stays
 *  inside the confirmation, and focus goes back to what had it (or to
 *  `restore`'s element, when that one has since been replaced). */
function useConfirmKeys(
  box: React.RefObject<HTMLElement | null>,
  safe: React.RefObject<HTMLElement | null>,
  onCancel: () => void,
  restore?: () => HTMLElement | null,
): void {
  const onCancelRef = useRef(onCancel)
  onCancelRef.current = onCancel
  const restoreRef = useRef(restore)
  restoreRef.current = restore
  useEffect(() => {
    const before = document.activeElement
    safe.current?.focus()
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onCancelRef.current()
      } else if (e.key === 'Enter') {
        e.preventDefault()
        e.stopPropagation()
      } else if (e.key === 'Tab' && box.current !== null) {
        const items = Array.from(box.current.querySelectorAll<HTMLElement>('button:not([disabled])'))
        if (items.length === 0) return
        const first = items[0]
        const last = items[items.length - 1]
        const at = document.activeElement
        if (!box.current.contains(at) || (e.shiftKey && at === first)) {
          e.preventDefault()
          ;(e.shiftKey ? last : first).focus()
        } else if (!e.shiftKey && at === last) {
          e.preventDefault()
          first.focus()
        }
      }
    }
    document.addEventListener('keydown', onKey, true)
    return () => {
      document.removeEventListener('keydown', onKey, true)
      const kept = before instanceof HTMLElement && before.isConnected && before !== document.body ? before : null
      const back = kept ?? restoreRef.current?.() ?? null
      back?.focus()
    }
  }, [box, safe])
}

/** The web's in-pane "Go back to ...?": the same keys as the turn-on sheet. */
function RollbackConfirm({
  version,
  lines,
  onKeep,
  onRollBack,
}: {
  version: string
  lines: string[]
  onKeep: () => void
  onRollBack: () => void
}) {
  const box = useRef<HTMLDivElement>(null)
  const keep = useRef<HTMLButtonElement>(null)
  useConfirmKeys(box, keep, onKeep)
  return (
    <div ref={box} role="group" aria-label={`Go back to ${version}?`} style={{ marginTop: '8px' }}>
      <div>Go back to {version}? It will be able to:</div>
      <ul style={{ margin: '4px 0', paddingLeft: '18px', lineHeight: 1.5 }}>
        {lines.length === 0 ? <li>Nothing. It asks for no permissions.</li> : lines.map((t) => <li key={t}>{t}</li>)}
      </ul>
      <div style={{ display: 'flex', gap: '6px' }}>
        <button ref={keep} type="button" style={buttonStyle()} onClick={onKeep}>
          Keep this version
        </button>
        <button type="button" style={buttonStyle(false, true)} onClick={onRollBack}>
          Roll back
        </button>
      </div>
    </div>
  )
}

/** The web's "Turn on this plugin?": Cancel has focus, Escape cancels and
 *  Enter does nothing, so a stray key never turns a plugin on. */
function TurnOnSheet({
  name,
  lines,
  onAccept,
  onCancel,
}: {
  name: string
  lines: string[]
  onAccept: () => void
  onCancel: () => void
}) {
  const box = useRef<HTMLDivElement>(null)
  const cancelButton = useRef<HTMLButtonElement>(null)
  useConfirmKeys(box, cancelButton, onCancel)

  return createPortal(
    <div
      onClick={(e) => e.stopPropagation()}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 500,
        background: 'var(--backdrop-dim, rgba(8,10,13,0.45))',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <div
        ref={box}
        role="dialog"
        aria-modal="true"
        aria-labelledby="plugin-turn-on-title"
        style={{
          width: 'min(420px, calc(100vw - 32px))',
          padding: '16px 18px',
          background: 'var(--surface-window, #1a1a1a)',
          color: 'var(--text-secondary, #ddd)',
          border: '1px solid var(--border-hairline, #333)',
          borderRadius: '10px',
          boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
          fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
          fontSize: '13px',
        }}
      >
        <h2 id="plugin-turn-on-title" style={{ margin: 0, fontSize: '15px', color: 'var(--text-primary, #eee)' }}>
          Turn on this plugin?
        </h2>
        <div style={{ marginTop: '8px', overflowWrap: 'anywhere' }}>
          {name} will run in a sandbox. It will be able to:
        </div>
        <ul style={{ margin: '4px 0 0', paddingLeft: '18px', lineHeight: 1.5 }}>
          {lines.length === 0 ? <li>Nothing. It asks for no permissions.</li> : lines.map((t) => <li key={t}>{t}</li>)}
        </ul>
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '8px', marginTop: '14px' }}>
          <button ref={cancelButton} type="button" onClick={onCancel} style={buttonStyle()}>
            Cancel
          </button>
          <button type="button" onClick={onAccept} style={buttonStyle(false, true)}>
            Turn On
          </button>
        </div>
      </div>
    </div>,
    document.body,
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
  return <RemoveConfirm id={id} label={label} onKeep={onKeep} onRemove={onRemove} />
}

/** The web's "Remove ...?": the same keys as the turn-on sheet; focus goes
 *  back to the Uninstall button, which comes back in this one's place. */
function RemoveConfirm({ id, label, onKeep, onRemove }: { id: string; label: string; onKeep: () => void; onRemove: () => void }) {
  const box = useRef<HTMLDivElement>(null)
  const keep = useRef<HTMLButtonElement>(null)
  useConfirmKeys(box, keep, onKeep, () => document.querySelector<HTMLElement>(`[data-uninstall="${CSS.escape(id)}"]`))
  return (
    <div ref={box} role="group" aria-label={`Uninstall ${label}?`} style={{ display: 'flex', flexDirection: 'column', gap: '6px', alignItems: 'flex-end' }}>
      <div style={noteStyle}>Remove {label} and its settings?</div>
      <div style={{ display: 'flex', gap: '6px' }}>
        <button ref={keep} type="button" style={buttonStyle()} onClick={onKeep}>
          Keep it
        </button>
        <button type="button" data-uninstall-confirm={id} style={buttonStyle(false, true)} onClick={onRemove}>
          Uninstall
        </button>
      </div>
    </div>
  )
}
