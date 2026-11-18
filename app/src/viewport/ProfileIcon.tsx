/* oxlint-disable lingui/no-unlocalized-strings -- SVG path data and Preview-only identifiers */
import type { FilletProfile } from '../tools/filletSettings'

/** End-on silhouettes of the four supported basic edge cuts. */
export function ProfileIcon({ kind }: { kind: FilletProfile['kind'] }) {
  const cut = {
    round: 'M3 21V3H9A12 12 0 0 0 21 15V21Z',
    chamfer: 'M3 21V3H9L21 15V21Z',
    cove: 'M3 21V3H9A12 12 0 0 1 21 15V21Z',
    bullnose: 'M3 21V3H15A9 9 0 0 1 15 21Z',
    shape: '',
  }[kind]
  return <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" style={{ flexShrink: 0 }}>
    <path d={cut} fill="currentColor" fillOpacity="0.16" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" />
  </svg>
}
