import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { requiresPreview } from '../test/requiresPreview'
import { ConsentSheet, REVIEW_LINE, UNSIGNED_LINE } from './ConsentSheet'
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
    expect(screen.getByText('Developer folder')).toBeInTheDocument()
    // The path in full, set apart as data, not inside a sentence.
    expect(document.querySelector('[data-path-text] code')!.textContent).toBe('/home/me/plugin')
    expect(screen.getByText('The files in this folder can change after you install.')).toBeInTheDocument()
  })

  it('in review mode lists what changed, then what it keeps, never a new permission twice', () => {
    const { dialog } = open({
      review: true,
      source: { kind: 'dir', path: '/home/me/plugin' },
      replaces: { name: 'Parts Lister', version: '1.0.0' },
      permissions: [
        { name: 'model.read', text: 'Read your model' },
        { name: 'clipboard', text: 'Read and write the clipboard' },
      ],
      update: {
        from_version: '1.0.0',
        added: [{ name: 'clipboard', text: 'Read and write the clipboard' }],
        new_optional: [],
        removed: [],
        removed_optional: [],
        startup_newly: false,
      },
    })
    expect(within(dialog).getAllByText('Read and write the clipboard')).toHaveLength(1)
    expect(within(dialog).getByText('Now asks for')).toBeInTheDocument()
    expect(within(dialog).getByText('It keeps')).toBeInTheDocument()
    expect(within(dialog).queryByText('It can')).toBeNull()
    expect(within(dialog).getByText('Read the model: every object, component, material, tag, and attribute')).toBeInTheDocument()
    expect(within(dialog).getByText('Now asks for').compareDocumentPosition(within(dialog).getByText('It keeps'))).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    )
  })

  it('in review mode shows the line the host chose for the reason, and the old name when it changed', () => {
    const { dialog } = open({
      review: true,
      review_line: 'This plugin now starts with Hew.',
      source: { kind: 'dir', path: '/home/me/plugin' },
      replaces: { name: 'Old Name', version: '1.0.0' },
    })
    expect(within(dialog).getByText('This plugin now starts with Hew.')).toBeInTheDocument()
    expect(within(dialog).queryByText(REVIEW_LINE)).toBeNull()
    const was = dialog.querySelector('[data-consent="was-named"]') as HTMLElement
    expect(within(was).getByText('It was named')).toBeInTheDocument()
    expect(was.querySelector('code')!.textContent).toBe('Old Name')
  })

  it('in review mode asks to allow the changes, says why in place of the replaces line, lists both sides, and accepts with Allow', () => {
    const { dialog, onInstall } = open({
      review: true,
      version: '1.0.0',
      source: { kind: 'dir', path: '/home/me/plugin' },
      replaces: { name: 'Parts Lister', version: '1.0.0' },
      update: {
        from_version: '1.0.0',
        added: [{ name: 'clipboard', text: '' }],
        new_optional: [],
        removed: [{ name: 'files', text: '' }],
        removed_optional: [{ name: 'network:api.example.com', text: 'Connect to api.example.com (checked by Hew)' }],
        startup_newly: false,
      },
    })
    expect(within(dialog).getByRole('heading', { name: "Allow this plugin's changes?" })).toBeInTheDocument()
    expect(within(dialog).getByText(REVIEW_LINE)).toBeInTheDocument()
    expect(dialog.querySelector('[data-consent="replaces"]')).toBeNull()
    expect(within(dialog).queryByText(/Install .* again\?/)).toBeNull()
    expect(within(dialog).getByText('Now asks for')).toBeInTheDocument()
    expect(within(dialog).getByText('Read and write the clipboard')).toBeInTheDocument()
    const removed = dialog.querySelector('[data-consent="removed"]') as HTMLElement
    expect(within(dialog).getByText('No longer asks for')).toBeInTheDocument()
    expect(within(removed).getByText('Ask you to pick files, and save files you choose')).toBeInTheDocument()
    expect(within(removed).getByText('No longer may ask for later:')).toBeInTheDocument()
    expect(within(removed).getByText('Connect to api.example.com (checked by Hew)')).toBeInTheDocument()
    expect(within(dialog).getByText('The files in this folder can change after you install.')).toBeInTheDocument()
    expect(within(dialog).queryByRole('button', { name: 'Install' })).toBeNull()
    fireEvent.click(within(dialog).getByRole('button', { name: 'Allow' }))
    expect(onInstall).toHaveBeenCalledOnce()
  })

  it('in review mode with only removals says it asks for nothing new, and still lists what it no longer asks for', () => {
    const { dialog } = open({
      review: true,
      source: { kind: 'dir', path: '/home/me/plugin' },
      update: { from_version: '1.2.0', added: [], new_optional: [], removed: [{ name: 'files', text: '' }], removed_optional: [], startup_newly: false },
    })
    expect(within(dialog).getByText('It asks for nothing new.')).toBeInTheDocument()
    const removed = dialog.querySelector('[data-consent="removed"]') as HTMLElement
    expect(within(removed).getByText('Ask you to pick files, and save files you choose')).toBeInTheDocument()
  })

  it('shows a folder path and a file name cleaned', () => {
    open({ source: { kind: 'dir', path: '/home/\u202Eme\u0000/plugin' } })
    expect(document.querySelector('[data-path-text] code')!.textContent).toBe('/home/me/plugin')
  })

  it('says a folder copied in on the web was copied from a folder, with its name as data', () => {
    open({ source: { kind: 'file', name: 'my-plugin', origin: 'folder' } })
    expect(screen.getByText('Copied from a folder')).toBeInTheDocument()
    expect(screen.queryByText('From file')).toBeNull()
    expect(document.querySelector('[data-path-text] code')!.textContent).toBe('my-plugin')
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
