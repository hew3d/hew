import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import { InfiniteGrid } from './InfiniteGrid'

describe('InfiniteGrid', () => {
  it('constructs a mesh with the given ground/minor/major colors as uniforms', () => {
    const grid = new InfiniteGrid(0x0c0e11, 0x282c32, 0x454b54)
    const material = grid.mesh.material as THREE.ShaderMaterial
    expect((material.uniforms.uGroundColor.value as THREE.Color).getHex()).toBe(0x0c0e11)
    expect((material.uniforms.uMinorColor.value as THREE.Color).getHex()).toBe(0x282c32)
    expect((material.uniforms.uMajorColor.value as THREE.Color).getHex()).toBe(0x454b54)
  })

  it('update() copies the camera position into the uCameraPos uniform', () => {
    const grid = new InfiniteGrid(0x0c0e11, 0x282c32, 0x454b54)
    const material = grid.mesh.material as THREE.ShaderMaterial
    grid.update(new THREE.Vector3(1, 2, 3))
    const uCameraPos = material.uniforms.uCameraPos.value as THREE.Vector3
    expect(uCameraPos.x).toBe(1)
    expect(uCameraPos.y).toBe(2)
    expect(uCameraPos.z).toBe(3)
  })

  it('setColors() updates the uniforms in place without rebuilding the mesh', () => {
    const grid = new InfiniteGrid(0x0c0e11, 0x282c32, 0x454b54)
    const mesh = grid.mesh
    grid.setColors(0xd7dee6, 0xd8dee5, 0xb0b8c2)
    const material = grid.mesh.material as THREE.ShaderMaterial
    expect(grid.mesh).toBe(mesh) // same mesh instance — no rebuild
    expect((material.uniforms.uGroundColor.value as THREE.Color).getHex()).toBe(0xd7dee6)
    expect((material.uniforms.uMinorColor.value as THREE.Color).getHex()).toBe(0xd8dee5)
    expect((material.uniforms.uMajorColor.value as THREE.Color).getHex()).toBe(0xb0b8c2)
  })

  it('the mesh geometry is a large plane (comfortably beyond the camera far-clip of 100)', () => {
    const grid = new InfiniteGrid(0x0c0e11, 0x282c32, 0x454b54)
    const geo = grid.mesh.geometry as THREE.PlaneGeometry
    expect(geo.parameters.width).toBeGreaterThan(200)
    expect(geo.parameters.height).toBeGreaterThan(200)
  })

  it('dispose() does not throw', () => {
    const grid = new InfiniteGrid(0x0c0e11, 0x282c32, 0x454b54)
    expect(() => grid.dispose()).not.toThrow()
  })

  it('defaults uAxesVisible to 1 (axes visible by default, matching originAxes.visible in Viewport.tsx)', () => {
    const grid = new InfiniteGrid(0x0c0e11, 0x282c32, 0x454b54)
    const material = grid.mesh.material as THREE.ShaderMaterial
    expect(material.uniforms.uAxesVisible.value).toBe(1)
  })

  it('an explicit initial axesVisible argument seeds uAxesVisible', () => {
    const grid = new InfiniteGrid(0x0c0e11, 0x282c32, 0x454b54, false)
    const material = grid.mesh.material as THREE.ShaderMaterial
    expect(material.uniforms.uAxesVisible.value).toBe(0)
  })

  it('setAxesVisible() flips the uAxesVisible uniform without rebuilding the mesh', () => {
    const grid = new InfiniteGrid(0x0c0e11, 0x282c32, 0x454b54)
    const mesh = grid.mesh
    grid.setAxesVisible(false)
    const material = grid.mesh.material as THREE.ShaderMaterial
    expect(grid.mesh).toBe(mesh) // same mesh instance — no rebuild
    expect(material.uniforms.uAxesVisible.value).toBe(0)
    grid.setAxesVisible(true)
    expect(material.uniforms.uAxesVisible.value).toBe(1)
  })

  it('the fragment shader suppresses through-origin lines only when uAxesVisible is set (contains the origin-line mask wired behind the uniform)', () => {
    const grid = new InfiniteGrid(0x0c0e11, 0x282c32, 0x454b54)
    const material = grid.mesh.material as THREE.ShaderMaterial
    // No WebGL context in vitest to render pixels, so this pins the shader
    // SOURCE contract instead: the uniform exists, the suppression helper is
    // present, and it's gated by the uniform (not applied unconditionally,
    // which would suppress the origin lines even with axes hidden).
    expect(material.fragmentShader).toContain('uniform float uAxesVisible')
    expect(material.fragmentShader).toContain('originLineFactor')
    expect(material.fragmentShader).toMatch(/if\s*\(\s*uAxesVisible\s*>\s*0\.5\s*\)/)
  })
})
