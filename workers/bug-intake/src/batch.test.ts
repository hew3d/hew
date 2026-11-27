/**
 * The admin list's batch actions (checkbox list, selection presets, batch
 * triage, two-step batch delete, streamed tar download) and the admin host
 * rule, end to end through `handleRequest` on the in-process Worker from
 * `testSupport/testEnv.ts`, plus the tar writer on its own.
 */

import { test, describe } from 'node:test'
import assert from 'node:assert/strict'

import { buildTestEnv, type TestEnv } from './testSupport/testEnv.ts'
import { tarHeader, tarPadding, tarStream, isSafeTarName, type TarSource } from './tar.ts'
import { renderList } from './adminPages.ts'
import { PIECE_BYTES, BATCH_TRIAGE_MAX, BATCH_DELETE_MAX, BATCH_DOWNLOAD_MAX, BATCH_DOWNLOAD_MAX_BYTES } from './constants.ts'

const PUBLIC_BASE = 'https://app.hew3d.com'
const ADMIN_BASE = 'https://reports.hew3d.com'
const ADMIN = `${ADMIN_BASE}/report/admin`

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

async function gzipJson(doc: unknown): Promise<Uint8Array> {
  const cs = new CompressionStream('gzip')
  const writer = cs.writable.getWriter()
  const out = new Response(cs.readable).arrayBuffer()
  await writer.write(new TextEncoder().encode(JSON.stringify(doc)))
  await writer.close()
  return new Uint8Array(await out)
}

let ipCounter = 0

async function startReport(t: TestEnv, extra: Record<string, string>, description: string) {
  const body = await gzipJson({ format: 1, report: { description }, system: { appVersion: '1.5.0', platform: 'desktop-macos' } })
  ipCounter++
  const res = await t.handle(
    new Request(`${PUBLIC_BASE}/report/`, {
      method: 'POST',
      body,
      headers: {
        'content-type': 'application/gzip',
        'content-length': String(body.byteLength),
        'hew-upload-length': String(body.byteLength),
        'cf-connecting-ip': `198.51.100.${(ipCounter % 250) + 1}`,
        ...extra,
      },
    }),
  )
  assert.equal(res.status, 201, await res.clone().text())
  const json = (await res.json()) as { id: string; token: string; follow?: string }
  const c = await t.handle(
    new Request(`${PUBLIC_BASE}/report/${json.id}/commit`, { method: 'POST', headers: { 'hew-upload-token': json.token } }),
  )
  assert.equal(c.status, 201)
  return { ...json, bytes: body }
}

/** A committed report with real stored bytes. `follow`: also followed. */
function report(t: TestEnv, description = 'the model disappeared after undo, reproducible', follow = false) {
  return startReport(t, follow ? { 'hew-follow': '1' } : {}, description)
}

function attachment(t: TestEnv, parentId: string, secret: string) {
  return startReport(t, { 'hew-follow-up': parentId, 'hew-follow-secret': secret }, `File for report ${parentId}: kitchen.skp`)
}

/** A committed index row with no bundle behind it, for the cap tests. */
async function bareRow(t: TestEnv, i: number, extra: { sizeBytes?: number; parentId?: string } = {}): Promise<string> {
  const id = `HEW-${String(i).padStart(4, '0')}-0000`
  await t.index().reserveReport(`client-${i}`, Date.now(), {
    id,
    receivedAt: Date.now() - i,
    appVersion: '1.5.0',
    platform: 'x',
    descriptionPreview: 'bare',
    sizeBytes: extra.sizeBytes ?? 10,
    parentId: extra.parentId ?? null,
  })
  await t.index().commitReport(id)
  return id
}

function form(t: TestEnv, path: string, fields: Array<[string, string]>, headers: Record<string, string> = {}) {
  return t.handle(
    new Request(`${ADMIN}/${path}`, {
      method: 'POST',
      headers: t.adminHeaders({ 'content-type': 'application/x-www-form-urlencoded', ...headers }),
      body: new URLSearchParams(fields).toString(),
    }),
  )
}

const ids = (list: string[]): Array<[string, string]> => list.map((id) => ['id', id])

function get(t: TestEnv, path: string, base = ADMIN, headers?: Record<string, string>) {
  return t.handle(new Request(`${base}/${path}`, { headers: headers ?? t.adminHeaders() }))
}

/** Which ids have a checked box in a rendered list. */
function checkedIds(html: string): string[] {
  return [...html.matchAll(/<input type="checkbox" name="id" value="([^"]+)"[^>]*? checked>/g)].map((m) => m[1])
}

function boxIds(html: string): string[] {
  return [...html.matchAll(/<input type="checkbox" name="id" value="([^"]+)"/g)].map((m) => m[1])
}

async function allRowIds(t: TestEnv): Promise<string[]> {
  return (await t.index().listReports()).map((r) => r.id).sort()
}

// ---------------------------------------------------------------------------
// The admin host
// ---------------------------------------------------------------------------

describe('the admin host', () => {
  test('every admin route is a plain 404 on the public intake hosts, even with a valid Access token', async () => {
    const t = await buildTestEnv()
    const r = await report(t)
    for (const base of [`${PUBLIC_BASE}/report/admin`, 'https://beta.hew3d.com/report/admin', 'https://example.com/report/admin']) {
      const headers = t.adminHeaders()
      for (const [method, path] of [
        ['GET', '/'],
        ['GET', `/${r.id}`],
        ['GET', `/${r.id}/download`],
        ['POST', `/${r.id}/triage`],
        ['POST', `/${r.id}/delete`],
        ['POST', '/batch/triage'],
        ['POST', '/batch/delete'],
        ['GET', `/batch/download?id=${r.id}`],
      ]) {
        const res = await t.handle(new Request(`${base}${path}`, { method, headers }))
        assert.equal(res.status, 404, `${method} ${base}${path}`)
        assert.equal(res.headers.get('cache-control'), 'no-store')
      }
    }
    assert.equal((await t.index().getReport(r.id))?.triaged, false)
  })

  test('the public intake still answers on the public hosts', async () => {
    const t = await buildTestEnv()
    const res = await t.handle(new Request(`${PUBLIC_BASE}/report/`))
    assert.equal(res.status, 200)
    assert.equal(((await res.json()) as { service: string }).service, 'hew-bug-intake')
  })

  test('a mutation from the old admin origin is refused on the admin host', async () => {
    const t = await buildTestEnv()
    const r = await report(t)
    const res = await form(t, 'batch/triage', [...ids([r.id]), ['value', '1']], { origin: PUBLIC_BASE })
    assert.equal(res.status, 403)
    assert.equal((await t.index().getReport(r.id))?.triaged, false)
  })

  test('redirects and email links point at the admin host', async () => {
    const t = await buildTestEnv()
    const r = await report(t)
    const res = await form(t, 'batch/triage', [...ids([r.id]), ['value', '1']])
    assert.ok(res.headers.get('location')!.startsWith(`${ADMIN_BASE}/report/admin/?`))
  })
})

