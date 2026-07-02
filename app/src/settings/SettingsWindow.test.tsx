/**
 *  — component tests for SettingsWindow, UnitsPane, and DebugPane.
 *
 * SettingsWindow: category nav (Units / Debug).
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
  it('shows "Units" heading by default (Units pane active)', () => {
    render(<SettingsWindow />)
    expect(screen.getByRole('heading', { name: /units/i })).toBeInTheDocument()
  })

  it('switches to the Debug pane when Debug category is clicked', () => {
    render(<SettingsWindow />)
    fireEvent.click(screen.getByText('Debug'))
    expect(screen.getByRole('heading', { name: /debug/i })).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: /^units$/i })).not.toBeInTheDocument()
  })

  it('returns to the Units pane when Units category is clicked after switching', () => {
    render(<SettingsWindow />)
    fireEvent.click(screen.getByText('Debug'))
    fireEvent.click(screen.getByText('Units'))
    expect(screen.getByRole('heading', { name: /^units$/i })).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: /^debug$/i })).not.toBeInTheDocument()
  })

  it('highlights the active category', () => {
    render(<SettingsWindow />)
    // 'Units' appears twice when the Units pane is active: the nav item and the
    // pane's <h3> heading. The nav item (not the heading) carries the active
    // highlight (the accent-on-tint token, Follow-up: — was a
    // hardcoded '#fff').
    const unitsNav = screen.getAllByText('Units').find((el) => el.tagName !== 'H3')
    expect(unitsNav?.style.color).toBe('var(--accent-text-on-tint, #fff)')
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

  it('renders the System and Format selectors', () => {
    render(<UnitsPane />)
    // The labels aren't associated to the selects (no htmlFor/id), so query the
    // two <select>s by role; the System/Format text labels render alongside them.
    expect(screen.getByText('System')).toBeInTheDocument()
    expect(screen.getByText('Format')).toBeInTheDocument()
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

  it('renders the Debug heading and checkbox', () => {
    render(<DebugPane />)
    expect(screen.getByRole('heading', { name: /debug/i })).toBeInTheDocument()
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
