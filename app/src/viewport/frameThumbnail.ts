/**
 * Downscale a raw `ViewportApi.captureFrame()` result (full-resolution RGBA
 * canvas pixels, WebGL's BOTTOM-to-TOP row order) into a small JPEG data
 * URL — the shared builder behind Shop Mode's Recents thumbnails and the
 * editor's Scene thumbnails (docs/design/scenes.md §5).
 *
 * Two canvas passes: `putImageData` onto a full-size offscreen canvas (the
 * only way to get raw RGBA bytes back into something `drawImage` can read
 * from), then `drawImage` that onto a `width`×`height` one with a "cover"
 * crop (scale so the SHORTER side fills the box, centering the longer side's
 * overflow off both edges equally) rather than a letterboxed contain-fit — a
 * JPEG has no transparency to letterbox onto, and a full-bleed swatch reads
 * better in a small list row than one with visible bars.
 *
 * `frame.pixels` is `captureFrame`'s raw `gl.readPixels` output — WebGL's
 * row order is BOTTOM-to-TOP, but `putImageData` treats row 0 as the TOP
 * row, so the source canvas is vertically flipped relative to what was on
 * screen. Corrected at the DRAW rather than by reversing the rows before
 * `putImageData`: mirroring the thumb canvas's own coordinate space around
 * its horizontal centerline (`translate` to the far edge, then `scale(1,
 * -1)`) leaves the cover-crop math untouched — the crop box is symmetric
 * about that centerline, so mirroring it lands back on the same span, just
 * reading the source rows in the opposite order.
 *
 * `null` on any failure (a 0×0 frame, no 2D context) — every caller treats
 * that as "no thumbnail", falling back to a placeholder, not an error.
 */
export function buildFrameThumbnail(
  frame: { width: number; height: number; pixels: Uint8Array },
  width: number,
  height: number,
  quality: number,
): string | null {
  if (frame.width === 0 || frame.height === 0 || width <= 0 || height <= 0) return null
  const source = document.createElement('canvas')
  source.width = frame.width
  source.height = frame.height
  const sourceCtx = source.getContext('2d')
  if (sourceCtx === null) return null
  // `ImageData`'s constructor wants a plain-`ArrayBuffer`-backed
  // `Uint8ClampedArray` (`ImageDataArray`), not the `ArrayBuffer |
  // SharedArrayBuffer`-backed one this project's TS/DOM lib version infers
  // for a wasm-returned view — same cast, same reasoning, as
  // `io/recents.ts`'s own `hashBytes`. The pixels are never mutated here.
  const clamped = new Uint8ClampedArray(frame.pixels.buffer, frame.pixels.byteOffset, frame.pixels.byteLength) as Uint8ClampedArray<ArrayBuffer>
  sourceCtx.putImageData(new ImageData(clamped, frame.width, frame.height), 0, 0)

  const thumb = document.createElement('canvas')
  thumb.width = width
  thumb.height = height
  const thumbCtx = thumb.getContext('2d')
  if (thumbCtx === null) return null
  const scale = Math.max(width / frame.width, height / frame.height)
  const drawWidth = frame.width * scale
  const drawHeight = frame.height * scale
  const drawX = (width - drawWidth) / 2
  const drawY = (height - drawHeight) / 2
  thumbCtx.save()
  thumbCtx.translate(0, height)
  thumbCtx.scale(1, -1)
  thumbCtx.drawImage(source, drawX, drawY, drawWidth, drawHeight)
  thumbCtx.restore()
  return thumb.toDataURL('image/jpeg', quality)
}
