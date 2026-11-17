import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
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
    expect(within(two).getByText('Developer folder')).toBeInTheDocument()
    expect(within(two).getByText('Folder')).toBeInTheDocument()
    expect(within(two).getByText('/home/me/two')).toBeInTheDocument()
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
    it('offers From file and (in developer mode) Developer folder under Install…, and a cancelled pick shows nothing', async () => {
      const { inst } = fake([], 'desktop')
      Object.assign(inst, { devMode: vi.fn(async () => true) })
      const { container } = render(<PluginsPane installer={inst} />)
      await waitFor(() => expect(screen.getByRole('switch', { name: 'Developer mode' })).toBeChecked())
      await screen.findByText('No plugins are installed.')
      expect(container.querySelector('input[type=file]')).toBeNull()
      expect(screen.queryByRole('button', { name: 'From file…' })).toBeNull()
      fireEvent.click(screen.getByRole('button', { name: 'Install…' }))
      inst.pickAndStage.mockResolvedValueOnce(null)
      fireEvent.click(screen.getByRole('button', { name: 'From file…' }))
      await waitFor(() => expect(inst.pickAndStage).toHaveBeenCalledWith('file'))
      expect(screen.queryByRole('dialog')).toBeNull()
      fireEvent.click(screen.getByRole('button', { name: 'Install…' }))
      fireEvent.click(screen.getByRole('button', { name: 'Developer folder…' }))
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

describe('PluginsPane on the desktop, developer mode and folder plugins', () => {
  const DEV = 'com.example.dev'
  const folderRow = (over: Partial<Installed> = {}): Listed => row(DEV, 'Dev', { source: { kind: 'dir', path: '/home/me/dev' }, ...over })

  /** A desktop installer with the developer calls, and a way to send
   *  `plugins-dev-changed`. */
  function devFake(rows: Listed[], on = true) {
    const f = fake(rows, 'desktop')
    let shellSays = on
    const devListeners = new Set<(c: { id: string; manifest_changed: boolean; reason: string | null; problem: string | null }) => void>()
    const requestListeners = new Set<(r: { id: string; path: string } | null) => void>()
    const extra = {
      devMode: vi.fn(async () => shellSays),
      setDevMode: vi.fn(async (next: boolean) => {
        shellSays = next
        return next
      }),
      review: vi.fn(async (): Promise<PendingView> => ({
        token: 'rev',
        consent: {
          ...consent,
          id: DEV,
          name: 'Dev',
          review: true,
          source: { kind: 'dir', path: '/home/me/dev' },
          replaces: { name: 'Dev', version: '1.0.0' },
          update: {
            added: [{ name: 'clipboard', text: '' }],
            new_optional: [],
            removed: [{ name: 'model.read', text: '' }],
            removed_optional: [],
            startup_newly: false,
            from_version: '1.0.0',
          },
        },
      })),
      devRequest: vi.fn(async (): Promise<{ id: string; path: string } | null> => null),
      dismissDevRequest: vi.fn(async () => undefined),
      onDevRequest: vi.fn((fn: (r: { id: string; path: string } | null) => void) => {
        requestListeners.add(fn)
        return () => void requestListeners.delete(fn)
      }),
      onDevChanged: vi.fn((fn: (c: { id: string; manifest_changed: boolean; reason: string | null; problem: string | null }) => void) => {
        devListeners.add(fn)
        return () => void devListeners.delete(fn)
      }),
    }
    Object.assign(f.inst, extra)
    return {
      ...f,
      inst: f.inst as typeof f.inst & typeof extra,
      shellSays: (v: boolean) => (shellSays = v),
      devChanged: (c: { id: string; manifest_changed: boolean; reason: string | null; problem: string | null }) => devListeners.forEach((l) => l(c)),
      devRequested: (r: { id: string; path: string } | null) => requestListeners.forEach((l) => l(r)),
    }
  }

  it('shows the switch as the shell says, and offers Developer folder only while it is on', async () => {
    const { inst } = devFake([], false)
    render(<PluginsPane installer={inst} />)
    const sw = await screen.findByRole('switch', { name: 'Developer mode' })
    await waitFor(() => expect(sw).not.toBeDisabled())
    expect(sw).not.toBeChecked()
    fireEvent.click(screen.getByRole('button', { name: 'Install…' }))
    expect(screen.getByRole('button', { name: 'From file…' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Developer folder…' })).toBeNull()
    fireEvent.click(sw)
    await waitFor(() => expect(inst.setDevMode).toHaveBeenCalledWith(true))
    await waitFor(() => expect(sw).toBeChecked())
    expect(screen.getByRole('button', { name: 'Developer folder…' })).toBeInTheDocument()
  })

  it('takes a "no" in the native confirmation quietly, and another open dialog as a plain notice', async () => {
    const { inst } = devFake([], false)
    render(<PluginsPane installer={inst} />)
    const sw = await screen.findByRole('switch', { name: 'Developer mode' })
    await waitFor(() => expect(sw).not.toBeDisabled())
    inst.setDevMode.mockRejectedValueOnce(new PluginError('developer_mode_declined', 'Developer mode was left off.'))
    fireEvent.click(sw)
    await waitFor(() => expect(inst.setDevMode).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(inst.devMode.mock.calls.length).toBeGreaterThanOrEqual(2))
    expect(sw).not.toBeChecked()
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.queryByRole('status')).toBeNull()
    inst.setDevMode.mockRejectedValueOnce(new PluginError('confirmation_open', 'raw'))
    fireEvent.click(sw)
    expect(await screen.findByRole('status')).toHaveTextContent('Another confirmation is already open.')
    expect(sw).not.toBeChecked()
  })

  it("shows what the shell says after a change, not the page's own idea of it", async () => {
    const { inst, shellSays } = devFake([], false)
    inst.setDevMode.mockImplementationOnce(async () => {
      shellSays(false)
      return true
    })
    render(<PluginsPane installer={inst} />)
    const sw = await screen.findByRole('switch', { name: 'Developer mode' })
    await waitFor(() => expect(sw).not.toBeDisabled())
    fireEvent.click(sw)
    await waitFor(() => expect(inst.devMode.mock.calls.length).toBeGreaterThanOrEqual(2))
    await waitFor(() => expect(sw).not.toBeChecked())
  })

  it('reads a folder row: its cleaned path, and "Needs developer mode" while it is off', async () => {
    const { inst } = devFake([folderRow({ source: { kind: 'dir', path: '/home/me/‮dev\u0007' } })], false)
    render(<PluginsPane installer={inst} />)
    const li = (await screen.findByText('Dev')).closest('li') as HTMLElement
    expect(within(li).getByText('Developer folder')).toBeInTheDocument()
    expect(within(li).getByText('/home/me/dev')).toBeInTheDocument()
    await within(li).findByText('Needs developer mode')
    expect(within(li).queryByRole('button', { name: 'Review' })).toBeNull()
  })

  it('offers Review and Turn Off for a folder that now asks for different permissions, and says why', async () => {
    const { inst } = devFake([folderRow({ needs_consent: true, needs_consent_reason: 'startup_added' })])
    render(<PluginsPane installer={inst} />)
    const li = (await screen.findByText('Dev')).closest('li') as HTMLElement
    expect(await within(li).findByText("This plugin's folder changed in a way you need to review.")).toBeInTheDocument()
    expect(within(li).getByText('It now asks to start whenever Hew starts.')).toBeInTheDocument()
    fireEvent.click(within(li).getByRole('button', { name: 'Turn Off' }))
    await waitFor(() => expect(inst.setEnabled).toHaveBeenCalledWith(DEV, false))
  })

  it('treats a marker of any value but false as waiting, and false or none as not', async () => {
    for (const [marker, waits] of [[true, true], ['yes', true], [null, true], [0, true], [false, false], [undefined, false]] as const) {
      const { inst } = devFake([folderRow(marker === undefined ? {} : { needs_consent: marker })])
      const { unmount } = render(<PluginsPane installer={inst} />)
      const li = (await screen.findByText('Dev')).closest('li') as HTMLElement
      await waitFor(() => expect(inst.devMode).toHaveBeenCalled())
      await new Promise((r) => setTimeout(r, 0))
      expect(within(li).queryByRole('button', { name: 'Review' }) !== null, String(marker)).toBe(waits)
      unmount()
    }
  })

  it('reviews a change in the sheet: what it now asks for, what it no longer does, the folder line, and Allow', async () => {
    const { inst } = devFake([folderRow({ needs_consent: true })])
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('button', { name: 'Review' }))
    const sheet = await screen.findByRole('dialog')
    expect(inst.review).toHaveBeenCalledWith(DEV)
    expect(within(sheet).getByRole('heading', { name: "Allow this plugin's changes?" })).toBeInTheDocument()
    expect(within(sheet).getByText("This plugin's folder now asks for different permissions.")).toBeInTheDocument()
    expect(sheet.querySelector('[data-consent="replaces"]')).toBeNull()
    expect(within(sheet).getByText('No longer asks for')).toBeInTheDocument()
    expect(within(sheet).getByText('The files in this folder can change after you install.')).toBeInTheDocument()
    fireEvent.click(within(sheet).getByRole('button', { name: 'Allow' }))
    await waitFor(() => expect(inst.commit).toHaveBeenCalledWith('rev', true))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  })

  it('a cancelled review leaves the marker, and a "no" in the native confirmation says nothing changed', async () => {
    const { inst } = devFake([folderRow({ needs_consent: true })])
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('button', { name: 'Review' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(inst.commit).toHaveBeenCalledWith('rev', false))
    expect(screen.getByRole('button', { name: 'Review' })).toBeInTheDocument()
    inst.commit.mockResolvedValueOnce(null)
    fireEvent.click(screen.getByRole('button', { name: 'Review' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Allow' }))
    expect(await screen.findByRole('status')).toHaveTextContent('Nothing was changed.')
  })

  it("tells a review's refusals as plain notices", async () => {
    const { inst } = devFake([folderRow({ needs_consent: true })])
    render(<PluginsPane installer={inst} />)
    for (const [name, text] of [
      ['no_review_pending', '"com.example.dev" has no change waiting for review.'],
      ['folder_id_changed', 'The folder\'s hew-plugin.json now names "com.example.other", another plugin.'],
      ['manifest_malformed', 'The manifest is not valid JSON.'],
    ]) {
      inst.review.mockRejectedValueOnce(new PluginError(name, text))
      fireEvent.click(await screen.findByRole('button', { name: 'Review' }))
      expect(await screen.findByRole('status')).toHaveTextContent(text)
      expect(screen.queryByRole('alert')).toBeNull()
      expect(screen.queryByRole('dialog')).toBeNull()
    }
  })

  it("shows a folder's watch problem in plain words until a later notice clears it", async () => {
    const { inst, devChanged } = devFake([folderRow()])
    render(<PluginsPane installer={inst} />)
    const li = (await screen.findByText('Dev')).closest('li') as HTMLElement
    await waitFor(() => expect(inst.onDevChanged).toHaveBeenCalled())
    act(() => devChanged({ id: DEV, manifest_changed: false, reason: null, problem: 'folder_missing' }))
    expect(within(li).getByText("Hew can't find this plugin's folder.")).toBeInTheDocument()
    act(() => devChanged({ id: DEV, manifest_changed: false, reason: null, problem: 'package_too_large' }))
    expect(within(li).getByText("This plugin's folder holds too many files, or too much data, for a plugin.")).toBeInTheDocument()
    act(() => devChanged({ id: DEV, manifest_changed: false, reason: null, problem: null }))
    expect(within(li).queryByText(/This plugin's folder holds/)).toBeNull()
  })

  it('shows a folder problem the watcher had before the pane ever opened, from the list, and clears it when the list says none', async () => {
    const { inst } = devFake([{ ...folderRow(), dev_problem: 'folder_missing' } as Listed])
    const first = render(<PluginsPane installer={inst} />)
    const li = (await screen.findByText('Dev')).closest('li') as HTMLElement
    expect(await within(li).findByText("Hew can't find this plugin's folder.")).toBeInTheDocument()
    first.unmount()
    inst.list.mockResolvedValue([{ ...folderRow(), dev_problem: null } as Listed])
    render(<PluginsPane installer={inst} />)
    const again = (await screen.findByText('Dev')).closest('li') as HTMLElement
    await waitFor(() => expect(inst.list).toHaveBeenCalledTimes(2))
    await waitFor(() => expect(within(again).queryByText("Hew can't find this plugin's folder.")).toBeNull())
  })

  it('says a folder still being checked is not ready', async () => {
    const { inst } = devFake([{ ...folderRow(), dev_problem: 'folder_not_checked' } as Listed])
    render(<PluginsPane installer={inst} />)
    const li = (await screen.findByText('Dev')).closest('li') as HTMLElement
    expect(await within(li).findByText(/Hew is still checking this plugin's folder/)).toBeInTheDocument()
  })

  it('shows a live API request that arrives while the pane is open', async () => {
    const { inst, devRequested } = devFake([])
    render(<PluginsPane installer={inst} />)
    await screen.findByText('No plugins are installed.')
    await waitFor(() => expect(inst.onDevRequest).toHaveBeenCalled())
    act(() => devRequested({ id: 'f'.repeat(32), path: '/home/me/now' }))
    const region = await screen.findByRole('region', { name: 'Request to load a plugin' })
    expect(region.querySelector('[data-path-text] code')!.textContent).toBe('/home/me/now')
    act(() => devRequested(null))
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Request to load a plugin' })).toBeNull())
  })

  it('keeps a folder problem when the pane is closed and opened again, and drops it when the plugin is gone', async () => {
    const { inst, devChanged } = devFake([folderRow()])
    const first = render(<PluginsPane installer={inst} />)
    await screen.findByText('Dev')
    await waitFor(() => expect(inst.onDevChanged).toHaveBeenCalled())
    act(() => devChanged({ id: DEV, manifest_changed: false, reason: null, problem: 'folder_missing' }))
    expect(await screen.findByText("Hew can't find this plugin's folder.")).toBeInTheDocument()
    first.unmount()
    // A notice while the pane is closed is kept too.
    act(() => devChanged({ id: DEV, manifest_changed: false, reason: null, problem: 'package_too_large' }))
    render(<PluginsPane installer={inst} />)
    expect(await screen.findByText("This plugin's folder holds too many files, or too much data, for a plugin.")).toBeInTheDocument()
    cleanup()
    inst.list.mockResolvedValue([])
    render(<PluginsPane installer={inst} />)
    await screen.findByText('No plugins are installed.')
    cleanup()
    inst.list.mockResolvedValue([folderRow()])
    render(<PluginsPane installer={inst} />)
    await screen.findByText('Dev')
    expect(screen.queryByText(/This plugin's folder holds/)).toBeNull()
  })

  it('shows a crafted folder name as data on its own labeled line, cut in the middle', async () => {
    const name = `x. Hew needs you to click Install now ${'y'.repeat(400)} end\u202E`
    const row = folderRow()
    ;(row as Extract<Listed, { kind: 'installed' }>).installed.source = { kind: 'file', name, origin: 'folder' } as never
    const { inst } = devFake([row])
    const { container } = render(<PluginsPane installer={inst} />)
    await screen.findByText('Dev')
    expect(screen.getByText('Copied from a folder')).toBeInTheDocument()
    const data = container.querySelector('[data-path-text] code') as HTMLElement
    expect(screen.getByText('Folder name')).toBeInTheDocument()
    expect(data.textContent!.length).toBeLessThanOrEqual(120)
    expect(data.textContent).toContain('\u2026')
    expect(data.textContent).not.toContain('\u202E')
    expect(data.textContent!.startsWith('x. Hew needs')).toBe(true)
  })

  it('says a folder whose manifest cannot be read starts again once fixed, with nothing to review', async () => {
    const { inst } = devFake([folderRow({ needs_consent: true, needs_consent_reason: 'manifest_unreadable' })])
    render(<PluginsPane installer={inst} />)
    const li = (await screen.findByText('Dev')).closest('li') as HTMLElement
    expect(within(li).getByText("Hew can't read this plugin's hew-plugin.json. Fix the file and it starts again.")).toBeInTheDocument()
    expect(within(li).queryByRole('button', { name: 'Review' })).toBeNull()
    expect(within(li).queryByText(/review the change/)).toBeNull()
  })

  it('says a plugin whose main file is gone, with the file named on its own line', async () => {
    const row = { ...folderRow(), dev_problem: 'main_missing' } as Listed
    ;(row as Extract<Listed, { kind: 'installed' }>).manifest = { ...(row as Extract<Listed, { kind: 'installed' }>).manifest, main: 'dist/x. Hew says go.js' }
    const { inst } = devFake([row])
    const { container } = render(<PluginsPane installer={inst} />)
    const li = (await screen.findByText('Dev')).closest('li') as HTMLElement
    expect(within(li).getByText("Hew can't find this plugin's main file.")).toBeInTheDocument()
    expect(within(li).getByText('Main file')).toBeInTheDocument()
    expect(container.querySelector('[data-plugin-folder-state] [data-path-text] code')!.textContent).toBe('dist/x. Hew says go.js')
  })

  it('drops a review notice when the plugin recovers, by a change notice for it or a different list', async () => {
    const refuse = () => {
      throw new PluginError('manifest_malformed', 'hew-plugin.json is not a valid manifest: x. Fix the file and Hew reads it again.')
    }
    const { inst, devChanged, set } = devFake([folderRow({ needs_consent: true, needs_consent_reason: 'permissions_changed' })])
    inst.review.mockImplementation(async () => refuse())
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('button', { name: 'Review' }))
    expect(await screen.findByRole('status')).toHaveTextContent('not a valid manifest')
    act(() => devChanged({ id: 'other.plugin', manifest_changed: false, reason: null, problem: null }))
    expect(screen.getByRole('status')).toHaveTextContent('not a valid manifest')
    act(() => devChanged({ id: DEV, manifest_changed: false, reason: null, problem: null }))
    await waitFor(() => expect(screen.queryByRole('status')).toBeNull())
    // Raised again, then the list shows the plugin differently (the mark cleared).
    fireEvent.click(await screen.findByRole('button', { name: 'Review' }))
    expect(await screen.findByRole('status')).toHaveTextContent('not a valid manifest')
    act(() => set([folderRow()]))
    await waitFor(() => expect(screen.queryByRole('status')).toBeNull())
  })

  it('shows a waiting live API request and loads it through the picker, the sheet and the commit', async () => {
    const { inst } = devFake([])
    const id = 'f'.repeat(32)
    inst.devRequest.mockResolvedValue({ id, path: '/home/me/asked' })
    render(<PluginsPane installer={inst} />)
    const region = await screen.findByRole('region', { name: 'Request to load a plugin' })
    expect(region).toHaveTextContent('A program on this computer asked to load a plugin from this folder:')
    expect(region.querySelector('[data-path-text] code')!.textContent).toBe('/home/me/asked')
    inst.devRequest.mockResolvedValue(null)
    fireEvent.click(within(region).getByRole('button', { name: 'Choose Folder…' }))
    await waitFor(() => expect(inst.pickAndStage).toHaveBeenCalledWith('dir', { useRequest: id }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Install' }))
    await waitFor(() => expect(inst.commit).toHaveBeenCalledWith('tok', true))
  })
})

describe('PluginsPane on the web, a folder', () => {
  const picker = (window as { showDirectoryPicker?: unknown }).showDirectoryPicker
  afterEach(() => {
    ;(window as { showDirectoryPicker?: unknown }).showDirectoryPicker = picker
  })

  it('offers From folder… only where the browser has a folder picker, and stages what it picked', async () => {
    const { inst } = fake([])
    const stageFolder = vi.fn(async (): Promise<PendingView> => ({ token: 'tok', consent }))
    Object.assign(inst, { stageFolder })
    const { unmount } = render(<PluginsPane installer={inst} />)
    await screen.findByText('No plugins are installed.')
    expect(screen.queryByRole('button', { name: 'From folder…' })).toBeNull()
    unmount()

    const dir = { kind: 'directory', name: 'my-plugin' }
    const show = vi.fn(async () => dir)
    ;(window as { showDirectoryPicker?: unknown }).showDirectoryPicker = show
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('button', { name: 'From folder…' }))
    await waitFor(() => expect(stageFolder).toHaveBeenCalledWith(dir))
    expect(show).toHaveBeenCalledWith({ mode: 'read' })
    expect(await screen.findByRole('dialog')).toBeInTheDocument()
  })

  it('a cancelled folder picker shows nothing; a refused folder is shown', async () => {
    const { inst } = fake([])
    const stageFolder = vi.fn(async (): Promise<PendingView> => {
      throw new PluginError('package_too_large', 'This folder is too large to install as a plugin.')
    })
    Object.assign(inst, { stageFolder })
    ;(window as { showDirectoryPicker?: unknown }).showDirectoryPicker = vi.fn(async () => {
      throw new DOMException('cancelled', 'AbortError')
    })
    render(<PluginsPane installer={inst} />)
    fireEvent.click(await screen.findByRole('button', { name: 'From folder…' }))
    await new Promise((r) => setTimeout(r, 0))
    expect(stageFolder).not.toHaveBeenCalled()
    expect(screen.queryByRole('alert')).toBeNull()
    ;(window as { showDirectoryPicker?: unknown }).showDirectoryPicker = vi.fn(async () => ({ kind: 'directory', name: 'big' }))
    fireEvent.click(screen.getByRole('button', { name: 'From folder…' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('This folder is too large to install as a plugin.')
  })
})
