/**
 * The compounds this window opened around plugin command forwards
 * (docs/agents/PLUGINS.md "Compounds"), innermost last, as the kernel keeps
 * them. The live bridge (a socket caller's forward) and the plugin host (a
 * realm's or the window's own) both open and close theirs here, so either
 * can see what the other has open above its own.
 *
 * The kernel closes only the innermost compound. A plugin can open an inner
 * compound and then answer its outer command without waiting for it (or
 * the outer is cancelled, times out, or its plugin stops while the inner
 * one runs). So whoever closes a compound first ends every compound opened
 * above it, innermost first, through each one's own owner (`unwind`): the
 * inner forward is answered `cancelled`, its compound aborted, its plugin
 * told. Only then is the outer one ended or aborted. No order of answers,
 * cancels, timeouts or crashes can leave a compound open under a closed
 * one.
 *
 * A compound the kernel would not close (a refusal this module did not
 * expect) is kept as stranded. When nothing of this window's is open any
 * more but the document is still held, each stranded compound is aborted by
 * its own token, which the kernel refuses unless it is the innermost: a
 * compound someone else opened (the test harness's) is never touched.
 * While any is stranded the sweep is tried again whenever the busy state
 * changes, so one stranded under someone else's compound is released when
 * that one ends. Each stranded compound is reported once.
 *
 * Every-build code: with the plugin host off there is at most one compound
 * open at a time, the bridge's, and this is the same begin, end and abort.
 */

import type { Scene } from '../wasm/loader'
import { subscribeBusy } from './busyState'

export interface OpenCompound {
  readonly kernelToken: bigint
  /** The kernel's token of the forward the compound is held for: what a
   *  call made while serving it is stamped with. */
  readonly forwardToken: number
  /** Ends the forward at once without its plugin's answer: answers its
   *  caller `cancelled`, tells its plugin, and aborts the compound through
   *  `abortCompound`. Called only while it is the innermost one. */
  readonly unwind: () => void
}

interface Stack {
  open: OpenCompound[]
  stranded: bigint[]
  /** Stranded tokens already reported. */
  reported: Set<bigint>
  /** Set while a stranded compound waits: unsubscribes the retry. */
  retry: (() => void) | null
}

/** Stranded tokens kept at most; older ones are forgotten. */
const MAX_STRANDED = 64

const stacks = new WeakMap<Scene, Stack>()
const listeners = new Set<() => void>()

function stackOf(scene: Scene): Stack {
  let st = stacks.get(scene)
  if (st === undefined) {
    st = { open: [], stranded: [], reported: new Set(), retry: null }
    stacks.set(scene, st)
  }
  return st
}

function changed(): void {
  for (const fn of [...listeners]) {
    try {
      fn()
    } catch (err) {
      console.error('a compound listener failed', err)
    }
  }
}

/** Calls `fn` after every begin and close; returns the unsubscribe. */
export function subscribeCompounds(fn: () => void): () => void {
  listeners.add(fn)
  return () => {
    listeners.delete(fn)
  }
}

/** Opens the compound for forward `forwardToken` on `conn`. Throws what
 *  `Scene.compound_begin` throws; nothing is held then. */
export function beginCompound(scene: Scene, conn: number, forwardToken: number, unwind: () => void): OpenCompound {
  const kernelToken = scene.compound_begin(conn, BigInt(forwardToken))
  const entry: OpenCompound = { kernelToken, forwardToken, unwind }
  stackOf(scene).open.push(entry)
  changed()
  return entry
}

/** Whether the compound held for forward `forwardToken` is still open. */
export function isForwardOpen(scene: Scene, forwardToken: number): boolean {
  return stackOf(scene).open.some((c) => c.forwardToken === forwardToken)
}

/** The forward token of the innermost compound this window holds, or null. */
export function innermostForward(scene: Scene): number | null {
  const open = stackOf(scene).open
  return open.length === 0 ? null : open[open.length - 1].forwardToken
}

