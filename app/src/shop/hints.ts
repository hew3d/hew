/**
 * hints — Shop Mode's ghost-affordance gesture-discoverability engine
 * (design_handoff_shop_mode/README.md "Gesture discoverability" +
 * `Shop Mode Screens.dc.html`'s option 1q spec detail, the fuller source
 * for this wave). Three tiny, install-scoped coach marks that teach
 * tap-to-inspect, orbit, and long-press-isolate — never a tutorial wall,
 * never more than one on screen, never taught twice.
 *
 * The two hints (design's own numbering — (c) 'hold' was removed: playtest
 * finding 7 found it redundant with `InspectCard.tsx`'s own permanent
 * "hold part to isolate" caption, which teaches the exact same gesture
 * every time a part card is up rather than once per install):
 *   (a) 'tap'   — first model open: a pulsing dot over the largest visible
 *       part + "Tap a part for its size". Eligible the moment a document
 *       with visible geometry opens.
 *   (b) 'orbit' — no orbit within 8s of a document open: a one-shot ghost-
 *       finger arc ("plays once", then gone regardless of outcome).
 *
 * Rules (design's "Gesture discoverability" spec, verbatim):
 *   - Only one hint on screen at a time.
 *   - Each hint fires AT MOST ONCE PER INSTALL, tracked via a `localStorage`
 *     flag (`HINT_STORAGE_KEYS`) written the MOMENT the hint starts
 *     showing, not when it's dismissed — "fires once" describes the
 *     display event itself, so a hint the user never acts on (they close
 *     the tab mid-display) still never returns next session.
 *   - Performing a hint's taught gesture kills it INSTANTLY if it's the one
 *     currently showing, and — the part that's easy to miss — ALSO marks
 *     it satisfied even if it was never shown yet at all: a user who
 *     already orbits inside the first 8s, or long-presses on their very
 *     first inspect, has demonstrably already learned the thing the hint
 *     would have taught, so it must never fire later just because the
 *     clock/counter happens to catch up (design: "no teaching what's
 *     already known").
 *
 * A plain reactive class, not a React hook or a set of free functions —
 * `ShopApp.tsx` holds one instance for the life of the session (a `useRef`,
 * like `MultiClickTracker`'s own precedent in `viewport/multiClick.ts`).
 * Every public method below is one event ("this just happened") and
 * re-evaluates which hint (if any) should be active, INCLUDING `tick()` —
 * the one input with no corresponding user gesture, which exists purely so
 * the 8s no-orbit delay and hint (b)'s own one-shot play-through have
 * something to re-check themselves against without the engine owning a
 * `setTimeout` of its own. Nothing in this file reads `Date.now()` or
 * touches the DOM except through the injected `now`/`storage` — the whole
 * point, so every rule above is exercised in `hints.test.ts` with a fully
 * synthetic clock and no mounted component.
 */

import type { PartsSheetSection } from './partsSheetModel'
import type { NodeRef } from '../panels/treeModel'

export type HintName = 'tap' | 'orbit'

/** What's currently on screen, or `null`. 'tap' alone carries a payload —
 *  the part it should point at — since 'orbit' is always anchored the same
 *  way regardless of which document triggered it (the viewport center). */
export type ActiveHint =
  | { name: 'tap'; targetPartId: string }
  | { name: 'orbit' }

/** Design: "If no orbit within 8s". */
export const NO_ORBIT_HINT_DELAY_MS = 8000
/** How long hint (b)'s one-shot ghost-arc gets to play before the engine
 *  auto-dismisses it on its own (design: "plays once") — kept in step with
 *  the CSS animation's own duration by a cross-reference comment on both
 *  sides (index.css's `shop-hint-orbit-finger` keyframes). */
export const ORBIT_HINT_PLAY_MS = 1800

/** `localStorage` keys, one per hint — `hew.shop.hint.<name>`, the app's
 *  existing dot-namespaced settings-key convention (`hew.settings.
 *  lengthUnit`, `settings/units.ts`) rather than `shellMode.ts`'s colon
 *  form, which names a single override rather than one flag per item in a
 *  small family — closer to this module's own shape. */
export const HINT_STORAGE_KEYS: Record<HintName, string> = {
  tap: 'hew.shop.hint.tap',
  orbit: 'hew.shop.hint.orbit',
}