// ---------------------------------------------------------------------------
// Authorization, Origin, methods
// ---------------------------------------------------------------------------

describe('batch routes: authorization, Origin and methods', () => {
  test('every new route is 403 without a valid Access token', async () => {
    const t = await buildTestEnv()
    const r = await report(t)
    const attempts: Array<[string, string]> = [
      ['GET', '/?select=all'],
      ['POST', '/batch/triage'],
      ['POST', '/batch/delete'],
      ['GET', `/batch/download?id=${r.id}`],
    ]
    for (const headers of [{ origin: ADMIN_BASE }, { origin: ADMIN_BASE, 'cf-access-jwt-assertion': 'garbage' }]) {
      for (const [method, path] of attempts) {
        const res = await t.handle(
          new Request(`${ADMIN}${path}`, {
            method,
            headers: { ...headers, 'content-type': 'application/x-www-form-urlencoded' },
            body: method === 'POST' ? new URLSearchParams([...ids([r.id]), ['value', '1'], ['confirm', '1']]).toString() : undefined,
          }),
        )
        assert.equal(res.status, 403, `${method} ${path}`)
      }
    }
    assert.equal((await t.index().getReport(r.id))?.triaged, false)
    assert.notEqual(await t.drop(r.id).head(), null)
  })

  test('every new route is 403 when the Access secrets are not configured', async () => {
    const t = await buildTestEnv({ admin: false })
    const r = await report(t)
    for (const [method, path] of [
      ['GET', '/?select=all'],
      ['POST', '/batch/triage'],
      ['POST', '/batch/delete'],
      ['GET', `/batch/download?id=${r.id}`],
    ]) {
      const res = await t.handle(new Request(`${ADMIN}${path}`, { method, headers: { origin: ADMIN_BASE } }))
      assert.equal(res.status, 403, `${method} ${path}`)
    }
  })

  test('POST routes need the exact Origin: missing, old host and foreign all 403 and change nothing', async () => {
    const t = await buildTestEnv()
    const r = await report(t, undefined, true)
    for (const path of ['batch/triage', 'batch/delete']) {
      for (const origin of [null, PUBLIC_BASE, 'https://evil.example', `${ADMIN_BASE}.evil.example`]) {
        const headers = t.adminHeaders({ 'content-type': 'application/x-www-form-urlencoded' })
        if (origin === null) delete headers.origin
        else headers.origin = origin
        const res = await t.handle(
          new Request(`${ADMIN}/${path}`, {
            method: 'POST',
            headers,
            body: new URLSearchParams([...ids([r.id]), ['value', '1'], ['confirm', '1']]).toString(),
          }),
        )
        assert.equal(res.status, 403, `${path} origin ${origin}`)
      }
    }
    assert.equal((await t.index().getReport(r.id))?.triaged, false)
    assert.notEqual(await t.drop(r.id).head(), null)
  })

  test('the download is a GET and needs no Origin, like the single download', async () => {
    const t = await buildTestEnv()
    const r = await report(t)
    const headers = t.adminHeaders()
    delete headers.origin
    const res = await get(t, `batch/download?id=${r.id}`, ADMIN, headers)
    assert.equal(res.status, 200)
  })

  test('the wrong method on a batch route is not routed (404, as the per-report action routes)', async () => {
    const t = await buildTestEnv()
    const r = await report(t)
    for (const [method, path] of [
      ['GET', 'batch/triage'],
      ['GET', 'batch/delete'],
      ['POST', `batch/download?id=${r.id}`],
      ['GET', 'batch'],
      ['POST', 'batch/other'],
      ['GET', 'batch/triage/extra'],
    ]) {
      const res = await t.handle(new Request(`${ADMIN}/${path}`, { method, headers: t.adminHeaders() }))
      assert.equal(res.status, 404, `${method} ${path}`)
    }
  })
})

// ---------------------------------------------------------------------------
// The list: checkboxes and selection presets
// ---------------------------------------------------------------------------

