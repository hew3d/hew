/**
 *  — component tests for LogPanel.
 *
 * LogPanel subscribes to the real LogStore singleton. Tests drive real log
 * entries through LogStore and assert the panel reflects them.
 */

import { render, screen, fireEvent, act } from '@testing-library/react'
import { describe, expect, it, beforeEach, vi } from 'vitest'
import { LogPanel } from './LogPanel'
import * as LogStore from './LogStore'

beforeEach(() => {
  LogStore.clear()
})

describe('LogPanel', () => {
  it('shows "No log entries" when the store is empty', () => {
    render(<LogPanel />)
    expect(screen.getByText(/no log entries/i)).toBeInTheDocument()
  })

  it('renders an info entry emitted to LogStore', () => {
    LogStore.log.info('app', 'kernel loaded')
    render(<LogPanel />)
    expect(screen.getByText('[app]')).toBeInTheDocument()
    expect(screen.getByText('kernel loaded')).toBeInTheDocument()
    expect(screen.getByText('INFO')).toBeInTheDocument()
  })

  it('renders a warn entry with the WARN label', () => {
    LogStore.log.warn('tool', 'face is degenerate')
    render(<LogPanel />)
    expect(screen.getByText('WARN')).toBeInTheDocument()
    expect(screen.getByText('face is degenerate')).toBeInTheDocument()
  })

  it('renders an error entry with the ERR label', () => {
    LogStore.log.error('kernel', 'unreachable code hit')
    render(<LogPanel />)
    // The label is rendered as 'ERR ' (padded to align with INFO/WARN), but
    // testing-library normalizes trailing whitespace away, so match 'ERR'.
    expect(screen.getByText('ERR')).toBeInTheDocument()
    expect(screen.getByText('unreachable code hit')).toBeInTheDocument()
  })

  it('shows the entry count badge', () => {
    LogStore.log.info('app', 'one')
    LogStore.log.info('app', 'two')
    render(<LogPanel />)
    expect(screen.getByText('2 entries')).toBeInTheDocument()
  })

  it('shows error/warn summary in badge when errors or warns exist', () => {
    LogStore.log.error('app', 'bad thing')
    LogStore.log.warn('tool', 'hmm')
    render(<LogPanel />)
    expect(screen.getByText(/1 err/)).toBeInTheDocument()
    expect(screen.getByText(/1 warn/)).toBeInTheDocument()
  })

  it('clears all entries when the Clear button is clicked', () => {
    LogStore.log.info('app', 'first message')
    render(<LogPanel />)
    expect(screen.getByText('first message')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /clear/i }))
    expect(screen.queryByText('first message')).not.toBeInTheDocument()
    expect(screen.getByText(/no log entries/i)).toBeInTheDocument()
  })

  it('hides the body when the header is clicked (collapse)', () => {
    LogStore.log.info('app', 'a message')
    render(<LogPanel />)
    // Body is visible initially — entry is present
    expect(screen.getByText('a message')).toBeInTheDocument()
    // Click the header to collapse
    fireEvent.click(screen.getByText(/▼ Log/))
    // Body is hidden — entry should no longer be in the DOM
    expect(screen.queryByText('a message')).not.toBeInTheDocument()
  })

  it('shows ▶ indicator when collapsed and ▼ when expanded', () => {
    render(<LogPanel />)
    expect(screen.getByText(/▼ Log/)).toBeInTheDocument()
    fireEvent.click(screen.getByText(/▼ Log/))
    expect(screen.getByText(/▶ Log/)).toBeInTheDocument()
  })

  it('copies the log entries to the clipboard when Copy is clicked', () => {
    const writeText = vi.fn((_text: string) => Promise.resolve())
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    LogStore.log.error('tool', '[DegenerateContact] combining needs real overlap')
    render(<LogPanel />)
    fireEvent.click(screen.getByRole('button', { name: /copy/i }))
    expect(writeText).toHaveBeenCalledTimes(1)
    const copied = writeText.mock.calls[0][0] as string
    expect(copied).toContain('ERR')
    expect(copied).toContain('[tool]')
    expect(copied).toContain('[DegenerateContact] combining needs real overlap')
    vi.unstubAllGlobals()
  })

  it('disables Copy when there are no entries', () => {
    render(<LogPanel />)
    expect(screen.getByRole('button', { name: /copy/i })).toBeDisabled()
  })

  it('reacts to log entries added after mount', () => {
    render(<LogPanel />)
    expect(screen.getByText(/no log entries/i)).toBeInTheDocument()
    // The emit triggers the panel's subscription → a React state update; wrap it
    // in act() so the re-render flushes before we assert.
    act(() => {
      LogStore.log.info('app', 'post-mount message')
    })
    expect(screen.getByText('post-mount message')).toBeInTheDocument()
  })
})
