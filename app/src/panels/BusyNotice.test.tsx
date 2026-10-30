import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { setBusy } from '../api/busyState'
import { BusyNotice } from './BusyNotice'

describe('BusyNotice', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    act(() => setBusy(null))
    vi.useRealTimers()
  })

  it('appears after a quarter second, not before', () => {
    render(<BusyNotice />)
    act(() =>
      setBusy({ plugin: 'com.example.parts', label: null, startedAt: performance.now(), cancel: () => {} }),
    )
    act(() => {
      vi.advanceTimersByTime(200)
    })
    expect(screen.queryByRole('status')).toBeNull()
    act(() => {
      vi.advanceTimersByTime(60)
    })
    expect(screen.getByRole('status')).toHaveTextContent('com.example.parts is working…')
    act(() => setBusy(null))
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('names the command by its label and cancels through its button', () => {
    const cancel = vi.fn()
    render(<BusyNotice />)
    act(() => setBusy({ plugin: 'com.example.parts', label: 'Cut list', startedAt: performance.now(), cancel }))
    act(() => {
      vi.advanceTimersByTime(300)
    })
    expect(screen.getByRole('status')).toHaveTextContent('Cut list is working…')
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(cancel).toHaveBeenCalledTimes(1)
  })
})