describe('the list: checkboxes, presets, notice', () => {
  async function seeded() {
    const t = await buildTestEnv()
    const a = await report(t, 'first report, untriaged one', true)
    const b = await report(t, 'second report, triaged one')
    const c = await report(t, 'third report, expired one', true)
    const child = await attachment(t, a.id, a.follow!)
    await t.index().setTriaged(b.id, true)
    await t.runDropAlarm(c.id) // followed, so the row stays with its bundle gone
    assert.equal((await t.index().getReport(c.id))?.bundleDeleted, true)
    return { t, a, b, c, child }
  }

  test('one form with a checkbox per top-level report and none on an attachment', async () => {
    const { t, a, b, c, child } = await seeded()
    const html = await (await get(t, '')).text()
    assert.deepEqual(boxIds(html).sort(), [a.id, b.id, c.id].sort())
    assert.ok(!html.includes(`value="${child.id}"`))
    assert.ok(html.includes(child.id), 'the attachment row is still listed')
    assert.equal((html.match(/<form /g) ?? []).length, 1)
    assert.deepEqual(checkedIds(html), [], 'nothing is checked by default')
  })

  test('the toolbar buttons carry their own action and method, above and below the table', async () => {
    const { t } = await seeded()
    const html = await (await get(t, '')).text()
    for (const [action, method] of [
      ['/report/admin/batch/triage', 'post'],
      ['/report/admin/batch/delete', 'post'],
      ['/report/admin/batch/download', 'get'],
    ]) {
      const n = html.split(`formmethod="${method}" formaction="${action}"`).length - 1
      assert.ok(n >= 2, `${action}: ${n}`)
    }
    assert.match(html, /name="value" value="1">Mark triaged/)
    assert.match(html, /name="value" value="0">Mark untriaged/)
  })

  test('each preset is a link, and checks the right rows', async () => {
    const { t, a, b, c } = await seeded()
    const html = await (await get(t, '')).text()
    for (const preset of ['all', 'untriaged', 'triaged', 'expired', 'none']) {
      assert.ok(html.includes(`<a href="/report/admin/?select=${preset}">${preset}</a>`), preset)
    }
    const picks = async (select: string) => checkedIds(await (await get(t, `?select=${select}`)).text()).sort()
    assert.deepEqual(await picks('all'), [a.id, b.id, c.id].sort())
    assert.deepEqual(await picks('untriaged'), [a.id, c.id].sort())
    assert.deepEqual(await picks('triaged'), [b.id])
    assert.deepEqual(await picks('expired'), [c.id])
    assert.deepEqual(await picks('none'), [])
  })

  test('an unknown or hostile select value checks nothing and is not echoed', async () => {
    const { t } = await seeded()
    for (const value of ['everything', '<script>x</script>', 'all%27%22', '']) {
      const res = await get(t, `?select=${value}`)
      assert.equal(res.status, 200)
      const html = await res.text()
      assert.deepEqual(checkedIds(html), [])
      assert.ok(!html.includes('<script>x'))
    }
  })

  test('the presets never check an attachment, whatever its state', async () => {
    const { t, child } = await seeded()
    await t.index().setTriaged(child.id, true)
    for (const preset of ['all', 'untriaged', 'triaged', 'expired']) {
      const html = await (await get(t, `?select=${preset}`)).text()
      assert.ok(!checkedIds(html).includes(child.id), preset)
    }
  })

  test('the result notice renders from validated values only', async () => {
    const { t, a } = await seeded()
    const ok = await (await get(t, `?notice=deleted&ok=3&failed=${a.id.toLowerCase()},HEW-0000-1111`)).text()
    assert.match(ok, /Deleted 3 reports\./)
    assert.ok(ok.includes(`<code>${a.id}</code>`), 'ids are normalized')
    assert.match(ok, /Not done for 2:/)
    assert.match(await (await get(t, '?notice=triaged&ok=1')).text(), /Marked triaged 1 report\./)
    assert.match(await (await get(t, '?notice=untriaged&ok=0')).text(), /Marked untriaged 0 reports\./)
    for (const query of [
      '?notice=<script>alert(1)</script>&ok=1',
      '?notice=deleted&ok=<b>1</b>',
      '?notice=deleted&ok=1&failed=<img src=x>',
      '?notice=deleted&ok=1&failed=HEW-0000-1111,<b>x</b>',
      '?notice=other&ok=1',
      '?notice=deleted&ok=99999',
      '?notice=deleted',
    ]) {
      const html = await (await get(t, encodeURI(query))).text()
      assert.ok(!html.includes('class="notice"'), query)
      assert.ok(!html.includes('<img src=x>') && !html.includes('<b>') && !html.includes('alert(1)'), query)
    }
  })

  test('a notice built by hand is escaped anyway', () => {
    const html = renderList([], { notice: { action: 'deleted', ok: 1, failed: ['<b>x</b>'] } })
    assert.ok(!html.includes('<b>x</b>'))
    assert.ok(html.includes('&lt;b&gt;x&lt;/b&gt;'))
  })
})

// ---------------------------------------------------------------------------
// Batch triage
// ---------------------------------------------------------------------------

describe('batch triage', () => {
  test('sets and never toggles a mixed selection, and redirects with the count', async () => {
    const t = await buildTestEnv()
    const a = await report(t)
    const b = await report(t)
    await t.index().setTriaged(b.id, true)
    const res = await form(t, 'batch/triage', [...ids([a.id, b.id]), ['value', '1']])
    assert.equal(res.status, 303)
    assert.equal(res.headers.get('location'), `${ADMIN_BASE}/report/admin/?notice=triaged&ok=2`)
    assert.equal((await t.index().getReport(a.id))?.triaged, true)
    assert.equal((await t.index().getReport(b.id))?.triaged, true)
    const back = await form(t, 'batch/triage', [...ids([a.id, b.id]), ['value', '0']])
    assert.equal(back.headers.get('location'), `${ADMIN_BASE}/report/admin/?notice=untriaged&ok=2`)
    assert.equal((await t.index().getReport(a.id))?.triaged, false)
    assert.equal((await t.index().getReport(b.id))?.triaged, false)
  })

  test('a lower-case id works; an unknown id and an attachment id are reported, not applied', async () => {
    const t = await buildTestEnv()
    const a = await report(t, undefined, true)
    const child = await attachment(t, a.id, a.follow!)
    const res = await form(t, 'batch/triage', [...ids([a.id.toLowerCase(), 'HEW-0000-1111', child.id]), ['value', '1']])
    assert.equal(res.status, 303)
    const location = new URL(res.headers.get('location')!)
    assert.equal(location.searchParams.get('ok'), '1')
    assert.equal(location.searchParams.get('failed'), `HEW-0000-1111,${child.id}`)
    assert.equal((await t.index().getReport(a.id))?.triaged, true)
    assert.equal((await t.index().getReport(child.id))?.triaged, false)
  })

  test('an invalid id fails the whole request with 400 and changes nothing', async () => {
    const t = await buildTestEnv()
    const a = await report(t)
    for (const bad of ['garbage', 'batch', '../etc', 'HEW-AAAA-BBB', '']) {
      const res = await form(t, 'batch/triage', [...ids([a.id, bad]), ['value', '1']])
      assert.equal(res.status, 400, bad)
      assert.equal((await t.index().getReport(a.id))?.triaged, false)
    }
  })

  test('no ids, or no/odd value, is 400', async () => {
    const t = await buildTestEnv()
    const a = await report(t)
    assert.equal((await form(t, 'batch/triage', [['value', '1']])).status, 400)
    assert.equal((await form(t, 'batch/triage', ids([a.id]))).status, 400)
    assert.equal((await form(t, 'batch/triage', [...ids([a.id]), ['value', '2']])).status, 400)
    assert.equal((await t.index().getReport(a.id))?.triaged, false)
  })

  test('the cap is enforced at cap + 1 and not at cap', async () => {
    const t = await buildTestEnv()
    const made: string[] = []
    for (let i = 0; i <= BATCH_TRIAGE_MAX; i++) made.push(await bareRow(t, i))
    const over = await form(t, 'batch/triage', [...ids(made), ['value', '1']])
    assert.equal(over.status, 400)
    assert.match(await over.text(), new RegExp(`${BATCH_TRIAGE_MAX + 1} reports`))
    assert.ok((await t.index().listReports()).every((r) => !r.triaged))
    const at = await form(t, 'batch/triage', [...ids(made.slice(0, BATCH_TRIAGE_MAX)), ['value', '1']])
    assert.equal(at.status, 303)
    assert.equal((await t.index().listReports()).filter((r) => r.triaged).length, BATCH_TRIAGE_MAX)
  })

  test('a duplicate id is counted once', async () => {
    const t = await buildTestEnv()
    const a = await report(t)
    const res = await form(t, 'batch/triage', [...ids([a.id, a.id, a.id.toLowerCase()]), ['value', '1']])
    assert.equal(new URL(res.headers.get('location')!).searchParams.get('ok'), '1')
  })
})

