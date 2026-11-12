import { describe, expect, it } from 'vitest'
import { buildKeymap } from './keymap'
import { defaultBindings } from './defaults'
import { nativeAccelerators, nativeChord, nativeChordFor, startNativeMenuSync } from './nativeMenu'

const macDefaults = defaultBindings({ isMac: true, isDesktop: true })
const macKeymap = (overrides: Record<string, string[]> = {}) => buildKeymap(macDefaults, overrides, true).bindings

describe('nativeChordFor', () => {
  it('is the first chord with Cmd/Ctrl/Option', () => {
    expect(nativeChordFor(['R', '2', 'Meta+K'], true)).toBe('Meta+K')
    expect(nativeChordFor(['Shift+L', 'Shift+Meta+L'], true)).toBe('Shift+Meta+L')
    expect(nativeChordFor(['R'], true)).toBeNull()
  })
})

describe('nativeAccelerators', () => {
  it('reproduces the accelerators the shell builds the menu with', () => {
    const a = nativeAccelerators(macKeymap(), true)
    expect(a['save']).toBe('CmdOrCtrl+S')
    expect(a['save-as']).toBe('Shift+CmdOrCtrl+S')
    expect(a['tool-rectangle']).toBe('CmdOrCtrl+K')
    expect(a['tool-zoom']).toBe('CmdOrCtrl+\\')
    expect(a['open-palette']).toBe('CmdOrCtrl+/')
    expect(a['toggle-model-info']).toBe('Shift+CmdOrCtrl+I')
    // The Library item dispatches the toggle-library alias.
    expect(a['toggle-library']).toBe('Shift+CmdOrCtrl+L')
    expect(a['open-library']).toBeUndefined()
  })

  it('never touches the Edit items the page claims first', () => {
    const a = nativeAccelerators(macKeymap({ 'edit-copy': ['Meta+J'] }), true)
    for (const id of ['undo', 'redo', 'edit-copy', 'edit-cut', 'edit-paste', 'edit-select-all', 'edit-delete']) {
      expect(id in a, id).toBe(false)
    }
  })

  it('follows a rebinding and clears an unbound item', () => {
    const a = nativeAccelerators(macKeymap({ 'tool-orbit': ['Ctrl+H'], 'save': [], 'zoom-extents': ['Meta+E'] }), true)
    expect(a['tool-orbit']).toBe('Ctrl+H')
    expect(a['save']).toBeNull()
    expect(a['zoom-extents']).toBe('CmdOrCtrl+E')
  })
})

describe('startNativeMenuSync', () => {
  it('records which chords the menu took, so the page runs the rest', async () => {
    let bindings = macKeymap()
    let listener: () => void = () => {}
    const pushes: Record<string, string | null>[] = []
    let refuse = new Set<string>()
    const off = startNativeMenuSync({
      isMac: true,
      defaults: macKeymap(),
      current: () => bindings,
      subscribe: (cb) => { listener = cb; return () => {} },
      invoke: async (accelerators) => {
        pushes.push(accelerators)
        return Object.keys(accelerators).filter((id) => !refuse.has(id))
      },
    })
    // Seeded from the defaults before the first answer lands.
    expect(nativeChord('save')).toBe('Meta+S')
    await Promise.resolve()
    await Promise.resolve()
    expect(nativeChord('tool-rectangle')).toBe('Meta+K')
    expect(nativeChord('open-library')).toBe('Shift+Meta+L')

    bindings = macKeymap({ 'save': ['Meta+Shift+K'], 'tool-rectangle': ['R'] })
    refuse = new Set(['save'])
    listener()
    await Promise.resolve()
    await Promise.resolve()
    expect(pushes).toHaveLength(2)
    // The menu refused Save's new accelerator: the page runs that chord.
    expect(nativeChord('save')).toBeNull()
    // Rectangle lost its Cmd-combo: the item carries none.
    expect(nativeChord('tool-rectangle')).toBeNull()
    off()
  })
})
