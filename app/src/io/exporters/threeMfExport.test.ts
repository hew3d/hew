/**
 * 3MF writer tests.
 *
 * The writer is pure (named colored triangle soups in → OPC container out),
 * so the container layout and model XML are asserted by unzipping the
 * writer's own output with fflate (the same library it zips with — the byte
 * layout of zip itself is not under test here) and string-matching the XML.
 */

import { describe, expect, it } from 'vitest'
import * as THREE from 'three'
import { unzipSync, strFromU8 } from 'three/examples/jsm/libs/fflate.module.js'
import {
  writeThreeMf,
  collectExportParts,
  formatCoord,
  type ThreeMfPart,
  type PartNameScene,
} from './threeMfExport'

/** One unit right triangle in the XY plane at z=0, CCW (normal +Z), meters. */
const TRI_XY = [0, 0, 0, 1, 0, 0, 0, 1, 0]

function unzipModel(bytes: Uint8Array): { entries: string[]; model: string } {
  const files = unzipSync(bytes)
  return {
    entries: Object.keys(files),
    model: strFromU8(files['3D/3dmodel.model']),
  }
}

function part(over: Partial<ThreeMfPart> = {}): ThreeMfPart {
  return {
    name: 'Part',
    triangles: TRI_XY,
    colors: ['#FF0000'],
    ...over,
  }
}

describe('formatCoord', () => {
  it('normalizes -0 and passes plain decimals through', () => {
    expect(formatCoord(-0)).toBe('0')
    expect(formatCoord(0)).toBe('0')
    expect(formatCoord(1.5)).toBe('1.5')
    expect(formatCoord(-1000)).toBe('-1000')
  })

  it('expands scientific notation to plain decimal', () => {
    expect(formatCoord(1e-7)).toBe('0.0000001')
    expect(formatCoord(-2.5e-7)).toBe('-0.00000025')
    expect(formatCoord(1e-7)).not.toContain('e')
  })

  it('expands huge magnitudes too (toFixed alone would stay scientific past 1e21)', () => {
    expect(formatCoord(1e21)).toBe('1000000000000000000000')
    expect(formatCoord(-1e21)).toBe('-1000000000000000000000')
  })
})

describe('writeThreeMf — container', () => {
  it('returns null when nothing contributes a triangle', () => {
    expect(writeThreeMf([])).toBeNull()
    expect(writeThreeMf([part({ triangles: [] })])).toBeNull()
  })

  it('emits the three OPC entries with the 3MF content types and root relationship', () => {
    const result = writeThreeMf([part()])!
    const files = unzipSync(result.bytes)
    expect(Object.keys(files).sort()).toEqual([
      '3D/3dmodel.model',
      '[Content_Types].xml',
      '_rels/.rels',
    ])
    const types = strFromU8(files['[Content_Types].xml'])
    expect(types).toContain('application/vnd.ms-package.3dmanufacturing-3dmodel+xml')
    expect(types).toContain('application/vnd.openxmlformats-package.relationships+xml')
    const rels = strFromU8(files['_rels/.rels'])
    expect(rels).toContain('Target="/3D/3dmodel.model"')
    expect(rels).toContain('http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel')
  })

  it('declares millimeter units and the core namespace, and stamps the app version', () => {
    const { model } = unzipModel(writeThreeMf([part()], '1.2.3')!.bytes)
    expect(model).toContain('<model unit="millimeter"')
    expect(model).toContain('xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"')
    expect(model).toContain('<metadata name="Application">Hew 1.2.3</metadata>')
  })

  it('is byte-for-byte deterministic across identical calls', () => {
    const parts = [part(), part({ name: 'Other', colors: ['#00FF00'] })]
    const a = writeThreeMf(parts, '1.2.3')!
    const b = writeThreeMf(parts, '1.2.3')!
    expect(Buffer.from(a.bytes).equals(Buffer.from(b.bytes))).toBe(true)
  })
})

