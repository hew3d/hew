/**
 * Shop Mode's plugin host (a Preview feature), started only while the plugin
 * loader is enabled. Shop never names plugin code: the host is reached only
 * through `preview/loaderRoots.ts` (`loadPluginHost`, `null` in a production
 * build), so a production Shop bundle holds none of it, and this file has no
 * words a person reads: whatever plugins show in Shop comes from the host's
 * own preview-only code.
 *
 * Shop loads each document into the SAME Scene in place, and nothing else
 * pumps the Scene's events here, so the document lifecycle the host follows
 * (`document_opened`, `document_closed`) is noted and pumped from this hook:
 * `noteOpened` after every successful open (which first closes the document
 * it replaces), and a close when the page goes away for good (not into the
 * back/forward cache). Both are no-ops while no host is running, so a build or a window
 * without plugins makes no extra kernel calls.
 */

import { useCallback, useEffect, useRef } from 'react'
import { pumpEvents } from '../api/eventPump'
import type { LiveBridgeDeps } from '../api/liveBridge'
import * as LogStore from '../log/LogStore'
import { loaderEnabled } from '../preview'
import { loadPluginHost } from '../preview/loaderRoots'
import type { Scene } from '../wasm/loader'

export interface ShopPluginHostOptions {
  scene: Scene | null
  /** Shop's own small bridge: scene, viewport, reconcile and the hidden-state
   *  hooks. Selection, isolate and entering are absent on purpose: those
   *  directives are no-ops in Shop. */
  bridge: LiveBridgeDeps
  toast: (message: string) => void
  /** The title of the open document, for the lifecycle frames. */
  title: () => string
  /** The window's test harness object (`__hew_shop_test`), when it exists. */
  harness: () => Record<string, unknown> | null
}

export interface ShopPluginHost {
  /** Tells a running host a document was just opened into the Scene. */
  noteOpened(scene: Scene, name: string): void
}

export function useShopPluginHost(opts: ShopPluginHostOptions): ShopPluginHost {
  const running = useRef(false)
  // The name of the document the host was last told is open, or null when
  // none is (so a close is sent for it once, and only for one that is open).
  const openName = useRef<string | null>(null)
  const optsRef = useRef(opts)
  optsRef.current = opts
  const { scene } = opts

  useEffect(() => {
    if (scene === null || loadPluginHost === null || !loaderEnabled('plugins')) return
    let stop: (() => void) | null = null
    let cancelled = false
    loadPluginHost()
      .then((start) => {
        if (cancelled) return
        const o = optsRef.current
        stop = start({ bridge: o.bridge, toast: o.toast, platform: 'shop', harness: o.harness() })
        running.current = true
      })
      .catch((err: unknown) => LogStore.log.error('plugins', `The plugin host did not start: ${String(err)}`))
    const onPageHide = (event: PageTransitionEvent): void => {
      // A page kept for the back/forward cache comes back as it was: nothing
      // closed. Only a page that is really going away closes its document.
      if (!running.current || event.persisted) return
      // Only a document the host was told is open closes, and only once.
      if (openName.current === null) return
      openName.current = null
      scene.note_document_event('closed', optsRef.current.title(), undefined, undefined)
      pumpEvents(scene)
    }
    window.addEventListener('pagehide', onPageHide)
    return () => {
      cancelled = true
      window.removeEventListener('pagehide', onPageHide)
      running.current = false
      stop?.()
    }
  }, [scene])

  const noteOpened = useCallback((liveScene: Scene, name: string): void => {
    if (!running.current) return
    // A second model opened in place: the first closes before the second opens.
    if (openName.current !== null) {
      liveScene.note_document_event('closed', openName.current, undefined, undefined)
      pumpEvents(liveScene)
    }
    openName.current = name
    liveScene.note_document_event('opened', name, undefined, name.toLowerCase().endsWith('.skp') ? 'skp' : 'hew')
    pumpEvents(liveScene)
  }, [])
  return { noteOpened }
}
