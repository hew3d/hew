/**
 * GPU capability triage for the viewport — runs BEFORE the renderer exists.
 *
 * Two questions, answered once at startup:
 *
 *  1. If WebGL2 context creation fails outright, WHY — so the failure overlay
 *     can say something the user can act on instead of a generic shrug. The
 *     landscape diverged in 2025/26: Chrome 137+ removed its SwiftShader
 *     software fallback, so on a GPU-less or locked-down machine context
 *     creation simply FAILS there (recoverable only via the
 *     `EnableUnsafeSwiftShader` enterprise policy or the
 *     `--enable-unsafe-swiftshader` flag); Firefox still falls back to
 *     software (llvmpipe) and keeps working, slowly; WebKitGTK — the Linux
 *     Tauri webview — works on llvmpipe but sometimes needs rescue
 *     environment variables. `classifyGpuEnvironment` +
 *     `webglUnavailableMessage` pick the right guidance.
 *
 *  2. If context creation SUCCEEDS, is it hardware GL or a software
 *     rasterizer? On software (llvmpipe/SwiftShader) fill rate is the
 *     bottleneck, so the viewport drops antialiasing and caps the pixel
 *     ratio at 1 (`renderSettingsFor`), and shows a one-time notice.
 *     Detection is a throwaway probe context created with
 *     `failIfMajorPerformanceCaveat: true` — a browser on software GL
 *     refuses THAT context while the ordinary one succeeds. This is the
 *     only signal that works on WebKitGTK, which masks the renderer string
 *     as "Apple GPU"; where the string ISN'T masked (llvmpipe, SwiftShader)
 *     it corroborates. The probe context is disposed immediately
 *     (`WEBGL_lose_context`) and its canvas is never attached to the DOM,
 *     so it cannot disturb the real renderer.
 *
 * The decision functions are pure so the environment × failure-mode matrix
 * is unit-testable (`gpuCapability.test.ts`); only `probeGpuAcceleration`
 * and the wrappers at the bottom touch the DOM, and they never throw.
 *
 * Debug/test override: set localStorage `hew.debug.gpuProfile` to
 * `'hardware'` or `'software'` to skip the probe and force a profile. The
 * E2E suite pins `'hardware'` (playwright.config.ts) because it deliberately
 * runs Chromium on SwiftShader — without the override every spec would get
 * the software notice and the degraded render path, invalidating the visual
 * goldens that were captured before this detection existed.
 */

import { isLinux } from '../platform'
import { isTauri } from '../io/fileHost'

// ---------------------------------------------------------------------------
// Failure-path environment classification
// ---------------------------------------------------------------------------

/**
 * Which guidance family applies when WebGL2 context creation fails.
 *
 * `tauri-linux` is the WebKitGTK desktop shell (rescue env vars exist).
 * A non-Linux Tauri shell is classified `other`, NOT `chromium`, even though
 * WebView2's user agent carries "Chrome/" — telling a desktop-app user to
 * relaunch *Chrome* with a flag would be nonsense.
 */
export type GpuEnvironment = 'chromium' | 'firefox' | 'tauri-linux' | 'other'

/** Inputs to `classifyGpuEnvironment`, injected so tests can build the matrix. */
export interface EnvironmentSignals {
  userAgent: string
  isLinuxPlatform: boolean
  isTauriShell: boolean
}

