/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * The Fillet tool's only path to the kernel's blend previews.
 *
 * The tool asks a `BlendPreviewer` and reads a plain-data answer, so it does
 * not know whether the answer was computed on the UI thread (today) or in a
 * Web Worker (later). Requests and results carry only strings, numbers and
 * typed arrays: no kernel handles, no three.js objects, nothing that cannot
 * be posted across a thread boundary.
 *
 * Contract: `request` may answer directly (a synchronous previewer) or with
 * a promise (an asynchronous one). A promise resolves to `'cancelled'` when
 * the request was superseded or `cancel()` ran before its answer. Consumers
 * go through `whenReady`, which applies a direct answer at once (so event
 * timing is unchanged for a synchronous previewer) and applies an
 * asynchronous one only if its request is still the latest.
 */

import type { Scene as WasmScene } from '../../wasm/loader'
import { parseKernelErrorCode } from '../../kernelErrors'

/** What to preview. `spec` is the kernel `BlendSpec` (create) or profile
 *  (edit) JSON, already in the object's own units. */
export type BlendPreviewRequest =
  | {
      kind: 'create'
      object: bigint
      spec: string
      /** True: preview as the commit would make it (a typed size). False:
       *  the quick preview, asked again on every pointer move. */
      typed: boolean
    }
  | { kind: 'edit'; object: bigint; blendId: number; spec: string }

export type BlendPreviewResult =
  | {
      kind: 'ok'
      /** The patch, in the object's own frame. */
      positions: Float32Array
      normals: Float32Array
      indices: Uint32Array
      /** Rail segments, pairs of points. */
      rails: Float32Array
      /** The faces the patch replaces. */
      replacedFaces: bigint[]
      /** The kernel held the size at a limit. `maxSize` and `limitedBy` are
       *  meaningful only then (0 and '' otherwise), in the object's units. */
      clamped: boolean
      maxSize: number
      limitedBy: string
      threeWayCorners: number
      /** How long the kernel call took. */
      elapsedMs: number
    }
  | {
      kind: 'refused'
      /** The kernel error text as thrown. */
      message: string
      /** Its leading error code, when it has one. */
      code: string | null
      elapsedMs: number
    }

export interface BlendPreviewer {
  /** A result now, or a promise of one (or of `'cancelled'`). */
  request(req: BlendPreviewRequest): BlendPreviewResult | Promise<BlendPreviewResult | 'cancelled'>
  /** Drops what is in flight: its promise resolves to `'cancelled'`. */
  cancel(): void
  dispose(): void
}

/** A value now, or a promise of it that may be cancelled. */
export type Deferred<T> = T | Promise<T | 'cancelled'>

function isPromiseLike(v: unknown): v is Promise<unknown> {
  return typeof v === 'object' && v !== null && typeof (v as { then?: unknown }).then === 'function'
}

/**
 * Applies a preview answer through one code path for both kinds of
 * previewer. A direct answer is applied immediately. A promised one is
 * applied when it arrives, unless it was cancelled or `isCurrent()` says a
 * newer request has replaced it (latest wins); then `'cancelled'` comes back
 * and nothing is applied.
 */
export function whenReady<T>(
  ret: BlendPreviewResult | Promise<BlendPreviewResult | 'cancelled'>,
  isCurrent: () => boolean,
  apply: (r: BlendPreviewResult) => Deferred<T>,
): Deferred<T> {
  if (!isPromiseLike(ret)) return apply(ret)
  return ret.then((r): Deferred<T> | 'cancelled' => (r === 'cancelled' || !isCurrent() ? 'cancelled' : apply(r)))
}

/** Runs `then` when `d` is a promise that settles with a value (not
 *  `'cancelled'`): the answer came after the event that asked, so whatever
 *  repaints must be asked for. A direct value does nothing. */
export function afterLate<T>(d: Deferred<T>, then: () => void): void {
  if (!isPromiseLike(d)) return
  void d.then((v) => {
    if (v !== 'cancelled') then()
  })
}

/** The preview computed on the calling thread, synchronously. */
export class SyncBlendPreviewer implements BlendPreviewer {
  constructor(
    private readonly scene: WasmScene,
    private readonly now: () => number,
  ) {}

  request(req: BlendPreviewRequest): BlendPreviewResult {
    const started = this.now()
    let preview: ReturnType<WasmScene['blend_preview']>
    try {
      preview =
        req.kind === 'edit'
          ? this.scene.resize_blend_preview(req.object, req.blendId, req.spec)
          : req.typed
            ? this.scene.blend_preview_typed(req.object, req.spec)
            : this.scene.blend_preview(req.object, req.spec)
    } catch (err) {
      const elapsedMs = this.now() - started
      return {
        kind: 'refused',
        message: err instanceof Error ? err.message : String(err),
        code: parseKernelErrorCode(err),
        elapsedMs,
      }
    }
    const elapsedMs = this.now() - started
    try {
      const clamped = preview.clamped()
      return {
        kind: 'ok',
        positions: preview.positions(),
        normals: preview.normals(),
        indices: preview.indices(),
        rails: preview.rail_positions(),
        replacedFaces: Array.from(preview.replaced_faces()),
        clamped,
        maxSize: clamped ? preview.max_size() : 0,
        limitedBy: clamped ? preview.limited_by() : '',
        threeWayCorners: preview.three_way_corners(),
        elapsedMs,
      }
    } finally {
      preview.free()
    }
  }

  cancel(): void {}

  dispose(): void {}
}
