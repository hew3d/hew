/**
 * AddSourceDialog — "Add a source…" (docs/design/shared-library-sources.md
 * 7.5): one address field. After the source loads it shows its name, author,
 * host, license (or "No license stated") and item count, and for a source the
 * Hew catalog does not list, "This source is not in the Hew catalog. Hew has
 * not checked it." Buttons: Add, Cancel.
 *
 * "Look up" only reads the source (`peekSource` writes nothing), and Add is
 * what puts it in the list (`addSource`), so Cancel has nothing to undo.
 */

import { useEffect, useRef, useState } from 'react'
import { plural } from '@lingui/core/macro'
import { Trans, useLingui } from '@lingui/react/macro'
import { isTauri } from '../../io/fileHost'
import { isComposingKey } from '../../input/keyboard'
import { failureMessage, sourceErrorMessage, sourcesOffText } from '../../library/sourceMessages'
import { sourceHost } from '../../library/sourceModel'
import type { SourcesBinding } from '../../library/sourcesDeps'
import type { PeekOk } from '../../library/sources'

export interface AddSourceDialogProps {
  binding: SourcesBinding
  /** The catalog is configured, so "not in the catalog" can be said. */
  catalogEnabled: boolean
  /** Done: the source (new or already there) is in the list. */
  onDone(source: string): void
  onClose(): void
  /** The sources list changed (a source was added). */
  onChanged(): void
}

/** Best-effort host of what the user typed, for the failure message. */
function hostOfText(text: string): string | null {
  const trimmed = text.trim()
  try {
    return new URL(trimmed).host
  } catch {
    // A short form such as github:owner/repo names its host through its kind.
    return trimmed.includes(':') ? sourceHost(trimmed) : null
  }
}

export function AddSourceDialog({ binding, catalogEnabled, onDone, onClose, onChanged }: AddSourceDialogProps) {
  const { t } = useLingui()
  const [text, setText] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [found, setFound] = useState<PeekOk | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const mounted = useRef(true)
  useEffect(() => {
    mounted.current = true
    inputRef.current?.focus()
    return () => {
      mounted.current = false
    }
  }, [])

  async function lookUp(): Promise<void> {
    const address = text.trim()
    if (address === '' || busy) return
    // The one rule a typed address can break before any request: https only.
    if (/^http:\/\//i.test(address)) {
      setError(sourceErrorMessage('not_https'))
      return
    }
    setBusy(true)
    setError(null)
    setFound(null)
    const r = await binding.sources.peekSource(address)
    if (!mounted.current) return
    setBusy(false)
    if ('off' in r) setError(sourcesOffText())
    else if (!r.ok) setError(failureMessage(r, { web: !isTauri, host: hostOfText(address) }).text)
    else if (r.existing && r.needsCheck === true) {
      // Nothing is known about it yet (setting off, no cached listing): the
      // facts below would read "0 items". Open it; its view offers Refresh.
      onDone(r.source)
    } else setFound(r)
  }

  /** Add: the source goes into the list now. One already there is just opened. */
  async function confirm(): Promise<void> {
    if (found === null || busy) return
    if (found.existing) {
      onDone(found.source)
      return
    }
    setBusy(true)
    setError(null)
    const r = await binding.sources.addSource(text.trim())
    if (!mounted.current) return
    setBusy(false)
    if ('off' in r) setError(sourcesOffText())
    else if (!r.ok) setError(failureMessage(r, { web: !isTauri, host: hostOfText(text) }).text)
    else {
      onChanged()
      onDone(r.entry.source)
    }
  }

  function cancel(): void {
    onClose()
  }

  // Escape closes this dialog and nothing under it.
  const cancelRef = useRef(cancel)
  cancelRef.current = cancel
  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key !== 'Escape' || isComposingKey(e)) return
      e.preventDefault()
      e.stopPropagation()
      cancelRef.current()
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [])

  const heading = t`Add a source`
  const summary = found?.summary
  return (
    <div className="hwlib__src-overlay" onClick={() => cancel()}>
      <div className="hwlib__src-dialog" role="dialog" aria-modal="true" aria-label={heading} data-testid="add-source-dialog" onClick={(e) => e.stopPropagation()}>
        <h2>{heading}</h2>
        <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <span><Trans>Address of the source</Trans></span>
          <input
            ref={inputRef}
            type="text"
            value={text}
            disabled={found !== null}
            placeholder={t`Paste a web address`}
            aria-label={t`Address of the source`}
            onChange={(e) => {
              setText(e.target.value)
              setError(null)
            }}
            onKeyDown={(e) => {
              if (isComposingKey(e)) return
              if (e.key === 'Enter') {
                e.preventDefault()
                if (found === null) void lookUp()
                else void confirm()
              }
            }}
          />
        </label>

        {error !== null && (
          <div className="hwlib__action-error" role="alert" data-testid="add-source-error">
            {error}
          </div>
        )}
        {busy && <div role="status"><Trans>Looking…</Trans></div>}

        {found !== null && summary !== undefined && (
          <>
            <dl className="hwlib__src-facts" data-testid="add-source-facts">
              <dt><Trans>Name</Trans></dt>
              <dd>{summary.name}</dd>
              {summary.author != null && summary.author !== '' && (
                <>
                  <dt><Trans>Author</Trans></dt>
                  <dd>{summary.author}</dd>
                </>
              )}
              <dt><Trans>Host</Trans></dt>
              <dd>{found.host}</dd>
              <dt><Trans>License</Trans></dt>
              <dd>{summary.license ?? t`No license stated`}</dd>
              <dt><Trans>Items</Trans></dt>
              <dd>{plural(summary.itemCount, { one: '# item', other: '# items' })}</dd>
            </dl>
            {summary.description != null && summary.description !== '' && <p style={{ margin: 0 }}>{summary.description}</p>}
            {found.existing && (
              <div role="status" data-testid="add-source-existing"><Trans>This source is already in your list.</Trans></div>
            )}
            {found.listing === 'unlisted' && (
              <div className="hwlib__src-warning" data-testid="add-source-unlisted">
                <Trans>This source is not in the Hew catalog. Hew has not checked it.</Trans>
              </div>
            )}
            {found.listing === 'unknown' && catalogEnabled && (
              <div className="hwlib__src-warning" data-testid="add-source-unknown">
                <Trans>Hew could not check whether this source is in the Hew catalog.</Trans>
              </div>
            )}
          </>
        )}

        <div className="hwlib__src-dialog-actions">
          <button type="button" className="hwlib__btn-secondary" onClick={() => cancel()}>
            <Trans>Cancel</Trans>
          </button>
          {found === null ? (
            <button type="button" className="hwlib__btn-primary" disabled={busy || text.trim() === ''} onClick={() => void lookUp()}>
              <Trans>Look up</Trans>
            </button>
          ) : (
            <button type="button" className="hwlib__btn-primary" autoFocus disabled={busy} onClick={() => void confirm()}>
              {found.existing ? t`Done` : t`Add`}
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
