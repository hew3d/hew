import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { requiresPreview } from '../test/requiresPreview'
import { ConsentSheet, UNSIGNED_LINE } from './ConsentSheet'
import type { ConsentPayload } from './types'

beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

const base: ConsentPayload = {
  id: 'com.example.x',
  name: 'Parts Lister',
  version: '1.2.0',
  permissions: [
    { name: 'model.read', text: 'ignored: the app has its own words' },
    { name: 'network:api.example.com', text: 'Connect to api.example.com (checked by Hew)' },
  ],
  optional: [{ name: 'files', text: 'Ask you to pick files, and save files you choose' }],
  warnings: ['Can send what it reads from your model to api.example.com.'],
  on_startup: false,
  activation_reason: null,
  signer: null,
  unsigned: true,
  source: { kind: 'file', name: 'parts.hewplugin' },
  replaces: null,
  update: null,
}

const open = (over: Partial<ConsentPayload> = {}, props: Partial<Parameters<typeof ConsentSheet>[0]> = {}) => {
  const onInstall = vi.fn()
  const onCancel = vi.fn()
  render(<ConsentSheet consent={{ ...base, ...over }} onInstall={onInstall} onCancel={onCancel} {...props} />)
  return { onInstall, onCancel, dialog: screen.getByRole('dialog') }
}

afterEach(() => vi.restoreAllMocks())

