import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import * as Diag from './diagnosticLog'
import * as inputRecorder from '../recording/inputRecorder'
import * as failedOpenStore from './failedOpen'
import type { FailedOpen } from './failedOpen'
import {
  buildReportBundle,
  buildReportFile,
  buildReportUpload,
  compressReportUpload,
  describeReport,
  encodeReportFile,
  gatherReport,
  measureUpload,
  ReportBuildCancelled,
  extractRecordingSummary,
  scrubHomeDir,
  detectPlatform,
  type BuildReportBundleOptions,
  type ReportableScene,
  type ReportFlags,
} from './reportBundle'

function fakeScene(overrides: Partial<ReportableScene> = {}): ReportableScene {
  return {
    save: () => new Uint8Array([1, 2, 3]),
    peek_recording: () => '{"version":2,"calls":[],"golden_hash":0}',
    object_ids: () => ({ length: 2 }),
    ...overrides,
  }
}

const ALL_ON: ReportFlags = {
  system: true,
  recording: true,
  imports: true,
  failedFile: true,
  model: true,
  log: true,
  input: true,
}
const ALL_OFF: ReportFlags = {
  system: false,
  recording: false,
  imports: false,
  failedFile: false,
  model: false,
  log: false,
  input: false,
}

/** A recording whose second step imports a SketchUp file, in the recorder's
 *  real shape: internally tagged, bytes as a JSON array, u64 golden hash. */
function importRecording(byteList: string): string {
  return `{"version":2,"calls":[{"method":"begin_ground_sketch"},{"method":"import_skp","bytes":[${byteList}]}],"golden_hash":18192258159662307868}`
}

function randomBytes(n: number): Uint8Array {
  const out = new Uint8Array(n)
  for (let i = 0; i < n; i += 65536) crypto.getRandomValues(out.subarray(i, Math.min(n, i + 65536)))
  return out
}

function randomByteList(n: number): string {
  return Array.from(randomBytes(n)).join(',')
}

async function gunzipText(bytes: Uint8Array): Promise<string> {
  const reader = new ReadableStream<Uint8Array<ArrayBuffer>>({
    start(c) {
      c.enqueue(bytes as Uint8Array<ArrayBuffer>)
      c.close()
    },
  })
    .pipeThrough(new DecompressionStream('gzip'))
    .getReader()
  const chunks: Uint8Array[] = []
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    chunks.push(value)
  }
  return new TextDecoder().decode(Buffer.concat(chunks))
}

/** The saved file's text: every ticked row. */
async function savedText(opts: BuildReportBundleOptions): Promise<string> {
  return new TextDecoder().decode(await buildReportFile(opts))
}

/** The saved file, parsed. Only the tests parse it; the recording inside
 *  stays a string, so its u64 values survive. */
async function saved(opts: BuildReportBundleOptions): Promise<ReturnType<typeof JSON.parse>> {
  return JSON.parse(await savedText(opts))
}

beforeEach(() => {
  Diag.clear()
  inputRecorder.stop()
  inputRecorder.take()
  failedOpenStore.clearOpenAttempt()
})

afterEach(() => {
  inputRecorder.stop()
  inputRecorder.take()
  vi.unstubAllGlobals()
})

