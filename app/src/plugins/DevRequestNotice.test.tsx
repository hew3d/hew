import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { requiresPreview } from '../test/requiresPreview'
import { DevRequestNotice, REQUEST_LINE } from './DevRequestNotice'
import { MAX_PATH_SHOWN } from './pathText'
import { PluginError } from './pluginError'
import type { ConsentPayload, DevRequest, Installed, PendingView, PluginInstaller } from './types'

beforeEach((ctx) => requiresPreview(ctx, 'plugins'))
afterEach(() => vi.restoreAllMocks())

const consent: ConsentPayload = {
  id: 'com.example.dev',
  name: 'Dev',
  version: '1.0.0',
  permissions: [],
  optional: [],
  warnings: [],
  on_startup: false,
  activation_reason: null,
  signer: null,
  unsigned: true,
  source: { kind: 'dir', path: '/home/me/picked' },
  replaces: null,
  update: null,
}

const A = 'a'.repeat(32)
const B = 'b'.repeat(32)
const asked = (path: string, id = A): DevRequest => ({ id, path })

/** An installer whose request event goes through `onDevRequest`, as every
 *  window hears it. */
function fake(waiting: DevRequest | null) {
  const changed = new Set<() => void>()
  const requests = new Set<(r: DevRequest | null) => void>()
  const inst = {
    tier: 'desktop' as const,
    devRequest: vi.fn(async () => waiting),
    dismissDevRequest: vi.fn(async (_id: string) => undefined),
    pickAndStage: vi.fn(async (_k: string, _o?: { useRequest?: string }): Promise<PendingView | null> => ({ token: 'tok', consent })),
    commit: vi.fn(async (_t: string, accept: boolean): Promise<Installed | null> => (accept ? ({ id: 'com.example.dev' } as Installed) : null)),
    onChanged: vi.fn((fn: () => void) => {
      changed.add(fn)
      return () => void changed.delete(fn)
    }),
    onDevRequest: vi.fn((fn: (r: DevRequest | null) => void) => {
      requests.add(fn)
      return () => void requests.delete(fn)
    }),
  }
  return {
    inst: inst as unknown as PluginInstaller & typeof inst,
    setWaiting: (next: DevRequest | null) => (waiting = next),
    request: (r: DevRequest | null) => act(() => requests.forEach((fn) => fn(r))),
    changed: () => act(() => changed.forEach((c) => c())),
  }
}

/** The folder the notice shows, set apart from its own line. */
const shownPath = (region: HTMLElement): string => region.querySelector('[data-path-text] code')!.textContent!