/** How many compounds this window holds. */
export function openCompoundCount(scene: Scene): number {
  return stackOf(scene).open.length
}

const remove = (st: Stack, entry: OpenCompound): void => {
  const i = st.open.indexOf(entry)
  if (i >= 0) st.open.splice(i, 1)
}

const strand = (scene: Scene, st: Stack, entry: OpenCompound): void => {
  remove(st, entry)
  st.stranded.push(entry.kernelToken)
  if (st.stranded.length > MAX_STRANDED) st.stranded.shift()
  st.retry ??= subscribeBusy(() => sweep(scene, st))
}

const settled = (st: Stack): void => {
  st.reported.clear()
  st.retry?.()
  st.retry = null
}

/** Ends every compound above `entry`, innermost first, each through its
 *  owner; one whose owner leaves it open is aborted here. */
function unwindAbove(scene: Scene, st: Stack, entry: OpenCompound): void {
  for (;;) {
    const at = st.open.indexOf(entry)
    if (at < 0 || at === st.open.length - 1) return
    const top = st.open[st.open.length - 1]
    try {
      top.unwind()
    } catch (err) {
      console.error('ending a nested plugin command failed', err)
    }
    if (st.open[st.open.length - 1] === top) {
      console.error('a nested plugin command was left open by its owner; aborting it')
      abortCompound(scene, top)
    }
  }
}

/**
 * The last resort, after every close: nothing of this window's is open,
 * yet the document is held, by a compound this window opened and the
 * kernel would not close. Each is aborted by its own token, innermost
 * first; the kernel refuses any that is not the innermost, so nothing
 * opened by anyone else is closed.
 */
function sweep(scene: Scene, st: Stack): void {
  if (st.open.length > 0) return
  let busy: boolean
  try {
    busy = scene.busy()
  } catch {
    return
  }
  if (!busy) {
    st.stranded.length = 0
    settled(st)
    return
  }
  if (st.stranded.length === 0) {
    settled(st)
    return
  }
  for (let i = st.stranded.length - 1; i >= 0; i--) {
    const token = st.stranded[i]
    const first = !st.reported.has(token)
    st.reported.add(token)
    try {
      scene.compound_abort(token)
      st.stranded.splice(i, 1)
      console.error('a plugin command compound that did not close held the document; it was aborted')
    } catch (err) {
      if (first) console.error('a plugin command compound that did not close still holds the document; it is aborted once it is the innermost', err)
    }
  }
  if (st.stranded.length === 0) settled(st)
}

const depth = (scene: Scene): number | null => {
  try {
    return scene.compound_depth()
  } catch {
    return null
  }
}

/**
 * Ends `entry` (one undo entry), after ending every compound above it.
 * Returns what `Scene.compound_end` answers. Throws what it throws; when
 * the kernel kept the compound open the entry stays here, and the caller
 * aborts it.
 */
export function endCompound(scene: Scene, entry: OpenCompound): string {
  const st = stackOf(scene)
  unwindAbove(scene, st, entry)
  const before = depth(scene)
  try {
    const out = scene.compound_end(entry.kernelToken)
    remove(st, entry)
    return out
  } catch (err) {
    // A disturbance refusal aborts the compound: it is no longer open.
    const after = depth(scene)
    if (before !== null && after !== null && after < before) remove(st, entry)
    throw err
  } finally {
    sweep(scene, st)
    changed()
  }
}

/** Aborts `entry` (the document goes back to its begin), after ending
 *  every compound above it. Never throws: a refusal is logged, and the
 *  compound kept as stranded for the last resort. */
export function abortCompound(scene: Scene, entry: OpenCompound): void {
  const st = stackOf(scene)
  if (!st.open.includes(entry)) return
  unwindAbove(scene, st, entry)
  try {
    scene.compound_abort(entry.kernelToken)
    remove(st, entry)
  } catch (err) {
    console.error('Scene.compound_abort refused a compound this window opened', err)
    strand(scene, st, entry)
  } finally {
    sweep(scene, st)
    changed()
  }
}
