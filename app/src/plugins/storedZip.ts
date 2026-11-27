/* oxlint-disable lingui/no-unlocalized-strings -- an error for a programming mistake; never shown to a person. */
/**
 * A stored (uncompressed) zip, the one shape a `.hewplugin` has, built in
 * memory: a local header, the data and a central directory entry per file,
 * with real CRC-32s, then the end record. No dependency. It only writes:
 * what it builds is read back by the kernel's own package reader, which
 * judges it as it judges any package.
 */

const TABLE = (() => {
  const t = new Uint32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    t[n] = c >>> 0
  }
  return t
})()

export function crc32(data: Uint8Array): number {
  let c = 0xffffffff
  for (const b of data) c = TABLE[(c ^ b) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

const LOCAL_HEADER = 30
const CENTRAL_HEADER = 46
const END_RECORD = 22

/** The size of the zip `storedZip` would build for files of these name
 *  lengths (in bytes) and sizes, without building it. */
export function storedZipSize(entries: Iterable<{ nameBytes: number; size: number }>): number {
  let total = END_RECORD
  for (const e of entries) total += LOCAL_HEADER + CENTRAL_HEADER + 2 * e.nameBytes + e.size
  return total
}

/** A stored zip of `entries`, in the order given. A name ending in `/` is a
 *  folder entry and should hold no bytes. More than 65,535 entries, or a
 *  zip past 4 GiB, does not fit this format and throws. */
export function storedZip(entries: Iterable<readonly [string, Uint8Array]>): Uint8Array {
  const enc = new TextEncoder()
  const list = [...entries].map(([name, data]) => ({ name: enc.encode(name), data }))
  const size = storedZipSize(list.map((e) => ({ nameBytes: e.name.length, size: e.data.length })))
  if (list.length > 0xffff || size > 0xffffffff) throw new RangeError('too many or too large entries for a plain zip')
  const out = new Uint8Array(size)
  const view = new DataView(out.buffer)
  let at = 0
  const offsets: number[] = []
  const crcs: number[] = []
  for (const e of list) {
    const crc = crc32(e.data)
    offsets.push(at)
    crcs.push(crc)
    view.setUint32(at, 0x04034b50, true)
    view.setUint16(at + 4, 20, true)
    view.setUint32(at + 14, crc, true)
    view.setUint32(at + 18, e.data.length, true)
    view.setUint32(at + 22, e.data.length, true)
    view.setUint16(at + 26, e.name.length, true)
    out.set(e.name, at + LOCAL_HEADER)
    out.set(e.data, at + LOCAL_HEADER + e.name.length)
    at += LOCAL_HEADER + e.name.length + e.data.length
  }
  const start = at
  list.forEach((e, i) => {
    view.setUint32(at, 0x02014b50, true)
    view.setUint16(at + 4, 20, true)
    view.setUint16(at + 6, 20, true)
    view.setUint32(at + 16, crcs[i], true)
    view.setUint32(at + 20, e.data.length, true)
    view.setUint32(at + 24, e.data.length, true)
    view.setUint16(at + 28, e.name.length, true)
    view.setUint32(at + 42, offsets[i], true)
    out.set(e.name, at + CENTRAL_HEADER)
    at += CENTRAL_HEADER + e.name.length
  })
  view.setUint32(at, 0x06054b50, true)
  view.setUint16(at + 8, list.length, true)
  view.setUint16(at + 10, list.length, true)
  view.setUint32(at + 12, at - start, true)
  view.setUint32(at + 16, start, true)
  return out
}

/** `storedZip` of a record of names to contents (text is UTF-8), for
 *  building a package by hand. */
export function storedZipOf(entries: Record<string, string | Uint8Array>): Uint8Array {
  const enc = new TextEncoder()
  return storedZip(Object.entries(entries).map(([name, content]) => [name, typeof content === 'string' ? enc.encode(content) : content] as const))
}
