/**
 * FoldersPane — component tests.
 *
 * The pane binds to `libraryStore()` (io/libraryStore.ts), mocked here so
 * both branches are exercised deliberately rather than at the mercy of
 * jsdom's real (always-unavailable, since isTauri is false there) facade:
 * one case reproduces the store's own `available()` false shape — the same
 * one the real web build reports — the other a fully mocked available store
 * covering the folder-path display and the Change… button.
 */

import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { FoldersPane } from './FoldersPane'
import { libraryStore, type LibraryStore } from '../io/libraryStore'

vi.mock('../io/libraryStore', () => ({ libraryStore: vi.fn() }))

function unavailableStore(): LibraryStore {
  const fail = (): Promise<never> => Promise.reject(new Error('unavailable'))
  return {
    available: () => false,
    folderInfo: () => Promise.resolve({ path: null }),
    chooseFolder: () => Promise.resolve(null),
    list: () => Promise.resolve([]),
    read: fail,
    write: fail,
    remove: fail,
    readThumbnail: () => Promise.resolve(null),
    writeThumbnail: fail,
    reveal: fail,
    itemPath: () => Promise.resolve(null),
    capabilities: () => ({ canReveal: false, canChooseFolder: false, canDownload: false }),
    subscribe: () => () => {},
  }
}

function availableStore(overrides: Partial<LibraryStore> = {}): LibraryStore {
  return {
    available: () => true,
    folderInfo: () => Promise.resolve({ path: '/Users/kurt/Hew Library' }),
    chooseFolder: () => Promise.resolve('/Users/kurt/New Library'),
    list: () => Promise.resolve([]),
    read: () => Promise.resolve(new Uint8Array()),
    write: () => Promise.resolve(),
    remove: () => Promise.resolve(),
    readThumbnail: () => Promise.resolve(null),
    writeThumbnail: () => Promise.resolve(),
    reveal: () => Promise.resolve(),
    itemPath: () => Promise.resolve(null),
    capabilities: () => ({ canReveal: true, canChooseFolder: true, canDownload: false }),
    subscribe: () => () => {},
    ...overrides,
  }
}

describe('FoldersPane', () => {
  it('web browser-storage mode: label, explanatory note, Change… enabled on Chromium', async () => {
    vi.mocked(libraryStore).mockReturnValue(
      availableStore({
        folderInfo: () => Promise.resolve({ path: 'Browser storage' }),
        webStorage: () => Promise.resolve({ mode: 'browser', needsReconnect: false }),
      }),
    )
    render(<FoldersPane />)
    await waitFor(() => expect(screen.getByDisplayValue('Browser storage')).toBeInTheDocument())
    expect(screen.getByText(/stored by this browser on this device/i)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Change…' })).toBeEnabled()
    // Own row (not inline — it crowded the path out of its box), disabled
    // while browser storage is already active.
    expect(screen.getByRole('button', { name: 'Use browser storage' })).toBeDisabled()
  })

  it('web browser-storage mode without a directory picker disables Change… (Firefox/Safari)', async () => {
    vi.mocked(libraryStore).mockReturnValue(
      availableStore({
        folderInfo: () => Promise.resolve({ path: 'Browser storage' }),
        capabilities: () => ({ canReveal: false, canChooseFolder: false, canDownload: true }),
        webStorage: () => Promise.resolve({ mode: 'browser', needsReconnect: false }),
      }),
    )
    render(<FoldersPane />)
    await waitFor(() => expect(screen.getByDisplayValue('Browser storage')).toBeInTheDocument())
    expect(screen.getByRole('button', { name: 'Change…' })).toBeDisabled()
    // The note explains WHY instead of leaving a mystery-disabled button.
    expect(screen.getByText(/doesn.t offer the folder-picker API/i)).toBeInTheDocument()
  })

  it('names the Brave flag when Brave is the reason the picker is missing', async () => {
    const nav = navigator as { brave?: unknown }
    nav.brave = { isBrave: () => Promise.resolve(true) }
    try {
      vi.mocked(libraryStore).mockReturnValue(
        availableStore({
          folderInfo: () => Promise.resolve({ path: 'Browser storage' }),
          capabilities: () => ({ canReveal: false, canChooseFolder: false, canDownload: true }),
          webStorage: () => Promise.resolve({ mode: 'browser', needsReconnect: false }),
        }),
      )
      render(<FoldersPane />)
      await waitFor(() => expect(screen.getByText(/brave:\/\/flags/i)).toBeInTheDocument())
    } finally {
      delete nav.brave
    }
  })

  it('web bound-folder mode: shows the folder note and switches back via Use browser storage', async () => {
    const useBrowserStorage = vi.fn().mockResolvedValue(undefined)
    vi.mocked(libraryStore).mockReturnValue(
      availableStore({
        folderInfo: () => Promise.resolve({ path: 'Hew Library' }),
        webStorage: () => Promise.resolve({ mode: 'folder', needsReconnect: false }),
        useBrowserStorage,
      }),
    )
    render(<FoldersPane />)
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Use browser storage' })).toBeInTheDocument(),
    )
    expect(screen.getByText(/ordinary \.hew files in the folder you picked/i)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Use browser storage' })).toBeEnabled()
    fireEvent.click(screen.getByRole('button', { name: 'Use browser storage' }))
    expect(useBrowserStorage).toHaveBeenCalledOnce()
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Use browser storage' })).toBeDisabled(),
    )
  })

  it('shows the disabled unavailable state when the store reports unavailable', () => {
    vi.mocked(libraryStore).mockReturnValue(unavailableStore())
    render(<FoldersPane />)
    expect(screen.getByText('Library folder:')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('Not available')).toBeInTheDocument()
    expect(screen.getByText('The library is not available in this browser.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Change…' })).toBeDisabled()
  })

  it('renders the configured path as the real absolute path in a read-only input, and calls chooseFolder when Change… is clicked', async () => {
    const chooseFolder = vi.fn().mockResolvedValue('/Users/kurt/New Library')
    vi.mocked(libraryStore).mockReturnValue(availableStore({ chooseFolder }))
    render(<FoldersPane />)

    const input = screen.getByLabelText('Library folder:') as HTMLInputElement
    await waitFor(() => {
      expect(input.value).toBe('/Users/kurt/Hew Library')
    })
    expect(input).toHaveAttribute('readonly')
    expect(input).not.toBeDisabled()

    fireEvent.click(screen.getByRole('button', { name: 'Change…' }))
    expect(chooseFolder).toHaveBeenCalledTimes(1)

    await waitFor(() => {
      expect(input.value).toBe('/Users/kurt/New Library')
    })
  })

  it('has no descriptive note text when a folder is configured', () => {
    vi.mocked(libraryStore).mockReturnValue(availableStore())
    render(<FoldersPane />)
    expect(screen.queryByText(/plain/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/sync it, back it up/i)).not.toBeInTheDocument()
  })
})