// ---------------------------------------------------------------------------
// Batch delete
// ---------------------------------------------------------------------------

describe('batch delete', () => {
  test('the first request changes nothing and asks, naming the counts and carrying the ids', async () => {
    const t = await buildTestEnv()
    const a = await report(t, undefined, true)
    const b = await report(t)
    const child = await attachment(t, a.id, a.follow!)
    const res = await form(t, 'batch/delete', ids([a.id, b.id]))
    assert.equal(res.status, 200)
    const html = await res.text()
    assert.match(html, /Delete 2 reports and their 1 attachment\?/)
    assert.match(html, /This cannot be undone/)
    assert.ok(html.includes(`<input type="hidden" name="id" value="${a.id}">`))
    assert.ok(html.includes(`<input type="hidden" name="id" value="${b.id}">`))
    assert.ok(html.includes('<input type="hidden" name="confirm" value="1">'))
    assert.ok(html.includes('href="/report/admin/">Back</a>'))
    assert.ok(html.includes('action="/report/admin/batch/delete"'))
    // nothing was touched
    assert.deepEqual(await allRowIds(t), [a.id, b.id, child.id].sort())
    for (const id of [a.id, b.id, child.id]) assert.notEqual(await t.drop(id).head(), null, id)
    assert.notEqual(t.threadStorage(a.id), undefined)
    assert.equal(await t.thread(a.id).adminView(Date.now()).then((v) => v?.following), true)
  })

  test('a confirm value other than 1 is still the first step', async () => {
    const t = await buildTestEnv()
    const a = await report(t)
    for (const confirm of ['0', '', 'yes', 'true']) {
      const res = await form(t, 'batch/delete', [...ids([a.id]), ['confirm', confirm]])
      assert.equal(res.status, 200, confirm)
    }
    assert.notEqual(await t.index().getReport(a.id), null)
  })

  test('the second request deletes threads, bundles, rows and attachments, and only the selected', async () => {
    const t = await buildTestEnv()
    const a = await report(t, undefined, true)
    const b = await report(t)
    const keep = await report(t, undefined, true)
    const child = await attachment(t, a.id, a.follow!)
    const res = await form(t, 'batch/delete', [...ids([a.id, b.id]), ['confirm', '1']])
    assert.equal(res.status, 303)
    assert.equal(res.headers.get('location'), `${ADMIN_BASE}/report/admin/?notice=deleted&ok=2`)
    for (const id of [a.id, b.id, child.id]) {
      assert.equal(await t.index().getReport(id), null, id)
      assert.equal(await t.drop(id).head(), null, id)
    }
    assert.equal(await t.thread(a.id).adminView(Date.now()), null, 'the thread went first')
    assert.deepEqual(await allRowIds(t), [keep.id])
    assert.notEqual(await t.drop(keep.id).head(), null)
    // the reporter's key stopped working
    const follow = await t.handle(
      new Request(`${PUBLIC_BASE}/report/${a.id}/thread`, { headers: { 'hew-follow-secret': a.follow!, 'cf-connecting-ip': '192.0.2.99' } }),
    )
    assert.equal(follow.status, 403)
  })

  test('an unknown id is skipped and reported; an attachment id on its own is not deleted', async () => {
    const t = await buildTestEnv()
    const a = await report(t, undefined, true)
    const child = await attachment(t, a.id, a.follow!)
    const b = await report(t)
    const res = await form(t, 'batch/delete', [...ids([b.id, 'HEW-0000-1111', child.id]), ['confirm', '1']])
    assert.equal(res.status, 303)
    const q = new URL(res.headers.get('location')!).searchParams
    assert.equal(q.get('ok'), '1')
    assert.equal(q.get('failed'), `HEW-0000-1111,${child.id}`)
    assert.equal(await t.index().getReport(b.id), null)
    assert.notEqual(await t.index().getReport(child.id), null)
    assert.notEqual(await t.drop(child.id).head(), null)
  })

  test('an invalid id is 400 at both steps and changes nothing', async () => {
    const t = await buildTestEnv()
    const a = await report(t)
    for (const confirm of [[], [['confirm', '1']] as Array<[string, string]>]) {
      const res = await form(t, 'batch/delete', [...ids([a.id, 'nope']), ...confirm])
      assert.equal(res.status, 400)
    }
    assert.equal((await form(t, 'batch/delete', [['confirm', '1']])).status, 400)
    assert.notEqual(await t.index().getReport(a.id), null)
    assert.notEqual(await t.drop(a.id).head(), null)
  })

  test('one failing report does not stop the others, and keeps its own row', async () => {
    const t = await buildTestEnv()
    const a = await report(t)
    const b = await report(t)
    const c = await report(t)
    const realGet = t.env.REPORT_DROP.get.bind(t.env.REPORT_DROP)
    t.env.REPORT_DROP.get = ((id: { toString(): string }) => {
      const stub = realGet(id as never) as unknown as Record<string, unknown>
      if (id.toString() !== b.id) return stub as never
      return { ...stub, destroy: () => Promise.reject(new Error('simulated failure')), head: (stub.head as () => unknown).bind(stub) } as never
    }) as typeof t.env.REPORT_DROP.get
    const res = await form(t, 'batch/delete', [...ids([a.id, b.id, c.id]), ['confirm', '1']])
    t.env.REPORT_DROP.get = realGet
    assert.equal(res.status, 303)
    const q = new URL(res.headers.get('location')!).searchParams
    assert.equal(q.get('ok'), '2')
    assert.equal(q.get('failed'), b.id)
    assert.deepEqual(await allRowIds(t), [b.id])
    assert.equal(await t.drop(a.id).head(), null)
    assert.equal(await t.drop(c.id).head(), null)
  })

  test('an attachment that will not go keeps its parent row, and a retry finishes', async () => {
    const t = await buildTestEnv()
    const a = await report(t, undefined, true)
    const child = await attachment(t, a.id, a.follow!)
    const realGet = t.env.REPORT_DROP.get.bind(t.env.REPORT_DROP)
    t.env.REPORT_DROP.get = ((id: { toString(): string }) => {
      const stub = realGet(id as never) as unknown as Record<string, unknown>
      if (id.toString() !== child.id) return stub as never
      return { ...stub, destroy: () => Promise.reject(new Error('simulated failure')) } as never
    }) as typeof t.env.REPORT_DROP.get
    const first = await form(t, 'batch/delete', [...ids([a.id]), ['confirm', '1']])
    t.env.REPORT_DROP.get = realGet
    const q = new URL(first.headers.get('location')!).searchParams
    assert.equal(q.get('ok'), '0')
    assert.equal(q.get('failed'), `${child.id},${a.id}`)
    assert.notEqual(await t.index().getReport(child.id), null)
    // The kept parent row says what was actually destroyed.
    const kept = await t.index().getReport(a.id)
    assert.notEqual(kept, null)
    assert.equal(kept!.bundleDeleted, true)
    assert.equal(kept!.following, false)
    assert.equal(kept!.sizeBytes, 0)
    const retry = await form(t, 'batch/delete', [...ids([a.id]), ['confirm', '1']])
    assert.equal(new URL(retry.headers.get('location')!).searchParams.get('ok'), '1')
    assert.deepEqual(await allRowIds(t), [])
  })

  test('the cap is enforced at cap + 1 reports, at both steps, and not at cap', async () => {
    const t = await buildTestEnv()
    const made: string[] = []
    for (let i = 0; i <= BATCH_DELETE_MAX; i++) made.push(await bareRow(t, i))
    for (const confirm of [[], [['confirm', '1']] as Array<[string, string]>]) {
      const res = await form(t, 'batch/delete', [...ids(made), ...confirm])
      assert.equal(res.status, 400)
      assert.match(await res.text(), new RegExp(`${BATCH_DELETE_MAX + 1} reports.*at most ${BATCH_DELETE_MAX}`))
    }
    assert.equal((await t.index().listReports()).length, BATCH_DELETE_MAX + 1)
    const at = await form(t, 'batch/delete', [...ids(made.slice(0, BATCH_DELETE_MAX)), ['confirm', '1']])
    assert.equal(at.status, 303)
    assert.equal((await t.index().listReports()).length, 1)
  })

  test('attachments count against the cap, since each costs a Durable Object call', async () => {
    const t = await buildTestEnv()
    const parents: string[] = []
    for (let i = 0; i < 2; i++) parents.push(await bareRow(t, i))
    for (let i = 0; i < BATCH_DELETE_MAX - 1; i++) await bareRow(t, 100 + i, { parentId: parents[0] })
    // 2 reports + 19 attachments = 21
    const over = await form(t, 'batch/delete', [...ids(parents), ['confirm', '1']])
    assert.equal(over.status, 400)
    assert.match(await over.text(), /Nothing was changed/)
    assert.equal((await t.index().listReports()).length, 2 + BATCH_DELETE_MAX - 1)
    const fits = await form(t, 'batch/delete', [...ids([parents[0]]), ['confirm', '1']])
    assert.equal(fits.status, 303)
  })

  test('the confirm page escapes everything it shows', async () => {
    const t = await buildTestEnv()
    const a = await report(t, '<img src=x onerror=alert(1)> a hostile description')
    const html = await (await form(t, 'batch/delete', ids([a.id]))).text()
    assert.ok(!html.includes('<img src=x'))
    assert.ok(html.includes('&lt;img src=x'))
  })
})

