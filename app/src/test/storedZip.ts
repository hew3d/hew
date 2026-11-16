/**
 * Builds a stored (uncompressed) zip, the one shape a `.hewplugin` has, for
 * tests and browser specs. No dependency: a local header, the data and a
 * central directory entry per file, with real CRC-32s.
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

function crc32(data: Uint8Array): number {
  let c = 0xffffffff
  for (const b of data) c = TABLE[(c ^ b) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

export function storedZip(entries: Record<string, string | Uint8Array>): Uint8Array {
  const enc = new TextEncoder()
  const chunks: Uint8Array[] = []
  const central: Uint8Array[] = []
  let offset = 0
  const push = (c: Uint8Array): void => {
    chunks.push(c)
    offset += c.length
  }
  for (const [name, content] of Object.entries(entries)) {
    const data = typeof content === 'string' ? enc.encode(content) : content
    const nameBytes = enc.encode(name)
    const crc = crc32(data)
    const local = new DataView(new ArrayBuffer(30))
    local.setUint32(0, 0x04034b50, true)
    local.setUint16(4, 20, true)
    local.setUint32(14, crc, true)
    local.setUint32(18, data.length, true)
    local.setUint32(22, data.length, true)
    local.setUint16(26, nameBytes.length, true)
    const head = new DataView(new ArrayBuffer(46))
    head.setUint32(0, 0x02014b50, true)
    head.setUint16(4, 20, true)
    head.setUint16(6, 20, true)
    head.setUint32(16, crc, true)
    head.setUint32(20, data.length, true)
    head.setUint32(24, data.length, true)
    head.setUint16(28, nameBytes.length, true)
    head.setUint32(42, offset, true)
    central.push(new Uint8Array(head.buffer), nameBytes)
    push(new Uint8Array(local.buffer))
    push(nameBytes)
    push(data)
  }
  const start = offset
  for (const c of central) push(c)
  const end = new DataView(new ArrayBuffer(22))
  end.setUint32(0, 0x06054b50, true)
  const count = Object.keys(entries).length
  end.setUint16(8, count, true)
  end.setUint16(10, count, true)
  end.setUint32(12, offset - start, true)
  end.setUint32(16, start, true)
  push(new Uint8Array(end.buffer))
  const out = new Uint8Array(offset)
  let at = 0
  for (const c of chunks) {
    out.set(c, at)
    at += c.length
  }
  return out
}
