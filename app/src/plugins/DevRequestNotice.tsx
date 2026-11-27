/* oxlint-disable lingui/no-unlocalized-strings -- plugin install copy is English only until the feature graduates, and stays out of the catalogs. */
/**
 * DevRequestNotice: a program on this computer asked, over the live API
 * (`hew.plugin.dev`), to load a plugin from a folder. The shell keeps the
 * one waiting request, with an id, and tells every window
 * (`plugins-dev-request`: the request, or null once it is taken, dismissed
 * or dropped); a page that missed that asks for it (`plugin_dev_request`),
 * which does not take it.
 *
 * The notice is small and asks nothing by itself: a fixed line, the folder
 * set apart as data (`PathText`), "Choose Folder…" and "Dismiss". The
 * folder is display text only, cleaned; the page hands back only the
 * request's id. "Choose Folder…" opens the native folder picker for that
 * request (refused when it is no longer the one waiting, and the notice
 * looks again); whatever the person picks there goes through the ordinary
 * install: the consent sheet, then the shell's own confirmation. Dismiss
 * tells the shell, which forgets the request in every window and turns the
 * socket away for a while. In a document window it sits in a corner over
 * the model; in Settings ▸ Plugins it is a line in the pane. Desktop only,
 * and only while the Plugins switch is on (the host and the pane exist only
 * then).
 */

import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react'
import { ConsentSheet } from './ConsentSheet'
import { PathText } from './pathText'
import { errorText, toPluginError } from './pluginError'
import { devRequestNotice } from './storeAccess'
import { buttonStyle } from './styles'
import type { DevRequest, PendingView, PluginInstaller } from './types'

/** Refusals that are advice, not failures. */
const NOTICES: Readonly<Record<string, string>> = {
  confirmation_open: 'Another confirmation is already open. Answer it first, then try again.',
  request_gone: 'That request is no longer waiting.',
}

const cornerStyle: CSSProperties = {
  position: 'fixed',
  right: '16px',
  bottom: '16px',
  zIndex: 400,
  width: 'min(380px, calc(100vw - 32px))',
  padding: '12px 14px',
  background: 'var(--surface-window, #1a1a1a)',
  color: 'var(--text-secondary, #ddd)',
  border: '1px solid var(--border-hairline, #333)',
  borderRadius: '10px',
  boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  fontSize: '13px',
}

const inlineStyle: CSSProperties = {
  padding: '10px 12px',
  border: '1px solid var(--border-hairline, rgba(128,128,128,0.25))',
  borderRadius: '8px',
  background: 'var(--surface-bar, #202020)',
}

export interface DevRequestNoticeProps {
  installer: PluginInstaller
  /** Where it sits: a corner of a document window, or a line in the pane. */
  placement: 'window' | 'pane'
}

/** The sentence that says what was asked, before the folder itself. */
export const REQUEST_LINE = 'A program on this computer asked to load a plugin from this folder:'

/** Pane notices mounted in this document. Where Settings is a page inside
 *  the document window (Windows), the pane shows the request, and the
 *  corner notice of that window stands aside so it is shown once. */
let panesShown = 0
const paneListeners = new Set<() => void>()
function panesChanged(by: number): void {
  panesShown += by
  paneListeners.forEach((l) => l())
}