describe('ConsentSheet', () => {
  it('shows the name, version, source, every permission, the optional ones, the warning and the unsigned line', () => {
    const { dialog } = open()
    expect(within(dialog).getByText('Install Parts Lister?')).toBeInTheDocument()
    expect(within(dialog).getByText('Version 1.2.0')).toBeInTheDocument()
    expect(within(dialog).getByText('From file')).toBeInTheDocument()
    expect(within(dialog).getByText('Read the model: every object, component, material, tag, and attribute')).toBeInTheDocument()
    expect(within(dialog).getByText('Connect to api.example.com (checked by Hew)')).toBeInTheDocument()
    expect(within(dialog).getByText('May ask for later')).toBeInTheDocument()
    expect(within(dialog).getByText('Ask you to pick files, and save files you choose')).toBeInTheDocument()
    expect(within(dialog).getByText(UNSIGNED_LINE)).toBeInTheDocument()
    const warnings = within(dialog).getByRole('group', { name: 'Warnings from Hew' })
    expect(within(warnings).getByText('Can send what it reads from your model to api.example.com.')).toBeInTheDocument()
  })

  it('shows the unsigned line only for an unsigned package', () => {
    open({ unsigned: false, signer: 'Example Co' })
    expect(screen.queryByText(UNSIGNED_LINE)).toBeNull()
  })

  it('says a folder install can change after install, and shows the folder path', () => {
    open({ source: { kind: 'dir', path: '/home/me/plugin' } })
    expect(screen.getByText('Unpacked folder: /home/me/plugin')).toBeInTheDocument()
    expect(screen.getByText('The files in this folder can change after you install.')).toBeInTheDocument()
  })

  it('shows the replaces line and the update diff, with the new lines highlighted', () => {
    open({
      replaces: { name: 'Parts Lister', version: '1.0.0' },
      update: {
        from_version: '1.0.0',
        added: [{ name: 'files', text: '' }],
        new_optional: [{ name: 'clipboard', text: '' }],
        startup_newly: true,
      },
    })
    expect(screen.getByText('Replaces Parts Lister v1.0.0. Hew cannot check it is the same author.')).toBeInTheDocument()
    expect(screen.getByText('Update Parts Lister?')).toBeInTheDocument()
    expect(screen.getByText('New since 1.0.0')).toBeInTheDocument()
    expect(screen.getByText('It now asks to start whenever Hew starts.')).toBeInTheDocument()
    const added = screen.getAllByText('Ask you to pick files, and save files you choose').find((el) => el.closest('h3') === null)
    expect(added).toBeDefined()
    expect(document.querySelectorAll('[data-permission="clipboard"]')).toHaveLength(1)
  })

  it('reads as a reinstall when the same version replaces itself', () => {
    const { dialog } = open({
      version: '1.0.0',
      replaces: { name: 'Parts Lister', version: '1.0.0' },
      update: { from_version: '1.0.0', added: [], new_optional: [], startup_newly: false },
    })
    expect(within(dialog).getByText('Install Parts Lister again?')).toBeInTheDocument()
    expect(within(dialog).getByText('Version 1.0.0')).toBeInTheDocument()
    expect(within(dialog).getByText('Replaces the installed Parts Lister v1.0.0 with this copy.')).toBeInTheDocument()
    expect(within(dialog).queryByText(/Hew cannot check it is the same author/)).toBeNull()
    expect(within(dialog).getByRole('button', { name: 'Install' })).toBeInTheDocument()
    expect(within(dialog).queryByRole('button', { name: 'Update' })).toBeNull()
  })

  it('names the incoming plugin and the installed one, with versions, when the same version replaces itself under another name', () => {
    const { dialog } = open({
      name: 'Parts Lister Pro',
      version: '1.0.0',
      replaces: { name: 'Parts Lister', version: '1.0.0' },
      update: { from_version: '1.0.0', added: [], new_optional: [], startup_newly: false },
    })
    expect(within(dialog).getByRole('heading', { name: 'Install Parts Lister Pro again?' })).toBeInTheDocument()
    expect(within(dialog).getByText('Version 1.0.0')).toBeInTheDocument()
    expect(within(dialog).getByText('Replaces the installed Parts Lister v1.0.0 with this copy.')).toBeInTheDocument()
  })

  it('shows the plugin own words apart, as text and never as markup', () => {
    const { dialog } = open({ activation_reason: '<img src=x onerror=alert(1)> and <b>bold</b>' })
    const says = dialog.querySelector('[data-consent="plugin-says"]') as HTMLElement
    expect(within(says).getByText('The plugin says')).toBeInTheDocument()
    expect(says.textContent).toContain('<img src=x onerror=alert(1)> and <b>bold</b>')
    expect(says.querySelector('img, b')).toBeNull()
    // Nothing of it is inside Hew's warning box or permission lists.
    expect(dialog.querySelector('[data-consent="warnings"]')?.textContent).not.toContain('onerror')
  })

  it('puts the plugin block after Hew lines and leaves it out when there is no reason', () => {
    const { dialog } = open({ activation_reason: 'Keeps a parts list up to date.' })
    const lines = dialog.textContent as string
    expect(lines.indexOf('It can')).toBeLessThan(lines.indexOf('The plugin says'))
    expect(lines.indexOf('Can send what')).toBeLessThan(lines.indexOf('The plugin says'))
  })

  it('starts with focus on Cancel; Escape cancels; Enter does nothing, anywhere', () => {
    const { onInstall, onCancel, dialog } = open()
    const cancel = within(dialog).getByRole('button', { name: 'Cancel' })
    expect(cancel).toHaveFocus()
    // Enter on the focused button, on the install button and on the dialog itself.
    for (const target of [cancel, within(dialog).getByRole('button', { name: 'Install' }), dialog]) {
      const ev = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
      target.dispatchEvent(ev)
      expect(ev.defaultPrevented).toBe(true)
    }
    expect(onInstall).not.toHaveBeenCalled()
    expect(onCancel).not.toHaveBeenCalled()
    fireEvent.keyDown(document.body, { key: 'Escape' })
    expect(onCancel).toHaveBeenCalledOnce()
    expect(onInstall).not.toHaveBeenCalled()
  })

  it('Install and Cancel buttons call their handlers', () => {
    const { onInstall, onCancel, dialog } = open()
    fireEvent.click(within(dialog).getByRole('button', { name: 'Install' }))
    expect(onInstall).toHaveBeenCalledOnce()
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }))
    expect(onCancel).toHaveBeenCalledOnce()
  })

  it('does not let Escape or clicks reach a window behind it', () => {
    const behind = vi.fn()
    document.addEventListener('keydown', behind)
    const click = vi.fn()
    render(<div onClick={click}><ConsentSheet consent={base} onInstall={() => undefined} onCancel={() => undefined} /></div>)
    fireEvent.keyDown(document.body, { key: 'Escape' })
    expect(behind).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('dialog'))
    expect(click).not.toHaveBeenCalled()
    document.removeEventListener('keydown', behind)
  })

  it('keeps Tab inside the sheet and shows a refusal in place', () => {
    const { dialog } = open({}, { error: 'This install waited too long for an answer. Start it again.' })
    expect(within(dialog).getByRole('alert')).toHaveTextContent('waited too long')
    const install = within(dialog).getByRole('button', { name: 'Install' })
    install.focus()
    const ev = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true })
    install.dispatchEvent(ev)
    expect(ev.defaultPrevented).toBe(true)
    expect(dialog.contains(document.activeElement)).toBe(true)
  })

  it('disables the buttons while an install is being written', () => {
    const { dialog } = open({}, { busy: true })
    expect(within(dialog).getByRole('button', { name: 'Install' })).toBeDisabled()
    expect(within(dialog).getByRole('button', { name: 'Cancel' })).toBeDisabled()
  })

  it('ignores Escape while an install is being written, and still keeps the window behind from seeing it', () => {
    const behind = vi.fn()
    document.addEventListener('keydown', behind)
    const { onCancel } = open({}, { busy: true })
    const ev = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    document.body.dispatchEvent(ev)
    expect(onCancel).not.toHaveBeenCalled()
    expect(ev.defaultPrevented).toBe(true)
    expect(behind).not.toHaveBeenCalled()
    document.removeEventListener('keydown', behind)
  })

  it('says when a plugin starts with Hew', () => {
    open({ on_startup: true })
    expect(screen.getByText('Starts with Hew')).toBeInTheDocument()
  })

  it('shows at most 12 lines of a long list, with Show all to see the rest, and the buttons stay', () => {
    const many = Array.from({ length: 40 }, (_, i) => ({ name: `network:host${i}.example.com`, text: '' }))
    const { dialog } = open({ permissions: many })
    expect(dialog.querySelectorAll('[data-permission^="network:"]')).toHaveLength(12)
    fireEvent.click(within(dialog).getByRole('button', { name: 'Show all 40' }))
    expect(dialog.querySelectorAll('[data-permission^="network:"]')).toHaveLength(40)
    expect(within(dialog).queryByRole('button', { name: /Show all/ })).toBeNull()
    expect(within(dialog).getByRole('button', { name: 'Install' })).toBeInTheDocument()
  })

  it('shows a refusal such as too many permissions as its message', () => {
    open({}, { error: 'This plugin asks for more than 64 permissions.' })
    expect(screen.getByRole('alert')).toHaveTextContent('more than 64 permissions')
  })
})
