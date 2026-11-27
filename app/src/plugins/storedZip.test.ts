import { describe, expect, it } from 'vitest'
import { crc32, storedZip, storedZipSize } from './storedZip'
import { indexStoredZip } from './testKit'

describe('storedZip', () => {
  it('computes the standard CRC-32', () => {
    expect(crc32(new TextEncoder().encode('123456789'))).toBe(0xcbf43926)
    expect(crc32(new Uint8Array())).toBe(0)
  })

  it('writes local headers, a central directory and the end record that agree, sized as said', () => {
    const enc = new TextEncoder()
    const entries = [
      ['hew-plugin.json', enc.encode('{}')],
      ['dist/a.js', enc.encode('// a')],
      ['empty.txt', new Uint8Array()],
    ] as const
    const zip = storedZip(entries)
    expect(zip.length).toBe(storedZipSize(entries.map(([n, d]) => ({ nameBytes: n.length, size: d.length }))))
    const view = new DataView(zip.buffer)
    const files = indexStoredZip(zip)
    expect(files.map((f) => f.path)).toEqual(['hew-plugin.json', 'dist/a.js', 'empty.txt'])
    for (const [i, [name, data]] of entries.entries()) {
      const f = files[i]
      expect(f.size).toBe(data.length)
      expect(zip.subarray(f.offset, f.offset + f.size)).toEqual(data)
      // The local header: signature, stored (method 0), its CRC and sizes.
      const local = f.offset - 30 - name.length
      expect(view.getUint32(local, true)).toBe(0x04034b50)
      expect(view.getUint16(local + 8, true)).toBe(0)
      expect(view.getUint32(local + 14, true)).toBe(crc32(data))
      expect(view.getUint32(local + 18, true)).toBe(data.length)
      expect(view.getUint32(local + 22, true)).toBe(data.length)
    }
    const end = zip.length - 22
    expect(view.getUint32(end, true)).toBe(0x06054b50)
    expect(view.getUint16(end + 10, true)).toBe(3)
  })
})
