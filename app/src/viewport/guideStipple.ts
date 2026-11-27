/**
 * guideStipple — a guide line's dash pattern as its 16-bit stipple mask
 * (bit 0 first along the line; `0x3333` short dashes is what Hew draws by
 * default, `0xFFFF` solid, `0x8888` dotted, `0xF0F0` long dashes, `0x1C47`
 * dash-dot-dash). The viewport's dashed-line material tests the mask per
 * fragment (`applyGuideStipple`), so any mask draws as the file stored it.
 */
import type * as THREE from 'three'

/** The mask of a guide Hew draws: short dashes. */
export const GUIDE_DEFAULT_STIPPLE = 0x3333

/** Bits in a mask; one pattern period is `STIPPLE_BITS` units long. */
const STIPPLE_BITS = 16

/** Whether the mask draws at `distance` along the line, one unit being
 *  `unit` long (the same test the shader makes). */
export function stippleOn(mask: number, distance: number, unit: number): boolean {
  if (unit <= 0) return true
  const bit = Math.floor(((distance / unit) % STIPPLE_BITS + STIPPLE_BITS) % STIPPLE_BITS)
  return ((mask >> bit) & 1) === 1
}

/** `three`'s dashed-line shaders rewritten to test a per-vertex `stipple`
 *  attribute instead of one dash/gap pair. Pure, so it is testable without
 *  a GL context. */
export function stippleShaders(vertex: string, fragment: string): { vertex: string; fragment: string } {
  const v = vertex
    .replace('void main() {', 'attribute float stipple;\nvarying float vStipple;\nvoid main() {\n\tvStipple = stipple;')
  const f = fragment
    .replace('void main() {', 'varying float vStipple;\nvoid main() {')
    .replace(
      /if \( mod\( vLineDistance, totalSize \) > dashSize \) \{[\s\S]*?\}/,
      `float stippleBit = floor(mod(vLineDistance / (totalSize * 0.25), ${STIPPLE_BITS}.0));
	float stippleMask = floor(vStipple + 0.5);
	for (int i = 0; i < ${STIPPLE_BITS}; i++) {
		if (float(i) >= stippleBit) break;
		stippleMask = floor(stippleMask * 0.5);
	}
	if (mod(stippleMask, 2.0) < 0.5) discard;`,
    )
  return { vertex: v, fragment: f }
}

/** Make a `LineDashedMaterial` draw each vertex's `stipple` mask; the dash
 *  and gap sizes then set the pattern's scale (one unit is a quarter of their sum). */
export function applyGuideStipple(mat: THREE.LineDashedMaterial): void {
  mat.onBeforeCompile = (shader) => {
    const s = stippleShaders(shader.vertexShader, shader.fragmentShader)
    shader.vertexShader = s.vertex
    shader.fragmentShader = s.fragment
  }
  mat.customProgramCacheKey = () => 'guide-stipple'
}
