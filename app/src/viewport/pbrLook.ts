/**
 * pbrLook — a material's metallic and roughness factors as the viewport's
 * Blinn-Phong parameters. The viewport shades with `MeshPhongMaterial`
 * (no environment map, no PBR pipeline), so this is an approximation that
 * keeps the two factors legible: rough surfaces get a wide, dim highlight,
 * smooth ones a tight, bright one, and a metal tints its highlight with its
 * own colour and loses most of its diffuse light. A material with no
 * physically based properties keeps three's Phong defaults, so nothing that
 * never had them changes.
 */

/** Phong parameters standing in for a surface. */
export interface PhongLook {
  shininess: number
  /** Linear RGB of the highlight, each 0..1. */
  specular: [number, number, number]
  /** Multiplier for the diffuse colour, 0..1. */
  diffuseScale: number
}

/** The reflectance of a non-metal at normal incidence. */
const DIELECTRIC_F0 = 0.04

/** Phong parameters for a surface of `baseLinear` colour (linear RGB,
 *  0..1) with the given factors (each clamped to 0..1). */
export function phongLook(
  baseLinear: readonly [number, number, number],
  metallic: number,
  roughness: number,
): PhongLook {
  const m = Math.min(1, Math.max(0, metallic))
  const r = Math.min(1, Math.max(0, roughness))
  const smooth = 1 - r
  // A smooth surface concentrates its highlight and a rough one spreads it.
  const shininess = 4 + 156 * smooth * smooth
  // Dielectrics reflect white, metals reflect their own colour.
  const strength = 0.2 + 0.8 * smooth
  const specular = baseLinear.map(
    (c) => (DIELECTRIC_F0 + (c - DIELECTRIC_F0) * m) * strength * (1 + 3 * m),
  ) as [number, number, number]
  return {
    shininess,
    specular: specular.map((c) => Math.min(1, c)) as [number, number, number],
    diffuseScale: 1 - 0.8 * m,
  }
}
