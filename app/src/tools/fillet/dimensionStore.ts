/* oxlint-disable lingui/no-unlocalized-strings -- Preview-only UI copy */
/** The active tool owns this connection; React never owns operation state. */
import type { GripId } from '../filletGrips'
import type { FilletProfile } from '../filletSettings'

export interface DimensionSnapshot {
  identity: string
  grip: GripId | null
  profile: FilletProfile | null
  label: string
  value: number | null
  requested: number | null
  limit: number | null
  reason: string | null
  draft?: string
}
export interface DimensionGesture {
  update(meters: number, text?: string): void
  finish(): void
  cancel(): void
}
export interface DimensionSource {
  snapshot(): DimensionSnapshot
  begin(identity: string, input?: 'slider' | 'typed'): DimensionGesture | null
}
export const NO_DIMENSION: DimensionSnapshot = {
  identity: '', grip: null, profile: null, label: 'Size', value: null,
  requested: null, limit: null, reason: 'Pick an edge or face to adjust its size.',
}
let owner: DimensionSource | null = null
let current = NO_DIMENSION
const listeners = new Set<() => void>()
export const getFilletDimension = (): DimensionSnapshot => current
export const subscribeFilletDimension = (cb: () => void): (() => void) => {
  listeners.add(cb)
  return () => { listeners.delete(cb) }
}
export function publishFilletDimension(source: DimensionSource): void {
  if (owner !== source) return
  const next = source.snapshot()
  if (next.identity === current.identity && next.profile === current.profile && next.label === current.label && next.value === current.value && next.requested === current.requested && next.limit === current.limit && next.reason === current.reason && next.draft === current.draft) return
  current = next
  for (const cb of [...listeners]) cb()
}
export function connectFilletDimension(source: DimensionSource): void {
  owner = source
  publishFilletDimension(source)
}
export function disconnectFilletDimension(source: DimensionSource): void {
  if (owner !== source) return
  owner = null
  current = NO_DIMENSION
  for (const cb of [...listeners]) cb()
}
export const beginFilletDimension = (identity: string, input: 'slider' | 'typed' = 'slider'): DimensionGesture | null => owner?.begin(identity, input) ?? null
