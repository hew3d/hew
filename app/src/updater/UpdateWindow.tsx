/**
 * UpdateWindow — the desktop shell's update window (`index.html#update`,
 * opened by the shell when a check finds an update; docs/design D3). It is
 * the only UI for an available update. What it offers follows the install
 * plan the shell reports:
 *
 *   selfInstall          Install Update, plus the automatic-updates checkbox
 *                        where the shell says it can work
 *   selfInstallElevated  Install Update, and a line that the computer will
 *                        ask for a password (no checkbox)
 *   notifyOnly           Update Help, the sentence naming how Hew was
 *                        installed, and the command with a Copy button
 *                        (install is never called)
 *   readOnly             "What's New": the notes and Close only
 *
 * Phases: offer -> progress bar -> restart step (or the Open Download Page
 * failure; a dismissed password prompt goes back to the offer). Where the
 * installer closes Hew (Windows) the download stops at the "ready" step and
 * "Install and Restart" runs it. Restart Now / Install and Restart first
 * close every document through its save prompt (the shell does this); the
 * buttons wait meanwhile, and a Cancel in any prompt brings the step back
 * with a line saying the update is still there. Closing the
 * window or pressing Escape during the offer is "Remind Me Later" (done in
 * the shell); the buttons call the shell's commands and the shell closes the
 * window. Release notes are untrusted Markdown, rendered by releaseNotes.tsx.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import type { CSSProperties } from 'react'
import { t } from '@lingui/core/macro'
import { Trans, useLingui } from '@lingui/react/macro'
import { openExternal } from '../panels/ReportBugDialog'
import { useAutoUpdateSetting } from '../settings/autoUpdate'
import { ReleaseNotes } from './releaseNotes'
import {
  onEvent,
  updateCopyCommand,
  updateInfo,
  updateInstall,
  updateRemind,
  updateRestart,
  updateSkip,
  type InstallChannel,
  type InstallResult,
  type UpdateInfo,
  type UpdateProgress,
} from './updateIpc'

/** The sentence naming how this copy was installed (tell-only rows). Same
 *  wording the native dialog had. */
export function channelSentence(channel: InstallChannel): string {
  switch (channel) {
    case 'flatpak':
      return t`This copy of Hew was installed with Flatpak, so Flatpak updates it. To update, run this in a terminal:`
    case 'pacman':
      return t`This copy of Hew was installed with pacman, so pacman updates it. To update, run this in a terminal:`
    case 'gentoo':
      return t`This copy of Hew was installed with Portage, so Portage updates it. To update, run this in a terminal:`
    default:
      return t`This copy of Hew can’t update itself. Update it the same way you installed it, or get the new version from the Download page.`
  }
}

const PAGE: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  height: '100vh',
  boxSizing: 'border-box',
  padding: '18px 20px 16px',
  gap: '12px',
  background: 'var(--surface-base, #1e1e1e)',
  color: 'var(--text-primary, #eee)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  fontSize: '13px',
}

const NOTES_BOX: CSSProperties = {
  flex: 1,
  minHeight: '80px',
  overflowY: 'auto',
  padding: '8px 12px',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '6px',
  background: 'var(--surface-input, #2a2a2a)',
  lineHeight: 1.45,
}

const BUTTON: CSSProperties = {
  padding: '5px 16px',
  background: 'var(--surface-input, #444)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, transparent)',
  borderRadius: 'var(--radius-control, 6px)',
  fontSize: '13px',
  fontFamily: 'inherit',
  cursor: 'pointer',
}

const PRIMARY: CSSProperties = {
  ...BUTTON,
  background: 'var(--accent-base, #3a5e9e)',
  color: 'var(--accent-text-strong, #fff)',
  border: 'none',
}

const LINK: CSSProperties = {
  color: 'var(--accent-text, var(--accent-base, #5b8cff))',
  textDecoration: 'underline',
  cursor: 'pointer',
  background: 'none',
  border: 'none',
  padding: 0,
  font: 'inherit',
}

const MUTED: CSSProperties = { color: 'var(--text-secondary, #bbb)' }

/** A button that waits on the shell. */
const WAITING: CSSProperties = { opacity: 0.6, cursor: 'default' }

export type Step =
  | { kind: 'offer' }
  | { kind: 'downloading'; downloaded: number; total: number | null }
  | { kind: 'installing'; closesApp: boolean }
  | {
      kind: 'restart'
      /** Downloaded, not installed: the installer closes Hew (Windows). */
      ready: boolean
      /** The shell is closing the documents before the restart. */
      pending?: boolean
      /** The last restart was cancelled from a save prompt. */
      cancelled?: boolean
    }
  | { kind: 'failed'; message: string }

