import { describe, expect, it, vi } from 'vitest'

vi.mock('../wasm/loader', () => ({ loadKernel: vi.fn().mockResolvedValue(undefined) }))
vi.mock('../wasm/pkg/wasm_api.js', () => ({
  bundled_profiles_json: () => JSON.stringify([
    { id: 'builtin:roman-ogee', name: 'Roman ogee', shape: { b: 1, segs: [{ kind: 'line', to: [0, 1] }] } },
    { id: 'builtin:ogee', name: 'Ogee', shape: { b: 2, segs: [] } },
  ]),
  profile_item_bytes_from_bundled: vi.fn(), profile_item_bytes_from_polyline: vi.fn(),
  profile_shape_from_item: vi.fn(), render_profile_thumbnail: vi.fn(),
}))
import { bundledProfiles, builtInFilletProfile, shapeProfile } from './profiles'

describe('built-in display names', () => {
  it('capitalizes Roman Ogee at the frontend boundary without changing its id or supplied shape', async () => {
    const profiles = await bundledProfiles()
    expect(profiles[0]).toEqual({ id: 'builtin:roman-ogee', name: 'Roman Ogee', shape: { b: 1, segs: [{ kind: 'line', to: [0, 1] }] } })
    expect(profiles[1].name).toBe('Ogee')
    expect(await builtInFilletProfile('builtin:roman-ogee')).toMatchObject({ kind: 'shape', id: 'builtin:roman-ogee', name: 'Roman Ogee', shape: profiles[0].shape })
    expect(shapeProfile('saved:mine', 'Roman ogee', profiles[0].shape)).toMatchObject({ kind: 'shape', name: 'Roman ogee' })
  })
})
