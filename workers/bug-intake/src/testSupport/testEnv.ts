/**
 * An in-process bug-intake: `handleRequest` over fake Durable Object
 * namespaces for all three classes (`ReportDrop`, `ReportIndex`,
 * `ReportThread`), a recording mailer, and a test Access JWKS — so the unit
 * suites and an end-to-end harness (a small local HTTP server in front of
 * `handle`) can drive the whole Worker, admin pages included, with no
 * `cloudflare:*` import and no test backdoor in the Worker itself.
 *
 * Wiring mirrors the real DO wrappers exactly: `ReportThread`'s destroy hook
 * calls `ReportIndex.threadEnded`, and `runDropAlarm` does what
 * `ReportDrop.alarm` does (`ReportStore.expire` into `expireReport`:
 * `ReportIndex.bundleExpired`, then the thread when the row went). Alarms never fire on their own here; a test
 * reads a DO's scheduled time with `threadAlarmAt`/`dropAlarmAt` and fires
 * it with `runThreadAlarm`/`runDropAlarm`.
 */

import { handleRequest, expireReport } from '../handlers.ts'
import { ReportStore } from '../reportStore.ts'
import { IndexStore } from '../indexStore.ts'
import { ThreadStore } from '../threadStore.ts'
import { INDEX_DO_NAME } from '../constants.ts'
import { FakeDurableObjectNamespace, type FakeDurableObjectStorage } from './fakeDurableObject.ts'
import type { BugIntakeEnv, ReportDropStub, ReportIndexStub, ReportThreadStub } from '../types.ts'
import type { RawMailSender } from '../email.ts'
import type { VerifyOptions } from '../adminAuth.ts'

export const TEST_TEAM_DOMAIN = 'hew-test.cloudflareaccess.com'
export const TEST_ACCESS_AUD = 'admin-aud'
export const TEST_ADMIN_ORIGIN = 'https://app.hew3d.com'