function stepFor(info: UpdateInfo): Step {
  if (info.phase === 'offer' && info.error !== null) return { kind: 'failed', message: info.error }
  if (info.phase === 'downloading') return { kind: 'downloading', downloaded: 0, total: null }
  if (info.phase === 'installed') return { kind: 'restart', ready: false }
  if (info.phase === 'ready') return { kind: 'restart', ready: true }
  return { kind: 'offer' }
}

/**
 * The step after the shell's `update_info` (on open, or on `update-changed`).
 * The shell's phase decides, but what only this window knows survives while
 * that phase has not moved on: a restart waiting on the documents (buttons
 * held, "Closing your documents…") or just cancelled, and the progress of a
 * running download or install. Without this, a Check for Updates or What's
 * New during a restart re-enabled Restart Now mid-pass.
 */
export function stepAfterChange(current: Step, next: Step): Step {
  if (current.kind === 'restart' && next.kind === 'restart' && current.ready === next.ready) return current
  if ((current.kind === 'downloading' || current.kind === 'installing') && next.kind === 'downloading') return current
  return next
}

/** The step an install's answer leads to (the command's return value and
 *  the `update-result` event carry the same answer). */
function stepAfterInstall(r: InstallResult): Step {
  if (r.status === 'installed') return { kind: 'restart', ready: false }
  if (r.status === 'ready') return { kind: 'restart', ready: true }
  if (r.status === 'cancelled') return { kind: 'offer' }
  return { kind: 'failed', message: r.error ?? '' }
}

async function closeThisWindow(): Promise<void> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  await getCurrentWindow().close()
}

