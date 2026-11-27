/**
 * Release notes renderer for the update window: the Markdown subset our own
 * CHANGELOG uses, turned into React elements. Supported: a `##` title (the
 * release's name and date, which the update feed puts first), `###` headings,
 * `-`/`*` bullets (one nesting level), paragraphs, `**bold**`, `` `code` ``
 * and `[text](url)` links. Anything else is shown as the characters it is.
 *
 * It never builds HTML from a string: every piece of text reaches the DOM as
 * a React text child (escaped), no image is ever created, and a link gets an
 * href only when its address is http(s) (the check Sent Reports uses).
 * A link with any other address, an image, or unclosed markup is left as the
 * literal source text. docs/design D3: no Markdown library, no sanitizer.
 */

import type { ReactNode } from 'react'
import { httpUrlOrNull } from '../panels/SentReportsDialog'
import { openExternal } from '../panels/ReportBugDialog'

/**
 * One piece of inline markup found by `scanInline`: `raw` is its source
 * text, from `index`. Exactly one of the groups is set: an image or link
 * (`label` and `url`, `image` true for `![…](…)`), `code`, or `bold`.
 */
export interface InlineMatch {
  index: number
  raw: string
  image: boolean
  label?: string
  url?: string
  code?: string
  bold?: string
}

/** Characters `.` does not match in a JavaScript regex. */
const LINE_BREAK = /[\n\r\u2028\u2029]/
const SPACE = /\s/

/**
 * The index of the first character at or after `p` that `stop` accepts, or
 * -1. Queries must come with `p` never decreasing; the answer is reused
 * while it still applies, so a whole line costs one pass however many
 * openers it holds.
 */
function nextFinder(text: string, stop: (c: string) => boolean): (p: number) => number {
  let from = -1
  let found = -1
  return (p: number) => {
    if (from !== -1 && p >= from && (found === -1 || p <= found)) return found
    let i = p
    while (i < text.length && !stop(text[i]!)) i++
    from = p
    found = i < text.length ? i : -1
    return found
  }
}

/**
 * The inline markup in `text`, left to right; at each position an image or
 * link is tried first, then code, then bold, and scanning resumes after a
 * match. It finds exactly what the single regex it replaces found (kept as
 * the oracle in releaseNotes.test.tsx), but in time linear in the line. The
 * regex rescanned to the end of the line from every unclosed `[` (or `](`),
 * which is quadratic: one hostile line of a few tens of thousands of `[`
 * froze the window for seconds.
 */
export function scanInline(text: string): InlineMatch[] {
  const out: InlineMatch[] = []
  const closeBracket = nextFinder(text, (c) => c === ']' || c === '\n')
  const urlEnd = nextFinder(text, (c) => c === ')' || SPACE.test(c))
  const backtick = nextFinder(text, (c) => c === '`' || c === '\n')
  const star = nextFinder(text, (c) => c === '*' || c === '\n')
  let i = 0
  while (i < text.length) {
    const c = text[i]
    let end = -1
    let match: InlineMatch | null = null
    if (c === '[' || (c === '!' && text[i + 1] === '[')) {
      const open = c === '!' ? i + 1 : i
      const close = closeBracket(open + 1)
      if (close !== -1 && text[close] === ']' && text[close + 1] === '(') {
        const stop = urlEnd(close + 2)
        if (stop !== -1 && text[stop] === ')') {
          end = stop + 1
          match = {
            index: i,
            raw: text.slice(i, end),
            image: c === '!',
            label: text.slice(open + 1, close),
            url: text.slice(close + 2, stop),
          }
        }
      }
    } else if (c === '`') {
      const close = backtick(i + 1)
      if (close > i + 1 && text[close] === '`') {
        end = close + 1
        match = { index: i, raw: text.slice(i, end), image: false, code: text.slice(i + 1, close) }
      }
    } else if (c === '*' && text[i + 1] === '*') {
      const close = star(i + 2)
      if (close > i + 2 && text[close] === '*' && text[close + 1] === '*') {
        end = close + 2
        match = { index: i, raw: text.slice(i, end), image: false, bold: text.slice(i + 2, close) }
      }
    }
    if (match !== null) {
      out.push(match)
      i = end
    } else {
      i++
    }
  }
  return out
}

