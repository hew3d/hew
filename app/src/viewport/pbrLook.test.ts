import { describe, expect, it } from 'vitest'
import { phongLook } from './pbrLook'

describe('phongLook', () => {
  const base: [number, number, number] = [0.8, 0.4, 0.1]

  it('keeps a non-metal fully lit and its highlight neutral', () => {
    const look = phongLook(base, 0, 0.5)
    expect(look.diffuseScale).toBe(1)
    expect(look.specular[0]).toBeCloseTo(look.specular[1], 6)
    expect(look.specular[1]).toBeCloseTo(look.specular[2], 6)
  })

  it('takes the diffuse light from a metal and tints its highlight', () => {
    const look = phongLook(base, 1, 0.3)
    expect(look.diffuseScale).toBeLessThan(0.3)
    expect(look.specular[0]).toBeGreaterThan(look.specular[2])
  })

  it('sharpens the highlight as the surface gets smoother', () => {
    expect(phongLook(base, 0, 0).shininess).toBeGreaterThan(phongLook(base, 0, 1).shininess)
    expect(phongLook(base, 0, 0).specular[0]).toBeGreaterThan(phongLook(base, 0, 1).specular[0])
  })

  it('clamps out-of-range factors', () => {
    expect(phongLook(base, 5, -3)).toEqual(phongLook(base, 1, 0))
    expect(phongLook(base, 1, 0).specular.every((c) => c <= 1)).toBe(true)
  })
})
