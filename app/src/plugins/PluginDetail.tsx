/* oxlint-disable lingui/no-unlocalized-strings -- plugin pane copy; English only until the feature graduates, and stays out of the catalogs. */
/**
 * PluginDetail — one plugin's page in Settings > Plugins, opened from its
 * row, with three tabs:
 *
 *  - Overview: its name, version and source, what it is doing, what it can
 *    do, the optional permissions it may ask for (each a switch the person
 *    turns on or off; on asks, off does not), and where its saved data is
 *    kept, in plain words.
 *  - Console: the lines it printed, newest first, each as wrapped text and
 *    cut to a bound. "Clear view" only empties what is shown here; nothing
 *    is deleted.
 *  - Activity: the changes of state it went through, newest first, in plain
 *    words.
 *
 * Everything a plugin wrote is data: names go through `displayName` and sit
 * in set-apart blocks, console text is cleaned by `paneData` and rendered as
 * text in its own block, and a stop reason is never shown, only the words
 * the pane matches it to. Escape closes the page (unless a question is open
 * over it) and the pane puts focus back on the row.
 */

import { useCallback, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react'
import { grantCapable } from './installer'
import { requestGrant } from './grantSheet'
import { PathText } from './pathText'
import { activityText, visibleActivity, runLines, type ActivityEntry, type ConsoleLine, type PaneData, type RunView } from './paneData'
import { errorText, PluginError, toPluginError } from './pluginError'
import { permissionText } from './permissionCopy'
import { displayName } from './pluginText'
import { buttonStyle } from './styles'
import type { InstallSource, Listed, PluginInstaller } from './types'

type InstalledRow = Extract<Listed, { kind: 'installed' }>

/** Refusals that are advice, not failures. */
const NOTICES: Readonly<Record<string, string>> = {
  confirmation_open: 'Another confirmation is already open. Answer it first, then try again.',
  plugin_changed: 'This plugin changed while you were deciding. Check it, then try again.',
}

const noteStyle: CSSProperties = { fontSize: '11px', lineHeight: 1.5, color: 'var(--text-faint, #888)', maxWidth: '36em' }
const headingStyle: CSSProperties = { margin: '14px 0 4px', fontSize: '12px', fontWeight: 600, color: 'var(--text-primary, #eee)' }
const listStyle: CSSProperties = { margin: 0, paddingLeft: '18px', lineHeight: 1.5 }

/** The source of a plugin, in Hew's words, with anything it named set apart. */
export function sourceLine(source: InstallSource): ReactNode {
  if (source.kind === 'file') {
    // A folder copied on the web is stored as a file with an `origin` the
    // kernel does not read.
    if (source.origin !== 'folder') return 'From file'
    return (
      <>
        <div>Copied from a folder</div>
        <PathText label="Folder name" value={source.name} />
      </>
    )
  }
  if (source.kind === 'dir') {
    return (
      <>
        <div>Developer folder</div>
        <PathText label="Folder" value={source.path} />
      </>
    )
  }
  return `From ${source.kind}`
}

type Tab = 'overview' | 'console' | 'activity'
const TABS: readonly { id: Tab; label: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'console', label: 'Console' },
  { id: 'activity', label: 'Activity' },
]

function when(at: number): string {
  if (!(at > 0)) return ''
  const d = new Date(at)
  return Number.isNaN(d.getTime()) ? '' : d.toLocaleTimeString()
}

export interface PluginDetailProps {
  row: InstalledRow
  installer: PluginInstaller
  data: PaneData
  /** What the row says it is doing, if anything. */
  run: RunView | null
  /** Why the optional-permission switches are locked (the plugin waits for
   *  review, needs developer mode, or is held back), or null when they work. */
  lockReason?: string | null
  onClose: () => void
  /** Asks the pane to list again after this page changed something. */
  onChanged: () => Promise<void>
}

