import { describe, it, expect, beforeEach } from 'vitest'
import { noteHandedBackOpen, beginOpenAttempt, clearOpenAttempt, forgetSettledFailure, getFailedOpen, openAttemptFailed } from './failedOpen'

const file = { name: 'Kitchen.skp', label: 'SketchUp model (.skp)', bytes: new Uint8Array([1, 2, 3]) }

beforeEach(() => clearOpenAttempt())

describe('failedOpen', () => {
  it('holds nothing until an attempt begins', () => {
    expect(getFailedOpen()).toBeNull()
    openAttemptFailed('no attempt to fail')
    expect(getFailedOpen()).toBeNull()
  })

  it('keeps an attempt the kernel never finished, with no error', () => {
    beginOpenAttempt(file)
    expect(getFailedOpen()).toEqual({ ...file, error: null })
  })

  it('keeps a refused attempt with its error', () => {
    beginOpenAttempt(file)
    openAttemptFailed('SKP: unsupported version')
    expect(getFailedOpen()?.error).toBe('SKP: unsupported version')
    expect(getFailedOpen()?.bytes).toBe(file.bytes)
  })

  it('drops the attempt once it lands', () => {
    beginOpenAttempt(file)
    clearOpenAttempt()
    expect(getFailedOpen()).toBeNull()
  })

  it('keeps only the latest attempt', () => {
    beginOpenAttempt(file)
    openAttemptFailed('first')
    beginOpenAttempt({ ...file, name: 'Den.skp' })
    expect(getFailedOpen()).toEqual({ ...file, name: 'Den.skp', error: null })
  })
})

describe('failedOpen: a new document', () => {
  beforeEach(() => clearOpenAttempt())

  it('forgets a failure that has settled', () => {
    beginOpenAttempt(file)
    openAttemptFailed('SKP: no')
    forgetSettledFailure()
    expect(getFailedOpen()).toBeNull()
  })

  it('keeps an attempt still under way, whose open loads a blank document of its own', () => {
    beginOpenAttempt(file)
    forgetSettledFailure()
    expect(getFailedOpen()).toEqual({ ...file, error: null })
  })

  it('keeps a file another window refused, reading its bytes from the path', async () => {
    await noteHandedBackOpen(
      { name: 'old.skp', label: 'SketchUp model (.skp)', error: 'Open failed: too old' },
      async () => new Uint8Array([1, 2, 3]),
    )
    expect(getFailedOpen()).toEqual({
      name: 'old.skp',
      label: 'SketchUp model (.skp)',
      bytes: new Uint8Array([1, 2, 3]),
      error: 'Open failed: too old',
    })
  })

  it('does not overwrite an attempt that began while the hand-back was still reading', async () => {
    let release: (b: Uint8Array) => void = () => undefined
    const pending = noteHandedBackOpen(
      { name: 'old.skp', label: 'x', error: 'handed back' },
      () => new Promise<Uint8Array>((r) => (release = r)),
    )
    beginOpenAttempt(file) // a newer open starts here meanwhile
    release(new Uint8Array([9]))
    await pending
    expect(getFailedOpen()).toEqual({ ...file, error: null })
  })

  it('does not resurrect a failure that was cleared while the hand-back was still reading', async () => {
    let release: (b: Uint8Array) => void = () => undefined
    const pending = noteHandedBackOpen(
      { name: 'old.skp', label: 'x', error: 'handed back' },
      () => new Promise<Uint8Array>((r) => (release = r)),
    )
    clearOpenAttempt() // a document landed meanwhile
    release(new Uint8Array([9]))
    await pending
    expect(getFailedOpen()).toBeNull()
  })

  it('keeps nothing when the file can no longer be read', async () => {
    clearOpenAttempt()
    await noteHandedBackOpen({ name: 'gone.skp', label: 'x', error: 'e' }, async () => {
      throw new Error('gone')
    })
    expect(getFailedOpen()).toBeNull()
  })
})