// ---------------------------------------------------------------------------
// Batch download
// ---------------------------------------------------------------------------

interface TarEntryOut {
  name: string
  size: number
  mtime: number
  data: Uint8Array
}

/** A reference reader: verifies every header checksum, magic, size field,
 *  512-byte padding and the two closing zero blocks. */
function readTar(bytes: Uint8Array): TarEntryOut[] {
  assert.equal(bytes.length % 512, 0, 'a tar is a whole number of blocks')
  const out: TarEntryOut[] = []
  let pos = 0
  const isZero = (block: Uint8Array) => block.every((b) => b === 0)
  const field = (h: Uint8Array, from: number, len: number) => new TextDecoder().decode(h.subarray(from, from + len)).replace(/\0.*$/s, '')
  while (true) {
    assert.ok(pos + 1024 <= bytes.length, 'ends with two zero blocks')
    const h = bytes.subarray(pos, pos + 512)
    if (isZero(h)) {
      assert.ok(isZero(bytes.subarray(pos, pos + 1024)), 'two zero blocks')
      assert.equal(pos + 1024, bytes.length, 'nothing after the end blocks')
      return out
    }
    const stated = parseInt(field(h, 148, 8).trim(), 8)
    const sum = h.reduce((n, b, i) => n + (i >= 148 && i < 156 ? 0x20 : b), 0)
    assert.equal(stated, sum, 'header checksum')
    assert.equal(field(h, 257, 6), 'ustar')
    assert.equal(h[156], 0x30)
    const name = field(h, 0, 100)
    assert.ok(isSafeTarName(name), `safe name ${name}`)
    const size = parseInt(field(h, 124, 12), 8)
    const mtime = parseInt(field(h, 136, 12), 8)
    const padded = Math.ceil(size / 512) * 512
    const data = bytes.slice(pos + 512, pos + 512 + size)
    assert.ok(bytes.subarray(pos + 512 + size, pos + 512 + padded).every((b) => b === 0), 'zero padding')
    out.push({ name, size, mtime, data })
    pos += 512 + padded
  }
}

async function tarOf(res: Response): Promise<TarEntryOut[]> {
  assert.equal(res.status, 200)
  return readTar(new Uint8Array(await res.arrayBuffer()))
}

