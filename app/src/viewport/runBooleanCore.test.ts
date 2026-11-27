/**
 * `runBooleanCore` against a mock scene: the N-way call shape and the
 * auto-explode retry when one of several operands is a component instance.
 * (The real-wasm accounting tests live in `booleanAutoExplode.wasm.test.ts`.)
 */

import { describe, expect, it, vi } from 'vitest'
import { runBooleanCore } from './Viewport'
import type { Scene as WasmScene } from '../wasm/loader'

const SUBTRACT = 1

function mockScene(overrides: Record<string, unknown>): WasmScene {
  return overrides as unknown as WasmScene
}

describe('runBooleanCore with more than two operands', () => {
  it('two operands still use the two-operand boolean_nodes call', () => {
    const boolean_nodes = vi.fn(() => ({ kind: 'object', id: 50n }))
    const boolean_nodes_many = vi.fn()
    const outcome = runBooleanCore(mockScene({ boolean_nodes, boolean_nodes_many }), SUBTRACT, [
      { kind: 'object', id: 1n },
      { kind: 'group', id: 2n },
    ])
    expect(outcome).toEqual({ kind: 'ok', node: { kind: 'object', id: 50n }, autoExploded: false })
    expect(boolean_nodes).toHaveBeenCalledWith(SUBTRACT, 0, 1n, 1, 2n)
    expect(boolean_nodes_many).not.toHaveBeenCalled()
  })

  it('three operands call boolean_nodes_many once, kinds and ids in selection order', () => {
    const boolean_nodes = vi.fn()
    const boolean_nodes_many = vi.fn(() => ({ kind: 'group', id: 77n }))
    const outcome = runBooleanCore(mockScene({ boolean_nodes, boolean_nodes_many }), SUBTRACT, [
      { kind: 'object', id: 3n },
      { kind: 'group', id: 1n },
      { kind: 'object', id: 2n },
    ])
    expect(outcome).toEqual({ kind: 'ok', node: { kind: 'group', id: 77n }, autoExploded: false })
    expect(boolean_nodes).not.toHaveBeenCalled()
    expect(boolean_nodes_many).toHaveBeenCalledTimes(1)
    const [op, kinds, ids] = boolean_nodes_many.mock.calls[0] as unknown as [number, Uint8Array, BigUint64Array]
    expect(op).toBe(SUBTRACT)
    expect(Array.from(kinds)).toEqual([0, 1, 0])
    expect(Array.from(ids)).toEqual([3n, 1n, 2n])
  })

  it('the second of three operands is an instance: it auto-explodes and the retry puts the exploded node in its place', () => {
    const refusal = new Error('BooleanOperandHasInstance: operand is an instance')
    const boolean_nodes_many = vi
      .fn()
      .mockImplementationOnce(() => {
        throw refusal
      })
      .mockImplementationOnce(() => ({ kind: 'object', id: 90n }))
    const make_unique = vi.fn()
    const explode_instance = vi.fn(() => BigUint64Array.from([40n]))
    const outcome = runBooleanCore(
      mockScene({ boolean_nodes: vi.fn(), boolean_nodes_many, make_unique, explode_instance }),
      SUBTRACT,
      [
        { kind: 'object', id: 1n },
        { kind: 'instance', id: 7n },
        { kind: 'object', id: 3n },
      ],
    )
    expect(outcome).toEqual({ kind: 'ok', node: { kind: 'object', id: 90n }, autoExploded: true })
    expect(make_unique).toHaveBeenCalledWith(7n)
    expect(explode_instance).toHaveBeenCalledWith(7n)
    expect(boolean_nodes_many).toHaveBeenCalledTimes(2)
    const [, kinds1, ids1] = boolean_nodes_many.mock.calls[0] as unknown as [number, Uint8Array, BigUint64Array]
    expect(Array.from(kinds1)).toEqual([0, 2, 0])
    expect(Array.from(ids1)).toEqual([1n, 7n, 3n])
    const [, kinds2, ids2] = boolean_nodes_many.mock.calls[1] as unknown as [number, Uint8Array, BigUint64Array]
    expect(Array.from(kinds2)).toEqual([0, 0, 0])
    expect(Array.from(ids2)).toEqual([1n, 40n, 3n])
  })
})
