import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { requiresPreview } from '../test/requiresPreview'
import { PluginsPane } from './PluginsPane'
import { PluginError } from './pluginError'
import type { ConsentPayload, Installed, Listed, PendingView, PluginInstaller } from './types'
import { recordFingerprint, rollbackFingerprint } from './webPluginStore'

beforeEach((ctx) => requiresPreview(ctx, 'plugins'))
afterEach(() => vi.restoreAllMocks())

function installed(id: string, over: Partial<Installed> = {}): Installed {
  return {
    schema: 1,
    id,
    version: '1.0.0',
    source: { kind: 'file', name: 'x.hewplugin' },
    installed_at_ms: 1,
    grant: [],
    optional_granted: [],
    enabled: true,
    startup_allowed: false,
    data_version: 1,
    signer: null,
    ...over,
  }
}

function row(id: string, name: unknown, over: Partial<Installed> = {}, extra: Partial<Extract<Listed, { kind: 'installed' }>> = {}): Listed {
  return {
    kind: 'installed',
    installed: installed(id, over),
    manifest: { id, name: name as string, version: over.version ?? '1.0.0', permissions: ['model.read', 'network:api.example.com'], optionalPermissions: ['files'] },
    previous_version: null,
    ...extra,
  }
}

const consent: ConsentPayload = {
  id: 'com.example.one',
  name: 'One',
  version: '1.0.0',
  permissions: [{ name: 'model.read', text: '' }],
  optional: [],
  warnings: [],
  on_startup: false,
  activation_reason: null,
  signer: null,
  unsigned: true,
  source: { kind: 'file', name: 'one.hewplugin' },
  replaces: null,
  update: null,
}

function fake(initial: Listed[], tier: 'desktop' | 'web' = 'web') {
  let rows = initial
  const listeners = new Set<() => void>()
  const inst = {
    tier,
    list: vi.fn(async () => rows),
    pickAndStage: vi.fn(async (): Promise<PendingView | null> => ({ token: 'tok', consent })),
    stageFile: vi.fn(async (): Promise<PendingView> => ({ token: 'tok', consent })),
    commit: vi.fn(async (_t: string, accept: boolean): Promise<Installed | null> => (accept ? installed('com.example.one') : null)),
    setEnabled: vi.fn(async (_id: string, _on: boolean): Promise<string[]> => []),
    uninstall: vi.fn(async (_id: string) => undefined),
    rollback: vi.fn(async (id: string) => installed(id)),
    iconUrl: vi.fn(async (): Promise<string | null> => null),
    onChanged: vi.fn((fn: () => void) => {
      listeners.add(fn)
      return () => void listeners.delete(fn)
    }),
  }
  return {
    inst: inst as unknown as PluginInstaller & typeof inst,
    set: (next: Listed[]) => {
      rows = next
      listeners.forEach((l) => l())
    },
    listeners,
  }
}