export function PluginDetail({ row, installer, data, run, lockReason = null, onClose, onChanged }: PluginDetailProps) {
  const id = row.installed.id
  const name = displayName(row.manifest)
  const [tab, setTab] = useState<Tab>('overview')
  const [asking, setAsking] = useState(false)
  // "Clear view" lasts as long as this page is open, across tab switches.
  const [cleared, setCleared] = useState<ClearedView | null>(null)
  const root = useRef<HTMLDivElement>(null)
  const back = useRef<HTMLButtonElement>(null)
  const closeRef = useRef(onClose)
  closeRef.current = onClose
  const askingRef = useRef(asking)
  askingRef.current = asking

  useEffect(() => {
    back.current?.focus()
  }, [])

  // Escape closes the page, ahead of the Settings window's own handler; not
  // while a question is open over it (its own handler answers that).
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key !== 'Escape' || askingRef.current) return
      // A permission or review sheet over this page (a plugin's own request
      // opens one from outside this page) takes Escape first: its own
      // handler answers it, so the page stays.
      if (document.querySelector('[data-plugin-sheet]') !== null) return
      e.preventDefault()
      e.stopPropagation()
      closeRef.current()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [])

  const tabKeys = (e: ReactKeyboardEvent<HTMLElement>): void => {
    const at = TABS.findIndex((t) => t.id === tab)
    const next = e.key === 'ArrowRight' ? (at + 1) % TABS.length : e.key === 'ArrowLeft' ? (at + TABS.length - 1) % TABS.length : -1
    if (next < 0) return
    e.preventDefault()
    setTab(TABS[next].id)
    requestAnimationFrame(() => root.current?.querySelector<HTMLElement>(`[data-detail-tab="${TABS[next].id}"]`)?.focus())
  }

  return (
    // Marked as owning Escape, so Settings' own handler (which runs first)
    // leaves it to this page (`settingsEscape.ts`).
    <div ref={root} role="region" aria-label={`Details of ${name}`} data-plugin-detail={id} data-settings-escape-child="" style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
        <button ref={back} type="button" style={buttonStyle()} onClick={onClose}>
          Back
        </button>
        <span style={{ color: 'var(--text-primary, #eee)', fontWeight: 600, overflowWrap: 'anywhere' }}>{name}</span>
        <span style={noteStyle}>{row.installed.version}</span>
      </div>
      <div role="tablist" aria-label="Plugin details" onKeyDown={tabKeys} style={{ display: 'flex', gap: '4px', borderBottom: '1px solid var(--border-hairline, rgba(128,128,128,0.25))' }}>
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            id={`plugin-tab-${t.id}`}
            data-detail-tab={t.id}
            aria-selected={tab === t.id}
            aria-controls={`plugin-panel-${t.id}`}
            tabIndex={tab === t.id ? 0 : -1}
            onClick={() => setTab(t.id)}
            style={{
              ...buttonStyle(),
              border: 'none',
              borderBottom: tab === t.id ? '2px solid var(--accent-strong, #3b6ae8)' : '2px solid transparent',
              borderRadius: 0,
              background: 'transparent',
            }}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div role="tabpanel" id={`plugin-panel-${tab}`} aria-labelledby={`plugin-tab-${tab}`}>
        {tab === 'overview' && <Overview row={row} installer={installer} run={run} lockReason={lockReason} onChanged={onChanged} setAsking={setAsking} />}
        {tab === 'console' && <ConsoleTab id={id} data={data} cleared={cleared} setCleared={setCleared} />}
        {tab === 'activity' && <ActivityTab id={id} data={data} />}
      </div>
    </div>
  )
}

