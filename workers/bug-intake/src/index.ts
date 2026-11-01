/**
 * bug-intake Worker entry point — thin glue over `handlers.ts`'s pure
 * logic, same split as share-relay's `index.ts`. This is the one module
 * that wires the real `cloudflare:workers`-based Durable Object classes and
 * the real `cloudflare:email`-based mailer together; `handlers.test.ts`
 * never imports this file.
 */

import { handleRequest } from './handlers.ts'
import { createMailer } from './emailMailer.ts'
import type { BugIntakeEnv } from './types.ts'

export { ReportDrop } from './reportDrop.ts'
export { ReportIndex } from './reportIndex.ts'
export { ReportThread } from './reportThread.ts'

export default {
  async fetch(request: Request, env: BugIntakeEnv): Promise<Response> {
    // `handleRequest` already catches everything it can anticipate and maps
    // it to §8's `503 {"error":"unavailable"}` contract (see its doc). This
    // is one more layer on top of that, at the actual runtime boundary —
    // belt and suspenders against a bug in `createMailer`/binding wiring
    // itself throwing before `handleRequest` even gets to run its own
    // try/catch.
    try {
      return await handleRequest(request, env, createMailer(env))
    } catch (err) {
      console.error('bug-intake: unhandled error at the fetch boundary', err)
      return new Response(JSON.stringify({ error: 'unavailable' }), {
        status: 503,
        headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
      })
    }
  },
}
