/**
 * Small inline glyphs shared by the Library browser's tiles, detail pane,
 * and menu — kept tiny and dependency-free (no icon font) to match the
 * rest of the panel chrome (`WelcomeScreen.tsx`'s `FolderIcon`/`ExternalIcon`
 * convention: plain `currentColor` SVG, `aria-hidden`).
 */

/** The thumbnail placeholder — an open-cube wireframe, shown while a real
 * thumbnail is loading or when an item genuinely has nothing to render. */
export function CubeGlyph() {
  return (
    <svg viewBox="0 0 32 32" aria-hidden="true" focusable="false">
      <g fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" strokeLinecap="round">
        <polygon points="16,4 27,10 27,22 16,28 5,22 5,10" />
        <line x1="16" y1="16" x2="16" y2="28" />
        <line x1="16" y1="16" x2="5" y2="10" />
        <line x1="16" y1="16" x2="27" y2="10" />
      </g>
    </svg>
  )
}

/** The errored-tile / errored-detail warning glyph. */
export function WarningGlyph() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <g fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" strokeLinecap="round">
        <path d="M8 1.6 14.8 13.6H1.2Z" />
        <line x1="8" y1="6.2" x2="8" y2="9.6" />
        <circle cx="8" cy="11.8" r="0.15" fill="currentColor" stroke="none" />
      </g>
    </svg>
  )
}

export function SearchGlyph() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <g fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round">
        <circle cx="6.8" cy="6.8" r="4.4" />
        <line x1="10.1" y1="10.1" x2="14" y2="14" />
      </g>
    </svg>
  )
}

export function CloseGlyph() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <g stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
        <line x1="3" y1="3" x2="13" y2="13" />
        <line x1="13" y1="3" x2="3" y2="13" />
      </g>
    </svg>
  )
}

/** Re-render (circular arrow) glyph for the detail pane's thumbnail overlay
 * and the ⋯ menu's "Re-render thumbnail" row. */
export function RerenderGlyph() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <path
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M13.2 6.2A5.4 5.4 0 1 0 13.9 9.4 M13.2 2.6v3.6h-3.6"
      />
    </svg>
  )
}

export function FolderGlyph() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <path
        fill="none"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinejoin="round"
        d="M1.8 4.2h4l1.4 1.6h7v6.4a1 1 0 0 1-1 1H2.8a1 1 0 0 1-1-1V4.2Z"
      />
    </svg>
  )
}

/** Grid-view toggle glyph — a 2×2 tile grid. */
export function GridViewGlyph() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <g fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round">
        <rect x="1.8" y="1.8" width="5.2" height="5.2" rx="0.8" />
        <rect x="9" y="1.8" width="5.2" height="5.2" rx="0.8" />
        <rect x="1.8" y="9" width="5.2" height="5.2" rx="0.8" />
        <rect x="9" y="9" width="5.2" height="5.2" rx="0.8" />
      </g>
    </svg>
  )
}

/** List-view toggle glyph — three stacked rows. */
export function ListViewGlyph() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <g stroke="currentColor" strokeWidth="1.4" strokeLinecap="round">
        <line x1="1.8" y1="3.2" x2="14.2" y2="3.2" />
        <line x1="1.8" y1="8" x2="14.2" y2="8" />
        <line x1="1.8" y1="12.8" x2="14.2" y2="12.8" />
      </g>
    </svg>
  )
}
