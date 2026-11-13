/**
 * Server-originated events (docs/agents/HEW_API.md §4.5): the one place
 * the app drains `Scene.drain_events` and hands each frame to whoever
 * carries it to its connection. The live bridge registers a sink that
 * sends a socket connection's frames to the shell (`hew://api-event`);
 * an in-page plugin host registers its own later.
 *
 * The app pumps after everything that can owe an event: every dispatch and
 * forward completion (the bridge), every committed or undone change
 * (`handleDocumentChanged`), a Scene edit, a units change, and each
 * document lifecycle step. With no sink registered nothing is drained:
 * nobody is listening, and the kernel's change ring keeps only so much.
 */

import type { Scene } from '../wasm/loader'

/** Receives one notification frame owed to the wasm connection `wasmConnId`. */
export type EventSink = (wasmConnId: number, frame: string) => void

const sinks = new Set<EventSink>()

/**
 * Receives a document lifecycle frame (`document_opened`, `_closed`,
 * `_saved`, `_will_save`) once, with the wasm connections that were
 * already sent it through their own sessions in this window. The shell
 * offers it to every other first-party connection that subscribed, so a
 * listener bound to one document hears of the others.
 */
export type BroadcastSink = (frame: string, delivered: ReadonlySet<number>) => void

const broadcastSinks = new Set<BroadcastSink>()

/** Adds a lifecycle broadcast sink; returns the function that removes it. */
export function registerBroadcastSink(sink: BroadcastSink): () => void {
  broadcastSinks.add(sink)
  return () => {
    broadcastSinks.delete(sink)
  }
}

/** Adds `sink`; returns the function that removes it. */
export function registerEventSink(sink: EventSink): () => void {
  sinks.add(sink)
  return () => {
    sinks.delete(sink)
  }
}

let hostDocumentId: string | null = null

/**
 * The id the shell gave this window's document. Told to the Scene before
 * every drain, so an event the app itself owes (a document just opened)
 * names its document even when the id reached the page before the Scene.
 */
export function setPumpDocumentId(id: string): void {
  hostDocumentId = id
}

interface Batch {
  /** The wasm connection, or `null` for the lifecycle frames offered to the host. */
  conn: number | null
  frames: string[]
}

/** Drains every frame `scene` owes and routes each to every sink, in order. */
export function pumpEvents(scene: Scene): void {
  if (sinks.size === 0 && broadcastSinks.size === 0) return
  let batches: Batch[]
  try {
    if (hostDocumentId !== null) scene.set_host_document_id(hostDocumentId)
    batches = JSON.parse(scene.drain_events()) as Batch[]
  } catch (err) {
    console.error('Scene.drain_events failed', err)
    return
  }
  for (const batch of batches) {
    if (batch.conn === null) continue
    const conn = batch.conn
    for (const frame of batch.frames) {
      for (const sink of [...sinks]) sink(conn, frame)
    }
  }
  for (const batch of batches) {
    if (batch.conn !== null) continue
    for (const frame of batch.frames) {
      const delivered = new Set<number>()
      for (const other of batches) {
        if (other.conn !== null && other.frames.includes(frame)) delivered.add(other.conn)
      }
      for (const sink of [...broadcastSinks]) sink(frame, delivered)
    }
  }
}
