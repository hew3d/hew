import { useSyncExternalStore } from 'react'
import type { InferenceInfo } from './Viewport'

/**
 * Live inference-cursor info (snap dot + tooltip chip), held outside React
 * state. The info carries the pointer's screen position, so it changes on
 * every pointer move; as `useState` in `App` that re-rendered the whole app
 * (menu, tool rail, document tree, view die) per move and made hovering a
 * large model sluggish. Only the two small overlays that draw it, and the
 * hover line that needs the hovered object's identity, subscribe.
 */
export interface InferenceStore {
  get(): InferenceInfo | null
  set(info: InferenceInfo | null): void
  subscribe(listener: () => void): () => void
}

export function createInferenceStore(): InferenceStore {
  let current: InferenceInfo | null = null
  const listeners = new Set<() => void>()
  return {
    get: () => current,
    set(info) {
      if (info === current) return
      current = info
      for (const l of [...listeners]) l()
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }
}

export function useInferenceInfo(store: InferenceStore): InferenceInfo | null {
  return useSyncExternalStore(store.subscribe, store.get)
}
