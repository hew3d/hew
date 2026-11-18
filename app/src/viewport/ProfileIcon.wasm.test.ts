import { readFile } from 'node:fs/promises'
import { beforeAll, expect, it } from 'vitest'
import init, { bundled_profiles_json } from '../wasm/pkg/wasm_api.js'
import { profileOutline } from './ProfileIcon'
import type { ProfileShapeJson } from '../tools/filletMath'

beforeAll(async () => {
  await init({ module_or_path: await readFile(new URL('../wasm/pkg/wasm_api_bg.wasm', import.meta.url)) })
})

it('renders each authoritative built-in profile directly from its line and arc definitions', () => {
  const profiles = JSON.parse(bundled_profiles_json()) as Array<{ name: string; shape: ProfileShapeJson }>
  expect(profiles.map(p => p.name)).toEqual(['Ogee', 'Roman ogee', 'Bead', 'Thumbnail', 'Cove and bead', 'Chamfer and bead', 'Cavetto'])
  const paths = profiles.map(({ shape }) => {
    const original = JSON.stringify(shape)
    const icon = profileOutline(shape)
    expect(icon).not.toBeNull()
    expect(icon!.path).not.toMatch(/NaN|Infinity/)
    for (const seg of shape.segs) expect(icon!.path).toContain(`${seg.to[0]} ${seg.to[1]}`)
    expect(JSON.stringify(shape)).toBe(original)
    return icon!.path
  })
  expect(new Set(paths).size).toBe(profiles.length)
})

it('retains major arc and sweep identity, and does not guess missing or malformed outlines', () => {
  const ccw = profileOutline({ b: 1, segs: [{ kind: 'arc', to: [0, -1], center: [0, 0], ccw: true }] })!
  const cw = profileOutline({ b: 1, segs: [{ kind: 'arc', to: [0, -1], center: [0, 0], ccw: false }] })!
  expect(ccw.path).toContain('A1 1 0 1 1')
  expect(cw.path).toContain('A1 1 0 0 0')
  for (const shape of [null, {}, { b: 1, segs: [] }, { b: 1, segs: [{ kind: 'spline', to: [0, 1] }] }]) expect(profileOutline(shape)).toBeNull()
})
