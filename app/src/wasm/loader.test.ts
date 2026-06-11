import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, it, expect, beforeAll } from 'vitest'
import init, { version, demo_mesh } from './pkg/wasm_api.js'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmPath = resolve(__dirname, 'pkg/wasm_api_bg.wasm')
  const wasmBytes = await readFile(wasmPath)
  // init accepts a BufferSource wrapped in the object form
  await init({ module_or_path: wasmBytes })
})

describe('WASM kernel smoke test', () => {
  it('version() returns a non-empty string', () => {
    const v = version()
    expect(typeof v).toBe('string')
    expect(v.length).toBeGreaterThan(0)
  })

  it('demo_mesh().positions() has 36 elements', () => {
    const mesh = demo_mesh()
    try {
      expect(mesh.positions().length).toBe(36)
    } finally {
      mesh.free()
    }
  })
})
