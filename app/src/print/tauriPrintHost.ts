/**
 * Tauri print host: `print_document` (shells/tauri/src-tauri/src/print.rs)
 * configures the platform print operation — paper size, orientation, zero
 * margins, 100 % — and opens the system dialog for the window's DOM. If the
 * native path reports an error, fall back to `window.print()` (works on
 * WebView2 and WebKitGTK; a no-op on macOS WKWebView, where the error is
 * surfaced instead).
 */
import { invoke } from '@tauri-apps/api/core'
import type { PrintHost, PrintSetup } from './printHost'

export class TauriPrintHost implements PrintHost {
  async defaults(): Promise<{ paperWmm: number; paperHmm: number; landscape: boolean } | null> {
    try {
      const d = await invoke<{ paper_w_mm: number; paper_h_mm: number; landscape: boolean } | null>('print_defaults')
      if (d === null || !(d.paper_w_mm > 0) || !(d.paper_h_mm > 0)) return null
      return { paperWmm: d.paper_w_mm, paperHmm: d.paper_h_mm, landscape: d.landscape }
    } catch {
      return null
    }
  }

  async screenMmPerPx(): Promise<number | null> {
    try {
      const v = await invoke<number | null>('screen_mm_per_px')
      // The shell already vets plausibility; only guard the type here.
      return typeof v === 'number' && Number.isFinite(v) && v > 0 ? v : null
    } catch {
      return null
    }
  }

  async print(setup: PrintSetup): Promise<void> {
    try {
      await invoke('print_document', {
        setup: { paper_w_mm: setup.paperWmm, paper_h_mm: setup.paperHmm, landscape: setup.landscape, job_title: setup.jobTitle },
      })
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err)
      if (typeof window !== 'undefined' && typeof window.print === 'function' && !/wkwebview|macos/i.test(msg)) {
        window.print()
        return
      }
      throw new Error(msg)
    }
  }
}