function Overview({
  row,
  installer,
  run,
  lockReason,
  onChanged,
  setAsking,
}: {
  row: InstalledRow
  installer: PluginInstaller
  run: RunView | null
  lockReason: string | null
  onChanged: () => Promise<void>
  setAsking: (asking: boolean) => void
}) {
  const id = row.installed.id
  const required = row.manifest.permissions ?? []
  const optional = row.manifest.optionalPermissions ?? []
  const granted = row.installed.optional_granted
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)

  const change = useCallback(
    async (name: string, on: boolean): Promise<void> => {
      setError(null)
      setNotice(null)
      setBusy(name)
      try {
        if (on) {
          setAsking(true)
          try {
            // Asks: the shell's confirmation on the desktop, a sheet on the web.
            await requestGrant(installer, id, [name])
          } finally {
            setAsking(false)
          }
        } else {
          // Taking one back asks nothing. The record is read again first,
          // so a name granted since this page last read it (in another
          // window, or by the plugin's own request) is kept.
          const cap = grantCapable(installer)
          if (cap === null) return
          const fresh = (await installer.list()).find((l) => l.kind === 'installed' && l.installed.id === id)
          if (fresh === undefined || fresh.kind !== 'installed') throw new PluginError('not_installed', 'The plugin is not installed.')
          await cap.setGrant(
            id,
            [...fresh.installed.grant],
            fresh.installed.optional_granted.filter((n) => n !== name),
          )
        }
      } catch (e) {
        const refusal = toPluginError(e).refusal
        if (NOTICES[refusal] !== undefined) setNotice(NOTICES[refusal])
        else setError(errorText(e))
      } finally {
        setBusy(null)
        await onChanged()
      }
    },
    [installer, id, onChanged, setAsking],
  )

  return (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      <div style={{ ...noteStyle, overflowWrap: 'anywhere' }}>{sourceLine(row.installed.source)}</div>
      {run !== null && (
        <div data-plugin-run="" style={{ marginTop: '8px', color: 'var(--text-primary, #eee)' }}>
          {runLines(run).map((line) => (
            <div key={line}>{line}</div>
          ))}
        </div>
      )}

      <h3 style={headingStyle}>What it can do</h3>
      {required.length === 0 ? (
        <div style={noteStyle}>It asks for no permissions.</div>
      ) : (
        <ul style={listStyle}>
          {required.map((p) => (
            <li key={p}>{permissionText(p)}</li>
          ))}
        </ul>
      )}

      {optional.length > 0 && (
        <>
          <h3 style={headingStyle}>It may ask for more</h3>
          <div style={noteStyle}>These are off until you allow them. Turning one on asks you first; turning one off does not.</div>
          {lockReason !== null && (
            <div data-optional-locked="" style={{ ...noteStyle, marginTop: '4px' }}>
              {lockReason}
            </div>
          )}
          <ul style={{ listStyle: 'none', margin: '4px 0 0', padding: 0, display: 'flex', flexDirection: 'column', gap: '6px' }}>
            {optional.map((p) => (
              <li key={p}>
                <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: busy === null && lockReason === null ? 'pointer' : 'default' }}>
                  <input
                    type="checkbox"
                    role="switch"
                    data-optional-permission={p}
                    aria-label={`Allow: ${permissionText(p)}`}
                    checked={granted.includes(p)}
                    disabled={busy !== null || lockReason !== null}
                    onChange={(e) => void change(p, e.target.checked)}
                  />
                  <span>{permissionText(p)}</span>
                </label>
              </li>
            ))}
          </ul>
        </>
      )}
      {error !== null && (
        <div role="alert" style={{ marginTop: '6px', color: 'var(--danger-base, #e0574f)' }}>
          {error}
        </div>
      )}
      {notice !== null && (
        <div role="status" style={{ marginTop: '6px' }}>
          {notice}
        </div>
      )}

      <h3 style={headingStyle}>Where its data is kept</h3>
      <div style={{ ...noteStyle, maxWidth: '40em' }}>
        Anything this plugin saves stays on this device. It does not travel with a model file, and it is not part of Undo.
        Removing the plugin removes what it saved.
      </div>
      <div style={{ ...noteStyle, maxWidth: '40em', marginTop: '4px' }}>
        What it saves for one model is kept under that model's id. Two copies of the same file share it, and a SketchUp
        file has no place for the id, so what was saved for one is not found again when you reopen it.
      </div>
    </div>
  )
}

/** Reads a list now and again whenever `watch` says it may have changed. */
function useRead<T>(read: () => Promise<T[]>, watch: (fn: () => void) => () => void): { list: T[] | null; failed: boolean } {
  const [state, setState] = useState<{ list: T[] | null; failed: boolean }>({ list: null, failed: false })
  const latest = useRef(0)
  const readRef = useRef(read)
  readRef.current = read
  useEffect(() => {
    const again = (): void => {
      const mine = ++latest.current
      readRef.current().then(
        (list) => {
          if (mine === latest.current) setState({ list, failed: false })
        },
        () => {
          if (mine === latest.current) setState((s) => ({ list: s.list ?? [], failed: true }))
        },
      )
    }
    again()
    const off = watch(again)
    return () => {
      latest.current++
      off()
    }
  }, [watch])
  return state
}