describe('PluginsPane', () => {
  it('says a plugin runs in a sandbox once turned on, and lists nothing when nothing is installed', async () => {
    const { inst } = fake([])
    render(<PluginsPane installer={inst} />)
    expect(screen.getByText(/runs in a sandbox, and can do only what you allowed/)).toBeInTheDocument()
    expect(screen.queryByText(/do not run yet/)).toBeNull()
    expect(await screen.findByText('No plugins are installed.')).toBeInTheDocument()
  })

  it('renders a row: name, version, source line, switch, permissions, uninstall', async () => {
    const { inst } = fake([
      row('com.example.one', 'One', {}, {}),
      row('com.example.two', 'Two', { version: '2.0.0', source: { kind: 'dir', path: '/home/me/two' }, enabled: false }),
    ])
    render(<PluginsPane installer={inst} />)
    const one = (await screen.findByText('One')).closest('li') as HTMLElement
    expect(within(one).getByText('1.0.0')).toBeInTheDocument()
    expect(within(one).getByText('From file')).toBeInTheDocument()
    expect(within(one).getByRole('switch', { name: 'Turn One on or off' })).toBeChecked()
    expect(within(one).getByText('Read the model: every object, component, material, tag, and attribute')).toBeInTheDocument()
    expect(within(one).getByText('Connect to api.example.com (checked by Hew)')).toBeInTheDocument()
    expect(within(one).getByText('Ask you to pick files, and save files you choose')).toBeInTheDocument()
    expect(within(one).queryByText(/Roll back/)).toBeNull()
    const two = screen.getByText('Two').closest('li') as HTMLElement
    expect(within(two).getByText('Unpacked: /home/me/two')).toBeInTheDocument()
    expect(within(two).getByRole('switch', { name: 'Turn Two on or off' })).not.toBeChecked()
  })

  it('shows plugin-written text cleaned, as text', async () => {
    const { inst } = fake([row('com.example.one', 'Evil‮<img src=x onerror=alert(1)>')])
    const { container } = render(<PluginsPane installer={inst} />)
    expect(await screen.findByText('Evil<img src=x onerror=alert(1)>')).toBeInTheDocument()
    expect(container.querySelector('li img')).toBeNull()
  })

  it('uses a generic glyph without an icon, and an <img> with a blob URL when there is one', async () => {
    const { inst } = fake([
      row('com.example.one', 'One'),
      {
        kind: 'installed',
        installed: installed('com.example.two'),
        manifest: { id: 'com.example.two', name: 'Two', version: '1.0.0', icon: 'icon.png' },
        previous_version: null,
      },
    ])
    inst.iconUrl.mockResolvedValue('blob:http://localhost/abc')
    const revoke = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    const { unmount } = render(<PluginsPane installer={inst} />)
    const two = (await screen.findByText('Two')).closest('li') as HTMLElement
    await waitFor(() => expect(two.querySelector('img')?.getAttribute('src')).toBe('blob:http://localhost/abc'))
    expect(inst.iconUrl).toHaveBeenCalledWith('com.example.two', 'icon.png')
    expect(inst.iconUrl).toHaveBeenCalledTimes(1)
    const one = screen.getByText('One').closest('li') as HTMLElement
    expect(one.querySelector('img')).toBeNull()
    expect(one.querySelector('svg')).not.toBeNull()
    unmount()
    expect(revoke).toHaveBeenCalledWith('blob:http://localhost/abc')
  })

  it('turns a plugin off and on through the installer', async () => {
    const { inst } = fake([row('com.example.one', 'One')])
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('switch', { name: 'Turn One on or off' }))
    await waitFor(() => expect(inst.setEnabled).toHaveBeenCalledWith('com.example.one', false))
  })

  it('names the plugins that were turned off along with it', async () => {
    const { inst } = fake([row('com.example.base', 'Base'), row('com.example.top', 'Top')])
    inst.setEnabled.mockResolvedValue(['com.example.top'])
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('switch', { name: 'Turn Base on or off' }))
    expect(await screen.findByRole('status')).toHaveTextContent('Turning off Base also turned off Top, because they need it.')
  })

  it('offers Roll back only with a previous version, and rolls back', async () => {
    const { inst } = fake([row('com.example.one', 'One', { version: '2.0.0' }, { previous_version: '1.0.0' })])
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('button', { name: 'Roll back to 1.0.0' }))
    await waitFor(() => expect(inst.rollback).toHaveBeenCalledWith('com.example.one'))
  })

  it('asks before uninstalling, and Keep it changes nothing', async () => {
    const { inst } = fake([row('com.example.one', 'One')])
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('button', { name: 'Uninstall' }))
    expect(inst.uninstall).not.toHaveBeenCalled()
    expect(screen.getByText('Remove One and its settings?')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Keep it' }))
    expect(inst.uninstall).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Uninstall' }))
    fireEvent.click(within(screen.getByRole('group', { name: 'Uninstall One?' })).getByRole('button', { name: 'Uninstall' }))
    await waitFor(() => expect(inst.uninstall).toHaveBeenCalledWith('com.example.one'))
  })

  it('the web uninstall question has the sheet keys: Keep it has focus, Enter does nothing, Escape keeps the plugin and focus returns', async () => {
    const { inst } = fake([row('com.example.one', 'One')])
    render(<PluginsPane installer={inst} />)
    const trigger = await screen.findByRole('button', { name: 'Uninstall' })
    trigger.focus()
    fireEvent.click(trigger)
    const group = screen.getByRole('group', { name: 'Uninstall One?' })
    const keep = within(group).getByRole('button', { name: 'Keep it' })
    await waitFor(() => expect(keep).toHaveFocus())
    const enter = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
    keep.dispatchEvent(enter)
    expect(enter.defaultPrevented).toBe(true)
    const confirm = within(group).getByRole('button', { name: 'Uninstall' })
    confirm.focus()
    const tab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true })
    confirm.dispatchEvent(tab)
    expect(tab.defaultPrevented).toBe(true)
    expect(keep).toHaveFocus()
    fireEvent.keyDown(document.activeElement as Element, { key: 'Escape' })
    expect(screen.queryByRole('group', { name: 'Uninstall One?' })).toBeNull()
    expect(inst.uninstall).not.toHaveBeenCalled()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Uninstall' })).toHaveFocus())
  })

  it('Escape does not cancel an install that is being written', async () => {
    const { inst } = fake([], 'web')
    let finish: (r: Installed) => void = () => undefined
    inst.commit.mockImplementation(() => new Promise<Installed | null>((resolve) => (finish = resolve)))
    render(<PluginsPane installer={inst} />)
    await screen.findByText('No plugins are installed.')
    const file = new File(['x'], 'one.hewplugin')
    fireEvent.change(screen.getByLabelText('Plugin package'), { target: { files: [file] } })
    const dialog = await screen.findByRole('dialog')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Install' }))
    await waitFor(() => expect(inst.commit).toHaveBeenCalledTimes(1))
    fireEvent.keyDown(document.body, { key: 'Escape' })
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(inst.commit).toHaveBeenCalledTimes(1)
    finish(installed('com.example.one'))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(inst.commit).toHaveBeenCalledTimes(1)
  })

  it('applies only the answer to the latest list request, even when an earlier one answers last', async () => {
    const { inst, set } = fake([row('com.example.one', 'One')])
    const answers: ((l: Listed[]) => void)[] = []
    inst.list.mockImplementation(() => new Promise<Listed[]>((resolve) => answers.push(resolve)))
    render(<PluginsPane installer={inst} />)
    await waitFor(() => expect(answers).toHaveLength(1))
    set([])
    await waitFor(() => expect(answers).toHaveLength(2))
    // The newer request answers first; the older, wrong one answers after.
    answers[1]([row('com.example.two', 'Two')])
    expect(await screen.findByText('Two')).toBeInTheDocument()
    answers[0]([{ kind: 'unreadable', id: 'com.example.one', reason: 'half way through a change' }])
    await new Promise((r) => setTimeout(r, 20))
    expect(screen.getByText('Two')).toBeInTheDocument()
    expect(screen.queryByText('com.example.one')).toBeNull()
  })

  it('shows an unreadable record as such and offers only removal', async () => {
    const { inst } = fake([{ kind: 'unreadable', id: 'com.example.new', reason: 'record format 2' }])
    render(<PluginsPane installer={inst} />)
    const li = (await screen.findByText('com.example.new')).closest('li') as HTMLElement
    expect(li).toHaveTextContent('Hew cannot read this plugin (record format 2)')
    expect(within(li).queryByRole('switch')).toBeNull()
    expect(within(li).getAllByRole('button').map((b) => b.textContent)).toEqual(['Uninstall'])
  })

  it('shows an install error as its text, without the name prefix', async () => {
    const { inst } = fake([row('com.example.one', 'One')])
    inst.uninstall.mockRejectedValue(new PluginError('dependency_missing', 'Other plugins still need it: com.example.top.'))
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('button', { name: 'Uninstall' }))
    fireEvent.click(screen.getAllByRole('button', { name: 'Uninstall' })[0])
    expect(await screen.findByRole('alert')).toHaveTextContent('Other plugins still need it: com.example.top.')
    expect(screen.getByRole('alert').textContent).not.toMatch(/dependency_missing/)
  })

  it('refreshes when the store changes elsewhere, and stops listening on unmount', async () => {
    const { inst, set, listeners } = fake([])
    const { unmount } = render(<PluginsPane installer={inst} />)
    await screen.findByText('No plugins are installed.')
    set([row('com.example.one', 'One')])
    expect(await screen.findByText('One')).toBeInTheDocument()
    unmount()
    expect(listeners.size).toBe(0)
  })

  describe('installing on the web', () => {
    it('stages the chosen file, shows the sheet, and commits only on Install', async () => {
      const { inst } = fake([])
      const { container } = render(<PluginsPane installer={inst} />)
      await screen.findByText('No plugins are installed.')
      const input = container.querySelector('input[type=file]') as HTMLInputElement
      expect(input.accept).toBe('.hewplugin')
      const file = new File([new Uint8Array([1])], 'one.hewplugin')
      fireEvent.change(input, { target: { files: [file] } })
      const dialog = await screen.findByRole('dialog')
      expect(inst.stageFile).toHaveBeenCalledWith(file)
      expect(inst.commit).not.toHaveBeenCalled()
      expect(within(dialog).getByRole('button', { name: 'Cancel' })).toHaveFocus()
      fireEvent.click(within(dialog).getByRole('button', { name: 'Install' }))
      await waitFor(() => expect(inst.commit).toHaveBeenCalledWith('tok', true))
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    })

    it('Cancel drops the pending install', async () => {
      const { inst } = fake([])
      const { container } = render(<PluginsPane installer={inst} />)
      await screen.findByText('No plugins are installed.')
      fireEvent.change(container.querySelector('input[type=file]') as HTMLInputElement, {
        target: { files: [new File([new Uint8Array([1])], 'one.hewplugin')] },
      })
      fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' }))
      await waitFor(() => expect(inst.commit).toHaveBeenCalledWith('tok', false))
      expect(screen.queryByRole('dialog')).toBeNull()
    })

    it('shows a refused package as an error and no sheet', async () => {
      const { inst } = fake([])
      inst.stageFile.mockRejectedValue(new PluginError('package_not_store_only', 'This package is compressed.'))
      const { container } = render(<PluginsPane installer={inst} />)
      await screen.findByText('No plugins are installed.')
      fireEvent.change(container.querySelector('input[type=file]') as HTMLInputElement, {
        target: { files: [new File([new Uint8Array([1])], 'one.hewplugin')] },
      })
      expect(await screen.findByRole('alert')).toHaveTextContent('This package is compressed.')
      expect(screen.queryByRole('dialog')).toBeNull()
    })

    it('keeps the sheet open with the refusal when the commit fails', async () => {
      const { inst } = fake([])
      inst.commit.mockRejectedValueOnce(new PluginError('pending_expired', 'This install waited too long.'))
      const { container } = render(<PluginsPane installer={inst} />)
      await screen.findByText('No plugins are installed.')
      fireEvent.change(container.querySelector('input[type=file]') as HTMLInputElement, {
        target: { files: [new File([new Uint8Array([1])], 'one.hewplugin')] },
      })
      fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Install' }))
      expect(await within(screen.getByRole('dialog')).findByRole('alert')).toHaveTextContent('This install waited too long.')
    })
  })

  describe('installing on the desktop', () => {
    it('offers From file and Unpacked folder under Install…, and a cancelled pick shows nothing', async () => {
      const { inst } = fake([], 'desktop')
      const { container } = render(<PluginsPane installer={inst} />)
      await screen.findByText('No plugins are installed.')
      expect(container.querySelector('input[type=file]')).toBeNull()
      expect(screen.queryByRole('button', { name: 'From file…' })).toBeNull()
      fireEvent.click(screen.getByRole('button', { name: 'Install…' }))
      inst.pickAndStage.mockResolvedValueOnce(null)
      fireEvent.click(screen.getByRole('button', { name: 'From file…' }))
      await waitFor(() => expect(inst.pickAndStage).toHaveBeenCalledWith('file'))
      expect(screen.queryByRole('dialog')).toBeNull()
      fireEvent.click(screen.getByRole('button', { name: 'Install…' }))
      fireEvent.click(screen.getByRole('button', { name: 'Unpacked folder…' }))
      expect(await screen.findByRole('dialog')).toBeInTheDocument()
      expect(inst.pickAndStage).toHaveBeenLastCalledWith('dir')
    })
  })

  describe('rolling back', () => {
    const withPrevious = () => {
      const f = fake([row('com.example.one', 'One', { version: '2.0.0' }, { previous_version: '1.0.0' })])
      f.inst.previous = vi.fn(async () => ({
        installed: installed('com.example.one', { version: '1.0.0' }),
        manifest: { id: 'com.example.one', name: 'One', version: '1.0.0', permissions: ['clipboard'] },
      }))
      return f
    }

    it('on the web asks first, listing what the earlier version can do', async () => {
      const { inst } = withPrevious()
      render(<PluginsPane installer={inst} />)
      fireEvent.click(await screen.findByRole('button', { name: 'Roll back to 1.0.0' }))
      const ask = await screen.findByRole('group', { name: 'Go back to 1.0.0?' })
      expect(within(ask).getByText('Read and write the clipboard')).toBeInTheDocument()
      expect(inst.rollback).not.toHaveBeenCalled()
      fireEvent.click(within(ask).getByRole('button', { name: 'Keep this version' }))
      expect(screen.queryByRole('group', { name: 'Go back to 1.0.0?' })).toBeNull()
      expect(inst.rollback).not.toHaveBeenCalled()
      fireEvent.click(screen.getByRole('button', { name: 'Roll back to 1.0.0' }))
      fireEvent.click(within(await screen.findByRole('group', { name: 'Go back to 1.0.0?' })).getByRole('button', { name: 'Roll back' }))
      // The call carries what the sheet showed, so a change since is refused.
      await waitFor(() => expect(inst.rollback).toHaveBeenCalledWith('com.example.one', rollbackFingerprint(installed('com.example.one', { version: '2.0.0' }), installed('com.example.one', { version: '1.0.0' }))))
    })

    it('a rollback refused as changed is a visible notice, on both tiers', async () => {
      for (const tier of ['web', 'desktop'] as const) {
        const f = fake([row('com.example.one', 'One', { version: '2.0.0' }, { previous_version: '1.0.0' })], tier)
        f.inst.previous = vi.fn(async () => ({
          installed: installed('com.example.one', { version: '1.0.0' }),
          manifest: { id: 'com.example.one', name: 'One', version: '1.0.0', permissions: [] },
        }))
        f.inst.rollback.mockRejectedValue(new PluginError('plugin_changed', 'raw text'))
        const { unmount } = render(<PluginsPane installer={tier === 'web' ? f.inst : { ...f.inst, previous: undefined }} />)
        fireEvent.click(await screen.findByRole('button', { name: 'Roll back to 1.0.0' }))
        if (tier === 'web') fireEvent.click(within(await screen.findByRole('group', { name: 'Go back to 1.0.0?' })).getByRole('button', { name: 'Roll back' }))
        await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('This plugin changed while you were deciding.'))
        expect(screen.queryByRole('alert')).toBeNull()
        unmount()
      }
    })

    it('the web confirmation has the sheet keys: Keep has focus, Enter does nothing, Escape keeps the version, focus stays inside', async () => {
      const { inst } = withPrevious()
      render(<PluginsPane installer={inst} />)
      const trigger = await screen.findByRole('button', { name: 'Roll back to 1.0.0' })
      trigger.focus()
      fireEvent.click(trigger)
      const ask = await screen.findByRole('group', { name: 'Go back to 1.0.0?' })
      const keep = within(ask).getByRole('button', { name: 'Keep this version' })
      const roll = within(ask).getByRole('button', { name: 'Roll back' })
      await waitFor(() => expect(keep).toHaveFocus())
      const enter = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
      keep.dispatchEvent(enter)
      expect(enter.defaultPrevented).toBe(true)
      expect(inst.rollback).not.toHaveBeenCalled()
      roll.focus()
      const tab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true })
      roll.dispatchEvent(tab)
      expect(tab.defaultPrevented).toBe(true)
      expect(keep).toHaveFocus()
      fireEvent.keyDown(document.activeElement as Element, { key: 'Escape' })
      expect(screen.queryByRole('group', { name: 'Go back to 1.0.0?' })).toBeNull()
      expect(inst.rollback).not.toHaveBeenCalled()
      expect(screen.getByRole('button', { name: 'Roll back to 1.0.0' })).toHaveFocus()
    })

    it('on the desktop a declined native confirmation is a quiet no-op', async () => {
      const { inst } = fake([row('com.example.one', 'One', { version: '2.0.0' }, { previous_version: '1.0.0' })], 'desktop')
      inst.rollback.mockRejectedValue(new PluginError('rollback_declined', 'The plugin was left as it is.'))
      render(<PluginsPane installer={inst} />)
      fireEvent.click(await screen.findByRole('button', { name: 'Roll back to 1.0.0' }))
      await waitFor(() => expect(inst.rollback).toHaveBeenCalled())
      await waitFor(() => expect(inst.list.mock.calls.length).toBeGreaterThan(1))
      expect(screen.queryByRole('alert')).toBeNull()
    })
  })

  it('says nothing was installed when the native confirmation answered no', async () => {
    const { inst } = fake([], 'desktop')
    inst.commit.mockResolvedValueOnce(null)
    render(<PluginsPane installer={inst} />)
    await screen.findByText('No plugins are installed.')
    fireEvent.click(screen.getByRole('button', { name: 'Install…' }))
    fireEvent.click(screen.getByRole('button', { name: 'From file…' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Install' }))
    expect(await screen.findByRole('status')).toHaveTextContent('Nothing was installed.')
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(screen.queryByRole('alert')).toBeNull()
  })
})

describe('one question per change', () => {
  const off = (tier: 'desktop' | 'web') => fake([row('com.example.one', 'One', { enabled: false })], tier)

  it('on the desktop, Uninstall goes straight through: the shell asks', async () => {
    const { inst } = fake([row('com.example.one', 'One'), { kind: 'unreadable', id: 'com.example.bad', reason: 'x' }], 'desktop')
    render(<PluginsPane installer={inst} />)
    const one = (await screen.findByText('One')).closest('li') as HTMLElement
    fireEvent.click(within(one).getByRole('button', { name: 'Uninstall' }))
    await waitFor(() => expect(inst.uninstall).toHaveBeenCalledWith('com.example.one'))
    expect(screen.queryByRole('group', { name: /Uninstall/ })).toBeNull()
    const bad = screen.getByText('com.example.bad').closest('li') as HTMLElement
    fireEvent.click(within(bad).getByRole('button', { name: 'Uninstall' }))
    await waitFor(() => expect(inst.uninstall).toHaveBeenCalledWith('com.example.bad'))
  })

  it('on the desktop, turning a plugin on calls straight through, and declined answers are quiet', async () => {
    const { inst } = off('desktop')
    inst.setEnabled.mockRejectedValue(new PluginError('enable_declined', 'It was left off.'))
    inst.uninstall.mockRejectedValue(new PluginError('uninstall_declined', 'It was kept.'))
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('switch', { name: 'Turn One on or off' }))
    await waitFor(() => expect(inst.setEnabled).toHaveBeenCalledWith('com.example.one', true))
    expect(screen.queryByRole('dialog')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Uninstall' }))
    await waitFor(() => expect(inst.uninstall).toHaveBeenCalled())
    await waitFor(() => expect(inst.list.mock.calls.length).toBeGreaterThan(2))
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('shows confirmation_open and plugin_changed as plain notices, not errors', async () => {
    const { inst } = off('desktop')
    inst.setEnabled.mockRejectedValueOnce(new PluginError('confirmation_open', 'raw text'))
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('switch', { name: 'Turn One on or off' }))
    expect(await screen.findByRole('status')).toHaveTextContent('Another confirmation is already open. Answer it first, then try again.')
    expect(screen.queryByRole('alert')).toBeNull()
    inst.setEnabled.mockRejectedValueOnce(new PluginError('plugin_changed', 'raw text'))
    fireEvent.click(screen.getByRole('switch', { name: 'Turn One on or off' }))
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('This plugin changed while you were deciding. Check it, then try again.'))
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('on the web, turning a plugin on asks first; Cancel has focus and Escape cancels', async () => {
    const { inst } = off('web')
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('switch', { name: 'Turn One on or off' }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText('Turn on this plugin?')).toBeInTheDocument()
    expect(within(dialog).getByText('One will run in a sandbox. It will be able to:')).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: 'Cancel' })).toHaveFocus()
    expect(inst.setEnabled).not.toHaveBeenCalled()
    // Enter does nothing.
    fireEvent.keyDown(document.activeElement as Element, { key: 'Enter' })
    expect(inst.setEnabled).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    fireEvent.keyDown(document.activeElement as Element, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(inst.setEnabled).not.toHaveBeenCalled()
    // Cancel with the button too.
    fireEvent.click(screen.getByRole('switch', { name: 'Turn One on or off' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' }))
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(inst.setEnabled).not.toHaveBeenCalled()
  })

  it('on the web, Turn On turns the plugin on; turning off asks nothing', async () => {
    const { inst, set } = off('web')
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('switch', { name: 'Turn One on or off' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Turn On' }))
    // The call carries the record the sheet showed.
    await waitFor(() => expect(inst.setEnabled).toHaveBeenCalledWith('com.example.one', true, recordFingerprint(installed('com.example.one', { enabled: false }))))
    expect(screen.queryByRole('dialog')).toBeNull()
    set([row('com.example.one', 'One', { enabled: true })])
    inst.setEnabled.mockClear()
    fireEvent.click(await screen.findByRole('switch', { name: 'Turn One on or off' }))
    await waitFor(() => expect(inst.setEnabled).toHaveBeenCalledWith('com.example.one', false))
    expect(screen.queryByRole('dialog')).toBeNull()
  })
})