export function UpdateWindow() {
  const { t: tr } = useLingui()
  const [info, setInfo] = useState<UpdateInfo | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [step, setStep] = useState<Step>({ kind: 'offer' })
  const [copied, setCopied] = useState(false)
  const [copyFailed, setCopyFailed] = useState(false)
  const auto = useAutoUpdateSetting()
  const stepRef = useRef(step)
  stepRef.current = step
  const infoRef = useRef(info)
  infoRef.current = info

  const load = useCallback(() => {
    updateInfo()
      .then((next) => {
        setInfo(next)
        setStep((current) => stepAfterChange(current, stepFor(next)))
        setLoadError(null)
      })
      .catch((err: unknown) => setLoadError(String(err)))
  }, [])

  useEffect(() => {
    load()
    const unsubs: Promise<() => void>[] = [
      onEvent<UpdateProgress>('update-progress', (p) => {
        if (p.phase === 'downloading') {
          setStep({ kind: 'downloading', downloaded: p.downloaded, total: p.total })
        } else {
          setStep({ kind: 'installing', closesApp: p.closesApp })
        }
      }),
      onEvent<InstallResult>('update-result', (r) => setStep(stepAfterInstall(r))),
      onEvent<null>('update-changed', () => load()),
    ]
    return () => {
      for (const u of unsubs) void u.then((fn) => fn()).catch(() => {})
    }
  }, [load])

  const readOnly = info?.readOnly === true
  const offerOpen = !readOnly && step.kind === 'offer'

  // Escape: during the open offer it is Remind Me Later; on the quiet steps
  // (read-only, restart, failure) it just closes. Not while a download or
  // install is running.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return
      const s = stepRef.current
      if (s.kind === 'downloading' || s.kind === 'installing') return
      if (s.kind === 'restart' && s.pending === true) return
      e.preventDefault()
      if (infoRef.current?.readOnly !== true && s.kind === 'offer') void updateRemind().catch(() => {})
      else void closeThisWindow().catch(() => {})
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const install = useCallback(() => {
    setStep({ kind: 'downloading', downloaded: 0, total: null })
    updateInstall()
      // The event normally got here first; the return value is the same answer.
      .then((r) => setStep(stepAfterInstall(r)))
      .catch((err: unknown) => setStep({ kind: 'failed', message: String(err) }))
  }, [])

  // The shell closes every document (each through its save prompt), then
  // restarts or runs the installer, and only answers when that did not
  // happen. A failed installer also arrives as an `update-result` event.
  const restart = useCallback((ready: boolean) => {
    setStep({ kind: 'restart', ready, pending: true })
    updateRestart()
      .then((r) => {
        setStep((s) =>
          s.kind === 'restart' ? { kind: 'restart', ready, cancelled: r.status === 'cancelled' } : s,
        )
      })
      .catch((err: unknown) => {
        setStep((s) => (s.kind === 'failed' ? s : { kind: 'failed', message: String(err) }))
      })
  }, [])

  const copyCommand = useCallback(() => {
    updateCopyCommand()
      .then(() => {
        setCopyFailed(false)
        setCopied(true)
        window.setTimeout(() => setCopied(false), 1500)
      })
      // No clipboard (some Linux sessions): say so; the command is
      // selectable text and Update Help is beside it.
      .catch(() => setCopyFailed(true))
  }, [])

  if (info === null) {
    return (
      <div style={PAGE} data-testid="update-window">
        {loadError !== null && (
          <div role="alert" style={{ color: 'var(--danger-text, #e88)' }}>
            {loadError}
          </div>
        )}
      </div>
    )
  }

  const { plan, version, currentVersion } = info
  const tellOnly = plan.mode === 'notifyOnly'
  const elevated = plan.mode === 'selfInstallElevated'
  const running = step.kind === 'downloading' || step.kind === 'installing'
  const showNotes = step.kind === 'offer' || step.kind === 'failed' || readOnly
  const heading = readOnly
    ? tr`What’s new in Hew ${version}`
    : step.kind === 'restart' && !step.ready
      ? tr`Hew ${version} is installed`
      : tr`A new version of Hew is available!`

  let message: string
  if (step.kind === 'restart') {
    message = step.ready
      ? tr`Hew ${version} is ready to install. Hew will close to finish installing.`
      : tr`Hew ${version} is installed. Restart now to use it?`
  } else if (tellOnly) {
    message = tr`Hew ${version} is available — you have ${currentVersion}.`
  } else {
    message = tr`Hew ${version} is now available. You have ${currentVersion}. Would you like to update now?`
  }

  const percent =
    step.kind === 'downloading' && step.total !== null && step.total > 0
      ? Math.min(100, Math.round((step.downloaded / step.total) * 100))
      : null

  return (
    <div style={PAGE} data-testid="update-window">
      <div>
        <div style={{ fontSize: '15px', fontWeight: 600, marginBottom: '6px' }} data-testid="update-heading">
          {heading}
        </div>
        {!readOnly && <div data-testid="update-message">{message}</div>}
        {tellOnly && !readOnly && step.kind === 'offer' && (
          <div style={{ marginTop: '8px' }}>
            <div data-testid="update-channel">{channelSentence(plan.channel)}</div>
            {plan.command !== null && (
              <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginTop: '6px' }}>
                <code
                  data-testid="update-command"
                  style={{
                    flex: 1,
                    padding: '4px 8px',
                    background: 'var(--surface-input, #2a2a2a)',
                    border: '1px solid var(--border-strong, #444)',
                    borderRadius: '6px',
                    fontFamily: 'var(--font-family-mono, ui-monospace, monospace)',
                    userSelect: 'text',
                  }}
                >
                  {plan.command}
                </code>
                <button type="button" style={BUTTON} data-testid="update-copy" onClick={copyCommand}>
                  {copied ? tr`Copied` : tr`Copy Command`}
                </button>
              </div>
            )}
            {copyFailed && (
              <div role="alert" style={{ ...MUTED, marginTop: '6px' }} data-testid="update-copy-failed">
                <Trans>Couldn’t copy the command. Select it above and copy it, or click Update Help.</Trans>
              </div>
            )}
          </div>
        )}
      </div>

      {showNotes && (
        <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0, gap: '4px' }}>
          <div style={MUTED}>
            <Trans>Release notes</Trans>
          </div>
          <div style={NOTES_BOX} data-testid="update-notes" tabIndex={0}>
            {info.notes !== null && info.notes.trim() !== '' ? (
              <ReleaseNotes markdown={info.notes} />
            ) : (
              <span style={MUTED}>
                <Trans>Release notes are on the Hew releases page.</Trans>
              </span>
            )}
          </div>
          <div>
            <button type="button" style={LINK} data-testid="update-all-notes" onClick={() => openExternal(info.changelogUrl)}>
              {tr`All release notes ↗`}
            </button>
          </div>
        </div>
      )}
      {!showNotes && <div style={{ flex: 1 }} />}

      {step.kind === 'failed' && (
        <div role="alert" data-testid="update-error" style={{ color: 'var(--danger-text, #e88)' }}>
          <div>
            <Trans>The update could not be installed.</Trans>
          </div>
          {step.message !== '' && <div style={{ marginTop: '4px', userSelect: 'text' }}>{step.message}</div>}
          <div style={{ marginTop: '4px', color: 'var(--text-primary, #eee)' }}>
            <Trans>You can download the new version from the Download page instead.</Trans>
          </div>
        </div>
      )}

      {step.kind === 'downloading' && (
        <div data-testid="update-progress">
          <div style={{ marginBottom: '4px' }}>{tr`Downloading Hew ${version}…`}</div>
          <progress
            style={{ width: '100%' }}
            max={percent === null ? undefined : 100}
            value={percent === null ? undefined : percent}
            aria-label={tr`Download progress`}
          />
        </div>
      )}
      {step.kind === 'installing' && (
        <div data-testid="update-installing">
          {step.closesApp ? tr`Hew will close to finish installing.` : tr`Installing Hew ${version}…`}
        </div>
      )}

      {step.kind === 'restart' && step.pending === true && (
        <div data-testid="update-restart-pending">{tr`Closing your documents…`}</div>
      )}
      {step.kind === 'restart' && step.cancelled === true && (
        <div style={MUTED} data-testid="update-restart-cancelled">
          {step.ready
            ? tr`Install cancelled. Hew ${version} is downloaded and ready when you are.`
            : tr`Restart cancelled. Hew ${version} will open the next time you start Hew.`}
        </div>
      )}

      {elevated && offerOpen && !running && (
        <div style={MUTED} data-testid="update-password-note">
          <Trans>Your computer will ask for your password to install it.</Trans>
        </div>
      )}
      {offerOpen && auto.visible && plan.mode === 'selfInstall' && (
        <label style={{ display: 'flex', alignItems: 'center', gap: '7px', cursor: 'pointer' }}>
          <input
            type="checkbox"
            data-testid="update-auto"
            checked={auto.enabled}
            onChange={(e) => auto.setEnabled(e.target.checked)}
            style={{ accentColor: 'var(--accent-base, #5b8cff)', margin: 0 }}
          />
          <Trans>Automatically download and install updates in the future</Trans>
        </label>
      )}

      {!running && (
        <div style={{ display: 'flex', justifyContent: 'space-between', gap: '8px' }}>
          {readOnly ? (
            <>
              <span />
              <button type="button" style={PRIMARY} data-testid="update-close" onClick={() => void closeThisWindow().catch(() => {})}>
                {tr`Close`}
              </button>
            </>
          ) : step.kind === 'restart' ? (
            <>
              <button
                type="button"
                style={step.pending === true ? { ...BUTTON, ...WAITING } : BUTTON}
                data-testid="update-later"
                disabled={step.pending === true}
                onClick={() => void closeThisWindow().catch(() => {})}
              >
                {tr`Later`}
              </button>
              <button
                type="button"
                style={step.pending === true ? { ...PRIMARY, ...WAITING } : PRIMARY}
                data-testid="update-restart"
                disabled={step.pending === true}
                onClick={() => restart(step.ready)}
              >
                {step.ready ? tr`Install and Restart` : tr`Restart Now`}
              </button>
            </>
          ) : step.kind === 'failed' ? (
            <>
              <button type="button" style={BUTTON} data-testid="update-later" onClick={() => void updateRemind().catch(() => {})}>
                {tr`Remind Me Later`}
              </button>
              <button type="button" style={PRIMARY} data-testid="update-download" onClick={() => openExternal(info.downloadUrl)}>
                {tr`Open Download Page`}
              </button>
            </>
          ) : (
            <>
              <button type="button" style={BUTTON} data-testid="update-skip" onClick={() => void updateSkip().catch(() => {})}>
                {tr`Skip This Version`}
              </button>
              <div style={{ display: 'flex', gap: '8px' }}>
                <button type="button" style={BUTTON} data-testid="update-remind" onClick={() => void updateRemind().catch(() => {})}>
                  {tr`Remind Me Later`}
                </button>
                {tellOnly ? (
                  <button type="button" style={PRIMARY} data-testid="update-help" onClick={() => openExternal(plan.helpUrl)}>
                    {tr`Update Help`}
                  </button>
                ) : (
                  <button type="button" style={PRIMARY} data-testid="update-install" onClick={install}>
                    {tr`Install Update`}
                  </button>
                )}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  )
}