describe('DevRequestNotice', () => {
  it('shows nothing while no request waits, and a request that every window hears', async () => {
    const f = fake(null)
    render(<DevRequestNotice installer={f.inst} placement="window" />)
    await waitFor(() => expect(f.inst.devRequest).toHaveBeenCalled())
    expect(screen.queryByRole('region')).toBeNull()
    f.request(asked('/home/me/asked'))
    const region = await screen.findByRole('region', { name: 'Request to load a plugin' })
    expect(region).toHaveTextContent(REQUEST_LINE)
    expect(shownPath(region)).toBe('/home/me/asked')
    expect(within(region).getByRole('button', { name: 'Choose Folder…' })).toBeInTheDocument()
    expect(within(region).getByRole('button', { name: 'Dismiss' })).toBeInTheDocument()
  })

  it('in the pane too, a request that comes while it is open', async () => {
    const f = fake(null)
    render(<DevRequestNotice installer={f.inst} placement="pane" />)
    await waitFor(() => expect(f.inst.onDevRequest).toHaveBeenCalled())
    f.request(asked('/home/me/while-open'))
    expect(shownPath(await screen.findByRole('region'))).toBe('/home/me/while-open')
  })

  it('shows a request it missed, asked for when it starts', async () => {
    const f = fake(asked('/home/me/missed'))
    render(<DevRequestNotice installer={f.inst} placement="window" />)
    expect(shownPath(await screen.findByRole('region'))).toBe('/home/me/missed')
  })

  it('shows the path as data under its own sentence, cleaned and cut in the middle for display, never as markup', async () => {
    const f = fake(null)
    const { container } = render(<DevRequestNotice installer={f.inst} placement="window" />)
    await waitFor(() => expect(f.inst.onDevRequest).toHaveBeenCalled())
    // What the installer hands on is the shell's text, already cut to 512
    // at the event boundary (storeAccess); the notice shows at most 120.
    f.request(asked(`/home/\u202Eme/<img src=x onerror=alert(1)>/${'a'.repeat(400)}/end`))
    const region = await screen.findByRole('region')
    expect(container.querySelector('img')).toBeNull()
    const path = shownPath(region)
    expect(path.startsWith('/home/me/<img src=x onerror=alert(1)>/')).toBe(true)
    expect(path.endsWith('/end')).toBe(true)
    expect(path).toContain('\u2026')
    expect(Array.from(path).length).toBeLessThanOrEqual(MAX_PATH_SHOWN)
    // The sentence is ordinary text of its own, not the path's label.
    const sentence = within(region).getByText(REQUEST_LINE)
    expect(sentence.closest('[data-path-text]')).toBeNull()
    // `null` says it is gone.
    f.request(null)
    await waitFor(() => expect(screen.queryByRole('region')).toBeNull())
  })

  it('keeps the card and says so when Hew could not dismiss it', async () => {
    const f = fake(asked('/home/me/asked'))
    f.inst.dismissDevRequest.mockRejectedValueOnce(new Error('io: no'))
    render(<DevRequestNotice installer={f.inst} placement="window" />)
    fireEvent.click(within(await screen.findByRole('region')).getByRole('button', { name: 'Dismiss' }))
    expect(await screen.findByRole('status')).toHaveTextContent("Hew couldn't dismiss the request.")
    expect(shownPath(screen.getByRole('region'))).toBe('/home/me/asked')
  })

  it('shows a request once where the pane and the corner are in one window', async () => {
    const f = fake(asked('/home/me/asked'))
    const { unmount } = render(
      <>
        <DevRequestNotice installer={f.inst} placement="window" />
        <DevRequestNotice installer={f.inst} placement="pane" />
      </>,
    )
    await waitFor(() => expect(screen.getAllByRole('region')).toHaveLength(1))
    unmount()
    render(<DevRequestNotice installer={f.inst} placement="window" />)
    expect(await screen.findAllByRole('region')).toHaveLength(1)
  })

  it('Choose Folder… opens the picker for the request shown, by its id, then the sheet, then installs', async () => {
    const f = fake(asked('/home/me/asked'))
    render(<DevRequestNotice installer={f.inst} placement="window" />)
    fireEvent.click(within(await screen.findByRole('region')).getByRole('button', { name: 'Choose Folder…' }))
    await waitFor(() => expect(f.inst.pickAndStage).toHaveBeenCalledWith('dir', { useRequest: A }))
    const sheet = await screen.findByRole('dialog')
    // The picked folder, in full, as data on the sheet.
    expect(within(sheet).getByText('Developer folder')).toBeInTheDocument()
    expect(sheet.querySelector('[data-path-text] code')!.textContent).toBe('/home/me/picked')
    f.setWaiting(null)
    fireEvent.click(within(sheet).getByRole('button', { name: 'Install' }))
    await waitFor(() => expect(f.inst.commit).toHaveBeenCalledWith('tok', true))
    expect(await screen.findByRole('status')).toHaveTextContent('Dev is installed.')
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByRole('region')).toBeNull()
  })

  it('a request gone by the time of the click is said, and the notice looks again', async () => {
    const f = fake(asked('/home/me/old'))
    f.inst.pickAndStage.mockRejectedValueOnce(new PluginError('request_gone', 'raw'))
    render(<DevRequestNotice installer={f.inst} placement="window" />)
    const region = await screen.findByRole('region')
    f.setWaiting(asked('/home/me/new', B))
    fireEvent.click(within(region).getByRole('button', { name: 'Choose Folder…' }))
    expect(await screen.findByRole('status')).toHaveTextContent('That request is no longer waiting.')
    await waitFor(() => expect(shownPath(screen.getByRole('region'))).toBe('/home/me/new'))
  })

  it('a cancelled picker or sheet installs nothing, and a request that came during the flow is shown after', async () => {
    const f = fake(asked('/home/me/asked'))
    f.inst.pickAndStage.mockResolvedValueOnce(null)
    render(<DevRequestNotice installer={f.inst} placement="window" />)
    const first = await screen.findByRole('region')
    f.setWaiting(null)
    fireEvent.click(within(first).getByRole('button', { name: 'Choose Folder…' }))
    await waitFor(() => expect(screen.queryByRole('region')).toBeNull())
    expect(screen.queryByRole('dialog')).toBeNull()
    f.request(asked('/home/me/again', B))
    fireEvent.click(within(await screen.findByRole('region')).getByRole('button', { name: 'Choose Folder…' }))
    const sheet = await screen.findByRole('dialog')
    // Another request arrives while the sheet is open: not shown over it.
    f.setWaiting(asked('/home/me/during'))
    f.request(asked('/home/me/during'))
    expect(screen.queryByRole('region')).toBeNull()
    fireEvent.click(within(sheet).getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(f.inst.commit).toHaveBeenCalledWith('tok', false))
    expect(screen.queryByRole('dialog')).toBeNull()
    await waitFor(() => expect(shownPath(screen.getByRole('region'))).toBe('/home/me/during'))
  })

  it('a cancelled or refused pick leaves the request on screen, so the person can choose again', async () => {
    const f = fake(asked('/home/me/asked'))
    f.inst.pickAndStage.mockResolvedValueOnce(null)
    render(<DevRequestNotice installer={f.inst} placement="window" />)
    fireEvent.click(within(await screen.findByRole('region')).getByRole('button', { name: 'Choose Folder…' }))
    await waitFor(() => expect(f.inst.pickAndStage).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(shownPath(screen.getByRole('region'))).toBe('/home/me/asked'))
    f.inst.pickAndStage.mockRejectedValueOnce(new PluginError('manifest_missing', 'This folder has no hew-plugin.json at its top level.'))
    fireEvent.click(within(screen.getByRole('region')).getByRole('button', { name: 'Choose Folder…' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('This folder has no hew-plugin.json')
    expect(shownPath(screen.getByRole('region'))).toBe('/home/me/asked')
    // And it can be chosen again.
    fireEvent.click(within(screen.getByRole('region')).getByRole('button', { name: 'Choose Folder…' }))
    expect(await screen.findByRole('dialog')).toBeInTheDocument()
  })

  it('Dismiss tells the shell, which forgets it in every window', async () => {
    const f = fake(asked('/home/me/asked'))
    render(<DevRequestNotice installer={f.inst} placement="window" />)
    fireEvent.click(within(await screen.findByRole('region')).getByRole('button', { name: 'Dismiss' }))
    await waitFor(() => expect(screen.queryByRole('region')).toBeNull())
    expect(f.inst.dismissDevRequest).toHaveBeenCalledWith(A)
    f.setWaiting(null)
    f.changed()
    await waitFor(() => expect(f.inst.devRequest).toHaveBeenCalledTimes(2))
    expect(screen.queryByRole('region')).toBeNull()
  })

  it('goes away when the shell no longer holds the request (taken in another window, or developer mode off)', async () => {
    const f = fake(asked('/home/me/asked'))
    render(<DevRequestNotice installer={f.inst} placement="window" />)
    await screen.findByRole('region')
    f.request(null)
    await waitFor(() => expect(screen.queryByRole('region')).toBeNull())
    f.request(asked('/home/me/asked'))
    await screen.findByRole('region')
    f.setWaiting(null)
    f.changed()
    await waitFor(() => expect(screen.queryByRole('region')).toBeNull())
  })

  it('shows a refusal in the notice: another dialog as a plain notice, anything else as an error', async () => {
    const f = fake(asked('/home/me/asked'))
    f.inst.pickAndStage.mockRejectedValueOnce(new PluginError('confirmation_open', 'raw'))
    render(<DevRequestNotice installer={f.inst} placement="window" />)
    fireEvent.click(within(await screen.findByRole('region')).getByRole('button', { name: 'Choose Folder…' }))
    expect(await screen.findByRole('status')).toHaveTextContent('Another confirmation is already open.')
    f.inst.pickAndStage.mockRejectedValueOnce(new PluginError('developer_mode_off', 'Turn on developer mode in Settings > Plugins to run a plugin from a folder.'))
    fireEvent.click(within(await screen.findByRole('region')).getByRole('button', { name: 'Choose Folder…' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Turn on developer mode in Settings > Plugins')
  })

  it('stops listening when it goes', async () => {
    const f = fake(null)
    const { unmount } = render(<DevRequestNotice installer={f.inst} placement="window" />)
    await waitFor(() => expect(f.inst.onDevRequest).toHaveBeenCalled())
    unmount()
    f.request(asked('/x'))
    expect(screen.queryByRole('region')).toBeNull()
  })
})
