/**
 * share-relay Worker entry point — thin glue over `handlers.ts`'s pure
 * logic. Keeping this file free of any actual request handling is what
 * lets `handlers.test.ts` unit-test the real behavior without `miniflare`
 * or a real Durable Object; this file is exercised only by `wrangler dev`/an
 * actual deploy. `ShareDrop` is re-exported here (not just defined in
 * `shareDrop.ts`) because wrangler resolves a `durable_objects.bindings`
 * `class_name` against this entry module's exports — see README.md for the
 * full picture.
 */

import { handleRequest } from './handlers.ts'
import type { DropEnv } from './types.ts'

export { ShareDrop } from './shareDrop.ts'

export default {
  fetch(request: Request, env: DropEnv): Promise<Response> {
    return handleRequest(request, env)
  },
}