/** Pure classification of the runtime for failure-message selection. */
export function classifyGpuEnvironment(signals: EnvironmentSignals): GpuEnvironment {
  if (signals.isTauriShell) {
    return signals.isLinuxPlatform ? 'tauri-linux' : 'other'
  }
  if (/Firefox\//.test(signals.userAgent)) return 'firefox'
  // Chrome, Chromium, Edge, and other Chromium derivatives all ship the
  // "Chrome/" token and all lost the software fallback in the same engine
  // change, so they share the guidance.
  if (/Chrome\/|Chromium\//.test(signals.userAgent)) return 'chromium'
  return 'other'
}

/** Title + paragraphs for the context-creation-failure overlay. */
export interface OverlayMessage {
  title: string
  lines: readonly string[]
}

/**
 * Environment-aware copy for the "WebGL2 context creation failed" overlay.
 *
 * Register follows kernelErrors.ts's ground rules: what happened in plain
 * words, then the next step as an action. Technical tokens appear only where
 * they're the thing to paste into a search box or hand to IT
 * (`EnableUnsafeSwiftShader`, `--enable-unsafe-swiftshader`, `about:config`,
 * the WebKitGTK env vars). No external links — there is no support page yet.
 */
export function webglUnavailableMessage(env: GpuEnvironment): OverlayMessage {
  const title = "Hew can't show the 3D view"
  switch (env) {
    case 'chromium':
      // "This browser", never "Chrome": this bucket also holds Edge, Brave,
      // Opera, and the other Chromium derivatives (everything carrying
      // "Chrome/" in the UA — see classifyGpuEnvironment), and telling an
      // Edge user to open Chrome's settings and relaunch Chrome would be
      // wrong. The two concrete artifacts stay verbatim, attributed to
      // Chromium-based browsers: the EnableUnsafeSwiftShader policy and the
      // --enable-unsafe-swiftshader flag are Chromium engine features, valid
      // across the family, and they're the searchable/paste-able tokens.
      return {
        title,
        lines: [
          "This browser couldn't reach your graphics hardware, and Chromium-based browsers no longer fall back to software rendering.",
          "Check that 'Use graphics acceleration when available' is turned on in the browser's system settings, then relaunch it.",
          'If this is a company-managed computer, ask IT to enable the EnableUnsafeSwiftShader policy for your browser. On your own computer, you can launch the browser with the --enable-unsafe-swiftshader flag.',
          'Firefox can run Hew on this computer without any of those changes — slowly, but it works.',
        ],
      }
    case 'firefox':
      return {
        title,
        lines: [
          "Firefox couldn't create the 3D graphics context Hew needs (WebGL2).",
          "Update your graphics drivers first — that fixes most cases. Then check that hardware acceleration is turned on in Firefox's Performance settings, and that webgl.disabled hasn't been switched on in about:config.",
        ],
      }
    case 'tauri-linux':
      return {
        title,
        lines: [
          "The system WebView couldn't create the 3D graphics context Hew needs. On Linux this usually means the WebView can't reach the GPU or its driver.",
          'Try launching Hew with these environment variables, which switch it to a slower but reliable software renderer:',
          'WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1 hew',
        ],
      }
    case 'other':
      return {
        title,
        lines: [
          "This browser couldn't create the 3D graphics context Hew needs (WebGL2).",
          'Check that your graphics drivers are installed and that hardware acceleration is turned on, then try again.',
        ],
      }
  }
}

/** `classifyGpuEnvironment` over the real runtime signals. */
export function currentGpuEnvironment(): GpuEnvironment {
  return classifyGpuEnvironment({
    userAgent: typeof navigator !== 'undefined' ? navigator.userAgent : '',
    isLinuxPlatform: isLinux,
    isTauriShell: isTauri,
  })
}

// ---------------------------------------------------------------------------
// Success-path software-rasterizer detection
// ---------------------------------------------------------------------------

/** What the throwaway probe context observed. */
export interface GpuProbeResult {
  /**
   * Whether a WebGL2 context with `failIfMajorPerformanceCaveat: true`
   * could be created. `false` on software rasterizers — and in environments
   * with no GL at all, which is why the caller must only interpret this
   * after the REAL context has been created successfully.
   */
  caveatContextOk: boolean
  /** Unmasked renderer string, or `''` when unavailable or blocked. */
  rendererString: string
}

/**
 * Known software-rasterizer names. Absence proves nothing (WebKitGTK masks
 * the string as "Apple GPU"), which is why the caveat probe is the primary
 * signal and this only corroborates.
 */
const SOFTWARE_RENDERER = /swiftshader|llvmpipe|softpipe|swrast|software rasterizer|microsoft basic render/i

/**
 * Decide "running on a software rasterizer" from probe results. Pure.
 *
 * Contract: call this only after the real (no-caveat) WebGL2 context was
 * created successfully — on that premise, a refused caveat context means
 * the context the browser DID hand out carries a major performance caveat,
 * i.e. software rendering. A hardware context passes the caveat probe and
 * carries no software renderer name, so this never returns true on real GL.
 */
export function isSoftwareRenderer(probe: GpuProbeResult): boolean {
  if (!probe.caveatContextOk) return true
  return SOFTWARE_RENDERER.test(probe.rendererString)
}

/** Renderer construction settings for a hardware vs software context. */
export interface RenderSettings {
  /** Constructor-time antialias flag (MSAA is a fill-rate multiplier). */
  antialias: boolean
  /** Upper bound for `setPixelRatio` — 1 on software, unbounded on hardware. */
  maxPixelRatio: number
}

/**
 * The opportunistic degrade: on software GL, drop antialiasing and cap the
 * pixel ratio at 1 — both are direct fill-rate wins on llvmpipe/SwiftShader,
 * where every extra fragment is a CPU-shaded fragment.
 */
export function renderSettingsFor(software: boolean): RenderSettings {
  return software
    ? { antialias: false, maxPixelRatio: 1 }
    : { antialias: true, maxPixelRatio: Number.POSITIVE_INFINITY }
}

/**
 * Create the throwaway caveat-probe context and read the renderer string.
 * Never throws; the probe context is released via `WEBGL_lose_context` and
 * its canvas never enters the DOM, so the real renderer is undisturbed.
 * In DOM-less environments (unit tests) this reports a failed probe, which
 * is harmless: there the real context fails too, taking the failure path.
 */
export function probeGpuAcceleration(): GpuProbeResult {
  let caveatContextOk = false
  let rendererString = ''
  try {
    const canvas = document.createElement('canvas')
    canvas.width = 1
    canvas.height = 1
    const gl = canvas.getContext('webgl2', { failIfMajorPerformanceCaveat: true })
    if (gl !== null) {
      caveatContextOk = true
      try {
        const ext = gl.getExtension('WEBGL_debug_renderer_info')
        if (ext !== null) {
          const renderer: unknown = gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)
          if (typeof renderer === 'string') rendererString = renderer
        }
      } catch {
        /* renderer string stays '' — the caveat result alone still decides */
      }
      gl.getExtension('WEBGL_lose_context')?.loseContext()
    }
  } catch {
    /* no DOM / hostile getContext — report a failed probe */
  }
  return { caveatContextOk, rendererString }
}

/** Everything the viewport needs to construct its renderer. */
export interface RenderProfile extends RenderSettings {
  /** True when the session runs on a software rasterizer. */
  software: boolean
  /** For logging: the unmasked renderer string, or a `forced:*` marker. */
  rendererString: string
}

const PROFILE_OVERRIDE_KEY = 'hew.debug.gpuProfile'

/** Debug/test override — see the module comment. */
function readProfileOverride(): 'hardware' | 'software' | null {
  try {
    const raw = localStorage.getItem(PROFILE_OVERRIDE_KEY)
    return raw === 'hardware' || raw === 'software' ? raw : null
  } catch {
    return null
  }
}

/**
 * Probe once (or honor the debug override) and derive the render profile.
 * Called by the viewport immediately before constructing the renderer.
 */
export function detectRenderProfile(): RenderProfile {
  const forced = readProfileOverride()
  if (forced !== null) {
    const software = forced === 'software'
    return { software, ...renderSettingsFor(software), rendererString: `forced:${forced}` }
  }
  const probe = probeGpuAcceleration()
  const software = isSoftwareRenderer(probe)
  return { software, ...renderSettingsFor(software), rendererString: probe.rendererString }
}

// ---------------------------------------------------------------------------
// One-time software-rendering notice persistence
// ---------------------------------------------------------------------------

/**
 * Persisted "already told the user" flag for the software-rendering notice —
 * the notice informs, it doesn't gate anything, so it appears once ever per
 * install rather than on every launch (same localStorage discipline as
 * settings/welcomeScreen.ts).
 */
const NOTICE_KEY = 'hew.notice.softwareRendering'

/** Whether the software-rendering notice has already been shown. */
function softwareNoticeAlreadyShown(): boolean {
  try {
    return localStorage.getItem(NOTICE_KEY) === 'shown'
  } catch {
    // Storage unavailable (privacy mode): show it — this launch may be the
    // only chance, and a repeat notice is the lesser harm.
    return false
  }
}

/** Record that the notice was shown, so later launches stay quiet. */
function markSoftwareNoticeShown(): void {
  try {
    localStorage.setItem(NOTICE_KEY, 'shown')
  } catch {
    /* storage unavailable — the notice simply shows again next launch */
  }
}

/**
 * Session-stable "show the notice now?" decision, memoized so the answer is
 * the same every time within one page load. The persistence write happens on
 * the FIRST call, but React StrictMode double-invokes the viewport's mount
 * effect in dev (mount → cleanup → mount), and a raw read-then-mark pair
 * would show the notice on the first mount and suppress it on the second —
 * i.e. never visibly show it at all. Memoizing keeps the effect idempotent.
 */
let noticeDecisionThisSession: boolean | null = null

/**
 * True exactly when the software notice should be shown this session:
 * the first session (per install) that asks. Persists the shown flag as a
 * side effect of the first call.
 */
export function shouldShowSoftwareNotice(): boolean {
  if (noticeDecisionThisSession === null) {
    noticeDecisionThisSession = !softwareNoticeAlreadyShown()
    if (noticeDecisionThisSession) markSoftwareNoticeShown()
  }
  return noticeDecisionThisSession
}

/** Test-only: clear the memoized session decision. */
export function resetSoftwareNoticeForTest(): void {
  noticeDecisionThisSession = null
}
