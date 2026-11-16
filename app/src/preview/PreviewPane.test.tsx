import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { SettingsWindow } from '../settings/SettingsWindow'
import { previewSwitches } from './switches'
import { PREVIEW_SETTINGS } from './copy'
import type { PreviewFeature } from './registry'
import { requiresPreview } from '../test/requiresPreview'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const FEATURES = (JSON.parse(readFileSync(join(ROOT, 'preview-features.json'), 'utf8')) as { features: PreviewFeature[] }).features

describe('Settings > Preview (preview builds)', () => {
  beforeEach((ctx) => { requiresPreview(ctx) })
  beforeEach(() => previewSwitches?.set({ off: [], on: [] }))
  afterEach(() => previewSwitches?.set({ off: [], on: [] }))

  it('lists one row per registry feature: name, state line, switch', () => {
    render(<SettingsWindow />)
    fireEvent.click(screen.getByRole('tab', { name: 'Preview' }))
    const pane = screen.getByRole('tabpanel')
    for (const f of FEATURES) {
      expect(within(pane).getByText(f.name)).toBeInTheDocument()
      expect(within(pane).getByText(f.state, { exact: false })).toBeInTheDocument()
      expect(pane.querySelector(`[data-preview-switch="${f.id}"]`)).not.toBeNull()
    }
    expect(within(pane).getAllByRole('switch')).toHaveLength(FEATURES.length)
    // Every feature starts on, except one the registry marks off until asked for.
    for (const f of FEATURES) {
      expect((pane.querySelector(`[data-preview-switch="${f.id}"]`) as HTMLInputElement).checked, f.id).toBe(f.defaultOn !== false)
    }
  })

  it('stores a switch under the preview setting and says it applies at the next start', () => {
    render(<SettingsWindow />)
    fireEvent.click(screen.getByRole('tab', { name: 'Preview' }))
    const first = FEATURES[0]
    const sw = document.querySelector(`[data-preview-switch="${first.id}"]`) as HTMLInputElement
    fireEvent.click(sw)
    expect(previewSwitches?.get()).toEqual({ off: [first.id], on: [] })
    expect(localStorage.getItem('hew.settings.preview')).toBe(JSON.stringify({ off: [first.id] }))
    expect(screen.getByText(PREVIEW_SETTINGS.pending(false), { exact: false })).toBeInTheDocument()
    fireEvent.click(sw)
    expect(previewSwitches?.get()).toEqual({ off: [], on: [] })
  })

  it('turns a default-off feature on by storing it under "on", and off again by removing it', () => {
    const gated = FEATURES.find((f) => f.defaultOn === false)
    expect(gated, 'the registry has a default-off feature').toBeDefined()
    render(<SettingsWindow />)
    fireEvent.click(screen.getByRole('tab', { name: 'Preview' }))
    const sw = document.querySelector(`[data-preview-switch="${gated!.id}"]`) as HTMLInputElement
    expect(sw.checked).toBe(false)
    fireEvent.click(sw)
    expect(sw.checked).toBe(true)
    expect(localStorage.getItem('hew.settings.preview')).toBe(JSON.stringify({ off: [], on: [gated!.id] }))
    expect(screen.getByText(PREVIEW_SETTINGS.pending(true), { exact: false })).toBeInTheDocument()
    fireEvent.click(sw)
    expect(previewSwitches?.get()).toEqual({ off: [], on: [] })
    expect(localStorage.getItem('hew.settings.preview')).toBe(JSON.stringify({ off: [] }))
  })

  it('reads a value stored before default-off features existed', () => {
    const first = FEATURES.find((f) => f.defaultOn !== false)!
    localStorage.setItem('hew.settings.preview', JSON.stringify({ off: [first.id] }))
    window.dispatchEvent(new StorageEvent('storage', { key: 'hew.settings.preview', newValue: JSON.stringify({ off: [first.id] }) }))
    expect(previewSwitches?.get()).toEqual({ off: [first.id], on: [] })
  })
})
