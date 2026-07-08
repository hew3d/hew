/**
 *  — component tests for SettingsWindow, UnitsPane, and DebugPane.
 *
 * SettingsWindow: macOS-style toolbar tab strip (Units / Theme / Debug tabs
 *   with aria-selected) switching the pane rendered in the tabpanel below.
 * UnitsPane: system selector changes format options; selecting a format updates
 *   the singleton (which persists to localStorage via in-memory Storage polyfill).
 * DebugPane: checkbox toggles Debug Mode.
 *
 * The units / debugMode singletons carry module-level state that is reset between
 * tests via the singleton's own setters (faster than vi.resetModules).
 */

import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest'
import { SettingsWindow } from './SettingsWindow'
import { UnitsPane } from './UnitsPane'
import { DebugPane } from './DebugPane'
import { setLengthUnit, getLengthUnit } from './units'
import { setDebugMode, getDebugMode } from './debugMode'

// DebugPane conditionally renders a "Download diagnostic log" button (web path)
// vs a <p> about the file path (Tauri path). We are always on the web path in
// tests because isTauri = false.  downloadDiagnosticLog is a real function that
// creates an <a> element; mock it to avoid DOM side-effects.
vi.mock('../log/diagnosticLog', async (importOriginal) => {
  const real = await importOriginal<typeof import('../log/diagnosticLog')>()
  return { ...real, downloadDiagnosticLog: vi.fn() }
})

describe('SettingsWindow', () => {
  it('shows the toolbar tab strip with one tab per pane', () => {
    render(<SettingsWindow />)
    const tabs = screen.getAllByRole('tab')
    expect(tabs.map((t) => t.textContent)).toEqual(['Units', 'Theme', 'Debug'])
  })

  it('shows the Units pane by default (Units tab selected)', () => {
    render(<SettingsWindow />)
    expect(screen.getByRole('tab', { name: 'Units' })).toHaveAttribute('aria-selected', 'true')
    // Units pane content: the System row label.
    expect(screen.getByText('System')).toBeInTheDocument()
  })

  it('switches to the Debug pane when the Debug tab is clicked', () => {
    render(<SettingsWindow />)
    fireEvent.click(screen.getByRole('tab', { name: 'Debug' }))
    expect(screen.getByRole('tab', { name: 'Debug' })).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByRole('tab', { name: 'Units' })).toHaveAttribute('aria-selected', 'false')
    // Debug pane content replaced Units pane content.
    expect(screen.getByRole('checkbox', { name: /enable debug mode/i })).toBeInTheDocument()
    expect(screen.queryByText('System')).not.toBeInTheDocument()
  })

  it('returns to the Units pane when the Units tab is clicked after switching', () => {
    render(<SettingsWindow />)
    fireEvent.click(screen.getByRole('tab', { name: 'Debug' }))
    fireEvent.click(screen.getByRole('tab', { name: 'Units' }))
    expect(screen.getByRole('tab', { name: 'Units' })).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByText('System')).toBeInTheDocument()
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument()
  })

  it('switches to the Theme pane when the Theme tab is clicked', () => {
    render(<SettingsWindow />)
    fireEvent.click(screen.getByRole('tab', { name: 'Theme' }))
    expect(screen.getByRole('tab', { name: 'Theme' })).toHaveAttribute('aria-selected', 'true')
    // Theme pane content: the Appearance selector.
    expect(screen.getByLabelText('Appearance')).toBeInTheDocument()
  })

  it('highlights the active tab with the accent-tint rounded rect (theme tokens, no hardcoded colors)', () => {
    render(<SettingsWindow />)
    const units = screen.getByRole('tab', { name: 'Units' })
    const debug = screen.getByRole('tab', { name: 'Debug' })
    expect(units.style.background).toContain('--accent-tint-15')
    expect(units.style.color).toContain('--accent-text-on-tint')
    expect(debug.style.background).toBe('transparent')
  })
})