const levelStyle = (level: ConsoleLine['level']): CSSProperties => ({
  color: level === 'error' ? 'var(--danger-base, #e0574f)' : level === 'warn' ? 'var(--status-warning, #e8c84a)' : 'var(--text-faint, #888)',
  flexShrink: 0,
  minWidth: '3.5em',
})

/** Where "Clear view" cut the console: the newest line then showing, and how
 *  many lines had that same time (a line that arrives in the same
 *  millisecond is told apart by count, not by time). */
interface ClearedView {
  at: number
  ties: number
}

/** The lines (newest first) that arrived after `cleared`. */
function afterCleared(list: ConsoleLine[], cleared: ClearedView | null): ConsoleLine[] {
  if (cleared === null) return list
  const sameTime = list.filter((l) => l.at === cleared.at).length
  let newAtThatTime = Math.max(0, sameTime - cleared.ties)
  return list.filter((l) => {
    if (l.at > cleared.at) return true
    if (l.at === cleared.at && newAtThatTime > 0) {
      newAtThatTime--
      return true
    }
    return false
  })
}

function ConsoleTab({ id, data, cleared, setCleared }: { id: string; data: PaneData; cleared: ClearedView | null; setCleared: (c: ClearedView) => void }) {
  const read = useCallback(() => data.console(id), [data, id])
  const watch = useCallback((fn: () => void) => data.onConsole(id, fn), [data, id])
  const { list, failed } = useRead<ConsoleLine>(read, watch)
  // "Clear view" hides what is already here; later lines show.
  const shown = afterCleared(list ?? [], cleared)
  const clear = (): void => {
    const all = list ?? []
    if (all.length === 0) return
    const at = Math.max(...all.map((l) => l.at))
    setCleared({ at, ties: all.filter((l) => l.at === at).length })
  }
  return (
    <div>
      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', margin: '6px 0' }}>
        <button type="button" style={buttonStyle(shown.length === 0)} disabled={shown.length === 0} onClick={clear}>
          Clear view
        </button>
        <span style={noteStyle}>This only clears what is shown here. Nothing is deleted.</span>
      </div>
      {failed && (
        <div role="alert" style={{ color: 'var(--danger-base, #e0574f)' }}>
          Hew could not read this plugin's console.
        </div>
      )}
      {list === null ? (
        <div style={noteStyle}>Loading…</div>
      ) : shown.length === 0 ? (
        <div style={noteStyle}>Nothing printed yet.</div>
      ) : (
        <ul data-plugin-console="" aria-label="Console lines, newest first" style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: '2px', maxHeight: '360px', overflowY: 'auto' }}>
          {shown.map((l, i) => (
            <li key={i} style={{ display: 'flex', gap: '8px', alignItems: 'baseline' }}>
              <span style={{ ...noteStyle, flexShrink: 0 }}>{when(l.at)}</span>
              <span style={levelStyle(l.level)}>{l.level}</span>
              {/* Plain text, wrapped, in its own block: never markup, never part of Hew's sentences. */}
              <code style={{ fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace', fontSize: '12px', whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', minWidth: 0 }}>{l.text}</code>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function ActivityTab({ id, data }: { id: string; data: PaneData }) {
  const read = useCallback(() => data.activity(id), [data, id])
  const watch = useCallback((fn: () => void) => data.onActivity(id, fn), [data, id])
  const { list: all, failed } = useRead<ActivityEntry>(read, watch)
  const list = all === null ? null : visibleActivity(all)
  return (
    <div>
      {failed && (
        <div role="alert" style={{ color: 'var(--danger-base, #e0574f)' }}>
          Hew could not read this plugin's activity.
        </div>
      )}
      {list === null ? (
        <div style={noteStyle}>Loading…</div>
      ) : list.length === 0 ? (
        <div style={{ ...noteStyle, marginTop: '6px' }}>Nothing has happened yet.</div>
      ) : (
        <ul data-plugin-activity="" aria-label="Activity, newest first" style={{ listStyle: 'none', margin: '6px 0 0', padding: 0, display: 'flex', flexDirection: 'column', gap: '2px', maxHeight: '360px', overflowY: 'auto' }}>
          {list.map((e, i) => (
            <li key={i} style={{ display: 'flex', gap: '8px', alignItems: 'baseline' }}>
              <span style={{ ...noteStyle, flexShrink: 0 }}>{when(e.at)}</span>
              <span>{activityText(e)}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
