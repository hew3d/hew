import { afterEach, describe, expect, it, vi } from 'vitest'
import { pumpEvents, registerBroadcastSink, registerEventSink, setPumpDocumentId } from './eventPump'
import type { Scene } from '../wasm/loader'

function sceneDraining(json: string): { scene: Scene; drain: ReturnType<typeof vi.fn> } {
  const drain = vi.fn(() => json)
  return { scene: { drain_events: drain } as unknown as Scene, drain }
}

describe('eventPump', () => {
  const removers: Array<() => void> = []
  afterEach(() => {
    for (const remove of removers.splice(0)) remove()
  })

  it('drains nothing while no sink is registered', () => {
    const { scene, drain } = sceneDraining('[]')
    pumpEvents(scene)
    expect(drain).not.toHaveBeenCalled()
  })

  it('routes every frame, in order, to every sink with its connection', () => {
    const a = vi.fn()
    const b = vi.fn()
    removers.push(registerEventSink(a), registerEventSink(b))
    const { scene, drain } = sceneDraining(
      JSON.stringify([
        { conn: 2, frames: ['f1', 'f2'] },
        { conn: 5, frames: ['f3'] },
      ]),
    )
    pumpEvents(scene)
    expect(drain).toHaveBeenCalledTimes(1)
    expect(a.mock.calls).toEqual([
      [2, 'f1'],
      [2, 'f2'],
      [5, 'f3'],
    ])
    expect(b.mock.calls).toEqual(a.mock.calls)
  })

  it('stops routing to a removed sink', () => {
    const sink = vi.fn()
    const remove = registerEventSink(sink)
    remove()
    removers.push(registerEventSink(() => {}))
    pumpEvents(sceneDraining(JSON.stringify([{ conn: 1, frames: ['f'] }])).scene)
    expect(sink).not.toHaveBeenCalled()
  })

  it('logs and drops a drain that does not parse', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    const sink = vi.fn()
    removers.push(registerEventSink(sink))
    pumpEvents(sceneDraining('not json').scene)
    expect(sink).not.toHaveBeenCalled()
    expect(error).toHaveBeenCalled()
    error.mockRestore()
  })

  it('offers each lifecycle frame once, naming the connections that already got it', () => {
    const sink = vi.fn()
    const perConn = vi.fn()
    removers.push(registerBroadcastSink(sink), registerEventSink(perConn))
    pumpEvents(
      sceneDraining(
        JSON.stringify([
          { conn: 2, frames: ['opened', 'other'] },
          { conn: 5, frames: ['other'] },
          { conn: null, frames: ['opened'] },
        ]),
      ).scene,
    )
    expect(perConn).toHaveBeenCalledTimes(3)
    expect(sink).toHaveBeenCalledTimes(1)
    expect(sink.mock.calls[0][0]).toBe('opened')
    expect([...sink.mock.calls[0][1]]).toEqual([2])
  })

  it('drains for a broadcast sink alone, and an owed frame with no session still goes out', () => {
    const sink = vi.fn()
    removers.push(registerBroadcastSink(sink))
    const { scene, drain } = sceneDraining(JSON.stringify([{ conn: null, frames: ['closed'] }]))
    pumpEvents(scene)
    expect(drain).toHaveBeenCalledTimes(1)
    expect(sink).toHaveBeenCalledWith('closed', new Set())
  })

  it('tells the Scene the window document id before each drain', () => {
    removers.push(registerEventSink(() => {}))
    const order: string[] = []
    const scene = {
      set_host_document_id: vi.fn((id: string) => order.push(`id ${id}`)),
      drain_events: vi.fn(() => {
        order.push('drain')
        return '[]'
      }),
    } as unknown as Scene
    setPumpDocumentId('doc_9')
    pumpEvents(scene)
    expect(order).toEqual(['id doc_9', 'drain'])
  })
})
