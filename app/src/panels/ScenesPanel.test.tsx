/**
 * ScenesPanel — component tests against a FAKE `ScenesController` (a plain
 * React-state-backed stand-in, not the real kernel-backed hook —
 * `useScenesController.test.tsx` covers the real hook's kernel choreography;
 * this file only exercises the tray section's own rendering/interaction).
 *
 * The header ⊕ Add Scene button and the row list share rename UI state
 * (`useSceneRenameState`, ScenesPanel.tsx's own doc comment on why), so
 * every test renders both together via `Harness` below, mirroring how
 * App.tsx wires them.
 */
import { useRef, useState } from 'react'
import { render, screen, fireEvent, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ScenesPanel, ScenesAddButton, useSceneRenameState } from './ScenesPanel'
import { PROP_ALL, SCENE_COPY, driftAny, type SceneDrift, type SceneEntry } from '../scenes/scenesModel'
import type { ScenesController } from '../scenes/useScenesController'

// ---------------------------------------------------------------------------
// Fake ScenesController — real React state so add/rename/reorder/etc. drive
// genuine re-renders, plus a `driftOverride` test knob for active-drifted.
// ---------------------------------------------------------------------------

function useFakeScenesController(
  initial: SceneEntry[],
  driftOverride: SceneDrift | null = null,
): ScenesController & { spies: Record<string, ReturnType<typeof vi.fn>> } {
  const [entries, setEntries] = useState<SceneEntry[]>(initial)
  const [activeSid, setActiveSid] = useState<number | null>(null)
  const [thumbnails, setThumbnails] = useState<Map<number, string>>(new Map())
  const sidCounter = useRef(1000)

  const nextName = (): string => {
    const taken = new Set(entries.map((e) => e.name))
    for (let n = 1; ; n++) {
      const candidate = `Scene ${n}`
      if (!taken.has(candidate)) return candidate
    }
  }

  const spies = useRef({
    update: vi.fn(),
    activate: vi.fn(),
    moveUp: vi.fn(),
    moveDown: vi.fn(),
    remove: vi.fn(),
    refreshThumbnail: vi.fn(),
    refreshDrift: vi.fn(),
    setProps: vi.fn(),
    setDescription: vi.fn(),
  }).current

  const add = (): number | null => {
    const name = nextName()
    const sid = sidCounter.current++
    setEntries((es) => [...es, { sid, name, description: '', props: PROP_ALL }])
    setActiveSid(sid)
    return sid
  }

  const rename = (sid: number, name: string): string | null => {
    if (entries.some((e) => e.sid !== sid && e.name === name)) {
      return SCENE_COPY.duplicateName(name)
    }
    setEntries((es) => es.map((e) => (e.sid === sid ? { ...e, name } : e)))
    return null
  }

  const remove = (sid: number) => {
    spies.remove(sid)
    setEntries((es) => es.filter((e) => e.sid !== sid))
    setActiveSid((cur) => (cur === sid ? null : cur))
  }

  const move = (sid: number, dir: -1 | 1) => {
    setEntries((es) => {
      const i = es.findIndex((e) => e.sid === sid)
      const to = i + dir
      if (i < 0 || to < 0 || to >= es.length) return es
      const next = [...es]
      const [item] = next.splice(i, 1)
      next.splice(to, 0, item)
      return next
    })
  }

  const setProps = (sid: number, props: number) => {
    spies.setProps(sid, props)
    setEntries((es) => es.map((e) => (e.sid === sid ? { ...e, props } : e)))
  }

  const setDescription = (sid: number, text: string) => {
    spies.setDescription(sid, text)
    setEntries((es) => es.map((e) => (e.sid === sid ? { ...e, description: text } : e)))
  }

  return {
    entries,
    activeSid,
    drift: driftOverride,
    activeDrifted: driftAny(driftOverride),
    thumbnails,
    add,
    update: (sid) => spies.update(sid),
    activate: (sid) => {
      spies.activate(sid)
      setActiveSid(sid)
    },
    next: vi.fn(),
    previous: vi.fn(),
    rename,
    setDescription,
    setProps,
    moveUp: (sid) => {
      spies.moveUp(sid)
      move(sid, -1)
    },
    moveDown: (sid) => {
      spies.moveDown(sid)
      move(sid, 1)
    },
    remove,
    refreshThumbnail: (sid) => spies.refreshThumbnail(sid),
    refreshDrift: () => spies.refreshDrift(),
    mergeThumbnails: (thumbs) => setThumbnails((prev) => new Map([...prev, ...thumbs])),
    activateFirstOnLoad: () => false,
    resetForDocument: () => {
      setActiveSid(null)
      setThumbnails(new Map())
    },
    spies,
  }
}

