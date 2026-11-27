/** Profile item bytes through the real wasm module (F-A7). */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { profile_item_bytes_from_polyline, profile_shape_from_item } from '../wasm/pkg/wasm_api.js'
import { readProfileShape } from './profiles'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  await init({ module_or_path: await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm')) })
})

function bytesOf(pts: Array<[number, number]>): Uint8Array {
  const flat = pts.flatMap(([x, y]) => [x, y, 0])
  const n = pts.length - 1
  return profile_item_bytes_from_polyline(
    Float64Array.from([0, 0, 1]),
    Float64Array.from(flat),
    Float64Array.from(new Array(n * 3).fill(0)),
    Float64Array.from(new Array(n).fill(Number.NaN)),
    JSON.stringify({ id: 'x', name: 'Kink', category: 'profile' }),
  )
}

const DOUBLED_BACK: Array<[number, number]> = [[0.5, 0.5], [0.44, 0.53], [0.4, 0.6], [0.45, 0.52]]

describe('a drawn chain saved as a profile item', () => {
  it('reads back its width when it runs from one side to the other', () => {
    const ok = bytesOf([[0.5, 0.5], [0.44, 0.53], [0.4, 0.6]])
    expect(JSON.parse(profile_shape_from_item(ok)).width).toBeGreaterThan(0)
  })

  it('a chain that turns back down and right is refused as an invalid profile when built', () => {
    // The kernel stops it at the first gate, so a doubled-back drawing never
    // reaches the bytes the Replace guard checks.
    expect(() => bytesOf(DOUBLED_BACK)).toThrow(/^InvalidProfile:/)
  })
})

describe('the replace guard reads the bytes with the real kernel', () => {
  const read = (b: Uint8Array): string => profile_shape_from_item(b)

  it('reads a good profile', () => {
    const ok = bytesOf([[0.5, 0.5], [0.44, 0.53], [0.4, 0.6]])
    expect(readProfileShape(read, ok, true)?.width).toBeGreaterThan(0)
  })

  it('bytes that are not a profile are refused with a code, so strict mode says null', () => {
    const junk = new Uint8Array([1, 2, 3, 4, 5])
    expect(() => profile_shape_from_item(junk)).toThrow(/^[A-Za-z][A-Za-z0-9_]*:\s/)
    expect(readProfileShape(read, junk, true)).toBeNull()
    expect(readProfileShape(read, junk, false)).toBeNull()
  })

  it('a failure that is not a kernel refusal is thrown in strict mode, never taken for a bad drawing', () => {
    const broken = (): string => {
      throw new WebAssembly.RuntimeError('unreachable')
    }
    expect(() => readProfileShape(broken, new Uint8Array(), true)).toThrow(/unreachable/)
    expect(readProfileShape(broken, new Uint8Array(), false)).toBeNull()
  })
})