describe('writeThreeMf — mesh encoding', () => {
  it('scales meters to millimeters and welds shared vertices exactly', () => {
    // Two triangles sharing the edge (1,0,0)-(0,1,0): 4 distinct vertices.
    const quad = [...TRI_XY, 1, 0, 0, 1, 1, 0, 0, 1, 0]
    const result = writeThreeMf([part({ triangles: quad, colors: ['#FF0000', '#FF0000'] })])!
    expect(result.triangleCount).toBe(2)
    const { model } = unzipModel(result.bytes)
    expect(model.match(/<vertex /g)).toHaveLength(4)
    expect(model).toContain('<vertex x="1000" y="0" z="0"/>')
    expect(model).toContain('<vertex x="1000" y="1000" z="0"/>')
    expect(model).toContain('<triangle v1="0" v2="1" v3="2"/>')
    expect(model).toContain('<triangle v1="1" v2="3" v3="2"/>')
  })

  it('welds bit-identical positions only — nearby vertices stay distinct', () => {
    const nearly = [...TRI_XY, 1, 0, 1e-12, 1, 1, 0, 0, 1, 0]
    const result = writeThreeMf([part({ triangles: nearly, colors: ['#FF0000', '#FF0000'] })])!
    const { model } = unzipModel(result.bytes)
    expect(model.match(/<vertex /g)).toHaveLength(5)
  })

  it('skips triangles whose welded indices collapse, and counts them', () => {
    const collapsed = [3, 3, 3, 3, 3, 3, 4, 4, 4]
    const result = writeThreeMf([
      part({ triangles: [...collapsed, ...TRI_XY], colors: ['#0000FF', '#FF0000'] }),
    ])!
    expect(result.triangleCount).toBe(1)
    expect(result.skippedDegenerate).toBe(1)
    // The skipped triangle's color never enters the palette.
    const { model } = unzipModel(result.bytes)
    expect(model).not.toContain('#0000FF')
    expect(model.match(/<base /g)).toHaveLength(1)
  })

  it('dedupes colors into one palette; triangles override only when they differ from the object default', () => {
    const three = [...TRI_XY, ...TRI_XY.map((v) => v + 2), ...TRI_XY.map((v) => v + 4)]
    const result = writeThreeMf([
      part({ triangles: three, colors: ['#FF0000', '#00FF0080', '#FF0000'] }),
    ])!
    const { model } = unzipModel(result.bytes)
    expect(model).toContain('<base name="#FF0000" displaycolor="#FF0000"/>')
    expect(model).toContain('<base name="#00FF0080" displaycolor="#00FF0080"/>')
    expect(model.match(/<base /g)).toHaveLength(2)
    // Object default = first kept triangle's color (palette index 0).
    expect(model).toContain('pid="1" pindex="0"')
    // Only the middle triangle carries an override.
    expect(model.match(/ p1="1"/g)).toHaveLength(1)
    expect(model.match(/ p1="0"/g)).toBeNull()
  })

  it('shares the palette across parts and numbers objects from id 2', () => {
    const result = writeThreeMf([
      part({ name: 'A', colors: ['#FF0000'] }),
      part({ name: 'B', colors: ['#FF0000'] }),
    ])!
    expect(result.objectCount).toBe(2)
    const { model } = unzipModel(result.bytes)
    expect(model.match(/<base /g)).toHaveLength(1)
    expect(model).toContain('<object id="2" type="model" name="A"')
    expect(model).toContain('<object id="3" type="model" name="B"')
    expect(model).toContain('<item objectid="2"/>')
    expect(model).toContain('<item objectid="3"/>')
  })

  it('escapes XML metacharacters in part names', () => {
    const { model } = unzipModel(
      writeThreeMf([part({ name: 'A<B & "C"' })])!.bytes,
    )
    expect(model).toContain('name="A&lt;B &amp; &quot;C&quot;"')
  })
})

