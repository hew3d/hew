/**
 * The desktop Library window's bridge to the document window (design 7.5,
 * "Insert on the desktop"): Insert from a source view hands the item to the
 * existing `insert` action, with no new action. The dialog adds the item
 * first (LibrarySources.test.tsx, scenario 9); this pins the payload.
 */
import { render } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { LibraryItem } from '../library/types'

const { mockInvoke, dialogProps } = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
  dialogProps: { current: null as null | Record<string, (...a: unknown[]) => void> },
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))
vi.mock('@tauri-apps/api/event', () => ({ emit: vi.fn(), listen: vi.fn(async () => () => {}) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ close: vi.fn() }) }))
vi.mock('../clipboard/useTextFieldMenuActions', () => ({ useTextFieldMenuActions: () => {} }))
vi.mock('../library/sourcesDeps', () => ({ defaultSourcesBinding: () => undefined }))
vi.mock('./LibraryDialog', () => ({
  LibraryDialog: (props: Record<string, (...a: unknown[]) => void>) => {
    dialogProps.current = props
    return null
  },
}))

import { LibraryWindow } from './LibraryWindow'

const item = {
  file: { fileName: 'Components/part-1-000001.hew', size: 1, mtimeMs: 0 },
  displayName: 'Part 1',
  category: 'component',
  meta: { id: '00000001-0000-4000-8000-000000000001' },
} as unknown as LibraryItem

beforeEach(() => {
  mockInvoke.mockReset()
  mockInvoke.mockResolvedValue(true)
})

describe('LibraryWindow dispatch', () => {
  it('Insert dispatches the existing insert action with the item\'s name, id and category, focusing the document window', () => {
    render(<LibraryWindow />)
    dialogProps.current!.onInsert(item, new Uint8Array())
    expect(mockInvoke).toHaveBeenCalledTimes(1)
    const [cmd, args] = mockInvoke.mock.calls[0] as [string, { payload: string; focusTarget: boolean }]
    expect(cmd).toBe('library_dispatch')
    expect(args.focusTarget).toBe(true)
    expect(JSON.parse(args.payload)).toEqual({
      action: 'insert',
      fileName: 'Components/part-1-000001.hew',
      displayName: 'Part 1',
      sourceId: '00000001-0000-4000-8000-000000000001',
      category: 'component',
    })
  })

  it('Review sends the review action with empty item fields', () => {
    render(<LibraryWindow />)
    dialogProps.current!.onReviewUpdates()
    const args = mockInvoke.mock.calls[0][1] as { payload: string }
    expect(JSON.parse(args.payload)).toMatchObject({ action: 'review', fileName: '' })
  })
})
