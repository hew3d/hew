/* oxlint-disable lingui/no-unlocalized-strings -- SVG path data for Preview-only primary shape artwork */
/** Static cross-sections for the four primary shapes: artwork only, no
 * operation geometry is derived here. */
const primaryOutlines = {
  round: 'M3 21V3H9A12 12 0 0 1 21 15V21Z',
  chamfer: 'M3 21V3H9L21 15V21Z',
  cove: 'M3 21V3H9A12 12 0 0 0 21 15V21Z',
  bullnose: 'M3 21V3H12A9 9 0 0 1 12 21Z',
} as const

/** Decorative cross-section; the adjacent label supplies its accessible name. */
export function ProfileMenuIcon({ kind }: { kind: keyof typeof primaryOutlines }) {
  return <svg aria-hidden="true" data-profile-kind={kind} viewBox="0 0 24 24" width="24" height="24" style={{ flexShrink: 0 }}>
    <path d={primaryOutlines[kind]} fill="none" stroke="currentColor" strokeWidth="1.25" vectorEffect="non-scaling-stroke" strokeLinejoin="round" strokeLinecap="round" />
  </svg>
}
