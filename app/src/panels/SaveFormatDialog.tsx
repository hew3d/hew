/**
 * SaveFormatDialog — the in-app format choice File ▸ Save As… shows before
 * the native save picker: Hew document, SketchUp 2021 and later (with the
 * option to avoid features older versions cannot open), or an older SketchUp
 * release picked from a list (the file's own release preselected). The picker's filter and forced extension follow the choice
 * (`FileHost.saveAs`'s `format`). The selection starts on the current
 * document's format (`hew` for a new document).
 *
 * Enter continues, Escape cancels; styled like ConfirmDialog.
 */

import { Fragment, useCallback, useEffect, useState } from 'react'
import { Trans, useLingui } from '@lingui/react/macro'
import type { DocFormat } from '../io/fileHost'
import {
  SAVE_FORMAT_CHOICES,
  compatLabel,
  isSkpStreamFormat,
  olderGroupLabel,
  olderReleaseFor,
  takesCompat,
} from '../io/docFormat'
import { isComposingKey } from '../input/keyboard'

/** What the dialog answers: the format and, for the 2021-and-later
 * container, whether to avoid features older versions cannot open. */
export interface SaveFormatAnswer {
  format: DocFormat
  compat: boolean
}

interface SaveFormatDialogProps {
  /** The format selected when the dialog opens. */
  initial: DocFormat
  /** The compat box starts ticked (a revision-17 file is being saved back). */
  initialCompat?: boolean
  onChoose: (answer: SaveFormatAnswer) => void
  onCancel: () => void
}

const OVERLAY_STYLE: React.CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'var(--backdrop-dim, rgba(0,0,0,0.6))',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  zIndex: 2000,
}

const DIALOG_STYLE: React.CSSProperties = {
  background: 'var(--surface-overlay, #2a2a2a)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 6px)',
  boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
  padding: '20px 24px',
  minWidth: '360px',
  maxWidth: '480px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  color: 'var(--text-secondary, #ddd)',
}

const HEADING_STYLE: React.CSSProperties = {
  fontSize: '15px',
  fontWeight: 600,
  color: 'var(--text-primary, #eee)',
  marginBottom: '14px',
}

const OPTION_STYLE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'flex-start',
  gap: '10px',
  padding: '8px 4px',
  cursor: 'pointer',
}

const OPTION_LABEL_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-menu-item, 13px)',
  color: 'var(--text-primary, #eee)',
}

const OPTION_HINT_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--text-tertiary, #ccc)',
  marginTop: '2px',
}

const BUTTON_ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'flex-end',
  gap: '10px',
  marginTop: '16px',
}

const CANCEL_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 20px',
  background: 'var(--surface-input, #444)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, transparent)',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-menu-item, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  cursor: 'pointer',
}

const CONFIRM_BUTTON_STYLE: React.CSSProperties = {
  ...CANCEL_BUTTON_STYLE,
  background: 'var(--accent-base, #3a5e9e)',
  color: 'var(--accent-text-strong, #fff)',
  border: 'none',
}

export function SaveFormatDialog({ initial, initialCompat = false, onChoose, onCancel }: SaveFormatDialogProps) {
  const { t } = useLingui()
  const [format, setFormat] = useState<DocFormat>(initial)
  const [compat, setCompat] = useState(initialCompat)
  // The older releases share one radio and a list; it keeps the last release
  // picked, starting on the file's own.
  const [older, setOlder] = useState(olderReleaseFor(initial))
  const olderChosen = isSkpStreamFormat(format)
  const answer = (): SaveFormatAnswer => ({ format, compat: takesCompat(format) && compat })

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (isComposingKey(e)) return
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onCancel()
      } else if (e.key === 'Enter') {
        // A focused button (Cancel, Continue) handles its own Enter.
        if (e.target instanceof HTMLButtonElement) return
        e.preventDefault()
        e.stopPropagation()
        onChoose({ format, compat: takesCompat(format) && compat })
      }
    },
    [onCancel, onChoose, format, compat],
  )

  useEffect(() => {
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  return (
    <div style={OVERLAY_STYLE} onClick={onCancel}>
      <div
        style={DIALOG_STYLE}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={t`Save As`}
      >
        <div style={HEADING_STYLE}>
          <Trans>Save As</Trans>
        </div>
        <div role="radiogroup" aria-label={t`File format`}>
          {SAVE_FORMAT_CHOICES.filter((c) => c.group === undefined).map((choice) => (
            <Fragment key={choice.format}>
            <label style={OPTION_STYLE}>
              <input
                type="radio"
                name="save-format"
                value={choice.format}
                checked={format === choice.format}
                onChange={() => setFormat(choice.format)}
                autoFocus={choice.format === initial}
              />
              <span>
                <div style={OPTION_LABEL_STYLE}>{choice.label}</div>
                <div style={OPTION_HINT_STYLE}>{choice.hint}</div>
              </span>
            </label>
            {takesCompat(choice.format) && format === choice.format && (
              <label style={{ ...OPTION_STYLE, paddingLeft: '30px', alignItems: 'center' }}>
                <input
                  type="checkbox"
                  checked={compat}
                  onChange={(e) => setCompat(e.target.checked)}
                  data-testid="save-skp-compat"
                />
                <span style={OPTION_HINT_STYLE}>{compatLabel()}</span>
              </label>
            )}
            </Fragment>
          ))}
          <label style={OPTION_STYLE}>
            <input
              type="radio"
              name="save-format"
              value="older"
              checked={olderChosen}
              onChange={() => setFormat(older)}
              autoFocus={isSkpStreamFormat(initial)}
            />
            <span>
              <div style={OPTION_LABEL_STYLE}>{olderGroupLabel()}</div>
              <select
                aria-label={t`Older SketchUp version`}
                data-testid="save-older-release"
                value={older}
                onChange={(e) => {
                  const release = olderReleaseFor(e.target.value as DocFormat)
                  setOlder(release)
                  setFormat(release)
                }}
                style={{ marginTop: '4px' }}
              >
                {SAVE_FORMAT_CHOICES.filter((c) => c.group === 'older').map((choice) => (
                  <option key={choice.format} value={choice.format}>
                    {choice.label}
                  </option>
                ))}
              </select>
            </span>
          </label>
        </div>
        <div style={BUTTON_ROW_STYLE}>
          <button style={CANCEL_BUTTON_STYLE} onClick={onCancel}>
            <Trans>Cancel</Trans>
          </button>
          <button style={CONFIRM_BUTTON_STYLE} onClick={() => onChoose(answer())}>
            <Trans>Continue</Trans>
          </button>
        </div>
      </div>
    </div>
  )
}
