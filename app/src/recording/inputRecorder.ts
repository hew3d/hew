/**
 * Low-level UI-input capture — the second half of  (docs/DEVELOPMENT.md).
 *
 * The high-level recorder (`wasm-api/src/recording.rs`) captures the *resolved*
 * kernel command stream (`calls`). That replays the kernel deterministically but
 * cannot reproduce **UI-level** flakiness — a bad snap, a tool-state race, a
 * camera-dependent raycast — because by the time an op reaches the kernel the
 * input has already been interpreted. This module captures the raw input that
 * *precedes* interpretation: pointer moves/clicks, camera changes, and keys,
 * timestamped and sequenced. Replaying it drives the whole stack (inference,
 * snapping, tool state machines), so it reproduces the layer where the kernel is
 * innocent.
 *
 * It lands as a **sibling `input` array in the same artifact** as the high-level
 * `calls` (see `docs/DIAGNOSTICS.md`). The kernel never sees raw input,
 * so this is wholly app-side; the Rust `Scene::replay` ignores the extra field.
 *
 * Capture is a module-level singleton mirroring the Rust recorder's
 * `start`/`stop`/`is_active`/`take` shape, and is a **no-op unless active** so
 * wiring it into hot pointer handlers costs nothing in normal use.
 */

export type Vec3 = [number, number, number]

export interface Mods {
  shift: boolean
  alt: boolean
  ctrl: boolean
  meta: boolean
}

/** A captured pointer event, coords in CSS px relative to the canvas top-left. */
export interface PointerInput {
  kind: 'pointermove' | 'pointerdown' | 'pointerup'
  seq: number
  /** ms since `start()` (recorder-relative; for pacing/diagnostics, not identity). */
  t: number
  /** Per-gesture correlation id (bumps on each pointerdown). */
  gesture: number
  x: number
  y: number
  /** `PointerEvent.button` for down/up (-1 / undefined for moves). */
  button?: number
  /** `PointerEvent.buttons` bitmask. */
  buttons: number
  mods: Mods
}

/** A captured camera state — enough to rebuild a PerspectiveCamera + OrbitControls. */
export interface CameraInput {
  kind: 'camera'
  seq: number
  t: number
  gesture: number
  position: Vec3
  target: Vec3
  up: Vec3
  fovDeg: number
}

/** A captured key event (Shift axis-lock, Esc, Enter, Del, …). */
export interface KeyInput {
  kind: 'keydown' | 'keyup'
  seq: number
  t: number
  gesture: number
  key: string
  mods: Mods
}

export type InputEvent = PointerInput | CameraInput | KeyInput

// --- module-level singleton state (mirrors the Rust thread_local recorder) ---

let active = false
let seq = 0
let gesture = 0
let t0 = 0
let events: InputEvent[] = []
/** Injectable clock so tests are deterministic; defaults to performance.now. */
let clock: () => number = () =>
  typeof performance !== 'undefined' ? performance.now() : Date.now()

/** Begin capture, discarding any prior buffer. Resets seq/gesture/clock origin. */
export function start(): void {
  events = []
  seq = 0
  gesture = 0
  t0 = clock()
  active = true
}

/** Stop capture; the buffer stays available to {@link take}. */
export function stop(): void {
  active = false
}

/** Whether capture is active. */
export function isActive(): boolean {
  return active
}

/** Take the captured events, clearing the buffer (mirrors Rust `take_calls`). */
export function take(): InputEvent[] {
  const out = events
  events = []
  return out
}

/** Test seam: override the clock used for `t`. Pass nothing to restore default. */
export function __setClockForTest(fn?: () => number): void {
  clock =
    fn ??
    (() => (typeof performance !== 'undefined' ? performance.now() : Date.now()))
}

function stamp(): { seq: number; t: number; gesture: number } {
  return { seq: seq++, t: clock() - t0, gesture }
}

function mods(e: { shiftKey: boolean; altKey: boolean; ctrlKey: boolean; metaKey: boolean }): Mods {
  return { shift: e.shiftKey, alt: e.altKey, ctrl: e.ctrlKey, meta: e.metaKey }
}

/**
 * Record a pointer event. `x`/`y` must already be canvas-relative CSS px (the
 * caller has the bounding rect). A no-op unless capture is active. A `pointerdown`
 * opens a new gesture (bumps the correlation id) *before* it is stamped, so the
 * down and its following moves/up share one id.
 */
export function recordPointer(
  kind: PointerInput['kind'],
  x: number,
  y: number,
  e: PointerEvent,
): void {
  if (!active) return
  if (kind === 'pointerdown') gesture++
  events.push({ kind, ...stamp(), x, y, button: e.button, buttons: e.buttons, mods: mods(e) })
}

/** Record a camera state. A no-op unless active. */
export function recordCamera(
  position: Vec3,
  target: Vec3,
  up: Vec3,
  fovDeg: number,
): void {
  if (!active) return
  events.push({ kind: 'camera', ...stamp(), position, target, up, fovDeg })
}

/** Record a key event. A no-op unless active. */
export function recordKey(kind: KeyInput['kind'], e: KeyboardEvent): void {
  if (!active) return
  events.push({ kind, ...stamp(), key: e.key, mods: mods(e) })
}