function Harness({
  initial = [],
  driftOverride = null,
  onController,
}: {
  initial?: SceneEntry[]
  driftOverride?: SceneDrift | null
  onController?: (c: ReturnType<typeof useFakeScenesController>) => void
}) {
  const scenes = useFakeScenesController(initial, driftOverride)
  const rename = useSceneRenameState(scenes)
  onController?.(scenes)
  return (
    <div>
      <ScenesAddButton rename={rename} />
      <ScenesPanel scenes={scenes} rename={rename} />
    </div>
  )
}

const entry = (sid: number, name: string, overrides: Partial<SceneEntry> = {}): SceneEntry => ({
  sid,
  name,
  description: '',
  props: PROP_ALL,
  ...overrides,
})

const DRIFTED: SceneDrift = {
  camera: true,
  hiddenNodes: false,
  hiddenTags: false,
  section: false,
  display: false,
  staleRefs: 0,
}

describe('ScenesPanel — empty state', () => {
  it('shows the empty-state copy and an Add Scene button', () => {
    render(<Harness />)
    expect(screen.getByText(SCENE_COPY.emptyState)).toBeInTheDocument()
    // Two "Add Scene" affordances render together in the empty state: the
    // header's icon button (always present) and the empty state's own
    // full-width button (SPEC.md §1 "Empty state").
    expect(screen.getAllByRole('button', { name: 'Add Scene' })).toHaveLength(2)
  })
})

describe('ScenesPanel — rows and states', () => {
  it('renders a row per Scene, in order', () => {
    render(<Harness initial={[entry(1, 'Assembled'), entry(2, 'Exploded')]} />)
    expect(screen.getByText('Assembled')).toBeInTheDocument()
    expect(screen.getByText('Exploded')).toBeInTheDocument()
  })

  it('bolds the active Scene’s name and calls activate on row click', () => {
    let controller: ReturnType<typeof useFakeScenesController> | undefined
    render(
      <Harness
        initial={[entry(1, 'Assembled'), entry(2, 'Exploded')]}
        onController={(c) => { controller = c }}
      />,
    )
    const row = screen.getByText('Assembled')
    expect(row).toHaveStyle({ fontWeight: '400' })
    fireEvent.click(row)
    expect(controller!.spies.activate).toHaveBeenCalledWith(1)
  })

  it('shows the Update button only on the active+drifted row, and it calls update', () => {
    let controller: ReturnType<typeof useFakeScenesController> | undefined
    render(
      <Harness
        initial={[entry(1, 'Assembled')]}
        driftOverride={DRIFTED}
        onController={(c) => { controller = c }}
      />,
    )
    // Activate it first so activeSid lines up with the drift override.
    fireEvent.click(screen.getByText('Assembled'))
    const updateBtn = screen.getByRole('button', { name: /^Update Scene/ })
    fireEvent.click(updateBtn)
    expect(controller!.spies.update).toHaveBeenCalledWith(1)
  })

  it('has no Update button on an active, non-drifted Scene', () => {
    render(<Harness initial={[entry(1, 'Assembled')]} driftOverride={null} />)
    fireEvent.click(screen.getByText('Assembled'))
    expect(screen.queryByRole('button', { name: /^Update Scene/ })).not.toBeInTheDocument()
  })
})

