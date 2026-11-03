import { DurableObject } from 'cloudflare:workers'

import { UploadBudgetStore } from './uploadBudgetStore.ts'
import type { DropEnv, DurableObjectStorage, UploadAdmission } from './types.ts'

/** The single admission ledger shared by every edge location and hostname. */
export class UploadBudget extends DurableObject<DropEnv> {
  private readonly store_: UploadBudgetStore

  constructor(ctx: DurableObjectState, env: DropEnv) {
    super(ctx, env)
    this.store_ = new UploadBudgetStore(ctx.storage as unknown as DurableObjectStorage)
  }

  reserve(token: string, clientHash: string, bytes: number): Promise<UploadAdmission> {
    return this.store_.reserve(token, clientHash, bytes)
  }

  resize(token: string, bytes: number): Promise<boolean> {
    return this.store_.resize(token, bytes)
  }

  release(token: string): Promise<void> {
    return this.store_.release(token)
  }

  commit(token: string): Promise<boolean> {
    return this.store_.commit(token)
  }

  alarm(): Promise<void> {
    return this.store_.alarm()
  }
}
