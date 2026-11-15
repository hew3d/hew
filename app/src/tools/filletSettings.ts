/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * filletSettings — the Fillet tool's session-sticky choices, shared between
 * the tool (which reads them on every preview and commit) and the two chips
 * floating above the viewport (which set them).
 *
 * A tiny external store: `subscribe` / `getSnapshot` match React's
 * `useSyncExternalStore`, and the tool reads the same snapshot without React.
 * Nothing here is persisted; the choices last until the app reloads, which is
 * what "sticky for the session" means in the design.
 */

/** A profile the user picked from the chip. */
export type FilletProfile =
  | { kind: 'round' }
  | { kind: 'chamfer' }
  | { kind: 'cove' }
  | { kind: 'bullnose' }
  | {
      /** A drawn or built-in shape. `shape` is the kernel's normalized
       *  `ProfileShape` JSON; `id` is the built-in id or the Library item id
       *  and `name` the label shown on the chip. */
      kind: 'shape'
      id: string
      name: string
      shape: unknown
    }

/** How three-way corners are built. */
export type FilletCorners = 'default' | 'mitered' | 'flat'

export interface FilletSettings {
  profile: FilletProfile
  /** Corner style for the current profile family (see `CornersChip`). */
  corners: FilletCorners
  /** A segment count typed as `8s`, sticky until reload; null = automatic. */
  segmentsOverride: number | null
}

const INITIAL: FilletSettings = {
  profile: { kind: 'round' },
  corners: 'default',
  segmentsOverride: null,
}

let current: FilletSettings = INITIAL
const listeners = new Set<() => void>()

function publish(next: FilletSettings): void {
  current = next
  for (const l of [...listeners]) l()
}

export function getFilletSettings(): FilletSettings {
  return current
}

export function subscribeFilletSettings(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function setFilletProfile(profile: FilletProfile): void {
  // Corner choices differ between the chamfer family and the rest; a choice
  // made for one would mean something else in the other, so it resets.
  const sameFamily = (profile.kind === 'chamfer') === (current.profile.kind === 'chamfer')
  publish({ ...current, profile, corners: sameFamily ? current.corners : 'default' })
}

export function setFilletCorners(corners: FilletCorners): void {
  publish({ ...current, corners })
}

export function setFilletSegmentsOverride(n: number | null): void {
  publish({ ...current, segmentsOverride: n })
}

/** The Round <-> Chamfer swap the Alt tap performs; any other profile goes to
 *  Chamfer first (Alt is "the other basic kind"), and Chamfer goes to Round. */
export function toggledProfile(p: FilletProfile): FilletProfile {
  return p.kind === 'chamfer' ? { kind: 'round' } : { kind: 'chamfer' }
}

/** The label the measurement box shows for the profile's size. */
export function profileSizeLabel(p: FilletProfile): string {
  switch (p.kind) {
    case 'chamfer':
      return 'Distance'
    case 'shape':
      return 'Width'
    default:
      return 'Radius'
  }
}

/** Tests only: back to the initial choices. */
export function resetFilletSettingsForTest(): void {
  publish(INITIAL)
  setFilletCornerOnly(false)
  setFilletMeasurementLabelOverride(null)
}

// ── Chip support ────────────────────────────────────────────────────────────

/** How many three-way corners the armed pick forms: the Corners chip shows
 *  while this is above zero. Written by the tool, read by the chip. */
let threeWayCorners = 0
const cornerListeners = new Set<() => void>()

export function getFilletThreeWayCorners(): number {
  return threeWayCorners
}

export function setFilletThreeWayCorners(n: number): void {
  if (n === threeWayCorners) return
  threeWayCorners = n
  for (const l of [...cornerListeners]) l()
}

export function subscribeFilletThreeWayCorners(listener: () => void): () => void {
  cornerListeners.add(listener)
  return () => {
    cornerListeners.delete(listener)
  }
}

/** Whether the pointer is on (or the tool is armed on) a drawn corner, which
 *  takes only Round and Chamfer: the Profile chip disables the rest while
 *  this is set. Written by the tool, read by the chip. */
let cornerOnly = false
const cornerOnlyListeners = new Set<() => void>()

export function getFilletCornerOnly(): boolean {
  return cornerOnly
}

export function setFilletCornerOnly(on: boolean): void {
  if (on === cornerOnly) return
  cornerOnly = on
  for (const l of [...cornerOnlyListeners]) l()
}

export function subscribeFilletCornerOnly(listener: () => void): () => void {
  cornerOnlyListeners.add(listener)
  return () => {
    cornerOnlyListeners.delete(listener)
  }
}

/** The measurement box's label while a grip is held ("Stop", "Position",
 *  "Radius"); null when the label is just the profile's size label. Written
 *  by the tool, read by the app. */
let measurementLabelOverride: string | null = null
const measurementLabelListeners = new Set<() => void>()

export function getFilletMeasurementLabelOverride(): string | null {
  return measurementLabelOverride
}

export function setFilletMeasurementLabelOverride(label: string | null): void {
  if (label === measurementLabelOverride) return
  measurementLabelOverride = label
  for (const l of [...measurementLabelListeners]) l()
}

export function subscribeFilletMeasurementLabel(listener: () => void): () => void {
  measurementLabelListeners.add(listener)
  return () => {
    measurementLabelListeners.delete(listener)
  }
}
