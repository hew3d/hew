/**
 * ReportBugDialog — docs/design/report-bug.md §2. `io/reportClient.ts` is
 * mocked (its own status→kind mapping and origin gating are covered by
 * reportClient.test.ts); this file covers the dialog's own contract: the
 * default checklist, the geometry note, description validation, the
 * sent/failed states, self-hosted web hiding Send report, and crash mode's
 * unavailable model row.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ReportBugDialog, excerptOf } from './ReportBugDialog'
import { ReportError } from '../io/reportClient'
import type { ReportableScene } from '../log/reportBundle'
import { beginOpenAttempt, clearOpenAttempt, openAttemptFailed } from '../log/failedOpen'

const mockCanSend = vi.hoisted(() => vi.fn(() => true))
const mockSubmitReport = vi.hoisted(() => vi.fn())
vi.mock('../io/reportClient', () => {
  class MockReportError extends Error {
    readonly kind: string
    readonly status?: number
    constructor(kind: string, message: string, status?: number) {
      super(message)
      this.name = 'ReportError'
      this.kind = kind
      this.status = status
    }
  }
  return { canSend: mockCanSend, submitReport: mockSubmitReport, ReportError: MockReportError }
})

// Default resolved value (a plain filename, as the web host would return) is
// set fresh in beforeEach below; desktop-path tests override it with an
// absolute path.
const mockExportBinary = vi.hoisted(() => vi.fn())
// `isTauri` is exposed as a getter reading this mutable flag (default web:
// false) rather than the real module's frozen `const`, so the homeDir-race
// tests below can flip the dialog into "desktop" mode without a
// resetModules/dynamic-import dance for the whole file. Wrapped in an object
// (not a bare `let`) because `vi.mock` factories are hoisted above every
// other statement, including a `let`'s own initializer — a factory that
// closed over a bare `let mockIsTauriValue` would read it while still in the
// temporal dead zone the moment some OTHER mocked module eagerly touches
// `isTauri` at its own top level (settings/debugMode.ts does). `vi.hoisted`
// runs its callback at that same hoisted point, sidestepping the TDZ.
const mockIsTauri = vi.hoisted(() => ({ value: false }))
vi.mock('../io/fileHost', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../io/fileHost')>()
  return {
    ...actual,
    get isTauri() {
      return mockIsTauri.value
    },
    makeFileHost: () => ({ exportBinary: mockExportBinary }),
  }
})

const mockHomeDir = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/path', () => ({ homeDir: mockHomeDir }))

const mockRevealItemInDir = vi.hoisted(() => vi.fn().mockResolvedValue(undefined))
const mockOpenUrl = vi.hoisted(() => vi.fn().mockResolvedValue(undefined))
vi.mock('@tauri-apps/plugin-opener', () => ({ revealItemInDir: mockRevealItemInDir, openUrl: mockOpenUrl }))

function fakeScene(overrides: Partial<ReportableScene> = {}): ReportableScene {
  return {
    save: () => new Uint8Array([1, 2, 3]),
    peek_recording: () => '{"version":2,"calls":["Foo","Bar"],"golden_hash":0}',
    object_ids: () => ({ length: 3 }),
    ...overrides,
  }
}

async function typeValidDescription() {
  fireEvent.change(screen.getByLabelText(/what happened/i), {
    target: { value: 'It crashed when I pushed the face outward.' },
  })
}

/** Send report once it's enabled: it waits for the report to be gathered
 *  and its compressed size measured for the checklist on screen. */
async function readySendButton(): Promise<HTMLElement> {
  const send = screen.getByRole('button', { name: /^send report$/i })
  await waitFor(() => expect(send).not.toBeDisabled())
  return send
}

/** Save to file… once it's enabled: it waits for the report to be gathered. */
async function readySaveButton(): Promise<HTMLElement> {
  const save = screen.getByRole('button', { name: /save to file/i })
  await waitFor(() => expect(save).not.toBeDisabled())
  return save
}

beforeEach(() => {
  mockCanSend.mockReset().mockReturnValue(true)
  mockSubmitReport.mockReset()
  mockExportBinary.mockReset().mockResolvedValue('hew-bug-report.json')
  mockHomeDir.mockReset()
  mockRevealItemInDir.mockReset().mockResolvedValue(undefined)
  mockOpenUrl.mockReset().mockResolvedValue(undefined)
  mockIsTauri.value = false
})