/**
 * E2E-only escape hatch: when this flag reads `'1'`, `ShopApp` never RENDERS
 * a hint regardless of what the engine itself is doing — the engine keeps
 * running exactly as it would in production (it still fires, still writes
 * `HINT_STORAGE_KEYS`), only the on-screen overlay is suppressed. This key
 * is never written by the app itself, only by a test's `page.addInitScript`
 * before navigation.
 *
 * Why this exists: Shop Mode's E2E suite (`shop-mode.spec.ts`) predates
 * this wave — every one of its ~15 existing specs boots a fresh,
 * flag-free `localStorage` and asserts on exact DOM content/roles. Hint (a)
 * fires unconditionally on the FIRST document open of a flag-free install,
 * which every one of those specs now is — without this, they'd all start
 * racing a pulsing dot + "Tap a part for its size" tag they never asked
 * for. The hint overlay is `pointer-events: none` (module doc's own "never
 * intercept" rule) so it can never break a TAP those specs make, but a
 * couple of them assert broader things (e.g. exact visible-text sets) that
 * an extra floating tag could still trip. Rather than gate on
 * `window.__hew_shop_test` existing (which would make the ONE spec that
 * actually tests hints impossible to write, since that spec needs the
 * harness too), every OTHER spec's shared boot helper
 * (`bootShopModeWith`) sets this flag explicitly; the hints spec leaves it
 * unset, matching a real fresh install (hints ON by default, this key
 * simply absent).
 */
export const HINT_TEST_SUPPRESS_KEY = 'hew.shop.test.suppressHints'

function safeLocalStorage(): Storage | undefined {
  return typeof localStorage === 'undefined' ? undefined : localStorage
}

/** Whether the E2E suppression flag above is set. Reads live `localStorage`
 *  by default; `storage` is injectable for tests, matching every other
 *  localStorage reader in `shop/` (`shellMode.ts`'s `readShellModeOverride`). */
export function testHintsSuppressed(storage: Pick<Storage, 'getItem'> | undefined = safeLocalStorage()): boolean {
  if (storage === undefined) return false
  try {
    return storage.getItem(HINT_TEST_SUPPRESS_KEY) === '1'
  } catch {
    return false
  }
}

type HintStorage = Pick<Storage, 'getItem' | 'setItem'>

function readSatisfied(storage: HintStorage | undefined): Record<HintName, boolean> {
  const read = (name: HintName): boolean => {
    if (storage === undefined) return false
    try {
      return storage.getItem(HINT_STORAGE_KEYS[name]) === '1'
    } catch {
      return false
    }
  }
  return { tap: read('tap'), orbit: read('orbit') }
}

function writeSatisfied(storage: HintStorage | undefined, name: HintName): void {
  if (storage === undefined) return
  try {
    storage.setItem(HINT_STORAGE_KEYS[name], '1')
  } catch {
    /* best-effort — matches every other localStorage write in shop/ */
  }
}

export interface HintEngineOptions {
  /** Injected clock — every internal time comparison reads through this,
   *  never `Date.now()` directly, so tests drive the 8s/1.8s windows with a
   *  fake, steppable clock instead of real timers. Defaults to `Date.now`. */
  now?: () => number
  /** Defaults to `localStorage` (or `undefined` off-DOM) — same injection
   *  pattern as `shellMode.ts`'s read/write helpers. */
  storage?: HintStorage | undefined
}

/**
 * The gesture-discoverability state machine (module doc). One instance per
 * Shop Mode session — construction reads the persisted per-hint flags once;
 * every later change is written straight through `storage`, so a second
 * instance built against the SAME storage a moment later observes identical
 * state (no separate load/save step to keep in sync).
 */
export class HintEngine {
  private readonly clock: () => number
  private readonly storage: HintStorage | undefined
  private readonly satisfied: Record<HintName, boolean>

  private active: ActiveHint | null = null
  private activeSince = 0
  private docOpenAt: number | null = null
  private tapTargetId: string | null = null

  constructor(options: HintEngineOptions = {}) {
    this.clock = options.now ?? Date.now
    this.storage = options.storage === undefined ? safeLocalStorage() : options.storage
    this.satisfied = readSatisfied(this.storage)
  }

  /** The hint to render right now, or `null`. */
  getActive(): ActiveHint | null {
    return this.active
  }