export function DevRequestNotice({ installer, placement }: DevRequestNoticeProps) {
  const [panes, setPanes] = useState(panesShown)
  const [request, setRequest] = useState<DevRequest | null>(null)
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null)
  const [busy, setBusy] = useState(false)
  const [pending, setPending] = useState<PendingView | null>(null)
  const [sheetBusy, setSheetBusy] = useState(false)
  const [sheetError, setSheetError] = useState<string | null>(null)
  // While the person is picking or reading the sheet, a refresh does not
  // bring a request back over the flow it started; the flow looks again
  // when it ends.
  const inFlow = useRef(false)
  // Answers can come out of order; only the latest look is applied.
  const latest = useRef(0)

  const look = useCallback(async (): Promise<void> => {
    if (installer.devRequest === undefined) return
    const mine = ++latest.current
    let found: DevRequest | null
    try {
      found = await installer.devRequest()
    } catch {
      return
    }
    if (mine === latest.current && !inFlow.current) setRequest(found)
  }, [installer])

  const endFlow = useCallback((): void => {
    inFlow.current = false
    void look()
  }, [look])

  useEffect(() => {
    if (placement === 'pane') {
      panesChanged(1)
      return () => panesChanged(-1)
    }
    const heard = (): void => setPanes(panesShown)
    paneListeners.add(heard)
    heard()
    return () => void paneListeners.delete(heard)
  }, [placement])

  useEffect(() => {
    void look()
    const offChanged = installer.onChanged(() => void look())
    // The installer has already checked the payload; it is checked and
    // cleaned again here, as display text.
    const offRequest = installer.onDevRequest?.((r) => {
      if (inFlow.current) return
      latest.current++
      setRequest(r === null ? null : devRequestNotice(r))
      setMessage(null)
    })
    return () => {
      offChanged()
      offRequest?.()
    }
  }, [installer, look])

  const say = (e: unknown): void => {
    const refusal = toPluginError(e).refusal
    setMessage({ text: NOTICES[refusal] ?? errorText(e), error: NOTICES[refusal] === undefined })
  }

  const choose = async (): Promise<void> => {
    if (request === null) return
    inFlow.current = true
    setBusy(true)
    setMessage(null)
    try {
      const view = await installer.pickAndStage('dir', { useRequest: request.id })
      // Only a staged folder uses the request up. A cancel leaves it waiting,
      // and the card with it, so the person can choose again.
      if (view !== null) {
        setRequest(null)
        setSheetError(null)
        setPending(view)
        return
      }
    } catch (e) {
      say(e)
      if (toPluginError(e).refusal === 'request_gone') setRequest(null)
    } finally {
      setBusy(false)
    }
    endFlow()
  }

  const install = async (): Promise<void> => {
    if (pending === null) return
    setSheetBusy(true)
    setSheetError(null)
    try {
      const record = await installer.commit(pending.token, true)
      const name = pending.consent.name
      setPending(null)
      setMessage({ text: record === null ? 'Nothing was installed.' : `${name} is installed.`, error: false })
      endFlow()
    } catch (e) {
      setSheetError(errorText(e))
    } finally {
      setSheetBusy(false)
    }
  }

  const cancel = (): void => {
    if (sheetBusy || pending === null) return
    const token = pending.token
    setPending(null)
    setSheetError(null)
    installer.commit(token, false).catch(() => undefined)
    endFlow()
  }

  const dismiss = async (): Promise<void> => {
    const shown = request
    setMessage(null)
    if (shown === null || installer.dismissDevRequest === undefined) {
      setRequest(null)
      return
    }
    try {
      await installer.dismissDevRequest(shown.id)
      setRequest(null)
    } catch {
      // Not dismissed in Hew: the card stays, and says so.
      setMessage({ text: "Hew couldn't dismiss the request. Try again, or let it expire in a few minutes.", error: false })
    }
  }

  if (pending !== null) {
    return <ConsentSheet consent={pending.consent} onInstall={() => void install()} onCancel={cancel} busy={sheetBusy} error={sheetError} />
  }
  if (request === null && message === null) return null
  if (placement === 'window' && panes > 0) return null
  return (
    <div role="region" aria-label="Request to load a plugin" data-plugin-dev-request="" style={placement === 'window' ? cornerStyle : inlineStyle}>
      {request !== null && (
        <>
          <div style={{ color: 'var(--text-primary, #eee)', marginBottom: '4px' }}>{REQUEST_LINE}</div>
          <PathText label="Folder" value={request.path} />
        </>
      )}
      {message !== null && (
        <div role={message.error ? 'alert' : 'status'} style={{ marginTop: request !== null ? '6px' : 0, color: message.error ? 'var(--danger-base, #e0574f)' : undefined }}>
          {message.text}
        </div>
      )}
      <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '8px', marginTop: '10px' }}>
        <button type="button" style={buttonStyle()} onClick={() => void dismiss()}>
          Dismiss
        </button>
        {request !== null && (
          <button type="button" disabled={busy} style={buttonStyle(busy, true)} onClick={() => void choose()}>
            Choose Folder…
          </button>
        )}
      </div>
    </div>
  )
}
