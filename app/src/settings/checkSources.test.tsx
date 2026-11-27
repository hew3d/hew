/**
 * "Check sources for new and changed items" (design 7.5): default on, saved
 * at once in both settings renderings, and shared across windows.
 */
import { act, fireEvent, render, renderHook, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { FoldersPane } from './FoldersPane'
import { FluentSettingsPage } from './FluentSettingsPage'
import { getCheckSources, setCheckSources, useCheckSources } from './checkSources'
import { libraryStore, type LibraryStore } from '../io/libraryStore'

vi.mock('../io/libraryStore', () => ({ libraryStore: vi.fn() }))

const KEY = 'hew.settings.checkSources'
const LABEL = 'Check sources for new and changed items'

function store(): LibraryStore {
  return {
    available: () => true,
    folderInfo: () => Promise.resolve({ path: '/tmp/Hew Library' }),
    chooseFolder: () => Promise.resolve(null),
    list: () => Promise.resolve([]),
    read: () => Promise.reject(new Error('none')),
    write: () => Promise.resolve(),
    remove: () => Promise.resolve(),
    readThumbnail: () => Promise.resolve(null),
    writeThumbnail: () => Promise.resolve(),
    reveal: () => Promise.resolve(),
    itemPath: () => Promise.resolve(null),
    nameForPath: () => Promise.resolve(null),
    capabilities: () => ({ canReveal: true, canChooseFolder: true, canDownload: false }),
    readSources: () => Promise.resolve(null),
    writeSources: () => Promise.resolve(),
    readSourceCache: () => Promise.resolve(null),
    writeSourceCache: () => Promise.resolve(),
    removeSourceCache: () => Promise.resolve(),
    subscribeSources: () => () => {},
    editingNames: () => Promise.resolve([]),
    subscribe: () => () => {},
  }
}

beforeEach(() => {
  localStorage.removeItem(KEY)
  vi.mocked(libraryStore).mockReturnValue(store())
})

describe('checkSources setting', () => {
  it('is on by default and stored under its own key', () => {
    expect(getCheckSources()).toBe(true)
    setCheckSources(false)
    expect(localStorage.getItem(KEY)).toBe('false')
    expect(getCheckSources()).toBe(false)
    setCheckSources(true)
    expect(localStorage.getItem(KEY)).toBe('true')
  })

  it('is separate from "Check the Library for newer parts"', () => {
    setCheckSources(false)
    expect(localStorage.getItem('hew.settings.checkLibraryUpdates')).not.toBe('false')
  })

  it('the hook follows a change made in this window and one made in another window', () => {
    const { result } = renderHook(() => useCheckSources())
    expect(result.current).toBe(true)
    act(() => setCheckSources(false))
    expect(result.current).toBe(false)
    // Another window wrote the key: only a `storage` event reaches this one.
    localStorage.setItem(KEY, 'true')
    act(() => {
      window.dispatchEvent(new StorageEvent('storage', { key: KEY }))
    })
    expect(result.current).toBe(true)
    // A storage event for some other key is ignored.
    localStorage.setItem(KEY, 'false')
    act(() => {
      window.dispatchEvent(new StorageEvent('storage', { key: 'hew.settings.other' }))
    })
    expect(result.current).toBe(true)
  })
})

describe('Settings ▸ Folders', () => {
  it('shows the toggle on by default and saves at once', () => {
    render(<FoldersPane />)
    const box = screen.getByRole('checkbox', { name: LABEL })
    expect(box).toBeChecked()
    fireEvent.click(box)
    expect(box).not.toBeChecked()
    expect(localStorage.getItem(KEY)).toBe('false')
    fireEvent.click(box)
    expect(localStorage.getItem(KEY)).toBe('true')
  })

  it('says what off means: checks only on Refresh', () => {
    render(<FoldersPane />)
    expect(screen.getByText(/Turn this off and Hew checks only when you press Refresh/)).toBeInTheDocument()
  })
})

describe('Settings (Windows layout)', () => {
  it('shows the switch and saves at once', async () => {
    render(<FluentSettingsPage onBack={() => {}} />)
    const sw = await screen.findByRole('switch', { name: LABEL })
    expect(sw).toHaveAttribute('aria-checked', 'true')
    fireEvent.click(sw)
    expect(localStorage.getItem(KEY)).toBe('false')
    expect(sw).toHaveAttribute('aria-checked', 'false')
  })
})