afterEach(() => {
  vi.restoreAllMocks()
  mockIsTauri.value = false
})

describe('ReportBugDialog — default checklist', () => {
  it('shows the four always-listed rows ticked, and hides Raw input events without Debug Mode', async () => {
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)

    for (const label of ['App version and system', 'Recorded steps', 'Model file', 'Diagnostic log']) {
      const checkbox = await screen.findByRole('checkbox', { name: label })
      expect(checkbox).toBeChecked()
    }
    expect(screen.queryByRole('checkbox', { name: 'Raw input events' })).not.toBeInTheDocument()
  })

  it('lists the file that didn’t open, ticked, only when there is one', async () => {
    clearOpenAttempt()
    const { unmount } = render(
      <ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />,
    )
    await screen.findByRole('checkbox', { name: 'Model file' })
    expect(screen.queryByRole('checkbox', { name: 'File that didn’t open' })).not.toBeInTheDocument()
    unmount()

    beginOpenAttempt({ name: 'Kitchen.skp', label: 'SketchUp model (.skp)', bytes: new Uint8Array([1, 2, 3]) })
    openAttemptFailed('SKP: unsupported version')
    try {
      render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
      expect(await screen.findByRole('checkbox', { name: 'File that didn’t open' })).toBeChecked()
    } finally {
      clearOpenAttempt()
    }
  })

  it('shows the geometry note', () => {
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    expect(
      screen.getByText(/recorded steps and the model file contain your model.s geometry/i),
    ).toBeInTheDocument()
  })

  it('unticking a row omits it from the row info shown as included', async () => {
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    const modelCheckbox = await screen.findByRole('checkbox', { name: 'Model file' })
    expect(modelCheckbox).toBeChecked()
    await waitFor(() => expect(modelCheckbox).not.toBeDisabled())
    fireEvent.click(modelCheckbox)
    await waitFor(() => expect(modelCheckbox).not.toBeChecked())
  })
})

describe('ReportBugDialog — gathering and busy states', () => {
  it('opens on "Gathering details…" with the checklist and both actions waiting, then enables them', async () => {
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    expect(screen.getByText('Gathering details…')).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: 'Model file' })).toBeDisabled()
    expect(screen.getByRole('button', { name: /save to file/i })).toBeDisabled()
    expect(screen.getByRole('button', { name: /^send report$/i })).toBeDisabled()

    await readySaveButton()
    expect(screen.queryByText('Gathering details…')).not.toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: 'Model file' })).not.toBeDisabled()
    expect(await screen.findByText(/about .* to send, compressed/i)).toBeInTheDocument()
  })

  it('reads the recording once, however many rows are toggled and whether it sends', async () => {
    mockSubmitReport.mockResolvedValue({ id: 'HEW-ONCE-0001' })
    let peeked = 0
    const scene = fakeScene({
      peek_recording: () => {
        peeked++
        return '{"version":2,"calls":["Foo"],"golden_hash":0}'
      },
    })
    render(<ReportBugDialog scene={scene} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    await readySendButton()
    const model = screen.getByRole('checkbox', { name: 'Model file' })
    fireEvent.click(model)
    fireEvent.click(model)
    fireEvent.click(await readySendButton())
    await screen.findByText(/HEW-ONCE-0001/)
    expect(peeked).toBe(1)
  })

  it('shows Sending… and locks the dialog at the click, before the report is compressed', async () => {
    mockSubmitReport.mockReturnValue(new Promise(() => {}))
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    fireEvent.click(await readySendButton())
    expect(screen.getByRole('button', { name: /^sending…$/i })).toBeDisabled()
    expect(screen.getByRole('checkbox', { name: 'Model file' })).toBeDisabled()
    expect(screen.getByText('Compressing the report…')).toBeInTheDocument()
  })

  it('shows Saving… at the click, before the file is built', async () => {
    mockExportBinary.mockReturnValue(new Promise(() => {}))
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    fireEvent.click(await readySaveButton())
    expect(screen.getByRole('button', { name: /^saving…$/i })).toBeDisabled()
    expect(screen.getByRole('checkbox', { name: 'Model file' })).toBeDisabled()
    expect(screen.getByText('Saving the report…')).toBeInTheDocument()
    expect(screen.queryByText('Measuring the compressed size…')).not.toBeInTheDocument()
  })

  it('does not compress the report again when a row is unticked and ticked back before it measures', async () => {
    const Real = globalThis.CompressionStream
    let made = 0
    vi.stubGlobal(
      'CompressionStream',
      class extends Real {
        constructor(format: CompressionFormat) {
          super(format)
          made++
        }
      },
    )
    try {
      render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
      expect(await screen.findByText(/about .* to send, compressed/i)).toBeInTheDocument()
      expect(made).toBe(1)
      const model = screen.getByRole('checkbox', { name: 'Model file' })
      fireEvent.click(model)
      fireEvent.click(model)
      await new Promise((resolve) => setTimeout(resolve, 300))
      expect(made).toBe(1)
      expect(screen.getByText(/about .* to send, compressed/i)).toBeInTheDocument()
    } finally {
      vi.unstubAllGlobals()
    }
  })
})

