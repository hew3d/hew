/**
 * The release-notes renderer: the Markdown subset renders as elements; every
 * released section of the real CHANGELOG.md renders with only the allowed
 * elements; hostile input comes out as inert text (docs/design D3).
 */
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fireEvent, render } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ReleaseNotes, bulletParts, headingText, parseNotes, scanInline, titleText } from './releaseNotes'

const opened = vi.hoisted(() => ({ open: vi.fn() }))
vi.mock('../panels/ReportBugDialog', () => ({ openExternal: (u: string) => opened.open(u) }))

const ALLOWED = new Set(['H2', 'H3', 'P', 'UL', 'LI', 'STRONG', 'CODE', 'A'])

function renderNotes(md: string) {
  const view = render(
    <div data-testid="root">
      <ReleaseNotes markdown={md} />
    </div>,
  )
  return { root: view.getByTestId('root'), ...view }
}

/** Every element is on the allow-list; every link is http(s) and nothing loads. */
function expectInert(root: HTMLElement) {
  for (const el of Array.from(root.querySelectorAll('*'))) {
    if (el === root) continue
    expect(ALLOWED.has(el.tagName), `unexpected <${el.tagName.toLowerCase()}>`).toBe(true)
    for (const attr of Array.from(el.attributes)) {
      expect(attr.name.startsWith('on'), `handler attribute ${attr.name}`).toBe(false)
    }
  }
  for (const a of Array.from(root.querySelectorAll('a'))) {
    expect(a.getAttribute('href')).toMatch(/^https?:\/\//)
  }
  expect(root.querySelector('img,script,iframe,style,svg,object,embed,form,input')).toBeNull()
}

beforeEach(() => opened.open.mockReset())

describe('ReleaseNotes: the supported subset', () => {
  it('renders headings, bullets with one nesting level, bold, code and links', () => {
    const { root } = renderNotes(
      [
        '### Added',
        '',
        '- **Lock:** keep an object `fixed`.',
        '- A second bullet',
        '  - nested one',
        '    - deeper stays at the same level',
        '',
        'A paragraph with [the site](https://hew3d.com/x).',
      ].join('\n'),
    )
    expect(root.querySelector('h3')?.textContent).toBe('Added')
    expect(root.querySelector('strong')?.textContent).toBe('Lock:')
    expect(root.querySelector('code')?.textContent).toBe('fixed')
    const lists = root.querySelectorAll('ul')
    expect(lists).toHaveLength(2)
    expect(root.querySelectorAll('ul > li > ul > li')).toHaveLength(2)
    expect(root.querySelector('p a')?.getAttribute('href')).toBe('https://hew3d.com/x')
    expectInert(root)
  })

  it('renders a ## line as the release title and leaves ### a heading', () => {
    const { root } = renderNotes(
      '## 1.4.0 (2026-11-08) - The "All In" Release\n\nIntro.\n\n### Fixed\n\n- A fix. [#48](https://github.com/hew3d/hew/issues/48)',
    )
    expect(root.querySelectorAll('h2')).toHaveLength(1)
    expect(root.querySelector('h2')?.textContent).toBe('1.4.0 (2026-11-08) - The "All In" Release')
    expect(root.querySelector('h3')?.textContent).toBe('Fixed')
    expect(root.querySelector('li a')?.getAttribute('href')).toBe('https://github.com/hew3d/hew/issues/48')
    expect(root.querySelector('li a')?.textContent).toBe('#48')
    // No space after the marks: not a title, shown as the characters it is.
    expect(parseNotes('##x')).toEqual([{ kind: 'paragraph', text: '##x' }])
    expect(parseNotes('##')).toEqual([{ kind: 'paragraph', text: '##' }])
    for (const s of ['## a', '##\ta', '##  a b', '##x', '### a', '#### a', '## a\u2028b', '## ' + ' '.repeat(50_000) + 'a\u2028b']) {
      const m = /^##\s+(.*)$/.exec(s)
      expect(titleText(s), JSON.stringify(s.slice(0, 12))).toBe(m ? (m[1] ?? '') : null)
    }
    expectInert(root)
  })

  it('opens an http link through openExternal, not the webview', () => {
    const { root } = renderNotes('[site](https://hew3d.com)')
    const a = root.querySelector('a')!
    expect(fireEvent.click(a)).toBe(false) // default prevented
    expect(opened.open).toHaveBeenCalledWith('https://hew3d.com/')
  })

  it('joins wrapped paragraph lines and bullet continuations', () => {
    const blocks = parseNotes('one\ntwo\n\n- a\n  more\n')
    expect(blocks[0]).toEqual({ kind: 'paragraph', text: 'one two' })
    expect(blocks[1]).toEqual({ kind: 'list', items: [{ text: 'a more', children: [] }] })
  })

  it('renders an empty string as nothing', () => {
    expect(renderNotes('').root.childElementCount).toBe(0)
  })
})

describe('ReleaseNotes: hostile input', () => {
  const cases: Record<string, string> = {
    script: '<script>alert(1)</script>',
    'script in a bullet': '- <script>window.pwn=1</script> **bold**',
    'javascript link': '[click](javascript:alert(1))',
    'JavaScript link, mixed case': '[click](JaVaScRiPt:alert(1))',
    'data link': '[x](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)',
    'vbscript link': '[x](vbscript:msgbox(1))',
    'file link': '[x](file:///etc/passwd)',
    'relative link': '[x](/etc/passwd)',
    image: '![alt](https://evil.example/p.png)',
    'image with handler': '<img src=x onerror=alert(1)>',
    'raw html': '<div onclick="x()"><a href="javascript:1">a</a></div>',
    'iframe': '<iframe src="https://evil.example"></iframe>',
    'html in link text': '[<b>x</b>](https://hew3d.com)',
    'nested unclosed bold': '**bold [link](https://hew3d.com) `code',
    'unclosed link': '[text](https://hew3d.com',
    'unclosed bracket': '[text(https://hew3d.com)',
    'nested brackets': '[[a](https://hew3d.com)](https://hew3d.com)',
    'empty link text': '[](https://hew3d.com)',
    'style injection': '<style>body{display:none}</style>',
    'heading with html': '### <h1>x</h1>',
    'entity': '&lt;script&gt;alert(1)&lt;/script&gt;',
  }
  for (const [name, md] of Object.entries(cases)) {
    it(`keeps "${name}" inert`, () => {
      const { root } = renderNotes(md)
      expectInert(root)
      // Nothing that would execute or load appears as a real element.
      expect(root.innerHTML).not.toMatch(/<(script|img|iframe|style)/i)
      expect(root.querySelector('a[href^="javascript" i], a[href^="data" i]')).toBeNull()
    })
  }

  it('shows rejected markup as the characters it is', () => {
    for (const md of [
      '<script>alert(1)</script>',
      '[x](javascript:alert(1))',
      '![a](https://e.example/p.png)',
      '[x](data:text/html,hi)',
    ]) {
      const { root, unmount } = renderNotes(md)
      expect(root.textContent).toBe(md)
      unmount()
    }
  })

  it('survives a very long, repetitive input without throwing', () => {
    const md = '**a '.repeat(5000) + '[x]('.repeat(5000)
    expectInert(renderNotes(md).root)
  })

  // Each of these took the regex parser seconds (it was quadratic in the
  // length of one line; 40,000 characters cost about 2.5 s). Linear now, so
  // lines several times longer finish in a few milliseconds; the bound is
  // loose so a loaded machine cannot fail it.
  it('parses one enormous hostile line in linear time', () => {
    const n = 120_000
    const lines = [
      '['.repeat(n),
      '[]('.repeat(n / 3),
      '![]('.repeat(n / 4),
      '`'.repeat(n),
      '**a'.repeat(n / 3),
      'a' + ' '.repeat(n) + 'b',
      '- ' + ' '.repeat(n) + 'a\u2028',
      '### ' + ' '.repeat(n) + 'a\u2028',
    ]
    for (const line of lines) {
      const started = performance.now()
      parseNotes(line)
      scanInline(line)
      expect(performance.now() - started, line.slice(0, 8)).toBeLessThan(1000)
    }
    const started = performance.now()
    const { root } = renderNotes(lines.join('\n'))
    expect(performance.now() - started).toBeLessThan(5000)
    expectInert(root)
  })
})

// The regexes the hand-written scanners replaced: the scanners must find
// exactly what these found, on any input.
const ORACLE_INLINE = /(!?)\[([^\]\n]*)\]\(([^)\s]*)\)|`([^`\n]+)`|\*\*([^*\n]+?)\*\*/g
const ORACLE_BULLET = /^(\s*)[-*]\s+(.*)$/
const ORACLE_HEADING = /^###\s+(.*)$/

describe('ReleaseNotes: the linear scanners match the regexes they replaced', () => {
  // A seeded generator, so a failure names a reproducible input.
  function* inputs(): Generator<string> {
    const alphabet = ['[', ']', '(', ')', '!', '`', '*', '-', '#', ' ', '\t', 'a', 'b', '\n', '\u2028', 'h', ':', '/']
    let seed = 12345
    const rand = () => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff
      return seed
    }
    for (let k = 0; k < 6000; k++) {
      const len = rand() % 24
      let s = ''
      for (let i = 0; i < len; i++) s += alphabet[rand() % alphabet.length]
      yield s
    }
    yield* ['[a](https://x.y)', '![a](b)', '***x**', '**a**b**', '`a``b`', '[a]](b)', '[a](b c)', '- x', '  * y', '### h', '####x']
  }

  it('scanInline', () => {
    for (const s of inputs()) {
      const want = [...s.matchAll(ORACLE_INLINE)].map((m) => ({
        index: m.index,
        raw: m[0],
        image: m[1] === '!',
        label: m[3] !== undefined ? m[2] : undefined,
        url: m[3],
        code: m[4],
        bold: m[5],
      }))
      const got = scanInline(s).map((m) => ({
        index: m.index,
        raw: m.raw,
        image: m.image,
        label: m.label,
        url: m.url,
        code: m.code,
        bold: m.bold,
      }))
      expect(got, JSON.stringify(s)).toEqual(want)
    }
  })

  it('headingText and bulletParts', () => {
    for (const s of inputs()) {
      const h = ORACLE_HEADING.exec(s)
      expect(headingText(s), JSON.stringify(s)).toBe(h ? (h[1] ?? '') : null)
      const b = ORACLE_BULLET.exec(s)
      expect(bulletParts(s), JSON.stringify(s)).toEqual(
        b ? { indent: (b[1] ?? '').length, text: b[2] ?? '' } : null,
      )
    }
  })
})

