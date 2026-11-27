/* oxlint-disable lingui/no-unlocalized-strings -- plugin copy; English only until the feature graduates. */
/**
 * A path or folder name that came from outside Hew (a folder the person
 * picked, a request over the live API) is data, and can be written to read
 * like a sentence of Hew's. It is shown on its own line in a fixed-width
 * font under a fixed label, cleaned and cut in the middle when long, never
 * inside one of Hew's sentences.
 */

import type { CSSProperties, ReactElement } from 'react'
import { cleanText } from './pluginText'

/** Characters of a path shown before it is cut in the middle. */
export const MAX_PATH_SHOWN = 120

/** `text` cleaned, and when longer than `max` characters cut in the middle
 *  with an ellipsis so both ends stay visible. */
export function elideMiddle(text: string, max: number = MAX_PATH_SHOWN): string {
  const clean = cleanText(text, Number.MAX_SAFE_INTEGER)
  const chars = Array.from(clean)
  if (chars.length <= max) return clean
  if (max < 3) return '…'.slice(0, max)
  const keep = max - 1
  const head = Math.ceil(keep / 2)
  const tail = keep - head
  return chars.slice(0, head).join('').trimEnd() + '…' + chars.slice(chars.length - tail).join('').trimStart()
}

const labelStyle: CSSProperties = { color: 'var(--text-faint, #888)', fontSize: '11px' }
const dataStyle: CSSProperties = {
  display: 'block',
  fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
  fontSize: '12px',
  overflowWrap: 'anywhere',
  whiteSpace: 'pre-wrap',
  userSelect: 'text',
}

/** A fixed label line, then the cleaned, bounded value in its own block. */
export function PathText({ label, value, max = MAX_PATH_SHOWN }: { label: string; value: string; max?: number }): ReactElement {
  return (
    <div data-path-text="">
      <div style={labelStyle}>{label}</div>
      <code style={dataStyle}>{elideMiddle(value, max)}</code>
    </div>
  )
}
