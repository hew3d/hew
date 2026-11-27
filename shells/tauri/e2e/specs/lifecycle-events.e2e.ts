/**
 * Desktop: document lifecycle events over the live socket
 * (docs/agents/HEW_API.md §4.5). A listener attached to the first window
 * subscribes to `document_opened`; a second connection opens a file into a
 * NEW window, and the listener must hear of it exactly once although it
 * holds no session in that window. Wiring proof for the real shell: the
 * wasm drain, the event pump, and the shell's fan-out have their own suites.
 */

import { connect, type Socket } from 'node:net'
import { existsSync, mkdtempSync, readdirSync, readFileSync, readlinkSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export {}

type Vec3 = [number, number, number]

interface HarnessSlice {
  isReady(): boolean
  drawBox(p0: Vec3, p1: Vec3, height: number): string
}
type HarnessWindow = Window & { __hew_test?: HarnessSlice }

interface Frame {
  id?: number
  method?: string
  params?: Record<string, unknown>
  result?: Record<string, unknown>
  error?: unknown
}

/** The discovery file of the app under test: the newest instance whose
 * process is the desktop binary this run launched. */
function discover(): { socket: string; token: string } {
  const runtime = process.env.HEW_RUNTIME_DIR ?? process.env.XDG_RUNTIME_DIR ?? `/tmp/hew-run-${process.getuid?.()}`
  const dir = join(runtime, 'hew')
  const app = process.env.HEW_DESKTOP_BIN
  const candidates = readdirSync(dir)
    .filter((f) => /^instance-\d+\.json$/.test(f))
    .map((f) => JSON.parse(readFileSync(join(dir, f), 'utf8')) as { socket: string; token: string; pid: number })
    .filter((d) => existsSync(`/proc/${d.pid}`) && (app === undefined || readlinkSafe(`/proc/${d.pid}/exe`) === app))
    .sort((a, b) => b.pid - a.pid)
  if (candidates.length === 0) throw new Error(`no live Hew instance under ${dir}`)
  return candidates[0]
}

function readlinkSafe(path: string): string | undefined {
  try {
    return readlinkSync(path)
  } catch {
    return undefined
  }
}

class Client {
  private buffer = ''
  readonly frames: Frame[] = []
  private nextId = 1
  private constructor(private readonly sock: Socket) {
    sock.setEncoding('utf8')
    sock.on('data', (chunk: string) => {
      this.buffer += chunk
      let at: number
      while ((at = this.buffer.indexOf('\n')) >= 0) {
        const line = this.buffer.slice(0, at)
        this.buffer = this.buffer.slice(at + 1)
        if (line.trim() !== '') this.frames.push(JSON.parse(line) as Frame)
      }
    })
  }

  static async open(): Promise<Client> {
    const { socket, token } = discover()
    const sock = connect(socket)
    await new Promise<void>((res, rej) => {
      sock.once('connect', () => res())
      sock.once('error', rej)
    })
    const client = new Client(sock)
    await client.call('hew.meta.hello', { protocol: 1, token })
    return client
  }

  async call(method: string, params: Record<string, unknown> = {}): Promise<Frame> {
    const id = this.nextId++
    this.sock.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`)
    const deadline = Date.now() + 60_000
    for (;;) {
      const reply = this.frames.find((f) => f.id === id)
      if (reply !== undefined) return reply
      if (Date.now() > deadline) throw new Error(`${method} never answered`)
      await new Promise((r) => setTimeout(r, 50))
    }
  }

  events(name: string): Frame[] {
    return this.frames.filter((f) => f.method === `hew.event.${name}`)
  }

  close(): void {
    this.sock.destroy()
  }
}

describe('Hew desktop: document lifecycle events over the socket', () => {
  it('a listener bound to one window hears a file opened into another', async () => {
    await browser.execute(() => localStorage.setItem('hew.settings.showWelcome', 'false'))
    await browser.refresh()
    await browser.waitUntil(
      async () => (await browser.execute(() => (window as HarnessWindow).__hew_test?.isReady() === true)) === true,
      { timeout: 30_000, timeoutMsg: 'harness/kernel never became ready' },
    )
    await browser.execute(() => (window as HarnessWindow).__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))

    const scratch = mkdtempSync(join(tmpdir(), 'hew-lifecycle-'))
    const file = join(scratch, 'lifecycle.hew')
    const listener = await Client.open()
    const driver = await Client.open()
    try {
      const saved = await driver.call('hew.doc.save', { path: file, copy: true })
      expect(saved.error).toBeUndefined()
      const first = ((await listener.call('hew.meta.documents')).result?.documents ?? []) as { id: string }[]
      expect(first.length).toBe(1)

      const sub = await listener.call('hew.event.subscribe', { events: ['document_opened', 'document_closed'] })
      expect(sub.result?.subscribed).toEqual(['document_closed', 'document_opened'])

      const opened = await driver.call('hew.doc.open', { path: file })
      expect(opened.error).toBeUndefined()
      await browser.waitUntil(() => listener.events('document_opened').length > 0, {
        timeout: 15_000,
        timeoutMsg: 'the listener never heard document_opened for the other window',
      })
      // Let a duplicate, if there were one, arrive.
      await new Promise((r) => setTimeout(r, 1_000))
      const heard = listener.events('document_opened')
      expect(heard.length).toBe(1)
      expect(heard[0].params?.document).not.toBe(first[0].id)
      expect(heard[0].params?.title).toContain('lifecycle')
      expect(heard[0].params?.path).toBe(file)
    } finally {
      listener.close()
      driver.close()
      rmSync(scratch, { recursive: true, force: true })
    }
  })
})
