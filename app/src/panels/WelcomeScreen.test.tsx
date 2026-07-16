/**
 * WelcomeScreen tests — the launch dialog's sections, callbacks, and
 * dismissal, plus the persisted show-on-startup setting module.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import {
  WelcomeScreen,
  BUNDLED_SAMPLES,
  GETTING_STARTED_URL,
} from './WelcomeScreen'
import { getShowWelcome, setShowWelcome } from '../settings/welcomeScreen'

function makeProps() {
  return {
    recentFiles: [] as string[],
    onClose: vi.fn(),
    onOpen: vi.fn(),
    onOpenRecent: vi.fn(),
    onOpenSample: vi.fn(),
    showOnStartup: true,
    onShowOnStartupChange: vi.fn(),
    unit: 'm' as const,
    onUnitChange: vi.fn(),
  }
}

describe('WelcomeScreen', () => {
  it('renders the dialog with both bundled samples and the guide link', () => {
    render(<WelcomeScreen {...makeProps()} />)
    expect(screen.getByRole('dialog', { name: /welcome to hew/i })).toBeInTheDocument()
    expect(screen.getByText('Wall Clock')).toBeInTheDocument()
    expect(screen.getByText('Café Table')).toBeInTheDocument()
    const link = screen.getByRole('link', { name: /getting-started guide/i })
    expect(link).toHaveAttribute('href', GETTING_STARTED_URL)
    expect(link).toHaveAttribute('target', '_blank')
  })

  it('clicking a sample reports that sample entry', () => {
    const props = makeProps()
    render(<WelcomeScreen {...props} />)
    fireEvent.click(screen.getByText('Café Table'))
    expect(props.onOpenSample).toHaveBeenCalledWith(
      BUNDLED_SAMPLES.find((s) => s.file === 'cafe-table.hew'),
    )
  })

  it('hides the Recent section when empty and lists basenames (capped at 5) when not', () => {
    const props = makeProps()
    const { rerender } = render(<WelcomeScreen {...props} />)
    expect(screen.queryByText('Recent')).not.toBeInTheDocument()

    const paths = [
      'C:\\models\\Bracket.hew',
      '/home/user/models/Enclosure.hew',
      '/a/1.hew', '/a/2.hew', '/a/3.hew', '/a/4.hew',
    ]
    rerender(<WelcomeScreen {...props} recentFiles={paths} />)
    expect(screen.getByText('Recent')).toBeInTheDocument()
    expect(screen.getByText('Bracket.hew')).toBeInTheDocument()
    expect(screen.getByText('Enclosure.hew')).toBeInTheDocument()
    // Six entries in, five rendered.
    expect(screen.queryByText('4.hew')).not.toBeInTheDocument()

    fireEvent.click(screen.getByText('Enclosure.hew'))
    expect(props.onOpenRecent).toHaveBeenCalledWith('/home/user/models/Enclosure.hew')
  })

  it('Open a file, Start modeling, and Escape all report their callbacks', () => {
    const props = makeProps()
    render(<WelcomeScreen {...props} />)
    fireEvent.click(screen.getByText(/open a file/i))
    expect(props.onOpen).toHaveBeenCalledOnce()
    fireEvent.click(screen.getByRole('button', { name: /start modeling/i }))
    expect(props.onClose).toHaveBeenCalledOnce()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(props.onClose).toHaveBeenCalledTimes(2)
  })

  it('the units dropdown lists every format flat and reports a choice', () => {
    const props = makeProps()
    render(<WelcomeScreen {...props} />)
    const select = screen.getByRole('combobox', { name: /units/i })
    expect(select).toHaveValue('m')
    // One flat dropdown — all six formats, no system pre-step.
    expect(select.querySelectorAll('option')).toHaveLength(6)
    fireEvent.change(select, { target: { value: 'cm' } })
    expect(props.onUnitChange).toHaveBeenCalledWith('cm')
  })

  it('the show-on-startup checkbox reflects and reports its value', () => {
    const props = makeProps()
    render(<WelcomeScreen {...props} showOnStartup={true} />)
    const box = screen.getByRole('checkbox', { name: /show on startup/i })
    expect(box).toBeChecked()
    fireEvent.click(box)
    expect(props.onShowOnStartupChange).toHaveBeenCalledWith(false)
  })
})

describe('welcomeScreen setting', () => {
  beforeEach(() => localStorage.clear())

  it('defaults to showing', () => {
    expect(getShowWelcome()).toBe(true)
  })

  it('persists an opt-out and an opt-back-in', () => {
    setShowWelcome(false)
    expect(getShowWelcome()).toBe(false)
    setShowWelcome(true)
    expect(getShowWelcome()).toBe(true)
  })

  it('treats junk storage values as showing', () => {
    localStorage.setItem('hew.settings.showWelcome', 'banana')
    expect(getShowWelcome()).toBe(true)
  })
})