  /**
   * A new document finished loading. `targetPartId` is a stable id for the
   * largest currently-visible part (`largestVisiblePart` below, fed through
   * `treeModel.nodeKey`), or `null` if there's nothing for hint (a) to
   * point at (an empty document, or one with only hidden/meshless parts).
   * Resets the no-orbit clock, and drops whatever hint was showing for the
   * PREVIOUS document — a hint's activation never carries across an open,
   * though its SATISFIED state (once fired, ever) always does.
   */
  documentOpened(targetPartId: string | null): void {
    const now = this.clock()
    this.docOpenAt = now
    this.tapTargetId = targetPartId
    this.active = null
    this.evaluate(now)
  }

  /** A tap just resolved to something inspectable (edge or whole-part —
   *  `InspectCard` is about to show either way). The gesture hint (a)
   *  teaches. */
  tapped(): void {
    this.satisfy('tap')
  }

  /** A camera drag started. `Viewport`'s `onCameraDragChange` fires for
   *  rotate/pan/pinch alike without distinguishing which — see
   *  `ShopApp.tsx`'s wiring comment for why treating any of them as "found
   *  the camera" is an acceptable simplification for a ghost hint rather
   *  than plumbing a real gesture-kind signal up through `ViewportApi`. The
   *  gesture hint (b) teaches. */
  orbited(): void {
    this.satisfy('orbit')
  }

  /**
   * Re-check the engine's own time-driven conditions — the 8s no-orbit
   * delay, and hint (b)'s one-shot play-through ending. The one input with
   * no user gesture behind it. Callers should invoke this periodically
   * (e.g. every few hundred ms) whenever a document is open and hints are
   * otherwise allowed to show; a caller that stops ticking while gated
   * (a menu/sheet is open, module doc) simply defers these two
   * conditions rather than spending their one shot unseen.
   */
  tick(): void {
    this.evaluate(this.clock())
  }

  private satisfy(name: HintName): void {
    if (!this.satisfied[name]) {
      this.satisfied[name] = true
      writeSatisfied(this.storage, name)
    }
    if (this.active !== null && this.active.name === name) {
      this.active = null
    }
    // Clearing the slot above can make room for something ELSE that was
    // already eligible but blocked ("one hint at a time", module doc) — re-
    // scan now rather than waiting for the next `tick()`.
    this.evaluate(this.clock())
  }

  private fire(hint: ActiveHint, now: number): void {
    this.active = hint
    this.activeSince = now
    if (!this.satisfied[hint.name]) {
      this.satisfied[hint.name] = true
      writeSatisfied(this.storage, hint.name)
    }
  }

  /**
   * The priority scan: at most one hint active, checked in the design's own
   * listed order (a, b) whenever nothing currently outranks it. Run after
   * EVERY state change (not just `tick()`) so a hint that became eligible
   * while another was showing gets picked up the moment that one clears,
   * rather than waiting for the next timer tick to notice.
   */
  private evaluate(now: number): void {
    if (this.active !== null && this.active.name === 'orbit' && now - this.activeSince >= ORBIT_HINT_PLAY_MS) {
      this.active = null
    }
    if (this.active !== null) return
    if (!this.satisfied.tap && this.tapTargetId !== null) {
      this.fire({ name: 'tap', targetPartId: this.tapTargetId }, now)
      return
    }
    if (!this.satisfied.orbit && this.docOpenAt !== null && now - this.docOpenAt >= NO_ORBIT_HINT_DELAY_MS) {
      this.fire({ name: 'orbit' }, now)
    }
  }
}

/**
 * The largest (by AABB volume) currently-VISIBLE part across `sections` —
 * hint (a)'s target (design: "on the largest part"). `null` if every
 * section is empty, every row is hidden, or no row resolves to a mesh (a
 * document of only empty groups/sketches). Ties resolve to whichever row
 * `sections` visits first (section order, then row order) — arbitrary but
 * stable, so the same document always seeds the same target.
 */
export function largestVisiblePart(sections: readonly PartsSheetSection[]): NodeRef | null {
  let best: NodeRef | null = null
  let bestVolume = -Infinity
  for (const section of sections) {
    for (const row of section.rows) {
      if (row.hidden || row.extentsM === null) continue
      const volume = row.extentsM[0] * row.extentsM[1] * row.extentsM[2]
      if (volume > bestVolume) {
        bestVolume = volume
        best = row.node
      }
    }
  }
  return best
}
