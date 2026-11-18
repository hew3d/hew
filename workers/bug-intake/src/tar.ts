/**
 * A streaming, uncompressed `ustar` writer for the admin list's batch
 * download. A tar entry is a 512-byte header (a checksum over the header
 * only), the entry's bytes exactly as given, and zero padding up to the
 * next 512; the archive ends with two zero blocks. No per-byte work is done
 * on the data, which is why this is a tar and not a zip: a zip needs a
 * CRC-32 over every byte, and a 60 MB bundle would blow the CPU budget of
 * the plan this Worker runs on. Each bundle is already gzip, so storing it
 * as-is loses nothing.
 *
 * The archive is produced lazily by one pull-based `ReadableStream`: an
 * entry's source is opened only when the previous one has been fully
 * written, so nothing is ever buffered beyond one chunk. A source that
 * delivers a different number of bytes than it declared ERRORS the stream
 * (a truncated download the reader notices) rather than ending in an
 * archive whose later headers are misaligned.
 */

const BLOCK = 512

/** An entry name this writer will accept: a plain file name in the archive
 *  root. No slash, no backslash, no `..`, no leading dot, ASCII only, so an
 *  extractor can never be steered outside its target directory. */
const SAFE_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$/

export function isSafeTarName(name: string): boolean {
  return SAFE_NAME.test(name) && !name.includes('..')
}

function writeAscii(block: Uint8Array, offset: number, text: string): void {
  for (let i = 0; i < text.length; i++) block[offset + i] = text.charCodeAt(i)
}

/** Writes `value` as zero-padded octal in `width - 1` digits, then a NUL. */
function writeOctal(block: Uint8Array, offset: number, width: number, value: number): void {
  const digits = value.toString(8).padStart(width - 1, '0')
  if (digits.length > width - 1) throw new RangeError(`tar: ${value} does not fit a ${width}-byte field`)
  writeAscii(block, offset, digits)
  block[offset + width - 1] = 0
}

/** The 512-byte header for a regular file `name` of `size` bytes. */
export function tarHeader(name: string, size: number, mtimeSeconds: number): Uint8Array {
  if (!isSafeTarName(name)) throw new RangeError(`tar: unsafe entry name ${JSON.stringify(name)}`)
  if (!Number.isInteger(size) || size < 0) throw new RangeError(`tar: bad size ${size}`)
  const h = new Uint8Array(BLOCK)
  writeAscii(h, 0, name) // name, 100 bytes
  writeAscii(h, 100, '0000644') // mode
  writeAscii(h, 108, '0000000') // uid
  writeAscii(h, 116, '0000000') // gid
  writeOctal(h, 124, 12, size) // size
  writeOctal(h, 136, 12, Math.max(0, Math.floor(mtimeSeconds))) // mtime
  h.fill(0x20, 148, 156) // the checksum field counts as spaces while summing
  h[156] = 0x30 // typeflag '0': a regular file
  writeAscii(h, 257, 'ustar') // magic "ustar\0"
  writeAscii(h, 263, '00') // version
  let sum = 0
  for (const b of h) sum += b
  writeAscii(h, 148, sum.toString(8).padStart(6, '0'))
  h[154] = 0
  h[155] = 0x20
  return h
}

/** Zero padding that brings `size` up to a multiple of 512. */
export function tarPadding(size: number): Uint8Array {
  return new Uint8Array((BLOCK - (size % BLOCK)) % BLOCK)
}

/** What one archive entry turns out to be when its turn comes. */
export type TarOpened = { size: number; body: ReadableStream<Uint8Array> } | { skipped: string }

export interface TarSource {
  name: string
  /** What `SKIPPED.txt` calls this entry when it is skipped (default: `name`). */
  label?: string
  mtimeSeconds: number
  /** Called once, when the previous entry is complete. */
  open(): Promise<TarOpened>
}

/** One entry the caller already knows is skipped (its reason goes into the
 *  trailing `SKIPPED.txt`). */
export interface TarSkip {
  label: string
  reason: string
}

export const SKIPPED_NAME = 'SKIPPED.txt'

/** The whole archive as a stream: every source in order, then `SKIPPED.txt`
 *  when anything was skipped (by `skipped` up front or by a source's own
 *  `open`), then two zero blocks. */
export function tarStream(sources: TarSource[], skipped: TarSkip[], nowSeconds: number): ReadableStream<Uint8Array> {
  const notes = [...skipped]
  let next = 0
  let reader: ReadableStreamDefaultReader<Uint8Array> | null = null
  let remaining = 0
  let currentSize = 0
  let finished = false

  function skippedText(): Uint8Array {
    const lines = notes.map((n) => `${n.label}: ${n.reason}`)
    return new TextEncoder().encode(`Not in this archive:\n${lines.join('\n')}\n`)
  }

  return new ReadableStream<Uint8Array>({
    async pull(controller) {
      try {
        while (true) {
          if (reader !== null) {
            const { done, value } = await reader.read()
            if (done) {
              if (remaining !== 0) throw new Error(`tar: a source ended ${remaining} bytes short`)
              reader = null
              const pad = tarPadding(currentSize)
              if (pad.length > 0) controller.enqueue(pad)
              continue
            }
            if (value.byteLength > remaining) throw new Error('tar: a source delivered more than it declared')
            remaining -= value.byteLength
            controller.enqueue(value)
            return
          }
          if (next < sources.length) {
            const source = sources[next++]
            const opened = await source.open()
            if ('skipped' in opened) {
              notes.push({ label: source.label ?? source.name, reason: opened.skipped })
              continue
            }
            currentSize = opened.size
            remaining = opened.size
            controller.enqueue(tarHeader(source.name, opened.size, source.mtimeSeconds))
            reader = opened.body.getReader()
            continue
          }
          if (!finished) {
            finished = true
            if (notes.length > 0) {
              const text = skippedText()
              controller.enqueue(tarHeader(SKIPPED_NAME, text.byteLength, nowSeconds))
              controller.enqueue(text)
              const pad = tarPadding(text.byteLength)
              if (pad.length > 0) controller.enqueue(pad)
            }
            controller.enqueue(new Uint8Array(BLOCK * 2))
            return
          }
          controller.close()
          return
        }
      } catch (err) {
        if (reader !== null) reader.cancel().catch(() => {})
        reader = null
        controller.error(err)
      }
    },
    cancel() {
      if (reader !== null) reader.cancel().catch(() => {})
      reader = null
    },
  })
}