describe('extractRecordingSummary', () => {
  // Real recorder output, captured from a draw → corner drag → push/pull
  // session. `RecordedCall` is internally tagged on "method"
  // (crates/wasm-api/src/recording.rs), and the handles and golden_hash
  // exceed 2^53, so a JSON.parse anywhere in the scanner would corrupt them.
  const REAL_RECORDING =
    '{"version":2,"calls":[{"method":"begin_ground_sketch"},{"method":"sketch_begin_gesture","sketch":4294967297},{"method":"sketch_add_segment","sketch":4294967297,"a":[0.0,0.0,0.0],"b":[2.0,0.0,0.0]},{"method":"sketch_add_segment","sketch":4294967297,"a":[2.0,0.0,0.0],"b":[2.0,2.0,0.0]},{"method":"sketch_add_segment","sketch":4294967297,"a":[2.0,2.0,0.0],"b":[0.0,2.0,0.0]},{"method":"sketch_add_segment","sketch":4294967297,"a":[0.0,2.0,0.0],"b":[0.0,0.0,0.0]},{"method":"sketch_end_gesture","sketch":4294967297},{"method":"move_sketch_vertex","sketch":4294967297,"vertex":4294967299,"p":[2.9999999023920587,2.600000338220482,0.0]},{"method":"extrude_region","sketch":4294967297,"region":4294967297,"distance":1.0}],"golden_hash":14997137823844688511}'

  it('names every step of a real recording by its method tag', () => {
    const summary = extractRecordingSummary(REAL_RECORDING)
    expect(summary.stepCount).toBe(9)
    expect(summary.lastStepNames).toEqual([
      'begin_ground_sketch',
      'sketch_begin_gesture',
      'sketch_add_segment',
      'sketch_add_segment',
      'sketch_add_segment',
      'sketch_add_segment',
      'sketch_end_gesture',
      'move_sketch_vertex',
      'extrude_region',
    ])
    expect(summary.lastStepNames).not.toContain('method')
  })

  it('returns only the last 20 step names when there are more', () => {
    const calls = Array.from({ length: 25 }, (_, i) => `{"method":"step_${i}"}`).join(',')
    const recording = `{"version":2,"calls":[${calls}],"golden_hash":0}`
    const summary = extractRecordingSummary(recording)
    expect(summary.stepCount).toBe(25)
    expect(summary.lastStepNames).toHaveLength(20)
    expect(summary.lastStepNames[0]).toBe('step_5')
    expect(summary.lastStepNames[19]).toBe('step_24')
  })

  it('handles an empty calls array', () => {
    expect(extractRecordingSummary('{"version":2,"calls":[],"golden_hash":0}')).toEqual({
      stepCount: 0,
      lastStepNames: [],
    })
  })

  it('never throws on malformed input', () => {
    expect(extractRecordingSummary('not json at all')).toEqual({ stepCount: 0, lastStepNames: [] })
    expect(extractRecordingSummary('')).toEqual({ stepCount: 0, lastStepNames: [] })
  })

  it('is unaffected by commas and brackets nested inside a call payload', () => {
    // A nested object with its own "method" key must not be mistaken for
    // the call's tag, which serde always writes first.
    const recording =
      '{"calls":[{"method":"api_dispatch","frames":[[1,2],[3,4]],"note":"a, b","inner":{"method":"not_a_step"}},{"method":"sketch_end_curve","sketch":4294967297}],"golden_hash":0}'
    const summary = extractRecordingSummary(recording)
    expect(summary.stepCount).toBe(2)
    expect(summary.lastStepNames).toEqual(['api_dispatch', 'sketch_end_curve'])
  })
})

describe('scrubHomeDir', () => {
  it('replaces every occurrence of the home directory with ~ (macOS/Linux, raw form)', () => {
    const text = 'opened /Users/kurt/models/a.hew and /Users/kurt/models/b.hew'
    expect(scrubHomeDir(text, '/Users/kurt')).toBe('opened ~/models/a.hew and ~/models/b.hew')
    const linuxText = 'opened /home/kurt/models/a.hew'
    expect(scrubHomeDir(linuxText, '/home/kurt')).toBe('opened ~/models/a.hew')
  })

  it('is a no-op with a null or empty home directory', () => {
    expect(scrubHomeDir('/Users/kurt/x', null)).toBe('/Users/kurt/x')
    expect(scrubHomeDir('/Users/kurt/x', '')).toBe('/Users/kurt/x')
  })

  // Regression: a Windows path inside real NDJSON (JSON.stringify per
  // record) carries DOUBLED backslashes — a literal split on the raw
  // "C:\Users\kurt" string never matches "C:\\Users\\kurt" in the log text,
  // so the username shipped in every Windows report.
  it('scrubs a Windows home directory out of real toNDJSON output', () => {
    Diag.clear()
    Diag.logUi('test', 'INFO', { path: 'C:\\Users\\kurt\\Documents\\broken.hew' })
    const ndjson = Diag.toNDJSON(Diag.getRecords())
    // Prove the raw path really does appear doubled-up in the JSON text,
    // so this test would have caught the old literal-only scrub.
    expect(ndjson).toContain('C:\\\\Users\\\\kurt')
    const scrubbed = scrubHomeDir(ndjson, 'C:\\Users\\kurt')
    expect(scrubbed).not.toContain('kurt')
    expect(scrubbed).toContain('~\\\\Documents\\\\broken.hew')
  })

  it('also scrubs a forward-slash rendering of a Windows home directory', () => {
    const text = 'failed to open C:/Users/kurt/Documents/broken.hew'
    expect(scrubHomeDir(text, 'C:\\Users\\kurt')).toBe('failed to open ~/Documents/broken.hew')
  })

  it('scrubs the raw form even when it never goes through JSON.stringify', () => {
    expect(scrubHomeDir('plain text with /Users/kurt inside', '/Users/kurt')).toBe(
      'plain text with ~ inside',
    )
  })
})

describe('detectPlatform', () => {
  it('combines the host and OS', () => {
    expect(detectPlatform(true, true, false, false)).toBe('desktop-macos')
    expect(detectPlatform(false, false, true, false)).toBe('web-windows')
    expect(detectPlatform(true, false, false, true)).toBe('desktop-linux')
    expect(detectPlatform(false, false, false, false)).toBe('web-other')
  })
})