describe('ReportBugDialog — validation', () => {
  it('disables Send report until the description reaches 10 characters', async () => {
    render(<ReportBugDialog scene={fakeScene()} documentName="x" crash={null} onClose={vi.fn()} />)
    const send = await screen.findByRole('button', { name: /^send report$/i })
    expect(send).toBeDisabled()

    fireEvent.change(screen.getByLabelText(/what happened/i), { target: { value: 'short' } })
    expect(send).toBeDisabled()

    await typeValidDescription()
    await waitFor(() => expect(send).not.toBeDisabled())
  })

  it('keeps Send report disabled until the preview matches the checklist on screen', async () => {
    render(<ReportBugDialog scene={fakeScene()} documentName="x" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    const send = screen.getByRole('button', { name: /^send report$/i })
    expect(send).toBeDisabled()
    await waitFor(() => expect(send).not.toBeDisabled())

    fireEvent.click(screen.getByRole('checkbox', { name: 'Model file' }))
    expect(send).toBeDisabled()
    await waitFor(() => expect(send).not.toBeDisabled())
  })
})

describe('ReportBugDialog — sending', () => {
  it('shows the sent state with the ID, Copy, and a GitHub issue link carrying description/version/platform/ID', async () => {
    mockSubmitReport.mockResolvedValue({ id: 'HEW-7K3F-Q9XB' })
    const openSpy = vi.spyOn(window, 'open').mockImplementation(() => null)

    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    fireEvent.click(await readySendButton())

    expect(await screen.findByText(/HEW-7K3F-Q9XB/)).toBeInTheDocument()
    expect(screen.getByRole('status')).toHaveTextContent('Report sent')

    fireEvent.click(screen.getByRole('button', { name: /also open a public github issue/i }))
    expect(openSpy).toHaveBeenCalledTimes(1)
    const url = openSpy.mock.calls[0][0] as string
    expect(url.startsWith('https://github.com/hew3d/hew/issues/new?')).toBe(true)
    expect(url).toContain('HEW-7K3F-Q9XB')
    expect(decodeURIComponent(url)).toContain('It crashed when I pushed the face outward.')
    expect(decodeURIComponent(url)).toContain('Version:')
    expect(decodeURIComponent(url)).toContain('Platform:')
    expect(url.length).toBeLessThan(6100)
  })

  it('shows a worded failure and keeps Save to file, preserving the typed text', async () => {
    mockSubmitReport.mockRejectedValue(new ReportError('rateLimited', 'too many'))

    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    fireEvent.click(await readySendButton())

    expect(await screen.findByRole('alert')).toHaveTextContent(/too many reports sent recently/i)
    expect(screen.getByRole('button', { name: /save to file/i })).toBeInTheDocument()
    expect(screen.getByLabelText(/what happened/i)).toHaveValue('It crashed when I pushed the face outward.')
  })

  it('Save to file… builds and writes the bundle without requiring Send to be available', async () => {
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    fireEvent.click(await readySaveButton())

    await waitFor(() => expect(mockExportBinary).toHaveBeenCalledTimes(1))
    expect(mockExportBinary.mock.calls[0][2]).toMatchObject({ ext: 'json', mime: 'application/json' })
  })
})

describe('ReportBugDialog — Save to file… completion', () => {
  it('replaces the form with a saved confirmation naming the file on the web, with no reveal button', async () => {
    mockIsTauri.value = false
    mockExportBinary.mockResolvedValue('hew-bug-report-2026-01-01.json')

    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    fireEvent.click(await readySaveButton())

    expect(await screen.findByText(/report saved/i)).toBeInTheDocument()
    expect(screen.getByText(/saved as "hew-bug-report-2026-01-01\.json"/i)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /show in/i })).not.toBeInTheDocument()
    // The form (description textarea) is gone, same as the sent state.
    expect(screen.queryByLabelText(/what happened/i)).not.toBeInTheDocument()
  })

  it('shows the full path with the home directory scrubbed to ~, plus a platform-worded reveal button, on desktop', async () => {
    mockIsTauri.value = true
    mockHomeDir.mockResolvedValue('/Users/kurt')
    mockExportBinary.mockResolvedValue('/Users/kurt/Desktop/hew-bug-report-2026-01-01.json')

    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    await waitFor(() => expect(screen.getByRole('button', { name: /save to file/i })).not.toBeDisabled())
    fireEvent.click(await readySaveButton())

    expect(await screen.findByText(/report saved/i)).toBeInTheDocument()
    expect(screen.getByText('~/Desktop/hew-bug-report-2026-01-01.json')).toBeInTheDocument()

    const reveal = screen.getByRole('button', { name: /^show in/i })
    fireEvent.click(reveal)
    await waitFor(() =>
      expect(mockRevealItemInDir).toHaveBeenCalledWith('/Users/kurt/Desktop/hew-bug-report-2026-01-01.json'),
    )
  })

  it('offers "Open a public GitHub issue" with no ID and a line asking to attach the file', async () => {
    mockIsTauri.value = false
    mockExportBinary.mockResolvedValue('hew-bug-report.json')
    const openSpy = vi.spyOn(window, 'open').mockImplementation(() => null)

    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    fireEvent.click(await readySaveButton())
    await screen.findByText(/report saved/i)

    fireEvent.click(screen.getByRole('button', { name: /^open a public github issue$/i }))
    expect(openSpy).toHaveBeenCalledTimes(1)
    const url = openSpy.mock.calls[0][0] as string
    const decoded = decodeURIComponent(url)
    expect(decoded).not.toContain('Report ID:')
    expect(decoded).toContain('attach the saved report file')
  })

  it('returns silently to the form on a cancelled picker — no error, no saved confirmation', async () => {
    mockExportBinary.mockResolvedValue(null)

    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    fireEvent.click(await readySaveButton())

    await waitFor(() => expect(mockExportBinary).toHaveBeenCalledTimes(1))
    expect(screen.queryByText(/report saved/i)).not.toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    // The form is still here, and Save to file… is clickable again.
    expect(screen.getByLabelText(/what happened/i)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /save to file/i })).not.toBeDisabled()
  })

  it('shows an error and keeps the form when the write fails', async () => {
    mockExportBinary.mockRejectedValue(new Error('disk full'))

    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    fireEvent.click(await readySaveButton())

    expect(await screen.findByRole('alert')).toHaveTextContent(/couldn.t save the file/i)
    expect(screen.getByLabelText(/what happened/i)).toBeInTheDocument()
  })

  // The playtest-found bug: a slow/hung write left "Saving…" showing forever
  // with no way out. This pins that build+write is ONE chain whose every
  // outcome — including a rejection arriving well after the click — settles
  // `saveState` out of 'saving'.
  it('"Saving…" always settles, even when the write rejects after a delay', async () => {
    let rejectWrite!: (err: Error) => void
    // Created when the write actually starts: Save builds (and gzips) the
    // bundle first, so a promise rejected any earlier would have no handler.
    mockExportBinary.mockImplementation(
      () =>
        new Promise((_resolve, reject) => {
          rejectWrite = reject
        }),
    )

    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    fireEvent.click(await readySaveButton())

    expect(await screen.findByRole('button', { name: /^saving…$/i })).toBeDisabled()
    await waitFor(() => expect(mockExportBinary).toHaveBeenCalledTimes(1))

    rejectWrite(new Error('write hung then failed'))

    expect(await screen.findByRole('alert')).toHaveTextContent(/couldn.t save the file/i)
    expect(screen.getByRole('button', { name: /save to file/i })).not.toBeDisabled()
  })
})

