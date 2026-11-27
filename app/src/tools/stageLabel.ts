/**
 * stageLabel — a tool's name for the value in the measurement box when it
 * changes with the tool's stage (the Arc tool's "Bulge" once both chord ends
 * are down). A tiny external store: the tool writes, the app reads, and
 * `MeasurementBox` falls back to its per-tool table while this is null.
 */

let current: string | null = null
const listeners = new Set<() => void>()

export function getToolStageLabel(): string | null {
  return current
}

export function setToolStageLabel(label: string | null): void {
  if (label === current) return
  current = label
  for (const l of [...listeners]) l()
}

export function subscribeToolStageLabel(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}
