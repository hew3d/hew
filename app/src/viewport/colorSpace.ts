/**
 * sRGB → linear conversion for tessellator vertex colors.
 *
 * The kernel's tessellation buffers carry per-vertex colors as normalized
 * sRGB components (authored material colors, e.g. 0xcc/255 for the default
 * face grey). three.js interprets `BufferAttribute` vertex colors as
 * *linear-light* values — it lights them in linear space and then encodes the
 * frame to sRGB on output. Feeding sRGB bytes straight in therefore
 * double-brightens every mid-tone and flattens contrast and saturation (the
 * "grey filter" look). Hex/style colors don't have this problem: three's
 * ColorManagement converts those automatically; raw attribute data is the
 * one path left to the caller.
 */

/** The standard sRGB EOTF for one normalized component (IEC 61966-2-1). */
export function srgbToLinear(c: number): number {
  return c < 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4)
}

/**
 * Convert a tessellator color buffer (r,g,b triples, normalized sRGB) to
 * linear-light in place, returning the same array. Call exactly once per
 * buffer, at the point it's copied out of the wasm boundary.
 */
export function srgbColorsToLinear(colors: Float32Array): Float32Array {
  for (let i = 0; i < colors.length; i++) {
    colors[i] = srgbToLinear(colors[i])
  }
  return colors
}