describe('ReportBugDialog — self-hosted / no send route', () => {
  it('hides Send report and explains why when canSend() is false', async () => {
    mockCanSend.mockReturnValue(false)
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)

    expect(screen.queryByRole('button', { name: /^send report$/i })).not.toBeInTheDocument()
    expect(await screen.findByText(/this build can.t send reports directly/i)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /save to file/i })).toBeInTheDocument()
  })
})

describe('ReportBugDialog — crash mode', () => {
  it('marks the model row unavailable and never offers it', async () => {
    render(
      <ReportBugDialog
        scene={null}
        documentName=""
        crash={{ at: '2026-01-01T00:00:00.000Z', message: 'panicked at document.rs:1', recording: null }}
        onClose={vi.fn()}
      />,
    )

    // The row's availability comes from gathering, which finishes after the
    // first paint; until then every row is disabled but not yet described.
    expect(await screen.findByText(/unavailable after a crash/i)).toBeInTheDocument()
    const modelCheckbox = screen.getByRole('checkbox', { name: 'Model file' })
    expect(modelCheckbox).toBeDisabled()
    expect(modelCheckbox).not.toBeChecked()
    expect(screen.getByRole('dialog', { name: /report this crash/i })).toBeInTheDocument()
  })
})

// Regression: `homeDir` starts null and used to be irrelevant to whether
// Send/Save were enabled — a fast submit on desktop could race the async
// `homeDir()` lookup and send/save the log unscrubbed.
describe('ReportBugDialog — desktop homeDir race', () => {
  it('disables Send report and Save to file until the homeDir() lookup settles, then enables them', async () => {
    mockIsTauri.value = true
    let resolveHomeDir!: (dir: string) => void
    mockHomeDir.mockReturnValue(
      new Promise<string>((resolve) => {
        resolveHomeDir = resolve
      }),
    )

    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()

    const send = screen.getByRole('button', { name: /^send report$/i })
    const save = screen.getByRole('button', { name: /save to file/i })
    expect(send).toBeDisabled()
    expect(save).toBeDisabled()

    resolveHomeDir('/Users/kurt')
    await waitFor(() => expect(send).not.toBeDisabled())
    expect(save).not.toBeDisabled()
  })

  it('also enables once the lookup FAILS (best-effort, not stuck forever)', async () => {
    mockIsTauri.value = true
    mockHomeDir.mockRejectedValue(new Error('no home dir'))

    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()

    await waitFor(() => expect(screen.getByRole('button', { name: /^send report$/i })).not.toBeDisabled())
  })

  it('is ready immediately on the web (no lookup to wait for)', async () => {
    mockIsTauri.value = false
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    await readySendButton()
    expect(mockHomeDir).not.toHaveBeenCalled()
  })
})