function base64Url(bytes: Uint8Array): string {
  let binary = ''
  for (const b of bytes) binary += String.fromCharCode(b)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

function base64UrlJson(obj: unknown): string {
  return base64Url(new TextEncoder().encode(JSON.stringify(obj)))
}

export interface TestAccess {
  /** Send as `Cf-Access-Jwt-Assertion`. */
  token: string
  /** Pass to `handleRequest`: a fake JWKS and a fixed clock. */
  authOptions: VerifyOptions
  now: number
}

/** A fresh RSA key, its JWKS, and a signed Access JWT valid for `ttlSeconds`
 *  after `now` — what `adminAuth.ts` verifies, with no network. */
export async function makeTestAccess(
  { now = Date.now(), ttlSeconds = 3600, teamDomain = TEST_TEAM_DOMAIN, aud = TEST_ACCESS_AUD } = {},
): Promise<TestAccess> {
  const { publicKey, privateKey } = await crypto.subtle.generateKey(
    { name: 'RSASSA-PKCS1-v1_5', modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: 'SHA-256' },
    true,
    ['sign', 'verify'],
  )
  const jwk = await crypto.subtle.exportKey('jwk', publicKey)
  const jwks = { keys: [{ ...jwk, kid: 'k1', alg: 'RS256' }] }
  const header = base64UrlJson({ alg: 'RS256', kid: 'k1' })
  const payload = base64UrlJson({ iss: `https://${teamDomain}`, aud, exp: Math.floor(now / 1000) + ttlSeconds })
  const signature = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', privateKey, new TextEncoder().encode(`${header}.${payload}`))
  const token = `${header}.${payload}.${base64Url(new Uint8Array(signature))}`
  return { token, now, authOptions: { getJwks: async () => jwks as never, now } }
}

export interface RecordingMailer extends RawMailSender {
  calls: Array<{ from: string; to: string; raw: string }>
  /** Each recorded message's Subject line. */
  subjects(): string[]
}

export function makeRecordingMailer(): RecordingMailer {
  const calls: Array<{ from: string; to: string; raw: string }> = []
  return {
    calls,
    async send(from, to, raw) {
      calls.push({ from, to, raw })
    },
    subjects() {
      return calls.map((c) => /^Subject: (.*)$/m.exec(c.raw)?.[1] ?? '')
    },
  }
}

export interface TestEnv {
  env: BugIntakeEnv
  mailer: RecordingMailer
  /** Set when `buildTestEnv({ admin: true })` (the default). */
  access: TestAccess | null
  /** `handleRequest` with this env's mailer and Access options. */
  handle(request: Request): Promise<Response>
  index(): IndexStore
  drop(id: string): ReportStore
  thread(id: string): ThreadStore
  dropStorage(id: string): FakeDurableObjectStorage | undefined
  threadStorage(id: string): FakeDurableObjectStorage | undefined
  threadAlarmAt(id: string): Promise<number | null>
  dropAlarmAt(id: string): Promise<number | null>
  /** Fires a thread's alarm logic at `now` (default `Date.now()`). */
  runThreadAlarm(id: string, now?: number): Promise<void>
  /** Fires a drop's alarm logic (retention or abandonment). */
  runDropAlarm(id: string): Promise<void>
  /** Headers an admin form POST needs (JWT + Origin). */
  adminHeaders(extra?: Record<string, string>): Record<string, string>
}

export interface BuildTestEnvOptions {
  /** Extra env fields (`NOTIFY_TO`, `NOTIFY_FROM`, …). */
  overrides?: Partial<BugIntakeEnv>
  /** Configure Access and mint a test JWT. Default true. */
  admin?: boolean
}

/** Builds the in-process Worker. Every DO the env ever constructs is kept,
 *  by name, so a test can inspect its storage. */
export async function buildTestEnv(options: BuildTestEnvOptions = {}): Promise<TestEnv> {
  const dropStores = new Map<string, { store: ReportStore; storage: FakeDurableObjectStorage }>()
  const threadStores = new Map<string, { store: ThreadStore; storage: FakeDurableObjectStorage }>()
  const env = {} as BugIntakeEnv

  // `FakeDurableObjectNamespace` hands the factory only the state, so the
  // name is captured through the namespace's own `get`.
  let pendingName = ''
  function named<T>(ns: FakeDurableObjectNamespace<T>): FakeDurableObjectNamespace<T> {
    const get = ns.get.bind(ns)
    ns.get = (id) => {
      pendingName = id.toString()
      return get(id)
    }
    return ns
  }

  env.REPORT_INDEX = new FakeDurableObjectNamespace<ReportIndexStub>((state) => new IndexStore(state.storage))
  const indexStore = () => env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)) as unknown as IndexStore
  env.REPORT_DROP = named(
    new FakeDurableObjectNamespace<ReportDropStub>((state) => {
      const store = new ReportStore(state.storage)
      dropStores.set(pendingName, { store, storage: state.storage as FakeDurableObjectStorage })
      return store
    }),
  )
  env.REPORT_THREAD = named(
    new FakeDurableObjectNamespace<ReportThreadStub>((state) => {
      const store = new ThreadStore(state.storage, (reportId) => indexStore().threadEnded(reportId))
      threadStores.set(pendingName, { store, storage: state.storage as FakeDurableObjectStorage })
      return store
    }),
  )
  env.IP_HASH_SECRET = 'test-secret'
  const admin = options.admin ?? true
  if (admin) {
    env.ACCESS_TEAM_DOMAIN = TEST_TEAM_DOMAIN
    env.ACCESS_AUD = TEST_ACCESS_AUD
  }
  Object.assign(env, options.overrides ?? {})

  const access = admin ? await makeTestAccess() : null
  const mailer = makeRecordingMailer()
  const drop = (id: string) => env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id)) as unknown as ReportStore
  const thread = (id: string) => env.REPORT_THREAD.get(env.REPORT_THREAD.idFromName(id)) as unknown as ThreadStore

  return {
    env,
    mailer,
    access,
    handle: (request) => handleRequest(request, env, mailer, access?.authOptions ?? {}),
    index: indexStore,
    drop,
    thread,
    dropStorage: (id) => dropStores.get(id)?.storage,
    threadStorage: (id) => threadStores.get(id)?.storage,
    threadAlarmAt: async (id) => (threadStores.get(id)?.storage.getAlarm() ?? null),
    dropAlarmAt: async (id) => (dropStores.get(id)?.storage.getAlarm() ?? null),
    runThreadAlarm: (id, now = Date.now()) => thread(id).alarm(now),
    runDropAlarm: (id) => drop(id).expire((reportId) => expireReport(env, reportId)),
    adminHeaders: (extra = {}) => ({
      ...(access ? { 'cf-access-jwt-assertion': access.token } : {}),
      origin: TEST_ADMIN_ORIGIN,
      ...extra,
    }),
  }
}
