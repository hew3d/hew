/**
 * Test helpers for mocking `Scene.api_dispatch`, whose answer is tagged
 * (`app/src/api/dispatchResult.ts`): a mock that builds plain reply frames
 * wraps them here.
 */

/** A reply frame as `Scene.api_dispatch` returns it. */
export function tagged(frame: string): string {
  return JSON.stringify({ reply: frame })
}

/** Wraps a mock that answers plain reply frames (or `undefined`) so it
 * answers them tagged. */
export function taggedDispatch<A extends unknown[]>(
  answer: (...args: A) => string | undefined,
): (...args: A) => string | undefined {
  return (...args: A) => {
    const frame = answer(...args)
    return frame === undefined ? undefined : tagged(frame)
  }
}