describe('batch download', () => {
  async function seeded() {
    const t = await buildTestEnv()
    const a = await report(t, 'first, with an attachment', true)
    const child = await attachment(t, a.id, a.follow!)
    const b = await report(t, 'second, plain')
    const c = await report(t, 'third, expired', true)
    await t.runDropAlarm(c.id)
    return { t, a, b, c, child }
  }

  test('is one uncompressed tar of the stored bundles: parses, bytes are exactly as stored, expired are listed as skipped', async () => {
    const { t, a, b, c, child } = await seeded()
    const res = await get(t, `batch/download?id=${a.id}&id=${b.id}&id=${c.id}`)
    assert.equal(res.status, 200)
    assert.equal(res.headers.get('content-type'), 'application/x-tar')
    assert.match(res.headers.get('content-disposition')!, /^attachment; filename="hew-reports-\d{8}T\d{6}Z\.tar"$/)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    assert.equal(res.headers.get('content-length'), null, 'streamed, so no length')
    const entries = await tarOf(res)
    assert.deepEqual(
      entries.map((e) => e.name),
      [`${a.id}.json.gz`, `${a.id}--${child.id}.json.gz`, `${b.id}.json.gz`, 'SKIPPED.txt'],
    )
    assert.deepEqual(entries[0].data, a.bytes)
    assert.deepEqual(entries[1].data, child.bytes)
    assert.deepEqual(entries[2].data, b.bytes)
    assert.equal(entries[0].size, a.bytes.byteLength)
    const skipped = new TextDecoder().decode(entries[3].data)
    assert.ok(skipped.includes(`${c.id}: bundle expired`))
    assert.ok(!skipped.includes(a.id))
    // each entry is a gzip stream that decompresses back to its JSON
    const doc = JSON.parse(new TextDecoder().decode(new Uint8Array(await new Response(new Blob([entries[2].data]).stream().pipeThrough(new DecompressionStream('gzip'))).arrayBuffer())))
    assert.equal(doc.report.description, 'second, plain')
    // mtime is the report's receive time
    const row = await t.index().getReport(a.id)
    assert.equal(entries[0].mtime, Math.floor(row!.receivedAt / 1000))
  })

  test('no SKIPPED.txt when nothing was skipped; an unknown id lands in it; a lower-case id works', async () => {
    const { t, b } = await seeded()
    const clean = await tarOf(await get(t, `batch/download?id=${b.id.toLowerCase()}`))
    assert.deepEqual(clean.map((e) => e.name), [`${b.id}.json.gz`])
    const odd = await tarOf(await get(t, `batch/download?id=${b.id}&id=HEW-0000-1111`))
    assert.equal(odd.length, 2)
    assert.match(new TextDecoder().decode(odd[1].data), /HEW-0000-1111: not a report/)
  })

  test('an attachment id alone is not a download of its own', async () => {
    const { t, child } = await seeded()
    const entries = await tarOf(await get(t, `batch/download?id=${child.id}`))
    assert.deepEqual(entries.map((e) => e.name), ['SKIPPED.txt'])
  })

  test('a bundle that is gone from its drop is skipped, not fatal', async () => {
    const { t, b, a } = await seeded()
    await t.drop(b.id).destroy()
    const entries = await tarOf(await get(t, `batch/download?id=${b.id}&id=${a.id}`))
    assert.deepEqual(entries.map((e) => e.name), [`${a.id}.json.gz`, `${a.id}--${(await t.index().listChildren(a.id))[0].id}.json.gz`, 'SKIPPED.txt'])
    assert.match(new TextDecoder().decode(entries[2].data), new RegExp(`${b.id}: bundle missing`))
  })

  test('no ids or an invalid id is 400 and sends no archive', async () => {
    const { t, b } = await seeded()
    for (const query of ['', `?id=${b.id}&id=garbage`, '?id=../../etc/passwd', '?id=batch']) {
      const res = await get(t, `batch/download${query}`)
      assert.equal(res.status, 400, query)
      assert.match(res.headers.get('content-type')!, /^text\/html/)
    }
  })

  test('the selection cap is enforced at cap + 1 and not at cap', async () => {
    const t = await buildTestEnv()
    const made: string[] = []
    for (let i = 0; i <= BATCH_DOWNLOAD_MAX; i++) made.push(await bareRow(t, i))
    const over = await get(t, `batch/download?${made.map((id) => `id=${id}`).join('&')}`)
    assert.equal(over.status, 400)
    assert.match(await over.text(), new RegExp(`${BATCH_DOWNLOAD_MAX + 1} reports.*at most ${BATCH_DOWNLOAD_MAX}`))
    const at = await get(t, `batch/download?${made.slice(0, BATCH_DOWNLOAD_MAX).map((id) => `id=${id}`).join('&')}`)
    assert.equal(at.status, 200)
    await at.body!.cancel()
  })

  test('attachments count against the bundle cap', async () => {
    const t = await buildTestEnv()
    const p = await bareRow(t, 0)
    for (let i = 0; i < BATCH_DOWNLOAD_MAX; i++) await bareRow(t, 100 + i, { parentId: p })
    const over = await get(t, `batch/download?id=${p}`)
    assert.equal(over.status, 400)
    assert.match(await over.text(), new RegExp(`${BATCH_DOWNLOAD_MAX + 1} bundles`))
  })

  test('the byte cap is enforced on the stored sizes', async () => {
    const t = await buildTestEnv()
    const big = await bareRow(t, 0, { sizeBytes: BATCH_DOWNLOAD_MAX_BYTES })
    const one = await bareRow(t, 1, { sizeBytes: 1 })
    const over = await get(t, `batch/download?id=${big}&id=${one}`)
    assert.equal(over.status, 400)
    assert.match(await over.text(), /at most 512\.0 MB/)
    const fits = await get(t, `batch/download?id=${big}`)
    assert.equal(fits.status, 200)
    await fits.body!.cancel()
  })

  test('is streamed: the bundles are opened one at a time as the body is read', async () => {
    const t = await buildTestEnv()
    const a = await report(t)
    const b = await report(t)
    let streams = 0
    const realGet = t.env.REPORT_DROP.get.bind(t.env.REPORT_DROP)
    t.env.REPORT_DROP.get = ((id: { toString(): string }) => {
      const stub = realGet(id as never) as unknown as { readStream: () => Promise<unknown> }
      return {
        head: (stub as unknown as { head: () => unknown }).head.bind(stub),
        readStream: () => {
          streams++
          return stub.readStream()
        },
      } as never
    }) as typeof t.env.REPORT_DROP.get
    const res = await get(t, `batch/download?id=${a.id}&id=${b.id}`)
    assert.equal(res.status, 200)
    assert.ok(streams <= 1, 'at most the first bundle is opened before the body is read')
    const reader = res.body!.getReader()
    await reader.read() // first header
    await reader.read() // first bundle's bytes
    assert.equal(streams, 1, 'the second bundle is not opened while the first is being sent')
    while (!(await reader.read()).done) {
      /* drain */
    }
    assert.equal(streams, 2)
    t.env.REPORT_DROP.get = realGet
  })

  test('a multi-piece bundle comes through whole', async () => {
    const t = await buildTestEnv()
    const filler = new Uint8Array(4_800_000)
    for (let at = 0; at < filler.length; at += 65536) crypto.getRandomValues(filler.subarray(at, Math.min(at + 65536, filler.length)))
    const doc = { format: 1, report: { description: 'a report big enough for several pieces' }, system: { appVersion: '1', platform: 'x' }, model: Buffer.from(filler).toString('base64') }
    const bytes = await gzipJson(doc)
    assert.ok(bytes.length > PIECE_BYTES * 2)
    const headers = { 'content-type': 'application/gzip', 'cf-connecting-ip': '203.0.113.200' }
    const first = bytes.subarray(0, PIECE_BYTES)
    const startRes = await t.handle(
      new Request(`${PUBLIC_BASE}/report/`, {
        method: 'POST',
        body: first,
        headers: { ...headers, 'content-length': String(first.length), 'hew-upload-length': String(bytes.length) },
      }),
    )
    assert.equal(startRes.status, 201, await startRes.clone().text())
    const { id, token } = (await startRes.json()) as { id: string; token: string }
    for (let at = PIECE_BYTES, i = 1; at < bytes.length; at += PIECE_BYTES, i++) {
      const piece = bytes.subarray(at, Math.min(at + PIECE_BYTES, bytes.length))
      const r = await t.handle(
        new Request(`${PUBLIC_BASE}/report/${id}/${i}`, { method: 'PUT', body: piece, headers: { ...headers, 'content-length': String(piece.length), 'hew-upload-token': token } }),
      )
      assert.equal(r.status, 204, await r.clone().text())
    }
    const c = await t.handle(new Request(`${PUBLIC_BASE}/report/${id}/commit`, { method: 'POST', headers: { 'hew-upload-token': token } }))
    assert.equal(c.status, 201, await c.clone().text())
    assert.ok((await t.drop(id).head())!.pieceCount >= 3)
    const entries = await tarOf(await get(t, `batch/download?id=${id}`))
    assert.equal(entries.length, 1)
    assert.deepEqual(entries[0].data, bytes)
  })
})