describe('ReleaseNotes: every released section of CHANGELOG.md', () => {
  const changelog = readFileSync(resolve(process.cwd(), '../CHANGELOG.md'), 'utf8')
  const sections = changelog.split(/^## \[/m).slice(1).map((chunk) => {
    const nl = chunk.indexOf('\n')
    return { name: chunk.slice(0, chunk.indexOf(']')), body: chunk.slice(nl + 1) }
  })

  it('finds the released sections', () => {
    expect(sections.length).toBeGreaterThan(10)
    expect(sections.some((s) => s.name === '1.4.0')).toBe(true)
  })

  for (const section of sections) {
    it(`renders ${section.name} with only allowed elements`, () => {
      const { root } = renderNotes(section.body)
      expectInert(root)
      const bullets = section.body.split('\n').filter((l) => /^\s*[-*]\s+/.test(l)).length
      expect(root.querySelectorAll('li')).toHaveLength(bullets)
      const headings = section.body.split('\n').filter((l) => /^###\s/.test(l)).length
      expect(root.querySelectorAll('h3')).toHaveLength(headings)
      // Plain words survive: the section's first bullet text is present.
      const first = section.body.split('\n').find((l) => /^[-*]\s+/.test(l))
      if (first) {
        const word = first.replace(/^[-*]\s+/, '').replace(/[*`[\]()]/g, '').split(' ').slice(0, 2).join(' ')
        expect(root.textContent).toContain(word)
      }
    })
  }
})
