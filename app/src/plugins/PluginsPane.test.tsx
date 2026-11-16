import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { requiresPreview } from '../test/requiresPreview'
import { PluginsPane } from './PluginsPane'
import { PluginError } from './pluginError'
import type { ConsentPayload, Installed, Listed, PendingView, PluginInstaller } from './types'

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
  it('says plugins do not run yet, and lists nothing when nothing is installed', async () => {
    const { inst } = fake([])
    render(<PluginsPane installer={inst} />)
    expect(screen.getByText(/do not run yet in this build/)).toBeInTheDocument()
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
      await waitFor(() => expect(inst.rollback).toHaveBeenCalledWith('com.example.one'))
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
