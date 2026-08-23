/**
 * A minimal usability proof for the generated TypeScript client SDK
 * (`./hewApi.gen.ts`, docs/agents/HEW_API.md §9): constructs a `HewApiClient`
 * against a mock {@link HewTransport} and drives one call through it, on
 * both the success and refusal paths. This is deliberately NOT wiring
 * the app onto the bus — that migration is future work (HEW_API.md §16)
 * — it only proves the generated file type-checks standalone and behaves
 * as documented.
 */

import { describe, expect, it } from 'vitest'
import {
  HewApiClient,
  HewApiError,
  HewErrorCode,
  type HewTransport,
  type JsonRpcRequest,
  type JsonRpcResponse,
} from './hewApi.gen'

describe('HewApiClient (smoke)', () => {
  it('dispatches a typed method call through the transport and unwraps the result', async () => {
    const seen: JsonRpcRequest[] = []
    const transport: HewTransport = {
      dispatch(request) {
        seen.push(request)
        const response: JsonRpcResponse = {
          jsonrpc: '2.0',
          id: request.id,
          // A model-mutating plain request is answered with the
          // one-command-transaction envelope (HEW_API.md §6); the client
          // unwraps results[0].
          result: {
            results: [{ sketch: 'skt_1', region_ids: ['reg_1'], region_id: 'reg_1' }],
            label: 'hew.sketch.draw_rect',
          },
        }
        return Promise.resolve(response)
      },
    }

    const client = new HewApiClient(transport)
    const result = await client.sketch.drawRect({
      plane: { ground: true },
      corner_a: [0, 0, 0],
      corner_b: [1, 1, 0],
    })

    expect(seen).toHaveLength(1)
    expect(seen[0]).toMatchObject({
      jsonrpc: '2.0',
      method: 'hew.sketch.draw_rect',
      params: { plane: { ground: true }, corner_a: [0, 0, 0], corner_b: [1, 1, 0] },
    })
    expect(result).toEqual({ sketch: 'skt_1', region_ids: ['reg_1'], region_id: 'reg_1' })
  })

  it('throws HewApiError carrying the canonical refusal shape on a refused call', async () => {
    const transport: HewTransport = {
      dispatch(request) {
        const response: JsonRpcResponse = {
          jsonrpc: '2.0',
          id: request.id,
          error: {
            code: HewErrorCode.REFUSED,
            message: 'refused',
            data: {
              refusal: 'distance_too_small',
              failed_index: 0,
              failed_method: request.method,
              detail: {},
              explanation: 'That distance is too small to build anything.',
            },
          },
        }
        return Promise.resolve(response)
      },
    }

    const client = new HewApiClient(transport)
    let error: unknown
    try {
      await client.solid.extrude({ region: 'reg_1', distance: 0 })
    } catch (e) {
      error = e
    }

    expect(error).toBeInstanceOf(HewApiError)
    const hewError = error as HewApiError
    expect(hewError.code).toBe(HewErrorCode.REFUSED)
    expect(hewError.refusal?.refusal).toBe('distance_too_small')
  })
})
