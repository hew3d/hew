/* oxlint-disable lingui/no-unlocalized-strings -- test data. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, configure, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { requiresPreview } from '../test/requiresPreview'
import { requestGrant, webGrantDecider, decideGrant } from './grantSheet'
import type { GrantCapable } from './installer'
import { PluginError } from './pluginError'
import type { Installed, Listed, PluginInstaller } from './types'

beforeEach((ctx) => requiresPreview(ctx, 'plugins'))
// A loaded machine can be slow to render the first sheet.
configure({ asyncUtilTimeout: 5000 })
afterEach(() => cleanup())

function record(over: Partial<Installed> = {}): Installed {
  return {
    schema: 1,
    id: 'com.example.one',
    version: '1.0.0',
    source: { kind: 'file', name: 'x.hewplugin' },
    installed_at_ms: 1,
    grant: ['model.read'],
    optional_granted: [],
    enabled: true,
    startup_allowed: true,
    data_version: 1,
    ...over,
  }
}

function listed(r: Installed, optional = ['files', 'view']): Listed {
  return { kind: 'installed', installed: r, manifest: { id: r.id, name: 'One', version: r.version, permissions: ['model.read'], optionalPermissions: optional }, previous_version: null }
}

function fakeInstaller(tier: 'desktop' | 'web', start = record()) {
  let current = start
  const inst = {
    tier,
    list: vi.fn(async () => [listed(current)]),
    setGrant: vi.fn(async (_id: string, grant: string[], optional: string[]) => {
      current = { ...current, grant, optional_granted: optional }
      return current
    }),
    grantLines: vi.fn(async () => ({ added: [{ name: 'files', text: 'Ask you to pick files, and save files you choose' }], warnings: [] })),
  }
  return { inst: inst as unknown as PluginInstaller & GrantCapable, mock: inst, change: (r: Installed) => (current = r) }
}

const refusal = async (p: Promise<unknown>): Promise<string> => {
  try {
    await p
  } catch (e) {
    return (e as PluginError).refusal
  }
  return 'none'
}

describe('requestGrant on the web', () => {
  it('shows the sheet over the page, and writes the wider grant only after Allow', async () => {
    const { inst, mock } = fakeInstaller('web')
    const answer = requestGrant(inst, 'com.example.one', ['files'])
    const dialog = await screen.findByRole('dialog', { name: 'Allow more for this plugin?' })
    expect(mock.setGrant).not.toHaveBeenCalled()
    // Focus moves in an effect after the sheet is in the page.
    await waitFor(() => expect(within(dialog).getByRole('button', { name: 'Cancel' })).toHaveFocus())
    fireEvent.click(within(dialog).getByRole('button', { name: 'Allow' }))
    expect(await answer).toBe('granted')
    expect(mock.setGrant).toHaveBeenCalledWith('com.example.one', ['model.read'], ['files'])
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  })

  it('keeps what was already granted when it adds more', async () => {
    const { inst, mock } = fakeInstaller('web', record({ optional_granted: ['view'] }))
    const answer = requestGrant(inst, 'com.example.one', ['files'])
    fireEvent.click(await screen.findByRole('button', { name: 'Allow' }))
    await answer
    expect(mock.setGrant).toHaveBeenCalledWith('com.example.one', ['model.read'], ['view', 'files'])
  })

  it('answers declined on Cancel and on Escape, and the record is untouched', async () => {
    const { inst, mock } = fakeInstaller('web')
    let answer = requestGrant(inst, 'com.example.one', ['files'])
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }))
    expect(await answer).toBe('declined')
    answer = requestGrant(inst, 'com.example.one', ['files'])
    await screen.findByRole('dialog')
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(await answer).toBe('declined')
    expect(mock.setGrant).not.toHaveBeenCalled()
  })

  it('does not take Enter as a yes', async () => {
    const { inst, mock } = fakeInstaller('web')
    const answer = requestGrant(inst, 'com.example.one', ['files'])
    await screen.findByRole('dialog')
    fireEvent.keyDown(document.activeElement as Element, { key: 'Enter' })
    expect(mock.setGrant).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(await answer).toBe('declined')
  })

  it('answers granted without a sheet for what the plugin already holds', async () => {
    const { inst, mock } = fakeInstaller('web', record({ optional_granted: ['files'] }))
    expect(await requestGrant(inst, 'com.example.one', ['files'])).toBe('granted')
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(mock.setGrant).not.toHaveBeenCalled()
  })

  it('refuses a name the plugin does not declare as optional, before asking anyone', async () => {
    const { inst, mock } = fakeInstaller('web')
    expect(await refusal(requestGrant(inst, 'com.example.one', ['model.write']))).toBe('unknown_permission')
    expect(await refusal(requestGrant(inst, 'com.example.one', ['model.read']))).toBe('unknown_permission')
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(mock.grantLines).not.toHaveBeenCalled()
  })

  it('refuses a plugin that is not installed', async () => {
    const { inst } = fakeInstaller('web')
    expect(await refusal(requestGrant(inst, 'com.example.other', ['files']))).toBe('not_installed')
  })

  it('has one question open at a time', async () => {
    const { inst } = fakeInstaller('web')
    const first = requestGrant(inst, 'com.example.one', ['files'])
    await screen.findByRole('dialog')
    expect(await refusal(requestGrant(inst, 'com.example.one', ['view']))).toBe('confirmation_open')
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    await first
  })

  it('refuses, and writes nothing, when the plugin changed while the person decided', async () => {
    const { inst, mock, change } = fakeInstaller('web')
    const answer = requestGrant(inst, 'com.example.one', ['files'])
    const allow = await screen.findByRole('button', { name: 'Allow' })
    change(record({ version: '2.0.0' }))
    const caught = refusal(answer)
    act(() => allow.click())
    expect(await caught).toBe('plugin_changed')
    expect(mock.setGrant).not.toHaveBeenCalled()
  })

  it('answers declined when the stored record does not hold what was asked for after a yes', async () => {
    const { inst, mock } = fakeInstaller('web')
    mock.setGrant.mockResolvedValueOnce(record())
    const answer = requestGrant(inst, 'com.example.one', ['files'])
    fireEvent.click(await screen.findByRole('button', { name: 'Allow' }))
    expect(await answer).toBe('declined')
  })

  it('needs an installer that can set a grant, and lines to show', async () => {
    const bare = { tier: 'web', list: async () => [listed(record())] } as unknown as PluginInstaller
    expect(await refusal(requestGrant(bare, 'com.example.one', ['files']))).toBe('unsupported')
    const { inst, mock } = fakeInstaller('web')
    delete (mock as { grantLines?: unknown }).grantLines
    expect(await refusal(requestGrant(inst, 'com.example.one', ['files']))).toBe('unsupported')
  })

  it('shows the plugin\'s name as text, never markup', async () => {
    const { inst, mock } = fakeInstaller('web')
    mock.list.mockResolvedValue([{ ...listed(record()), manifest: { id: 'com.example.one', name: 'Evil<img src=x onerror=alert(1)>', version: '1.0.0', permissions: ['model.read'], optionalPermissions: ['files'] } } as Listed])
    const answer = requestGrant(inst, 'com.example.one', ['files'])
    const dialog = await screen.findByRole('dialog')
    expect(dialog.querySelector('img')).toBeNull()
    expect(within(dialog).getByText('Evil<img src=x onerror=alert(1)>')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    await answer
  })
})

describe('requestGrant on the desktop', () => {
  it('lets the shell ask: no sheet, and the answer is read from the stored record', async () => {
    const { inst, mock } = fakeInstaller('desktop')
    expect(await requestGrant(inst, 'com.example.one', ['files'])).toBe('granted')
    expect(mock.setGrant).toHaveBeenCalledWith('com.example.one', ['model.read'], ['files'])
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(mock.grantLines).not.toHaveBeenCalled()
  })

  it('answers declined when the shell leaves the record as it was (a no)', async () => {
    const { inst, mock } = fakeInstaller('desktop')
    mock.setGrant.mockResolvedValueOnce(record())
    expect(await requestGrant(inst, 'com.example.one', ['files'])).toBe('declined')
  })

  it('passes the shell\'s refusals on', async () => {
    const { inst, mock } = fakeInstaller('desktop')
    mock.setGrant.mockRejectedValueOnce(new PluginError('confirmation_open', 'x'))
    expect(await refusal(requestGrant(inst, 'com.example.one', ['files']))).toBe('confirmation_open')
  })
})

describe('the web decider the host passes for a plugin\'s own request', () => {
  it('is the sheet entry point, shaped as the host\'s GrantDecider', () => {
    expect(webGrantDecider.decide).toBe(decideGrant)
    expect(decideGrant.length).toBe(2)
  })
})
