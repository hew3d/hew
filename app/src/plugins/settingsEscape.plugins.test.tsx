// @vitest-environment jsdom
/**
 * Escape inside the Windows (Fluent) Settings page: Settings' own handler
 * is registered first, on the window, in the capture phase, so it sees
 * Escape before any plugin sheet or page does. Every plugin sheet is a
 * dialog, and the detail page and the pane's confirmations are marked, so
 * Settings leaves Escape to them (`settingsEscape.ts`).
 */

import { act, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import { FluentSettingsPage } from '../settings/FluentSettingsPage'
import { ConsentSheet } from './ConsentSheet'
import type { ConsentPayload } from './types'

const consent: ConsentPayload = {
  id: 'com.example.x',
  name: 'Parts Lister',
  version: '1.2.0',
  permissions: [],
  optional: [],
  warnings: [],
  on_startup: false,
  activation_reason: null,
  signer: null,
  unsigned: true,
  source: { kind: 'file', name: 'parts.hewplugin' },
  replaces: null,
  update: null,
}

const escape = (): void => {
  act(() => {
    ;(document.activeElement ?? document.body).dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
  })
}

describe('Escape in the Fluent Settings page with a plugin sheet open', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('cancels the sheet and leaves Settings open; the next Escape closes Settings', () => {
    const onBack = vi.fn()
    const onCancel = vi.fn()
    const settings = render(<FluentSettingsPage onBack={onBack} />)
    const sheet = render(<ConsentSheet consent={consent} onInstall={() => {}} onCancel={onCancel} />)
    escape()
    expect(onCancel).toHaveBeenCalledTimes(1)
    expect(onBack).not.toHaveBeenCalled()
    sheet.unmount()
    escape()
    expect(onBack).toHaveBeenCalledTimes(1)
    settings.unmount()
  })

  it('leaves Escape to a part of Settings marked as owning it (the detail page, a confirmation)', () => {
    const onBack = vi.fn()
    render(<FluentSettingsPage onBack={onBack} />)
    const page = document.createElement('div')
    page.setAttribute('data-settings-escape-child', '')
    document.body.appendChild(page)
    escape()
    expect(onBack).not.toHaveBeenCalled()
    page.remove()
    escape()
    expect(onBack).toHaveBeenCalledTimes(1)
  })
})

describe('Escape in the Fluent Settings page with nothing open over it', () => {
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('closes Settings on the first Escape, as on main (no plugin sheet, no detail page)', () => {
    const onBack = vi.fn()
    render(<FluentSettingsPage onBack={onBack} />)
    escape()
    expect(onBack).toHaveBeenCalledTimes(1)
  })
})