describe('collectExportParts', () => {
  const names = (over: Partial<PartNameScene> = {}): PartNameScene => ({
    object_name: () => undefined,
    instance_name: () => undefined,
    ...over,
  })

  /** A one-triangle mesh with per-vertex colors and a vertex-color material. */
  function coloredMesh(linear: [number, number, number]): THREE.Mesh {
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(TRI_XY), 3))
    const cols = new Float32Array([...linear, ...linear, ...linear])
    geo.setAttribute('color', new THREE.BufferAttribute(cols, 3))
    geo.addGroup(0, 3, 0)
    return new THREE.Mesh(geo, [
      new THREE.MeshStandardMaterial({ vertexColors: true }),
    ])
  }

  function buildRoot(...children: THREE.Object3D[]): THREE.Group {
    const root = new THREE.Group()
    for (const c of children) root.add(c)
    root.updateMatrixWorld(true)
    return root
  }

  it('names world objects from the document, with a stable id fallback', () => {
    const named = coloredMesh([1, 0, 0])
    named.userData.hewObjectId = '5'
    const anon = coloredMesh([1, 0, 0])
    anon.userData.hewObjectId = '6'
    const parts = collectExportParts(
      buildRoot(named, anon),
      names({ object_name: (id) => (id === 5n ? 'Roof' : undefined) }),
    )
    expect(parts.map((p) => p.name)).toEqual(['Roof', 'Object 6'])
  })

  it('re-encodes linear vertex colors as sRGB hex', () => {
    const mesh = coloredMesh([0.5, 0.5, 0.5])
    mesh.userData.hewObjectId = '1'
    const [p] = collectExportParts(buildRoot(mesh), names())
    const expected = new THREE.Color()
      .setRGB(0.5, 0.5, 0.5, THREE.LinearSRGBColorSpace)
      .getHexString(THREE.SRGBColorSpace)
      .toUpperCase()
    expect(p.colors).toEqual([`#${expected}`])
  })

  it('uses the material base color (with palette alpha) for non-vertex-color groups', () => {
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(TRI_XY), 3))
    const mat = new THREE.MeshStandardMaterial({ vertexColors: false })
    mat.color.setRGB(1, 0, 0, THREE.SRGBColorSpace)
    mat.opacity = 0.5
    const mesh = new THREE.Mesh(geo, mat)
    mesh.userData.hewObjectId = '1'
    const [p] = collectExportParts(buildRoot(mesh), names())
    expect(p.colors).toEqual(['#FF000080'])
  })

  it('bakes instance poses into world space and names single-member instances after the instance', () => {
    const mesh = coloredMesh([1, 0, 0])
    mesh.userData.hewObjectId = '9'
    const node = new THREE.Group()
    node.userData.hewInstanceId = '7'
    node.position.set(10, 0, 0)
    node.add(mesh)
    const parts = collectExportParts(
      buildRoot(node),
      names({ instance_name: (id) => (id === 7n ? 'Leg' : undefined) }),
    )
    expect(parts).toHaveLength(1)
    expect(parts[0].name).toBe('Leg')
    expect(Array.from(parts[0].triangles).slice(0, 3)).toEqual([10, 0, 0])
  })

  it('gives fully anonymous placements of one component distinct per-instance names', () => {
    // Two placements of the same unnamed member by unnamed instances — the
    // multi-part-print case. Falling back to the shared member id alone
    // would name both identically.
    const nodes = ['7', '8'].map((iid) => {
      const mesh = coloredMesh([1, 0, 0])
      mesh.userData.hewObjectId = '20'
      const node = new THREE.Group()
      node.userData.hewInstanceId = iid
      node.add(mesh)
      return node
    })
    const parts = collectExportParts(buildRoot(...nodes), names())
    expect(parts.map((p) => p.name)).toEqual([
      'Object 20 (Instance 7)',
      'Object 20 (Instance 8)',
    ])
  })

  it('keeps a human-given member name unqualified across placements', () => {
    const nodes = ['7', '8'].map((iid) => {
      const mesh = coloredMesh([1, 0, 0])
      mesh.userData.hewObjectId = '20'
      const node = new THREE.Group()
      node.userData.hewInstanceId = iid
      node.add(mesh)
      return node
    })
    const parts = collectExportParts(
      buildRoot(...nodes),
      names({ object_name: () => 'Bolt' }),
    )
    expect(parts.map((p) => p.name)).toEqual(['Bolt', 'Bolt'])
  })

  it('qualifies multi-member instance parts with their member names', () => {
    const m1 = coloredMesh([1, 0, 0])
    m1.userData.hewObjectId = '20'
    const m2 = coloredMesh([1, 0, 0])
    m2.userData.hewObjectId = '21'
    const node = new THREE.Group()
    node.userData.hewInstanceId = '7'
    node.add(m1, m2)
    const parts = collectExportParts(
      buildRoot(node),
      names({
        instance_name: () => 'Leg',
        object_name: (id) => (id === 20n ? 'Bolt' : 'Cap'),
      }),
    )
    expect(parts.map((p) => p.name)).toEqual(['Leg · Bolt', 'Leg · Cap'])
  })

  it('flips winding under a reflected (negative-determinant) pose so triangles stay outward', () => {
    const mesh = coloredMesh([1, 0, 0])
    mesh.userData.hewObjectId = '1'
    mesh.scale.set(-1, 1, 1)
    const [p] = collectExportParts(buildRoot(mesh), names())
    // TRI_XY mirrored in X: (0,0,0) (−1,0,0) (0,1,0); winding flip swaps the
    // last two vertices so the cross product still points +Z (outward).
    expect(Array.from(p.triangles as number[])).toEqual([0, 0, 0, 0, 1, 0, -1, 0, 0])
  })
})
