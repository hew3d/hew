/** React bindings for the keymap: the keydown dispatcher and a re-render
 *  hook for everything that shows a shortcut. */

import { useEffect, useRef, useSyncExternalStore } from 'react'
import { resolveKeydown } from './dispatch'
import { commandForChord, currentKeymap, subscribeKeymap, type Keymap } from './keymap'
import { nativeChord } from './nativeMenu'

export interface KeymapDispatchOptions {
  isMac: boolean
  /** The macOS native menu bar is attached (macOS desktop shell). */
  nativeMenu: boolean
  /** Runs a command id (App.tsx's `menuActionRef.current`). */
  run: (id: string) => void
  /** Read on every keydown, so a ref-backed getter is enough. */
  welcomeOpen?: () => boolean
  /** Read on every keydown: the active tool claims `key` for its typed
   *  measurement. */
  toolCapturesKey?: (key: string) => boolean
  /** Restricts which commands this surface runs (Shop Mode's three tools);
   *  a chord of any other command is left alone. */
  allow?: (id: string) => boolean
}

/** A `role="dialog"` that owns the keyboard is open — the same test as
 *  Viewport.tsx's `keyboardBelongsToFieldOrDialog` (a non-blocking floating
 *  window says `aria-modal="false"`). */
function dialogOwnsKeyboard(): boolean {
  return document.querySelector('[role="dialog"]:not([aria-modal="false"])') !== null
}

/** Installs the window keydown listener that runs keymap commands. */
export function useKeymapDispatch(options: KeymapDispatchOptions): void {
  const ref = useRef(options)
  ref.current = options
  useEffect(() => {
    const onKeyDown = (ev: KeyboardEvent) => {
      const o = ref.current
      const res = resolveKeydown(ev, {
        isMac: o.isMac,
        nativeMenu: o.nativeMenu,
        welcomeOpen: o.welcomeOpen?.() ?? false,
        toolCapturesKey: (key) => o.toolCapturesKey?.(key) ?? false,
        dialogOpen: dialogOwnsKeyboard,
        commandForChord,
        nativeChord,
      })
      if (res.kind === 'ignore') return
      if (res.kind === 'run' && o.allow !== undefined && !o.allow(res.id)) return
      ev.preventDefault()
      if (res.kind === 'run') o.run(res.id)
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])
}

/** The current keymap; re-renders the caller whenever it changes. */
export function useKeymap(): Keymap {
  return useSyncExternalStore(subscribeKeymap, currentKeymap, currentKeymap)
}
