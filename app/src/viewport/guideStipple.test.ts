import { describe, expect, it } from 'vitest'
import * as THREE from 'three'
import { applyGuideStipple, GUIDE_DEFAULT_STIPPLE, stippleOn, stippleShaders } from './guideStipple'

describe('guideStipple', () => {
  it('the default mask is the short dashes Hew has always drawn', () => {
    expect(GUIDE_DEFAULT_STIPPLE).toBe(0x3333)
  })

  it('reads a mask least-significant bit first, one bit per unit', () => {
    // 0x3333: two on, two off, repeating.
    const seen = Array.from({ length: 8 }, (_, i) => stippleOn(0x3333, i + 0.5, 1))
    expect(seen).toEqual([true, true, false, false, true, true, false, false])
    // Solid never breaks; dotted is one unit in four.
    expect(Array.from({ length: 16 }, (_, i) => stippleOn(0xffff, i + 0.5, 1)).every(Boolean)).toBe(true)
    expect(Array.from({ length: 8 }, (_, i) => stippleOn(0x8888, i + 0.5, 1))).toEqual([
      false, false, false, true, false, false, false, true,
    ])
    // Long dashes: four on, four off. The pattern repeats every 16 units,
    // and runs the same way on the negative side of the origin.
    expect(stippleOn(0xf0f0, 4.5, 1)).toBe(true)
    expect(stippleOn(0xf0f0, 0.5, 1)).toBe(false)
    expect(stippleOn(0xf0f0, 16.5, 1)).toBe(false)
    expect(stippleOn(0xf0f0, -0.5, 1)).toBe(stippleOn(0xf0f0, 15.5, 1))
  })

  it('rewrites the dashed-line shaders to test a per-vertex mask', () => {
    // The texts three's compile hook receives, shaped as three ships them.
    const vertex = 'void main() {\n\tvLineDistance = scale * lineDistance;\n}'
    const fragment = 'void main() {\n\tif ( mod( vLineDistance, totalSize ) > dashSize ) {\n\n\t\tdiscard;\n\n\t}\n}'
    const out = stippleShaders(vertex, fragment)
    expect(out.vertex).toContain('attribute float stipple;')
    expect(out.vertex).toContain('vStipple = stipple;')
    expect(out.fragment).toContain('varying float vStipple;')
    expect(out.fragment).not.toContain('> dashSize')
    expect(out.fragment).toContain('stippleMask')
  })

  it('installs the compile hook on the material', () => {
    const mat = new THREE.LineDashedMaterial()
    applyGuideStipple(mat)
    const shader = {
      vertexShader: 'void main() {\n}',
      fragmentShader: 'void main() {\n\tif ( mod( vLineDistance, totalSize ) > dashSize ) {\n\t\tdiscard;\n\t}\n}',
    }
    mat.onBeforeCompile(shader as unknown as THREE.WebGLProgramParametersWithUniforms, undefined as never)
    expect(shader.vertexShader).toContain('vStipple')
    expect(shader.fragmentShader).toContain('stippleMask')
  })
})