describe('ReportBugDialog — GitHub issue link with non-BMP characters', () => {
  it('truncates a long emoji-laden description by code point, never throwing on a lone surrogate', async () => {
    mockSubmitReport.mockResolvedValue({ id: 'HEW-EMOJ-I001' })
    const openSpy = vi.spyOn(window, 'open').mockImplementation(() => null)

    // 4500 emoji code points = 9000 UTF-16 units (under the 10,000 max), but
    // each encodes to 12 URI-escaped characters — 54,000 encoded chars, far
    // past the ~6000-char URL budget, guaranteeing truncation kicks in. A
    // UTF-16-unit slice (the old bug) can land inside either half of any of
    // these surrogate pairs; a code-point slice (Array.from) never can.
    const emojiDescription = '😀'.repeat(4500)
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/what happened/i), { target: { value: emojiDescription } })

    fireEvent.click(await readySendButton())
    await screen.findByText(/HEW-EMOJ-I001/)

    expect(() =>
      fireEvent.click(screen.getByRole('button', { name: /also open a public github issue/i })),
    ).not.toThrow()

    expect(openSpy).toHaveBeenCalledTimes(1)
    const url = openSpy.mock.calls[0][0] as string
    expect(() => decodeURIComponent(url)).not.toThrow()
    expect(decodeURIComponent(url)).toContain('[truncated]')
    expect(decodeURIComponent(url)).not.toContain('�') // no mangled/replacement chars
  })
})

