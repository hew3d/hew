// @vitest-environment jsdom
//
// A `.hewplugin` is not a model. Open and drop must treat one the way they
// treat any file they do not recognize: the host refuses it as an unsupported
// kind before any importer runs, and nothing in the app routes it to plugin
// code.
// Installing a plugin is only ever done from Settings.

import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { UnsupportedFileError } from './fileHost'
import { WebFileHost } from './webFileHost'

const SRC = resolve(dirname(fileURLToPath(import.meta.url)), '..')

describe('opening a .hewplugin', () => {
  it('File > Open refuses it as an unrecognized kind, never as a model or a plugin', async () => {
    const opened = new WebFileHost().openAny()
    const input = document.querySelector('input[type=file]') as HTMLInputElement
    const bytes = new Uint8Array([0x50, 0x4b, 3, 4])
    Object.defineProperty(input, 'files', {
      value: [{ name: 'tool.hewplugin', arrayBuffer: () => Promise.resolve(bytes.buffer) }],
    })
    input.dispatchEvent(new Event('change'))
    // The same answer any unknown extension gets: an UnsupportedFileError.
    // Not a `hew` pick, not the COLLADA importer, and not a plugin kind.
    await expect(opened).rejects.toBeInstanceOf(UnsupportedFileError)
  })

  it('the picker offers no .hewplugin', () => {
    new WebFileHost().openAny().catch(() => undefined)
    const input = document.querySelector('input[type=file]') as HTMLInputElement
    expect(input.accept).not.toMatch(/hewplugin/)
  })

  it('the drop handlers match their extensions so a .hewplugin name matches none (traced in source)', () => {
    const app = readFileSync(join(SRC, 'App.tsx'), 'utf8')
    // Every extension test on a dropped name or path in App.tsx.
    const tests = [...app.matchAll(/\/(\\\.[^/\n]*\$)\/i?\.test\(\s*(?:file\.name|p)\s*\)/g)].map((m) => new RegExp(m[1], 'i'))
    expect(tests.length).toBeGreaterThanOrEqual(2)
    for (const re of tests) expect(re.test('tool.hewplugin'), String(re)).toBe(false)
    expect("tool.hewplugin".endsWith('.hew')).toBe(false)
    // And nothing outside the plugins folder mentions the package type.
    for (const rel of ['App.tsx', 'io/webFileHost.ts', 'io/tauriFileHost.ts', 'io/fileHost.ts']) {
      expect(readFileSync(join(SRC, rel), 'utf8'), rel).not.toMatch(/hewplugin/i)
    }
  })
})
