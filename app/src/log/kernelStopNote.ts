/**
 * kernelStopNote — what a batch operation does with a failed kernel call
 * (KB-38). A wasm trap (`WebAssembly.RuntimeError`) is not a refusal: the
 * kernel's stack unwound without running anything, the Scene stays
 * borrowed, and every later call throws "recursive use of an object". The
 * open paths already hand such a trap to the crash screen
 * (App.tsx's `noteOpenFailure`); a Model Health batch that capped
 * thousands of objects hit the same stop and showed an ordinary refusal
 * toast instead, so the next render tripped over the borrow and the generic
 * crash screen named nothing.
 */
import { t } from '@lingui/core/macro'
import * as LogStore from './LogStore'
import { getPanicCapture, recordKernelStop } from './panicCapture'
import { kernelMemoryBytes } from '../wasm/kernelMemory'
import { formatBytes } from './reportBundle'

/** Kernel memory this close to the 4 GiB a wasm32 kernel can address, when
 *  a trap ends an operation, means it ran out of memory (see App.tsx). */
const KERNEL_MEMORY_NEAR_CEILING = 3 * 1024 * 1024 * 1024

/** If `err` is a kernel stop (a wasm trap the panic hook did not catch),
 *  record it for the crash screen and return true: the caller must not show
 *  a refusal toast. Returns false for an ordinary typed refusal, or when a
 *  panic was already captured (that capture and its crash screen stand). */
/* oxlint-disable lingui/no-unlocalized-strings -- the log line and recorded message go into the diagnostic log and bug-report bundle, English for the maintainer; only the crash-screen headline/detail are wrapped */
export function noteKernelStop(what: string, err: unknown): boolean {
  if (getPanicCapture() !== null || !(err instanceof WebAssembly.RuntimeError)) return false
  const memory = kernelMemoryBytes()
  const memoryNote = memory !== null ? `; kernel memory ${formatBytes(memory)}` : ''
  const outOfMemory = memory !== null && memory >= KERNEL_MEMORY_NEAR_CEILING
  LogStore.log.warn('app', `${what} stopped the kernel${memoryNote}${outOfMemory ? '; out of memory' : '; kernel trap'}`)
  if (outOfMemory) {
    recordKernelStop(`Out of memory during ${what}${memoryNote}`, {
      headline: t`This model is too large for Hew to work on`,
      detail: t`Hew ran out of memory partway through a repair. Hew can use up to 4 GB of memory, and this model needs more. Reload to keep working, and use Report this crash to send what happened.`,
    })
  } else {
    recordKernelStop(`Kernel trap during ${what}: ${err.message}${memoryNote}`, {
      headline: t`Hew stopped during a repair`,
      detail: t`Hew stopped partway through a repair and can't continue. Reload to keep working, and use Report this crash to send what happened.`,
    })
  }
  return true
}
/* oxlint-enable lingui/no-unlocalized-strings */