// ---------------------------------------------------------------------------
// The tar writer
// ---------------------------------------------------------------------------

function source(name: string, data: Uint8Array, claim = data.byteLength, opened?: () => void): TarSource {
  return {
    name,
    mtimeSeconds: 1_700_000_000,
    async open() {
      opened?.()
      return {
        size: claim,
        body: new ReadableStream<Uint8Array>({
          start(c) {
            // two chunks, to exercise the chunk loop
            c.enqueue(data.subarray(0, Math.floor(data.length / 2)))
            c.enqueue(data.subarray(Math.floor(data.length / 2)))
            c.close()
          },
        }),
      }
    },
  }
}

async function drain(stream: ReadableStream<Uint8Array>): Promise<Uint8Array> {
  return new Uint8Array(await new Response(stream).arrayBuffer())
}

describe('tar writer', () => {
  test('headers: checksum, octal fields, ustar magic', () => {
    const h = tarHeader('HEW-AAAA-BBBB.json.gz', 1234, 1_700_000_000)
    assert.equal(h.length, 512)
    assert.equal(new TextDecoder().decode(h.subarray(0, 21)), 'HEW-AAAA-BBBB.json.gz')
    assert.equal(new TextDecoder().decode(h.subarray(124, 135)), (1234).toString(8).padStart(11, '0'))
    assert.equal(new TextDecoder().decode(h.subarray(257, 262)), 'ustar')
    const sum = h.reduce((n, b, i) => n + (i >= 148 && i < 156 ? 0x20 : b), 0)
    assert.equal(parseInt(new TextDecoder().decode(h.subarray(148, 154)), 8), sum)
  })

  test('padding brings every size to a multiple of 512', () => {
    for (const n of [0, 1, 511, 512, 513, 1024, 1500]) assert.equal((n + tarPadding(n).length) % 512, 0, String(n))
    assert.equal(tarPadding(512).length, 0)
  })

  test('refuses unsafe entry names', () => {
    for (const bad of ['', '../x', 'a/b', '/etc/passwd', '..', '.hidden', 'a\\b', 'a b', 'nul\0', 'é.txt', 'x'.repeat(101), 'a..b']) {
      assert.throws(() => tarHeader(bad, 1, 0), RangeError, JSON.stringify(bad))
      assert.equal(isSafeTarName(bad), false, JSON.stringify(bad))
    }
    assert.equal(isSafeTarName('HEW-AAAA-BBBB--HEW-CCCC-DDDD.json.gz'), true)
    assert.equal(isSafeTarName('SKIPPED.txt'), true)
    assert.equal(isSafeTarName('x'.repeat(100)), true)
  })

  test('refuses an unsafe size', () => {
    assert.throws(() => tarHeader('a', -1, 0), RangeError)
    assert.throws(() => tarHeader('a', 1.5, 0), RangeError)
    assert.throws(() => tarHeader('a', 8 ** 11, 0), RangeError)
  })

  test('an unsafe source name errors the stream rather than writing it', async () => {
    await assert.rejects(drain(tarStream([source('../evil', new Uint8Array(10))], [], 0)))
  })

  test('a stream of several sizes round-trips through the reference reader', async () => {
    const sizes = [0, 1, 511, 512, 513, 5000]
    const sources = sizes.map((n, i) => source(`f${i}.bin`, new Uint8Array(n).map((_, j) => (j * 7 + i) & 0xff)))
    const entries = readTar(await drain(tarStream(sources, [], 0)))
    assert.deepEqual(entries.map((e) => e.size), sizes)
    assert.deepEqual(entries[5].data, new Uint8Array(5000).map((_, j) => (j * 7 + 5) & 0xff))
  })

  test('an empty archive is two zero blocks', async () => {
    const out = await drain(tarStream([], [], 0))
    assert.equal(out.length, 1024)
    assert.ok(out.every((b) => b === 0))
  })

  test('sources open lazily, one at a time', async () => {
    const opens: number[] = []
    const sources = [0, 1, 2].map((i) => source(`f${i}`, new Uint8Array(100), 100, () => opens.push(i)))
    const stream = tarStream(sources, [], 0)
    assert.deepEqual(opens, [])
    const reader = stream.getReader()
    await reader.read()
    assert.deepEqual(opens, [0])
    await reader.cancel()
    assert.deepEqual(opens, [0])
  })

  test('a source shorter or longer than it declared errors the stream', async () => {
    await assert.rejects(drain(tarStream([source('short', new Uint8Array(100), 200)], [], 0)), /short/)
    await assert.rejects(drain(tarStream([source('long', new Uint8Array(100), 50)], [], 0)), /more than it declared/)
  })

  test('skips (up front and at open) land in SKIPPED.txt', async () => {
    const skip: TarSource = { name: 'gone.bin', mtimeSeconds: 0, open: async () => ({ skipped: 'bundle missing' }) }
    const entries = readTar(await drain(tarStream([source('a', new Uint8Array(3)), skip], [{ label: 'X', reason: 'expired' }], 5)))
    assert.deepEqual(entries.map((e) => e.name), ['a', 'SKIPPED.txt'])
    const text = new TextDecoder().decode(entries[1].data)
    assert.ok(text.includes('X: expired') && text.includes('gone.bin: bundle missing'))
    assert.equal(entries[1].mtime, 5)
  })
})

