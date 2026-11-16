/**
 * Text and bytes that came from a plugin package are shown as text or as an
 * `<img>` blob URL, never as markup. This reads the plugin sources and fails
 * on any way of turning a string into elements or a frame.
 */

import { readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const HERE = dirname(fileURLToPath(import.meta.url))
const FORBIDDEN = ['dangerouslySetInnerHTML', 'innerHTML', 'outerHTML', 'insertAdjacentHTML', 'srcdoc', '<object', '<embed', '<iframe']

function* sources(dir: string): Generator<string> {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) {
      if (name !== 'probe') yield* sources(path)
    } else if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name)) {
      yield path
    }
  }
}

describe('plugin sources', () => {
  it('never build markup, a frame or an embed from a string', () => {
    const files = [...sources(HERE)]
    expect(files.length).toBeGreaterThan(5)
    for (const file of files) {
      const text = readFileSync(file, 'utf8')
      for (const word of FORBIDDEN) {
        expect(text.includes(word), `${relative(HERE, file)} uses ${word}`).toBe(false)
      }
    }
  })

  it('only the icon <img> and the blob URL it uses take package bytes', () => {
    const installer = readFileSync(join(HERE, 'installer.ts'), 'utf8')
    expect(installer).toMatch(/createObjectURL/)
    // The one MIME table is images only.
    const mime = /const MIME[^{]*\{([^}]*)\}/.exec(installer)?.[1] ?? ''
    expect([...mime.matchAll(/'([a-z]+\/[^']+)'/g)].every((m) => m[1].startsWith('image/'))).toBe(true)
    expect(mime).not.toMatch(/html|javascript/i)
  })
})
