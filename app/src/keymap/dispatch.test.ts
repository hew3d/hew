import { describe, expect, it } from 'vitest'
import { buildKeymap } from './keymap'
import { defaultBindings } from './defaults'
import { resolveKeydown, type DispatchContext } from './dispatch'

type KeyInit = Partial<Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey' | 'isComposing' | 'keyCode'>> & {
  target?: { tagName: string; isContentEditable?: boolean }
}

function ev(init: KeyInit): KeyboardEvent {
  return {
    key: '', code: '', ctrlKey: false, altKey: false, shiftKey: false, metaKey: false,
    isComposing: false, keyCode: 0, target: { tagName: 'BODY', isContentEditable: false },
    ...init,
  } as unknown as KeyboardEvent
}

function ctx(over: Partial<DispatchContext> = {}, isMac = false, isDesktop = false): DispatchContext {
  const km = buildKeymap(defaultBindings({ isMac, isDesktop }), {}, isMac)
  return {
    isMac,
    nativeMenu: false,
    welcomeOpen: false,
    toolCapturesKey: () => false,
    dialogOpen: () => false,
    commandForChord: (c) => km.byChord.get(c),
    nativeChord: () => null,
    ...over,
  }
}

describe('resolveKeydown', () => {
  it('runs the command a chord is bound to', () => {
    expect(resolveKeydown(ev({ key: 'r', code: 'KeyR' }), ctx())).toEqual({ kind: 'run', id: 'tool-rectangle' })
    expect(resolveKeydown(ev({ key: 's', code: 'KeyS', ctrlKey: true }), ctx())).toEqual({ kind: 'run', id: 'save' })
    expect(resolveKeydown(ev({ key: ' ', code: 'Space' }), ctx())).toEqual({ kind: 'run', id: 'tool-select' })
    expect(resolveKeydown(ev({ key: 'L', code: 'KeyL', shiftKey: true }), ctx())).toEqual({ kind: 'run', id: 'open-library' })
  })

  it('ignores an unbound chord (a shifted tool letter is not the tool)', () => {
    expect(resolveKeydown(ev({ key: 'R', code: 'KeyR', shiftKey: true }), ctx())).toEqual({ kind: 'ignore' })
    expect(resolveKeydown(ev({ key: 'k', code: 'KeyK' }), ctx())).toEqual({ kind: 'ignore' })
  })

  it('leaves an IME composition alone', () => {
    expect(resolveKeydown(ev({ key: 'Process', code: 'KeyR', keyCode: 229 }), ctx())).toEqual({ kind: 'ignore' })
    expect(resolveKeydown(ev({ key: 'r', code: 'KeyR', isComposing: true }), ctx())).toEqual({ kind: 'ignore' })
  })

  it('leaves a focused text field its keys, Cmd/Ctrl chords included', () => {
    const input = { tagName: 'INPUT' }
    expect(resolveKeydown(ev({ key: 'r', code: 'KeyR', target: input }), ctx())).toEqual({ kind: 'ignore' })
    expect(resolveKeydown(ev({ key: 'c', code: 'KeyC', ctrlKey: true, target: input }), ctx())).toEqual({ kind: 'ignore' })
    expect(resolveKeydown(ev({ key: 'Backspace', code: 'Backspace', target: { tagName: 'DIV', isContentEditable: true } }), ctx())).toEqual({ kind: 'ignore' })
  })

  it('keeps bare keys out while a tool types a measurement, but not Cmd/Ctrl chords', () => {
    const typing = ctx({ toolCapturesKey: () => true })
    expect(resolveKeydown(ev({ key: 'm', code: 'KeyM' }), typing)).toEqual({ kind: 'ignore' })
    expect(resolveKeydown(ev({ key: '4', code: 'Digit4' }), typing)).toEqual({ kind: 'ignore' })
    expect(resolveKeydown(ev({ key: 'z', code: 'KeyZ', ctrlKey: true }), typing)).toEqual({ kind: 'run', id: 'undo' })
  })

  it('keeps bare keys out while a dialog owns the keyboard', () => {
    const dialog = ctx({ dialogOpen: () => true })
    expect(resolveKeydown(ev({ key: 'r', code: 'KeyR' }), dialog)).toEqual({ kind: 'ignore' })
    expect(resolveKeydown(ev({ key: 'Delete', code: 'Delete' }), dialog)).toEqual({ kind: 'ignore' })
    expect(resolveKeydown(ev({ key: 's', code: 'KeyS', ctrlKey: true }), dialog)).toEqual({ kind: 'run', id: 'save' })
  })

  it('always swallows Backspace outside a text field (the webview would navigate back)', () => {
    expect(resolveKeydown(ev({ key: 'Backspace', code: 'Backspace' }), ctx({ toolCapturesKey: () => true }))).toEqual({ kind: 'swallow' })
    expect(resolveKeydown(ev({ key: 'Backspace', code: 'Backspace' }), ctx({ dialogOpen: () => true }))).toEqual({ kind: 'swallow' })
    const unbound = ctx({ commandForChord: () => undefined })
    expect(resolveKeydown(ev({ key: 'Backspace', code: 'Backspace' }), unbound)).toEqual({ kind: 'swallow' })
    expect(resolveKeydown(ev({ key: 'Backspace', code: 'Backspace' }), ctx())).toEqual({ kind: 'run', id: 'edit-delete' })
  })

  it('runs nothing behind the welcome screen, swallowing Cmd/Ctrl chords', () => {
    const welcome = ctx({ welcomeOpen: true })
    expect(resolveKeydown(ev({ key: 'r', code: 'KeyR' }), welcome)).toEqual({ kind: 'ignore' })
    expect(resolveKeydown(ev({ key: 's', code: 'KeyS', ctrlKey: true }), welcome)).toEqual({ kind: 'swallow' })
    expect(resolveKeydown(ev({ key: 'Backspace', code: 'Backspace' }), welcome)).toEqual({ kind: 'swallow' })
    // On the macOS desktop the native menu owns Cmd chords: never swallowed.
    expect(resolveKeydown(ev({ key: 's', code: 'KeyS', metaKey: true }), ctx({ welcomeOpen: true, nativeMenu: true }, true, true)))
      .toEqual({ kind: 'ignore' })
  })

  describe('macOS native menu', () => {
    const mac = (nativeChord: (id: string) => string | null) => ctx({ nativeMenu: true, nativeChord }, true, true)

    it('leaves a chord to the native item that carries it', () => {
      const c = mac((id) => (id === 'save' ? 'Meta+S' : null))
      expect(resolveKeydown(ev({ key: 's', code: 'KeyS', metaKey: true }), c)).toEqual({ kind: 'ignore' })
    })

    it('runs a chord the native item does not carry (a second binding, or one the menu refused)', () => {
      const c = mac(() => null)
      expect(resolveKeydown(ev({ key: 's', code: 'KeyS', metaKey: true }), c)).toEqual({ kind: 'run', id: 'save' })
      expect(resolveKeydown(ev({ key: 'k', code: 'KeyK', metaKey: true }), mac(() => 'R'))).toEqual({ kind: 'run', id: 'tool-rectangle' })
    })

    it('always runs the Edit commands the page claims first', () => {
      const c = mac((id) => (id === 'undo' ? 'Meta+Z' : null))
      expect(resolveKeydown(ev({ key: 'z', code: 'KeyZ', metaKey: true }), c)).toEqual({ kind: 'run', id: 'undo' })
    })

    it('runs bare keys in the page', () => {
      expect(resolveKeydown(ev({ key: 'r', code: 'KeyR' }), mac(() => null))).toEqual({ kind: 'run', id: 'tool-rectangle' })
    })
  })
})
