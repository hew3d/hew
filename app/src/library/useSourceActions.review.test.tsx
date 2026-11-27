/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures */
/**
 * Review round, sources UI wiring: what the actions hook and the review card
 * say when a landed update left the old copy behind (`removeFailed`), and the
 * message an Add shows when the item is open for editing.
 */

import { act, renderHook, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { SourceUpdateReview } from '../panels/SourceUpdateReview'
import { removeFailedText } from './sourceMessages'
import { reviewKey, useSourceActions, type ActionDeps } from './useSourceActions'

const OK = { ok: true, file: 'Components/New.hew', id: 'i1', replaced: true, sha256: 'x' }
const OPEN_MSG = 'Open for editing. Close it and update again.'

function setup(sources: Record<string, unknown>, snapshot: Record<string, unknown> = {}) {
  const deps = {
    binding: { sources, available: () => true, subscribe: () => () => {} },
    snapshot: { statuses: [], locals: [], ...snapshot },
    refresh: vi.fn(async () => {}),
    bumpLibrary: vi.fn(),
    insertFile: vi.fn(async () => {}),
  } as unknown as ActionDeps
  return renderHook(() => useSourceActions(deps))
}

describe('removeFailed is said, not swallowed', () => {
  it('updateItem returns a notice and sets the message', async () => {
    const { result } = setup({ updateItem: async () => ({ ...OK, removeFailed: true }) })
    let out: Awaited<ReturnType<typeof result.current.updateItem>> | undefined
    await act(async () => {
      out = await result.current.updateItem({ source: 'github:a/b', id: 'i1', name: 'Part' })
    })
    expect(out).toMatchObject({ ok: true, notice: { text: removeFailedText() } })
    expect(result.current.message?.text).toBe(removeFailedText())
  })

  it('a plain update has no notice', async () => {
    const { result } = setup({ updateItem: async () => OK })
    let out: Awaited<ReturnType<typeof result.current.updateItem>> | undefined
    await act(async () => {
      out = await result.current.updateItem({ source: 'github:a/b', id: 'i1', name: 'Part' })
    })
    expect(out).toEqual({ ok: true, file: OK.file })
    expect(result.current.message).toBeNull()
  })

  it('Update all puts the note on the row', async () => {
    const entry = { source: 'github:a/b', name: 'B' }
    const { result } = setup(
      {
        itemStates: async () => ({ ok: true, rows: [{ id: 'i1', name: 'Part', state: 'newer', kept: false, edited: false, entry: {}, skipped: false }] }),
        updateAll: async () => ({ ok: true, needsReview: [], rows: [{ id: 'i1', result: { ...OK, removeFailed: true } }] }),
      },
      { statuses: [{ entry, removed: null, followed: false }] },
    )
    await act(async () => {
      await result.current.updateAllFromBar()
    })
    expect(result.current.review?.results[reviewKey(entry.source, 'i1')]).toMatchObject({ kind: 'updated', note: { text: removeFailedText() } })
  })

  it('the review card shows the note beside the result', () => {
    render(
      <SourceUpdateReview
        review={{
          items: [{ key: 'k', source: 's', sourceName: 'S', id: 'i1', name: 'Part', localName: 'Part', kind: 'newer', edited: false }],
          results: { k: { kind: 'updated', note: { text: removeFailedText() } } },
          busy: false,
        }}
        onUpdateSelected={() => {}}
        onDecide={() => {}}
        onClose={() => {}}
      />,
    )
    expect(screen.getByTestId('source-review-result')).toHaveTextContent('Updated')
    expect(screen.getByTestId('source-review-note')).toHaveTextContent(removeFailedText())
  })
})

describe('Add on an item open for editing says so, like Update', () => {
  it.each([
    ['without replace', undefined],
    ['with replace (after Yes)', { replace: true }],
  ])('%s', async (_label, _opts) => {
    const addItem = vi
      .fn()
      .mockResolvedValueOnce(_opts === undefined ? { ok: false, code: 'open_for_editing' } : { ok: false, code: 'needs_confirm', confirm: 'replace_item' })
      .mockResolvedValue({ ok: false, code: 'open_for_editing' })
    const { result } = setup({ addItem })
    let out: Awaited<ReturnType<typeof result.current.addItem>> | undefined
    await act(async () => {
      out = await result.current.addItem({ source: 'github:a/b', id: 'i1', name: 'Part' })
    })
    if (_opts !== undefined) {
      expect(result.current.confirm?.kind).toBe('replace_item')
      await act(async () => {
        if (result.current.confirm?.kind === 'replace_item') result.current.confirm.yes()
      })
      expect(addItem).toHaveBeenLastCalledWith('github:a/b', 'i1', { replace: true })
    } else {
      expect(out).toMatchObject({ ok: false, message: { text: OPEN_MSG } })
    }
    expect(result.current.message?.text).toBe(OPEN_MSG)
  })
})
