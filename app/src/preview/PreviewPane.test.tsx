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
  beforeEach(() => previewSwitches?.set([]))
  afterEach(() => previewSwitches?.set([]))

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
    // Every feature starts on.
    for (const sw of within(pane).getAllByRole('switch')) expect((sw as HTMLInputElement).checked).toBe(true)
  })

  it('stores a switch under the preview setting and says it applies at the next start', () => {
    render(<SettingsWindow />)
    fireEvent.click(screen.getByRole('tab', { name: 'Preview' }))
    const first = FEATURES[0]
    const sw = document.querySelector(`[data-preview-switch="${first.id}"]`) as HTMLInputElement
    fireEvent.click(sw)
    expect(previewSwitches?.get()).toEqual([first.id])
    expect(localStorage.getItem('hew.settings.preview')).toBe(JSON.stringify({ off: [first.id] }))
    expect(screen.getByText(PREVIEW_SETTINGS.pending(false), { exact: false })).toBeInTheDocument()
    fireEvent.click(sw)
    expect(previewSwitches?.get()).toEqual([])
  })
})