const CODE_STYLE: React.CSSProperties = {
  fontFamily: 'var(--font-family-mono, ui-monospace, SFMono-Regular, monospace)',
  fontSize: '0.92em',
  background: 'var(--surface-input, rgba(127,127,127,0.18))',
  borderRadius: '3px',
  padding: '0 3px',
}

const LINK_STYLE: React.CSSProperties = {
  color: 'var(--accent-text, var(--accent-base, #5b8cff))',
  textDecoration: 'underline',
  cursor: 'pointer',
}

/** The URL a link opens, or null when it is not http(s). Exported for tests. */
export function safeLinkTarget(raw: string): string | null {
  return httpUrlOrNull(raw)
}

function renderInline(text: string, keyBase: string): ReactNode[] {
  const out: ReactNode[] = []
  let last = 0
  let n = 0
  for (const m of scanInline(text)) {
    const start = m.index
    if (start > last) out.push(text.slice(last, start))
    const key = `${keyBase}-${n++}`
    if (m.image) {
      // An image: never loaded. Shown as the characters it is.
      out.push(m.raw)
    } else if (m.url !== undefined) {
      const label = m.label ?? ''
      const url = safeLinkTarget(m.url)
      if (url === null || label.length === 0) {
        out.push(m.raw)
      } else {
        out.push(
          <a
            key={key}
            href={url}
            target="_blank"
            rel="noopener noreferrer"
            style={LINK_STYLE}
            onClick={(e) => {
              e.preventDefault()
              openExternal(url)
            }}
          >
            {label}
          </a>,
        )
      }
    } else if (m.code !== undefined) {
      out.push(
        <code key={key} style={CODE_STYLE}>
          {m.code}
        </code>,
      )
    } else if (m.bold !== undefined) {
      out.push(<strong key={key}>{m.bold}</strong>)
    }
    last = start + m.raw.length
  }
  if (last < text.length) out.push(text.slice(last))
  return out
}

interface Bullet {
  text: string
  children: string[]
}

type Block =
  | { kind: 'title'; text: string }
  | { kind: 'heading'; text: string }
  | { kind: 'paragraph'; text: string }
  | { kind: 'list'; items: Bullet[] }

/** `s` from `from` on, without its leading whitespace; null when what is
 *  left holds a line break (what `\s+(.*)$` would refuse). */
function restOfLine(s: string, from: number): string | null {
  let i = from
  while (i < s.length && SPACE.test(s[i]!)) i++
  const rest = s.slice(i)
  return LINE_BREAK.test(rest) ? null : rest
}

/**
 * `### text`: what `/^###\s+(.*)$/` matched, by hand. That regex (and the
 * bullet one below) backtracks over a long run of spaces when the line ends
 * in a character `.` refuses, which is quadratic.
 */
export function headingText(line: string): string | null {
  if (!line.startsWith('###') || !SPACE.test(line[3] ?? '')) return null
  return restOfLine(line, 3)
}

/** `## text`: the release's title line. A `###` line is never one, because
 *  its third character is not a space. */
export function titleText(line: string): string | null {
  if (!line.startsWith('##') || !SPACE.test(line[2] ?? '')) return null
  return restOfLine(line, 2)
}

/** `- text` or `* text`, indented or not: what `/^(\s*)[-*]\s+(.*)$/`
 *  matched, by hand. `indent` is the length of the leading whitespace. */
