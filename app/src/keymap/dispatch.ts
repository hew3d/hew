/**
 * The one keydown dispatcher for commands: a keystroke is turned into a
 * chord (`chord.ts`), looked up in the keymap (`keymap.ts`), and the command
 * it names runs through the caller's `run` — App.tsx's `menuActionRef`, the
 * same path every menu click and palette pick takes.
 *
 * It replaces the hand-written if-chains that App.tsx and Viewport.tsx each
 * carried (and that had drifted apart). The guards are the ones those
 * chains applied, in the same order:
 *
 * 1. An input-method composition owns every keystroke.
 * 2. Backspace outside a text field is always swallowed, bound or not: the
 *    webview would otherwise take it as "navigate back".
 * 3. A modal welcome screen blocks every command; a Cmd/Ctrl chord is still
 *    swallowed so the browser doesn't act on it (Ctrl+S "save page").
 * 4. A focused text field keeps its keys (its own copy, paste, select all).
 * 5. A chord with no Cmd/Ctrl/Option — a bare or shifted key — stays out of
 *    the commands while an open dialog owns the keyboard, or while the active
 *    tool is typing a measurement ("1cm", "5' 2-1/4\"": letters are unit
 *    suffixes there, digits are the value).
 * 6. On the macOS desktop a chord the native menu carries is left to it
 *    (`nativeMenu.ts`).
 */

import { isComposingKey } from '../input/keyboard'
import { chordFromEvent, chordToString, hasCommandModifier } from './chord'
import { JS_FIRST } from './nativeMenu'

export interface DispatchContext {
  isMac: boolean
  /** The macOS native menu bar is attached (macOS desktop shell). */
  nativeMenu: boolean
  welcomeOpen: boolean
  /** The active tool claims this key for its typed measurement. */
  toolCapturesKey: (key: string) => boolean
  /** A dialog that owns the keyboard is open. */
  dialogOpen: () => boolean
  commandForChord: (chord: string) => string | undefined
  /** The chord a command's native menu item carries now (`nativeMenu.ts`). */
  nativeChord: (id: string) => string | null
}

export type Resolution =
  | { kind: 'ignore' }
  /** preventDefault, run nothing. */
  | { kind: 'swallow' }
  | { kind: 'run'; id: string }

function isTextField(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null
  return el !== null && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable === true)
}

/** What a keydown should do. Pure: the hook below applies the answer. */
export function resolveKeydown(ev: KeyboardEvent, ctx: DispatchContext): Resolution {
  if (isComposingKey(ev)) return { kind: 'ignore' }
  const typing = isTextField(ev.target)
  const chord = chordFromEvent(ev, ctx.isMac)
  if (chord === null) return { kind: 'ignore' }
  const commandChord = hasCommandModifier(chord)

  // oxlint-disable-next-line lingui/no-unlocalized-strings -- KeyboardEvent.key identifier
  const backspace = ev.key === 'Backspace' && !commandChord
  if (ctx.welcomeOpen) {
    if (typing) return { kind: 'ignore' }
    return backspace || (commandChord && !ctx.nativeMenu) ? { kind: 'swallow' } : { kind: 'ignore' }
  }
  if (typing) return { kind: 'ignore' }

  if (!commandChord && (ctx.dialogOpen() || ctx.toolCapturesKey(ev.key))) {
    return backspace ? { kind: 'swallow' } : { kind: 'ignore' }
  }

  const text = chordToString(chord)
  const id = ctx.commandForChord(text)
  if (id === undefined) return backspace ? { kind: 'swallow' } : { kind: 'ignore' }

  // The native item carries exactly this chord: let the menu fire it.
  if (ctx.nativeMenu && commandChord && !JS_FIRST.has(id) && ctx.nativeChord(id) === text) {
    return { kind: 'ignore' }
  }
  return { kind: 'run', id }
}