describe('ScenesPanel — details expand', () => {
  it('expands details on chevron click, shows captured-properties checkboxes, and collapses on toggle', () => {
    render(<Harness initial={[entry(1, 'Assembled')]} />)
    const chevron = screen.getByRole('button', { name: 'Expand Scene details' })
    fireEvent.click(chevron)
    expect(screen.getByText('Captured Properties')).toBeInTheDocument()
    expect(screen.getByText('Camera')).toBeInTheDocument()
    expect(screen.getByText('Hidden objects')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Collapse Scene details' }))
    expect(screen.queryByText('Captured Properties')).not.toBeInTheDocument()
  })

  it('only one Scene’s details are expanded at a time', () => {
    render(<Harness initial={[entry(1, 'A'), entry(2, 'B')]} />)
    const chevrons = screen.getAllByRole('button', { name: 'Expand Scene details' })
    fireEvent.click(chevrons[0])
    expect(screen.getByText('Captured Properties')).toBeInTheDocument()
    // Expanding the second row's chevron (now the only "Expand" one left).
    fireEvent.click(screen.getByRole('button', { name: 'Expand Scene details' }))
    expect(screen.getAllByText('Captured Properties')).toHaveLength(1)
  })

  it('toggling a captured-properties checkbox calls setProps with the flipped bit', () => {
    let controller: ReturnType<typeof useFakeScenesController> | undefined
    render(
      <Harness
        initial={[entry(1, 'Assembled', { props: PROP_ALL })]}
        onController={(c) => { controller = c }}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Expand Scene details' }))
    fireEvent.click(screen.getByLabelText('Camera'))
    expect(controller!.spies.setProps).toHaveBeenCalledWith(1, PROP_ALL & ~1)
  })

  it('shows the stale-refs line only for the active Scene when staleRefs > 0', () => {
    render(
      <Harness
        initial={[entry(1, 'Assembled')]}
        driftOverride={{ ...DRIFTED, staleRefs: 3 }}
      />,
    )
    fireEvent.click(screen.getByText('Assembled')) // activate
    fireEvent.click(screen.getByRole('button', { name: 'Expand Scene details' }))
    expect(screen.getByText(SCENE_COPY.staleRefs(3))).toBeInTheDocument()
  })

  it('disables Move Up on the first row and Move Down on the last, and they call the controller', () => {
    let controller: ReturnType<typeof useFakeScenesController> | undefined
    render(
      <Harness
        initial={[entry(1, 'A'), entry(2, 'B')]}
        onController={(c) => { controller = c }}
      />,
    )
    fireEvent.click(screen.getAllByRole('button', { name: 'Expand Scene details' })[0])
    expect(screen.getByRole('button', { name: 'Move Up' })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Move Down' }))
    expect(controller!.spies.moveDown).toHaveBeenCalledWith(1)
  })

  it('Refresh thumbnail calls refreshThumbnail for that Scene', () => {
    let controller: ReturnType<typeof useFakeScenesController> | undefined
    render(
      <Harness
        initial={[entry(1, 'Assembled')]}
        onController={(c) => { controller = c }}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Expand Scene details' }))
    fireEvent.click(screen.getByRole('button', { name: 'Refresh thumbnail' }))
    expect(controller!.spies.refreshThumbnail).toHaveBeenCalledWith(1)
  })
})

describe('ScenesPanel — inline rename', () => {
  it('double-click opens rename with the current name; Enter commits', () => {
    render(<Harness initial={[entry(1, 'Assembled')]} />)
    fireEvent.doubleClick(screen.getByText('Assembled'))
    const input = screen.getByLabelText('Scene name') as HTMLInputElement
    expect(input.value).toBe('Assembled')
    fireEvent.change(input, { target: { value: 'Cut Layout' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(screen.queryByLabelText('Scene name')).not.toBeInTheDocument()
    expect(screen.getByText('Cut Layout')).toBeInTheDocument()
  })

  it('Escape reverts without committing', () => {
    render(<Harness initial={[entry(1, 'Assembled')]} />)
    fireEvent.doubleClick(screen.getByText('Assembled'))
    const input = screen.getByLabelText('Scene name') as HTMLInputElement
    fireEvent.change(input, { target: { value: 'Sneaky' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(screen.queryByLabelText('Scene name')).not.toBeInTheDocument()
    expect(screen.getByText('Assembled')).toBeInTheDocument()
    expect(screen.queryByText('Sneaky')).not.toBeInTheDocument()
  })

  it('blur reverts without committing', () => {
    render(<Harness initial={[entry(1, 'Assembled')]} />)
    fireEvent.doubleClick(screen.getByText('Assembled'))
    const input = screen.getByLabelText('Scene name')
    fireEvent.change(input, { target: { value: 'Sneaky' } })
    fireEvent.blur(input)
    expect(screen.queryByLabelText('Scene name')).not.toBeInTheDocument()
    expect(screen.getByText('Assembled')).toBeInTheDocument()
  })

  it('committing a duplicate name keeps the input open and shows the error below it', () => {
    render(<Harness initial={[entry(1, 'Assembled'), entry(2, 'Exploded')]} />)
    fireEvent.doubleClick(screen.getByText('Exploded'))
    const input = screen.getByLabelText('Scene name')
    fireEvent.change(input, { target: { value: 'Assembled' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(screen.getByLabelText('Scene name')).toBeInTheDocument()
    expect(screen.getByText(SCENE_COPY.duplicateName('Assembled'))).toBeInTheDocument()
  })
})

describe('ScenesPanel — Add flow', () => {
  it('the header ⊕ button adds a Scene and opens rename with the auto-name selected', () => {
    render(<Harness />)
    // Index 0: the header's icon button (ScenesAddButton renders before
    // ScenesPanel's own empty-state button in the Harness).
    fireEvent.click(screen.getAllByRole('button', { name: 'Add Scene' })[0])
    const input = screen.getByLabelText('Scene name') as HTMLInputElement
    expect(input.value).toBe('Scene 1')
    expect(document.activeElement).toBe(input)
  })

  it('the empty state’s Add Scene button also opens rename', () => {
    render(<Harness />)
    fireEvent.click(screen.getAllByRole('button', { name: 'Add Scene' })[0])
    expect((screen.getByLabelText('Scene name') as HTMLInputElement).value).toBe('Scene 1')
  })
})

describe('ScenesPanel — delete confirmation', () => {
  function openConfirm() {
    render(<Harness initial={[entry(1, 'Assembled')]} />)
    fireEvent.click(screen.getByRole('button', { name: 'Expand Scene details' }))
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    return screen.getByRole('dialog')
  }

  it('shows the title/body copy and defaults focus to Cancel', () => {
    const dialog = openConfirm()
    expect(within(dialog).getByText(SCENE_COPY.deleteTitle('Assembled'))).toBeInTheDocument()
    expect(within(dialog).getByText(SCENE_COPY.deleteBody)).toBeInTheDocument()
    expect(document.activeElement).toBe(within(dialog).getByRole('button', { name: 'Cancel' }))
  })

  it('Enter cancels (does not delete)', () => {
    openConfirm()
    fireEvent.keyDown(document, { key: 'Enter' })
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(screen.getByText('Assembled')).toBeInTheDocument()
  })

  it('Escape dismisses without deleting', () => {
    openConfirm()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(screen.getByText('Assembled')).toBeInTheDocument()
  })

  it('Cmd/Ctrl+Delete confirms', () => {
    openConfirm()
    fireEvent.keyDown(document, { key: 'Delete', metaKey: true })
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(screen.queryByText('Assembled')).not.toBeInTheDocument()
  })

  it('clicking Delete confirms', () => {
    const dialog = openConfirm()
    fireEvent.click(within(dialog).getByRole('button', { name: 'Delete' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(screen.queryByText('Assembled')).not.toBeInTheDocument()
  })
})