// Regression: Escape, an overlay click, and Cancel all called onClose
// unconditionally — closing mid-send loses the report ID the moment it
// would have arrived and risks a duplicate resend.
describe('ReportBugDialog — cannot close mid-send', () => {
  async function renderSending() {
    let resolveSubmit!: (v: { id: string }) => void
    mockSubmitReport.mockReturnValue(
      new Promise((resolve) => {
        resolveSubmit = resolve
      }),
    )
    const onClose = vi.fn()
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={onClose} />)
    await typeValidDescription()
    fireEvent.click(await readySendButton())
    await screen.findByRole('button', { name: /sending/i })
    return { onClose, resolveSubmit }
  }

  it('ignores Escape, the overlay click, and a disabled Cancel while sending', async () => {
    const { onClose, resolveSubmit } = await renderSending()

    const cancel = screen.getByRole('button', { name: /^cancel$/i })
    expect(cancel).toBeDisabled()
    fireEvent.click(cancel)
    expect(onClose).not.toHaveBeenCalled()

    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onClose).not.toHaveBeenCalled()

    const overlay = screen.getByRole('dialog', { name: /^report bug$/i }).parentElement as HTMLElement
    fireEvent.click(overlay)
    expect(onClose).not.toHaveBeenCalled()

    resolveSubmit({ id: 'HEW-BUSY-0001' })
    expect(await screen.findByText(/HEW-BUSY-0001/)).toBeInTheDocument()
  })

  it('locks the checklist and the text fields while sending', async () => {
    const { resolveSubmit } = await renderSending()
    expect(screen.getByRole('checkbox', { name: 'Model file' })).toBeDisabled()
    expect(screen.getByLabelText(/what happened/i)).toBeDisabled()
    expect(screen.getByLabelText(/what did you expect/i)).toBeDisabled()
    resolveSubmit({ id: 'HEW-LOCK-0001' })
    expect(await screen.findByText(/HEW-LOCK-0001/)).toBeInTheDocument()
  })

  it('does not update state (and does not warn) after unmounting mid-send', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    let resolveSubmit!: (v: { id: string }) => void
    mockSubmitReport.mockReturnValue(
      new Promise((resolve) => {
        resolveSubmit = resolve
      }),
    )
    const { unmount } = render(
      <ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />,
    )
    await typeValidDescription()
    fireEvent.click(await readySendButton())
    unmount()

    resolveSubmit({ id: 'HEW-GONE-0001' })
    await new Promise((r) => setTimeout(r, 0))

    const stateUpdateWarning = errorSpy.mock.calls.some((call) =>
      String(call[0]).includes('unmounted component'),
    )
    expect(stateUpdateWarning).toBe(false)
  })
})

describe('ReportBugDialog — imported files and the compressed send', () => {
  const importScene = () =>
    fakeScene({
      peek_recording: () =>
        '{"version":2,"calls":[{"method":"begin_ground_sketch"},{"method":"import_skp","bytes":[255,254,255,14,83]}],"golden_hash":18192258159662307868}',
    })

  it('lists Imported files, ticked, only when a recorded step embeds a file', async () => {
    const { unmount } = render(
      <ReportBugDialog scene={importScene()} documentName="House.hew" crash={null} onClose={vi.fn()} />,
    )
    expect(await screen.findByRole('checkbox', { name: 'Imported files' })).toBeChecked()
    unmount()

    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    await screen.findByRole('checkbox', { name: 'Recorded steps' })
    expect(screen.queryByRole('checkbox', { name: 'Imported files' })).not.toBeInTheDocument()
  })

  it('says how large the compressed upload will be', async () => {
    render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
    expect(await screen.findByText(/about .* to send, compressed/i)).toBeInTheDocument()
  })

  it('sends the gzip bytes, not the JSON text', async () => {
    mockSubmitReport.mockResolvedValue({ id: 'HEW-GZIP-0001' })
    render(<ReportBugDialog scene={importScene()} documentName="House.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    const send = screen.getByRole('button', { name: /^send report$/i })
    await waitFor(() => expect(send).not.toBeDisabled())
    fireEvent.click(send)

    await screen.findByText(/HEW-GZIP-0001/)
    const sent = mockSubmitReport.mock.calls[0][0] as Uint8Array
    expect(ArrayBuffer.isView(sent)).toBe(true)
    expect([sent[0], sent[1]]).toEqual([0x1f, 0x8b])
  })

  it('blocks Send with the reason when this runtime cannot compress, and keeps Save to file', async () => {
    vi.stubGlobal('CompressionStream', undefined)
    try {
      render(<ReportBugDialog scene={fakeScene()} documentName="My Model.hew" crash={null} onClose={vi.fn()} />)
      await typeValidDescription()
      expect(await screen.findByText(/can.t compress reports/i)).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /^send report$/i })).toBeDisabled()
      expect(screen.getByRole('button', { name: /save to file/i })).not.toBeDisabled()
    } finally {
      vi.unstubAllGlobals()
    }
  })
})