describe('the Durable Object bound-parameter limit', () => {
  test('the fake storage refuses a statement with more than 100 bound parameters', async () => {
    const t = await buildTestEnv()
    const many = Array.from({ length: 101 }, (_, i) => `HEW-${String(i).padStart(4, '0')}-0000`)
    const marks = many.map(() => '?').join(', ')
    await t.index().listReports() // run the migration
    const storage = (t.index() as unknown as { storage: { sql: { exec(q: string, ...b: unknown[]): unknown } } }).storage
    assert.throws(() => storage.sql.exec(`SELECT id FROM reports WHERE id IN (${marks})`, ...many), /too many SQL variables/)
  })

  test('every multi-id index method works on 250 ids under that limit', async () => {
    const t = await buildTestEnv()
    const made: string[] = []
    for (let i = 0; i < 250; i++) made.push(await bareRow(t, i))
    const kids: string[] = []
    for (let i = 250; i < 350; i++) kids.push(await bareRow(t, i, { parentId: made[i - 250] }))
    const idx = t.index()
    const rows = await idx.getReports(made)
    assert.equal(rows.length, 250)
    assert.deepEqual(rows.map((r) => r.receivedAt), [...rows.map((r) => r.receivedAt)].sort((x, y) => y - x))
    await idx.setTriagedMany(made, true)
    assert.ok((await idx.getReports(made)).every((r) => r.triaged))
    const children = await idx.listChildrenOf(made)
    assert.equal(children.length, 100)
    assert.deepEqual(children.map((r) => r.receivedAt), [...children.map((r) => r.receivedAt)].sort((x, y) => x - y))
    await idx.markDestroyed(made)
    assert.ok((await idx.getReports(made)).every((r) => r.bundleDeleted && !r.following))
    await idx.deleteReports([...made, ...kids])
    assert.equal((await idx.getReports([...made, ...kids])).length, 0)
  })

  test('triage of 200 ids, and delete and download at their caps, all work', async () => {
    const t = await buildTestEnv()
    const made: string[] = []
    for (let i = 0; i < BATCH_TRIAGE_MAX; i++) made.push(await bareRow(t, i))
    const res = await form(t, 'batch/triage', [...ids(made), ['value', '1']])
    assert.equal(res.status, 303)
    assert.equal((await t.index().listReports()).filter((r) => r.triaged).length, BATCH_TRIAGE_MAX)
    const few = made.slice(0, BATCH_DELETE_MAX)
    const del = await form(t, 'batch/delete', [...ids(few), ['confirm', '1']])
    assert.equal(new URL(del.headers.get('location')!).searchParams.get('ok'), String(BATCH_DELETE_MAX))
    const dl = await t.handle(
      new Request(`${ADMIN}/batch/download?${new URLSearchParams(ids(made.slice(20, 20 + BATCH_DOWNLOAD_MAX))).toString()}`, { headers: t.adminHeaders() }),
    )
    assert.equal(dl.status, 200)
    await dl.arrayBuffer()
  })
})

describe('batch delete when the index rows cannot be removed', () => {
  test('is reported as not done with a notice that the list entries remain, and a retry clears them', async () => {
    const t = await buildTestEnv()
    const a = await report(t)
    const idx = t.index()
    const real = idx.deleteReports.bind(idx)
    idx.deleteReports = () => Promise.reject(new Error('simulated index failure'))
    const res = await form(t, 'batch/delete', [...ids([a.id]), ['confirm', '1']])
    idx.deleteReports = real
    assert.equal(res.status, 303)
    const loc = new URL(res.headers.get('location')!)
    assert.equal(loc.searchParams.get('ok'), '0')
    assert.equal(loc.searchParams.get('failed'), a.id)
    assert.equal(loc.searchParams.get('rows'), 'kept')
    assert.equal(await t.drop(a.id).head(), null)
    assert.notEqual(await idx.getReport(a.id), null)
    const page = await (await get(t, `?${loc.searchParams.toString()}`)).text()
    assert.match(page, /Not done for 1/)
    assert.match(page, /entries are still in this list/)
    const retry = await form(t, 'batch/delete', [...ids([a.id]), ['confirm', '1']])
    assert.equal(new URL(retry.headers.get('location')!).searchParams.get('ok'), '1')
    assert.deepEqual(await allRowIds(t), [])
  })
})