describe('UnitsPane', () => {
  beforeEach(() => {
    // Reset to a known state before each test
    setLengthUnit('m')
  })

  afterEach(() => {
    setLengthUnit('m')
  })

  it('renders the System and Format selectors with associated labels', () => {
    render(<UnitsPane />)
    // The form-grid labels are real <label htmlFor> now (SettingsForm.tsx), so
    // the selects are queryable by their accessible names.
    expect(screen.getByLabelText('System')).toBeInTheDocument()
    expect(screen.getByLabelText('Format')).toBeInTheDocument()
    expect(screen.getAllByRole('combobox')).toHaveLength(2)
  })

  it('defaults to Metric system when the unit is meters', () => {
    setLengthUnit('m')
    render(<UnitsPane />)
    const [systemSelect] = screen.getAllByRole('combobox') as HTMLSelectElement[]
    expect(systemSelect.value).toBe('metric')
  })

  it('defaults to Imperial system when the unit is architectural', () => {
    setLengthUnit('arch')
    render(<UnitsPane />)
    const [systemSelect] = screen.getAllByRole('combobox') as HTMLSelectElement[]
    expect(systemSelect.value).toBe('imperial')
  })

  it('switching system to Imperial selects the default Imperial format', () => {
    setLengthUnit('m')
    render(<UnitsPane />)
    const [systemSelect] = screen.getAllByRole('combobox') as HTMLSelectElement[]
    fireEvent.change(systemSelect, { target: { value: 'imperial' } })
    // DEFAULT_FORMAT_FOR_SYSTEM['imperial'] = 'arch' (Architectural)
    expect(getLengthUnit()).toBe('arch')
  })

  it('switching format within Metric updates the singleton', () => {
    setLengthUnit('m')
    render(<UnitsPane />)
    const [, formatSelect] = screen.getAllByRole('combobox') as HTMLSelectElement[]
    fireEvent.change(formatSelect, { target: { value: 'cm' } })
    expect(getLengthUnit()).toBe('cm')
  })

  it('shows only Metric formats when the system is Metric', () => {
    setLengthUnit('m')
    render(<UnitsPane />)
    const [, formatSelect] = screen.getAllByRole('combobox') as HTMLSelectElement[]
    const options = Array.from(formatSelect.options).map((o) => o.value)
    // Metric formats: m, cm, mm — no imperial formats
    expect(options).toContain('m')
    expect(options).toContain('cm')
    expect(options).toContain('mm')
    expect(options).not.toContain('arch')
    expect(options).not.toContain('frac')
  })

  it('shows only Imperial formats when the system is Imperial', () => {
    setLengthUnit('arch')
    render(<UnitsPane />)
    const [, formatSelect] = screen.getAllByRole('combobox') as HTMLSelectElement[]
    const options = Array.from(formatSelect.options).map((o) => o.value)
    expect(options).toContain('arch')
    expect(options).not.toContain('m')
  })
})

describe('DebugPane', () => {
  beforeEach(() => {
    setDebugMode(false)
    vi.clearAllMocks()
  })

  afterEach(() => {
    setDebugMode(false)
  })

  it('renders the Debug Mode row label and checkbox', () => {
    render(<DebugPane />)
    expect(screen.getByText('Debug Mode')).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: /enable debug mode/i })).toBeInTheDocument()
  })

  it('checkbox is unchecked when debug mode is off', () => {
    setDebugMode(false)
    render(<DebugPane />)
    expect(screen.getByRole('checkbox')).not.toBeChecked()
  })

  it('checkbox is checked when debug mode is on', () => {
    setDebugMode(true)
    render(<DebugPane />)
    expect(screen.getByRole('checkbox')).toBeChecked()
  })

  it('enables Debug Mode when the checkbox is clicked while off', () => {
    setDebugMode(false)
    render(<DebugPane />)
    fireEvent.click(screen.getByRole('checkbox'))
    expect(getDebugMode()).toBe(true)
  })

  it('disables Debug Mode when the checkbox is clicked while on', () => {
    setDebugMode(true)
    render(<DebugPane />)
    fireEvent.click(screen.getByRole('checkbox'))
    expect(getDebugMode()).toBe(false)
  })

  it('renders a Download diagnostic log button on web (not Tauri)', () => {
    // isTauri is false in jsdom — the web branch should render a button
    render(<DebugPane />)
    expect(screen.getByRole('button', { name: /download diagnostic log/i })).toBeInTheDocument()
  })
})