describe('ReportBugDialog — following the report', () => {
  const FOLLOW_LABEL = /let me know what happens with this report/i

  beforeEach(() => {
    localStorage.removeItem('hew.report.follow')
  })

  it('shows the checkbox ticked by default, only when Send is available', async () => {
    const { unmount } = render(<ReportBugDialog scene={fakeScene()} documentName="d.hew" crash={null} onClose={vi.fn()} />)
    expect(screen.getByRole('checkbox', { name: FOLLOW_LABEL })).toBeChecked()
    expect(screen.getByText(/no account, no email/i)).toBeInTheDocument()
    unmount()

    mockCanSend.mockReturnValue(false)
    render(<ReportBugDialog scene={fakeScene()} documentName="d.hew" crash={null} onClose={vi.fn()} />)
    expect(screen.queryByRole('checkbox', { name: FOLLOW_LABEL })).not.toBeInTheDocument()
  })

  it('remembers an unticked choice', () => {
    const { unmount } = render(<ReportBugDialog scene={fakeScene()} documentName="d.hew" crash={null} onClose={vi.fn()} />)
    fireEvent.click(screen.getByRole('checkbox', { name: FOLLOW_LABEL }))
    expect(localStorage.getItem('hew.report.follow')).toBe('0')
    unmount()
    render(<ReportBugDialog scene={fakeScene()} documentName="d.hew" crash={null} onClose={vi.fn()} />)
    expect(screen.getByRole('checkbox', { name: FOLLOW_LABEL })).not.toBeChecked()
  })

  it('passes follow, the first line, and the failed-open file name to submitReport', async () => {
    mockSubmitReport.mockResolvedValue({ id: 'HEW-7K3F-Q9XB', following: true })
    beginOpenAttempt({ name: 'Kitchen.skp', label: 'SketchUp model (.skp)', bytes: new Uint8Array([1, 2, 3]) })
    openAttemptFailed('bad')
    try {
      render(<ReportBugDialog scene={fakeScene()} documentName="d.hew" crash={null} onClose={vi.fn()} />)
      fireEvent.change(screen.getByLabelText(/what happened/i), { target: { value: 'First line here\nsecond line' } })
      fireEvent.click(await readySendButton())
      await screen.findByText(/HEW-7K3F-Q9XB/)
      expect(mockSubmitReport.mock.calls[0]![2]).toEqual({ follow: true, excerpt: 'First line here', fileName: 'Kitchen.skp' })
      expect(screen.getByTestId('report-follow-confirmation')).toHaveTextContent('Help ▸ Sent Reports…')
      expect(screen.getByRole('button', { name: /also open a public github issue/i })).toBeInTheDocument()
    } finally {
      clearOpenAttempt()
    }
  })

  it('omits the Sent Reports sentence when the report is not followed', async () => {
    mockSubmitReport.mockResolvedValue({ id: 'HEW-7K3F-Q9XB', following: false })
    render(<ReportBugDialog scene={fakeScene()} documentName="d.hew" crash={null} onClose={vi.fn()} />)
    await typeValidDescription()
    fireEvent.click(screen.getByRole('checkbox', { name: FOLLOW_LABEL }))
    fireEvent.click(await readySendButton())
    await screen.findByText(/HEW-7K3F-Q9XB/)
    expect(mockSubmitReport.mock.calls[0]![2]).toMatchObject({ follow: false })
    expect(screen.queryByTestId('report-follow-confirmation')).not.toBeInTheDocument()
  })
})

describe('excerptOf', () => {
  it('slices by code points so an emoji is never cut in half', () => {
    const s = 'a'.repeat(199) + '😀' + 'tail'
    const out = excerptOf(s)
    expect(Array.from(out)).toHaveLength(200)
    expect(out.endsWith('😀')).toBe(true)
    expect(JSON.stringify(out)).not.toMatch(/\\ud[89ab]/i)
  })
  it('leaves short text alone', () => {
    expect(excerptOf('hi')).toBe('hi')
  })
})