describe('buildReportBundle', () => {
  const baseOpts = {
    scene: fakeScene(),
    documentName: 'My Model.hew',
    platform: 'desktop-macos',
    fields: { description: 'it crashed', expected: 'it should not', contact: '' },
    debugMode: false,
    crash: null,
    homeDir: null,
  }

  it('is format 1 and includes every ticked row', async () => {
    const bundle = await saved({ ...baseOpts, flags: ALL_ON })
    expect(bundle.format).toBe(1)
    expect(bundle.report).toEqual({ description: 'it crashed', expected: 'it should not' })
    expect(bundle.system).toBeDefined()
    expect(bundle.recording).toBe('{"version":2,"calls":[],"golden_hash":0}')
    expect(bundle.hew).toBe(btoa(String.fromCharCode(1, 2, 3)))
    expect(typeof bundle.log).toBe('string')
    // input is gated by debugMode, which is false here even with the flag on.
    expect(bundle.input).toBeUndefined()
  })

  it('writes format, report, and system first, the head the intake service validates', async () => {
    expect(Object.keys(await saved({ ...baseOpts, flags: ALL_ON })).slice(0, 3)).toEqual(['format', 'report', 'system'])
  })

  it('omits contact when blank, includes it when filled in', async () => {
    expect((await saved({ ...baseOpts, flags: ALL_ON })).report.contact).toBeUndefined()

    const withContact = await saved({
      ...baseOpts,
      fields: { ...baseOpts.fields, contact: 'me@example.com' },
      flags: ALL_ON,
    })
    expect(withContact.report.contact).toBe('me@example.com')
  })

  it('omits every unticked row rather than nulling it', async () => {
    const bundle = await saved({ ...baseOpts, flags: ALL_OFF })
    expect(bundle.system).toBeUndefined()
    expect(bundle.recording).toBeUndefined()
    expect(bundle.hew).toBeUndefined()
    expect(bundle.log).toBeUndefined()
    expect(bundle.input).toBeUndefined()
    const result = await buildReportBundle({ ...baseOpts, flags: ALL_OFF })
    expect(result.rows.system.included).toBe(false)
    expect(result.rows.model.included).toBe(false)
  })

  it('reports a row size even when unticked, so the checklist can show it', async () => {
    const result = await buildReportBundle({ ...baseOpts, flags: ALL_OFF })
    expect(result.rows.model.bytes).toBe(4)
    expect(result.rows.recording.bytes).toBeGreaterThan(0)
  })

  it('never includes input events outside Debug Mode, even when ticked', async () => {
    inputRecorder.start()
    inputRecorder.recordPointer('pointerdown', 1, 2, {
      button: 0,
      buttons: 1,
      shiftKey: false,
      altKey: false,
      ctrlKey: false,
      metaKey: false,
    } as PointerEvent)

    const result = await buildReportBundle({ ...baseOpts, flags: ALL_ON, debugMode: false })
    expect((await saved({ ...baseOpts, flags: ALL_ON, debugMode: false })).input).toBeUndefined()
    expect(result.rows.input.included).toBe(false)
    expect(result.rows.input.disabledReason).toBe('only recorded in Debug Mode')

    const withDebug = await buildReportBundle({ ...baseOpts, flags: ALL_ON, debugMode: true })
    expect((await saved({ ...baseOpts, flags: ALL_ON, debugMode: true })).input).toHaveLength(1)
    expect(withDebug.rows.input.included).toBe(true)
  })

  it('peeks the recording rather than taking it (repeat builds see the same data)', async () => {
    let peeked = 0
    const scene = fakeScene({
      peek_recording: () => {
        peeked++
        return '{"version":2,"calls":["Foo"],"golden_hash":0}'
      },
    })
    await buildReportBundle({ ...baseOpts, scene, flags: ALL_ON })
    await buildReportBundle({ ...baseOpts, scene, flags: ALL_ON })
    expect(peeked).toBe(2)
  })

  it('scrubs the home directory from the log BEFORE the preview is computed', async () => {
    Diag.logUi('test', 'INFO', { path: '/Users/kurt/models/broken.hew' })
    const opts = { ...baseOpts, flags: ALL_ON, homeDir: '/Users/kurt' }
    const bundle = await saved(opts)
    expect(bundle.log).not.toContain('/Users/kurt/')
    expect(bundle.log).toContain('~/models/broken.hew')
    const result = await buildReportBundle(opts)
    expect(result.rows.log.preview).not.toContain('/Users/kurt/')
    expect(result.rows.log.preview).toContain('~/models/broken.hew')
  })

  it('scrubs the home directory out of the crash message, not just the log', async () => {
    const bundle = await saved({
      ...baseOpts,
      scene: null,
      flags: ALL_ON,
      homeDir: 'C:\\Users\\kurt',
      crash: {
        at: '2026-01-01T00:00:00.000Z',
        message: 'failed to read C:\\Users\\kurt\\Documents\\broken.hew: not found',
        recording: null,
      },
    })
    expect(bundle.crash.message).not.toContain('kurt')
    expect(bundle.crash.message).toContain('~\\Documents\\broken.hew')
  })

  it('scrubs the home directory out of the document name shown in the model preview', async () => {
    const result = await buildReportBundle({
      ...baseOpts,
      documentName: '/Users/kurt/models/My Model.hew',
      flags: ALL_ON,
      homeDir: '/Users/kurt',
    })
    expect(result.rows.model.preview).not.toContain('/Users/kurt')
    expect(result.rows.model.preview).toContain('~/models/My Model.hew')
  })

  describe('saved file text', () => {
    // Everything that can go wrong at a piece boundary: multi-byte UTF-8,
    // surrogate pairs, escaped quotes and backslashes, and a lone surrogate
    // JSON.stringify escapes.
    const recording =
      '{"version":2,"calls":[{"method":"api_dispatch","note":"q\\"🎉a🎉🎉 é \\\\ \ud800 z"}],"golden_hash":18192258159662307868}'
    const model = new Uint8Array([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
    const opts = {
      ...baseOpts,
      scene: fakeScene({ save: () => model, peek_recording: () => recording }),
      fields: { description: 'Ünïcode 🎉 description', expected: '', contact: 'x@example.com' },
      flags: ALL_ON,
    }

    it('is exactly JSON.stringify of the bundle, whatever the piece size', async () => {
      Diag.logUi('test', 'INFO', { note: '🎉 ü "quoted"' })
      const text = await savedText(opts)
      const bundle = JSON.parse(text)
      expect(JSON.stringify(bundle)).toBe(text)
      expect(bundle.recording).toBe(recording)
      expect(bundle.hew).toBe(btoa(String.fromCharCode(...model)))
      expect(bundle.report.description).toBe('Ünïcode 🎉 description')
      expect(bundle.log).toContain('🎉 ü')
      for (const segmentChars of [1, 2, 5, 7]) {
        expect(await savedText({ ...opts, segmentChars })).toBe(text)
      }
    })

    it('uploads the same text when written in small pieces', async () => {
      const text = await savedText(opts)
      const upload = await buildReportUpload({ ...opts, segmentChars: 5 })
      expect(await gunzipText(upload.gzip!)).toBe(text)
    })
  })

  describe('imported files', () => {
    const skpBytes = '255,254,255,14,83'
    const scene = fakeScene({ peek_recording: () => importRecording(skpBytes) })

    it('lists the embedded file and includes it with the steps by default', async () => {
      const result = await buildReportBundle({ ...baseOpts, scene, flags: ALL_ON })
      expect(result.importedFiles).toEqual([expect.objectContaining({ method: 'import_skp', fileBytes: 5 })])
      expect(result.rows.imports.included).toBe(true)
      expect(result.rows.imports.bytes).toBe(5)
      expect(result.rows.imports.preview).toContain('SketchUp model (.skp)')
      const bundle = await saved({ ...baseOpts, scene, flags: ALL_ON })
      expect(bundle.recording).toContain(`"bytes":[${skpBytes}]`)
      expect(bundle.importsStripped).toBeUndefined()
    })

    it('strips the file bytes when the row is unticked, keeping u64 values exact', async () => {
      const opts = { ...baseOpts, scene, flags: { ...ALL_ON, imports: false } }
      expect((await buildReportBundle(opts)).rows.imports.included).toBe(false)
      const bundle = await saved(opts)
      expect(bundle.importsStripped).toBe(true)
      expect(bundle.recording).toBe(importRecording(''))
      expect(bundle.recording).toContain('"golden_hash":18192258159662307868')
    })

    it('ties the Imported files row to the recorded steps', async () => {
      const result = await buildReportBundle({ ...baseOpts, scene, flags: { ...ALL_ON, recording: false } })
      expect(result.rows.imports.included).toBe(false)
      expect(result.rows.imports.disabledReason).toBe('sent only with recorded steps')
    })

    it('withholds the steps entirely when they cannot be scanned for imports', async () => {
      const broken = fakeScene({ peek_recording: () => '{"version":2,"calls":[{"method":"import_skp","bytes":[1,2' })
      const opts = { ...baseOpts, scene: broken, flags: ALL_ON }
      expect((await saved(opts)).recording).toBeUndefined()
      const result = await buildReportBundle(opts)
      expect(result.rows.recording.included).toBe(false)
      expect(result.rows.recording.disabledReason).toBe('couldn’t separate imported files from the steps')
    })

    it('reports no imports for a recording without import steps', async () => {
      const result = await buildReportBundle({ ...baseOpts, flags: ALL_ON })
      expect(result.importedFiles).toEqual([])
      expect(result.rows.imports.disabledReason).toBe('no imported files')
    })
  })

  describe('send payload', () => {
    const UNCAPPED = Number.MAX_SAFE_INTEGER

    it('uploads exactly the saved file when it fits, at the size the preview showed', async () => {
      const opts = { ...baseOpts, flags: ALL_ON }
      const upload = await buildReportUpload(opts)
      expect(upload.unavailableReason).toBeNull()
      expect(upload.gzip).not.toBeNull()
      expect(upload.compressedBytes).toBe(upload.gzip!.byteLength)
      expect((await buildReportBundle(opts)).send.compressedBytes).toBe(upload.compressedBytes)
      expect(await gunzipText(upload.gzip!)).toBe(await savedText(opts))
    })

    it('drops the model first when the compressed report passes the cap, and still saves it', async () => {
      const model = randomBytes(200_000)
      const byteList = randomByteList(2_000)
      const scene = fakeScene({ save: () => model, peek_recording: () => importRecording(byteList) })
      const full = await buildReportBundle({ ...baseOpts, scene, flags: ALL_ON, maxSendBytes: UNCAPPED })
      const noModel = await buildReportBundle({
        ...baseOpts,
        scene,
        flags: { ...ALL_ON, model: false },
        maxSendBytes: UNCAPPED,
      })
      const cap = Math.floor((full.send.compressedBytes! + noModel.send.compressedBytes!) / 2)
      const opts = { ...baseOpts, scene, flags: ALL_ON, maxSendBytes: cap }

      const result = await buildReportBundle(opts)
      expect(result.rows.model.droppedFromSend).toBe(true)
      expect(result.rows.imports.droppedFromSend).toBe(false)
      const sent = JSON.parse(await gunzipText((await buildReportUpload(opts)).gzip!))
      expect(sent.hew).toBeUndefined()
      expect(sent.recording).toContain(`"bytes":[${byteList}]`)
      expect((await saved(opts)).hew).toBeDefined()
    })

    it('then drops imported files, stripping them from the sent steps only', async () => {
      const byteList = randomByteList(60_000)
      const scene = fakeScene({ peek_recording: () => importRecording(byteList) })
      const noModel = await buildReportBundle({
        ...baseOpts,
        scene,
        flags: { ...ALL_ON, model: false },
        maxSendBytes: UNCAPPED,
      })
      const bare = await buildReportBundle({
        ...baseOpts,
        scene,
        flags: { ...ALL_ON, model: false, imports: false },
        maxSendBytes: UNCAPPED,
      })
      const cap = Math.floor((noModel.send.compressedBytes! + bare.send.compressedBytes!) / 2)
      const opts = { ...baseOpts, scene, flags: ALL_ON, maxSendBytes: cap }

      const result = await buildReportBundle(opts)
      expect(result.rows.model.droppedFromSend).toBe(true)
      expect(result.rows.imports.droppedFromSend).toBe(true)
      const sent = JSON.parse(await gunzipText((await buildReportUpload(opts)).gzip!))
      expect(sent.importsStripped).toBe(true)
      expect(sent.recording).toBe(importRecording(''))
      expect((await saved(opts)).recording).toContain(`"bytes":[${byteList}]`)
    })

    it('refuses to send a report still over the cap after dropping both', async () => {
      const opts = { ...baseOpts, flags: ALL_ON, maxSendBytes: 10 }
      const upload = await buildReportUpload(opts)
      expect(upload.gzip).toBeNull()
      expect(upload.unavailableReason).toMatch(/over the .* limit/)
      expect((await buildReportBundle(opts)).send.unavailableReason).toBe(upload.unavailableReason)
      expect(await savedText(opts)).toContain('"format":1')
    })

    it('explains when this runtime cannot compress', async () => {
      vi.stubGlobal('CompressionStream', undefined)
      const opts = { ...baseOpts, flags: ALL_ON }
      const upload = await buildReportUpload(opts)
      expect(upload.gzip).toBeNull()
      expect(upload.compressedBytes).toBeNull()
      expect(upload.unavailableReason).toMatch(/can’t compress/)
      expect((await buildReportBundle(opts)).send.compressedBytes).toBeNull()
      expect(await savedText(opts)).toContain('"format":1')
    })
  })

  describe('gathering once', () => {
    it('builds the rows, the size, the file, and the upload from one gather', async () => {
      let peeked = 0
      const scene = fakeScene({
        peek_recording: () => {
          peeked++
          return importRecording('1,2,3')
        },
      })
      const content = await gatherReport({ ...baseOpts, scene })
      const opts = { fields: baseOpts.fields, flags: ALL_ON }
      expect(describeReport(content, ALL_ON).rows.imports.included).toBe(true)
      expect(describeReport(content, { ...ALL_ON, imports: false }).rows.imports.included).toBe(false)
      await measureUpload(content, opts)
      const file = JSON.parse(new TextDecoder().decode(await encodeReportFile(content, opts)))
      const upload = await compressReportUpload(content, opts)
      expect(peeked).toBe(1)
      expect(file.recording).toBe(importRecording('1,2,3'))
      expect(JSON.parse(await gunzipText(upload.gzip!))).toEqual(file)
    })

    it('marks the rows a measurement dropped', async () => {
      const content = await gatherReport(baseOpts)
      const rows = describeReport(content, ALL_ON, ['model']).rows
      expect(rows.model.droppedFromSend).toBe(true)
      expect(rows.imports.droppedFromSend).toBe(false)
    })

    it('stops gathering when its signal aborts, before saving the model', async () => {
      const controller = new AbortController()
      let saved = 0
      const scene = fakeScene({
        peek_recording: () => {
          controller.abort()
          return importRecording('1,2')
        },
        save: () => {
          saved++
          return new Uint8Array([1])
        },
      })
      await expect(gatherReport({ ...baseOpts, scene, signal: controller.signal })).rejects.toBeInstanceOf(
        ReportBuildCancelled,
      )
      expect(saved).toBe(0)
    })

    it('stops a measurement or a file encoding whose signal aborted', async () => {
      const content = await gatherReport(baseOpts)
      const controller = new AbortController()
      controller.abort()
      const opts = { fields: baseOpts.fields, flags: ALL_ON, signal: controller.signal }
      await expect(measureUpload(content, opts)).rejects.toBeInstanceOf(ReportBuildCancelled)
      await expect(encodeReportFile(content, opts)).rejects.toBeInstanceOf(ReportBuildCancelled)
    })
  })

  describe('file that didn’t open', () => {
    const skpBytes = new Uint8Array([0xff, 0xfe, 0xff, 0x0e, 0x53, 0x00])
    const failed: FailedOpen = {
      name: 'Kitchen.skp',
      label: 'SketchUp model (.skp)',
      bytes: skpBytes,
      error: 'SKP: bad section at /Users/kurt/Kitchen.skp',
    }

    it('carries the file byte for byte, with its name, kind, size, and error', async () => {
      const bundle = await saved({ ...baseOpts, failedOpen: failed, homeDir: '/Users/kurt', flags: ALL_ON })
      expect(bundle.failedFile).toEqual({
        name: 'Kitchen.skp',
        label: 'SketchUp model (.skp)',
        size: skpBytes.byteLength,
        error: 'SKP: bad section at ~/Kitchen.skp',
        bytes: btoa(String.fromCharCode(...skpBytes)),
      })
    })

    it('writes the name, kind, size, and error ahead of the bytes', async () => {
      const text = await savedText({ ...baseOpts, failedOpen: failed, flags: ALL_ON, segmentChars: 4 })
      const at = text.indexOf('"failedFile":')
      expect(text.slice(at)).toMatch(/^"failedFile":\{"name":"Kitchen\.skp","label":"[^"]*","size":6,"error":"[^"]*","bytes":"/)
    })

    it('keeps the file across piece boundaries', async () => {
      const big = randomBytes(10_000)
      const bundle = await saved({ ...baseOpts, failedOpen: { ...failed, bytes: big }, flags: ALL_ON, segmentChars: 64 })
      expect(Uint8Array.from(atob(bundle.failedFile.bytes), (ch) => ch.charCodeAt(0))).toEqual(big)
    })

    it('says Hew stopped mid-open when the attempt never reported an error', async () => {
      const result = await buildReportBundle({ ...baseOpts, failedOpen: { ...failed, error: null }, flags: ALL_ON })
      expect(result.rows.failedFile.preview).toContain('Hew stopped while opening it.')
      expect((await saved({ ...baseOpts, failedOpen: { ...failed, error: null }, flags: ALL_ON })).failedFile.error).toBeNull()
    })

    it('is left out when unticked, and unavailable when no file failed', async () => {
      expect((await saved({ ...baseOpts, failedOpen: failed, flags: { ...ALL_ON, failedFile: false } })).failedFile).toBeUndefined()
      const none = await buildReportBundle({ ...baseOpts, failedOpen: null, flags: ALL_ON })
      expect(none.rows.failedFile.included).toBe(false)
      expect(none.rows.failedFile.disabledReason).toBe('no file failed to open')
      expect((await saved({ ...baseOpts, failedOpen: null, flags: ALL_ON })).failedFile).toBeUndefined()
    })

    it('reads the latest failed attempt when the caller passes none', async () => {
      failedOpenStore.beginOpenAttempt({ name: 'Den.skp', label: 'SketchUp model (.skp)', bytes: skpBytes })
      failedOpenStore.openAttemptFailed('SKP: no')
      expect((await saved({ ...baseOpts, flags: ALL_ON })).failedFile.name).toBe('Den.skp')
    })

    it('rides along in crash mode, where the model can’t', async () => {
      const bundle = await saved({
        ...baseOpts,
        scene: null,
        crash: { at: '2026-01-01T00:00:00.000Z', message: 'recursive use of an object', recording: null },
        failedOpen: { ...failed, error: null },
        flags: ALL_ON,
      })
      expect(bundle.hew).toBeUndefined()
      expect(bundle.failedFile.bytes).toBe(btoa(String.fromCharCode(...skpBytes)))
      expect(Object.keys(bundle).slice(0, 4)).toEqual(['format', 'report', 'system', 'crash'])
    })

    it('is the last row Send drops, after the model and the imported files', async () => {
      const UNCAPPED = Number.MAX_SAFE_INTEGER
      const scene = fakeScene({ save: () => randomBytes(50_000), peek_recording: () => importRecording(randomByteList(20_000)) })
      const file = { ...failed, bytes: randomBytes(50_000) }
      const opts = { ...baseOpts, scene, failedOpen: file, maxSendBytes: UNCAPPED }
      const keepsFile = await buildReportBundle({ ...opts, flags: { ...ALL_ON, model: false, imports: false } })
      const bare = await buildReportBundle({ ...opts, flags: { ...ALL_ON, model: false, imports: false, failedFile: false } })

      // A cap the file alone fits under once the model and imports are gone
      // (with room for the `dropped` marker naming those two).
      const fits = keepsFile.send.compressedBytes! + 128
      const kept = await buildReportBundle({ ...opts, flags: ALL_ON, maxSendBytes: fits })
      expect(kept.rows.model.droppedFromSend).toBe(true)
      expect(kept.rows.imports.droppedFromSend).toBe(true)
      expect(kept.rows.failedFile.droppedFromSend).toBe(false)
      const sent = JSON.parse(await gunzipText((await buildReportUpload({ ...opts, flags: ALL_ON, maxSendBytes: fits })).gzip!))
      expect(sent.failedFile.size).toBe(50_000)

      // A cap only the bare report fits under drops the file too, but Save keeps it.
      const cap = Math.floor((keepsFile.send.compressedBytes! + bare.send.compressedBytes!) / 2)
      const dropped = await buildReportBundle({ ...opts, flags: ALL_ON, maxSendBytes: cap })
      expect(dropped.rows.failedFile.droppedFromSend).toBe(true)
      expect(dropped.send.unavailableReason).toBeNull()
      expect((await saved({ ...opts, flags: ALL_ON, maxSendBytes: cap })).failedFile.size).toBe(50_000)
    })

    it('sent without the file, the report still says what the file was and that it was dropped', async () => {
      const scene = fakeScene({ save: () => randomBytes(50_000), peek_recording: () => importRecording(randomByteList(20_000)) })
      const file = { ...failed, bytes: randomBytes(50_000) }
      const opts = { ...baseOpts, scene, failedOpen: file }
      const bare = await buildReportBundle({ ...opts, flags: { ...ALL_ON, model: false, imports: false, failedFile: false } })
      const cap = bare.send.compressedBytes! + 256

      const sent = JSON.parse(await gunzipText((await buildReportUpload({ ...opts, flags: ALL_ON, maxSendBytes: cap })).gzip!))
      expect(sent.dropped).toEqual(['model', 'imports', 'failedFile'])
      expect(sent.failedFile).toEqual({
        name: 'Kitchen.skp',
        label: 'SketchUp model (.skp)',
        size: 50_000,
        error: failed.error,
        omitted: 'too large to send',
      })
      expect(sent.hew).toBeUndefined()
      expect(sent.importsStripped).toBe(true)
      // `dropped` sits in the head, right after the fixed-order keys.
      expect(Object.keys(sent).slice(0, 3)).toEqual(['format', 'report', 'system'])
      expect(Object.keys(sent)[3]).toBe('dropped')

      // A report that fits, and every saved file, carries neither marker.
      const whole = await saved({ ...opts, flags: ALL_ON, maxSendBytes: cap })
      expect(whole.dropped).toBeUndefined()
      expect(whole.failedFile.omitted).toBeUndefined()
      expect(whole.failedFile.bytes).toBeTypeOf('string')
      const fits = JSON.parse(await gunzipText((await buildReportUpload({ ...opts, flags: ALL_ON, maxSendBytes: Number.MAX_SAFE_INTEGER })).gzip!))
      expect(fits.dropped).toBeUndefined()
      expect(fits.failedFile.omitted).toBeUndefined()
    })
  })

  describe('crash mode', () => {
    const crashOpts = {
      ...baseOpts,
      scene: null,
      crash: { at: '2026-01-01T00:00:00.000Z', message: 'panicked at document.rs:1', recording: '{"version":2,"calls":["Foo"],"golden_hash":0}' },
    }

    it('never includes hew, marks the model row unavailable, and adds a crash block', async () => {
      const bundle = await saved({ ...crashOpts, flags: ALL_ON })
      expect(bundle.hew).toBeUndefined()
      expect(bundle.crash).toEqual({ at: '2026-01-01T00:00:00.000Z', message: 'panicked at document.rs:1' })
      const result = await buildReportBundle({ ...crashOpts, flags: ALL_ON })
      expect(result.rows.model.included).toBe(false)
      expect(result.rows.model.disabledReason).toBe('unavailable after a crash')
    })

    it('writes the crash block right after system, inside the head the intake service reads', async () => {
      expect(Object.keys(await saved({ ...crashOpts, flags: ALL_ON }))).toEqual([
        'format',
        'report',
        'system',
        'crash',
        'recording',
        'log',
      ])
      expect(Object.keys(await saved({ ...crashOpts, flags: { ...ALL_ON, system: false } })).slice(0, 3)).toEqual([
        'format',
        'report',
        'crash',
      ])
    })

    it('sources recording from the panic capture, not a live scene', async () => {
      expect((await saved({ ...crashOpts, flags: ALL_ON })).recording).toBe('{"version":2,"calls":["Foo"],"golden_hash":0}')
    })
  })

  it('never throws when the scene throws on every method', async () => {
    const scene = fakeScene({
      save: () => {
        throw new Error('save boom')
      },
      peek_recording: () => {
        throw new Error('peek boom')
      },
      object_ids: () => {
        throw new Error('ids boom')
      },
    })
    await expect(buildReportBundle({ ...baseOpts, scene, flags: ALL_ON })).resolves.toBeDefined()
    await expect(buildReportUpload({ ...baseOpts, scene, flags: ALL_ON })).resolves.toBeDefined()
    const bundle = await saved({ ...baseOpts, scene, flags: ALL_ON })
    expect(bundle.hew).toBeUndefined()
    expect(bundle.recording).toBeUndefined()
  })
})

describe('scrubHomeDir: home-shaped paths that are not this user\'s own', () => {
  it('redacts another account\'s macOS or Linux home path, in prose and inside NDJSON', () => {
    expect(scrubHomeDir('read_file failed for "/Users/alice/Desktop/a.hew": denied', '/Users/kurt')).toBe(
      'read_file failed for "~/Desktop/a.hew": denied',
    )
    expect(scrubHomeDir('/home/bob/models/x.hew', '/Users/kurt')).toBe('~/models/x.hew')
    expect(scrubHomeDir('{"path":"/Users/alice/a.hew"}', '/Users/kurt')).toBe('{"path":"~/a.hew"}')
  })

  it('redacts a Windows home path in any letter case, JSON-escaped, or extended-length form', () => {
    // JSON-escaped (doubled backslashes), and not this user's spelling.
    expect(scrubHomeDir('failed for "c:\\\\users\\\\Kurt\\\\Documents\\\\b.hew"', 'C:\\Users\\kurt')).toBe(
      'failed for "~\\\\Documents\\\\b.hew"',
    )
    // Extended-length prefix, raw backslashes, another account.
    expect(scrubHomeDir('\\\\?\\C:\\Users\\dave\\x.hew', 'C:\\Users\\kurt')).toBe('~\\x.hew')
    // Forward-slash spelling, another drive.
    expect(scrubHomeDir('D:/Users/erin/y.hew', 'C:\\Users\\kurt')).toBe('~/y.hew')
  })

  it('leaves the web build alone: no home dir means no filesystem paths to scrub', () => {
    expect(scrubHomeDir('/Users/alice/x', null)).toBe('/Users/alice/x')
  })

  it('does not touch a path that merely contains a home-like segment mid-path', () => {
    expect(scrubHomeDir('/opt/Users/x/y', '/Users/kurt')).toBe('/opt/Users/x/y')
    expect(scrubHomeDir('see https://example.com/home/page', '/Users/kurt')).toBe('see https://example.com/home/page')
  })
})

describe('follow-up bundles', () => {
  const att = { kind: 'attachment' as const, name: 'kitchen.skp', label: 'SketchUp model (.skp)', bytes: new Uint8Array([1, 2, 3, 250]) }
  const base = { reportId: 'HEW-7K3F-Q9XB', note: '', appVersion: '1.3.1', platform: 'web-linux' }

  it('carries the attachment key after report and system, with base64 bytes', async () => {
    const { encodeFollowUpBundle } = await import('./reportBundle')
    const text = new TextDecoder().decode(encodeFollowUpBundle({ ...base, file: att }))
    const obj = JSON.parse(text)
    expect(Object.keys(obj)).toEqual(['format', 'report', 'system', 'attachment'])
    expect(obj.report.description).toBe('File for report HEW-7K3F-Q9XB: kitchen.skp')
    expect(obj.attachment).toEqual({ name: 'kitchen.skp', label: 'SketchUp model (.skp)', size: 4, bytes: 'AQID+g==' })
  })

  it('appends the note and uses the failedFile row for a file that did not open', async () => {
    const { encodeFollowUpBundle } = await import('./reportBundle')
    const failed = { name: 'bad.skp', label: 'SketchUp model (.skp)', bytes: new Uint8Array([9, 9]), error: 'boom' }
    const obj = JSON.parse(new TextDecoder().decode(encodeFollowUpBundle({ ...base, note: ' here you go ', file: { kind: 'failedFile', failed }, segmentChars: 4 })))
    expect(obj.report.description).toBe('File for report HEW-7K3F-Q9XB: bad.skp\n\nhere you go')
    expect(obj.attachment).toBeUndefined()
    expect(obj.failedFile).toMatchObject({ name: 'bad.skp', size: 2, error: 'boom', bytes: 'CQk=' })
  })

  it('builds a gzip upload, or says why not when over the cap', async () => {
    const { buildFollowUpUpload } = await import('./reportBundle')
    const ok = await buildFollowUpUpload({ ...base, file: att })
    expect(ok.gzip).not.toBeNull()
    expect(ok.gzip![0]).toBe(0x1f)
    const big = await buildFollowUpUpload({ ...base, file: att, maxSendBytes: 10 })
    expect(big.gzip).toBeNull()
    expect(big.unavailableReason).not.toBeNull()
  })
})