export function bulletParts(line: string): { indent: number; text: string } | null {
  let i = 0
  while (i < line.length && SPACE.test(line[i]!)) i++
  const marker = line[i]
  if ((marker !== '-' && marker !== '*') || !SPACE.test(line[i + 1] ?? '')) return null
  const text = restOfLine(line, i + 1)
  return text === null ? null : { indent: i, text }
}

export function parseNotes(markdown: string): Block[] {
  const blocks: Block[] = []
  let list: Bullet[] | null = null
  let para: string[] | null = null
  const flush = () => {
    if (para !== null) blocks.push({ kind: 'paragraph', text: para.join(' ') })
    para = null
  }
  for (const raw of markdown.replace(/\r\n?/g, '\n').split('\n')) {
    // trimEnd strips exactly what `\s` matches, without the quadratic
    // backtracking `/\s+$/` does over a run of spaces inside a line.
    const line = raw.trimEnd()
    if (line.trim() === '') {
      flush()
      list = null
      continue
    }
    const title = titleText(line)
    if (title !== null) {
      flush()
      list = null
      blocks.push({ kind: 'title', text: title })
      continue
    }
    const heading = headingText(line)
    if (heading !== null) {
      flush()
      list = null
      blocks.push({ kind: 'heading', text: heading })
      continue
    }
    const bullet = bulletParts(line)
    if (bullet) {
      flush()
      if (list === null) {
        list = []
        blocks.push({ kind: 'list', items: list })
      }
      const nested = bullet.indent > 0
      const text = bullet.text
      const parent = list[list.length - 1]
      // One nesting level: deeper indentation is flattened into it.
      if (nested && parent) parent.children.push(text)
      else list.push({ text, children: [] })
      continue
    }
    if (list !== null && /^\s/.test(line)) {
      // A wrapped continuation of the last bullet.
      const parent = list[list.length - 1]
      if (parent) {
        if (parent.children.length > 0) parent.children[parent.children.length - 1] += ' ' + line.trim()
        else parent.text += ' ' + line.trim()
        continue
      }
    }
    list = null
    if (para === null) para = []
    para.push(line.trim())
  }
  flush()
  return blocks
}

const TITLE_STYLE: React.CSSProperties = { margin: '0 0 8px', fontSize: '15px', fontWeight: 600 }
const HEADING_STYLE: React.CSSProperties = { margin: '10px 0 4px', fontSize: '13px', fontWeight: 600 }
const PARA_STYLE: React.CSSProperties = { margin: '0 0 8px' }
const LIST_STYLE: React.CSSProperties = { margin: '0 0 8px', paddingLeft: '20px' }

/** Render release notes. `markdown` is untrusted text; the result contains
 *  only h2, h3, p, ul, li, strong, code and (http/https) a elements. */
export function ReleaseNotes({ markdown }: { markdown: string }) {
  const blocks = parseNotes(markdown)
  return (
    <>
      {blocks.map((block, i) => {
        const key = `b${i}`
        if (block.kind === 'title') {
          return (
            <h2 key={key} style={TITLE_STYLE}>
              {renderInline(block.text, key)}
            </h2>
          )
        }
        if (block.kind === 'heading') {
          return (
            <h3 key={key} style={HEADING_STYLE}>
              {renderInline(block.text, key)}
            </h3>
          )
        }
        if (block.kind === 'paragraph') {
          return (
            <p key={key} style={PARA_STYLE}>
              {renderInline(block.text, key)}
            </p>
          )
        }
        return (
          <ul key={key} style={LIST_STYLE}>
            {block.items.map((item, j) => (
              <li key={`${key}-${j}`}>
                {renderInline(item.text, `${key}-${j}`)}
                {item.children.length > 0 && (
                  <ul style={{ margin: '2px 0', paddingLeft: '18px' }}>
                    {item.children.map((child, k) => (
                      <li key={k}>{renderInline(child, `${key}-${j}-${k}`)}</li>
                    ))}
                  </ul>
                )}
              </li>
            ))}
          </ul>
        )
      })}
    </>
  )
}
