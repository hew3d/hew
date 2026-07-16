import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import {
  classifyGpuEnvironment,
  webglUnavailableMessage,
  isSoftwareRenderer,
  renderSettingsFor,
  probeGpuAcceleration,
  detectRenderProfile,
  shouldShowSoftwareNotice,
  resetSoftwareNoticeForTest,
  type EnvironmentSignals,
  type GpuEnvironment,
} from './gpuCapability'

// ---------------------------------------------------------------------------
// Environment classification (failure-path message selection)
// ---------------------------------------------------------------------------

const UA = {
  chromeLinux:
    'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36',
  chromeWindows:
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36',
  edgeWindows:
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36 Edg/137.0.0.0',
  chromiumBare:
    'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chromium/136.0.0.0 Safari/537.36',
  firefoxLinux: 'Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0',
  firefoxWindows:
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:139.0) Gecko/20100101 Firefox/139.0',
  safariMac:
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15',
  // WebKitGTK (Linux Tauri shell) — Safari-flavored UA, no Chrome/Firefox token.
  webkitGtk:
    'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15',
} as const

function signals(
  userAgent: string,
  opts: { linux?: boolean; tauri?: boolean } = {},
): EnvironmentSignals {
  return {
    userAgent,
    isLinuxPlatform: opts.linux ?? false,
    isTauriShell: opts.tauri ?? false,
  }
}

describe('classifyGpuEnvironment', () => {
  it('classifies Chrome on any OS as chromium', () => {
    expect(classifyGpuEnvironment(signals(UA.chromeLinux, { linux: true }))).toBe('chromium')
    expect(classifyGpuEnvironment(signals(UA.chromeWindows))).toBe('chromium')
  })

  it('classifies Chromium derivatives (Edge, bare Chromium) as chromium', () => {
    expect(classifyGpuEnvironment(signals(UA.edgeWindows))).toBe('chromium')
    expect(classifyGpuEnvironment(signals(UA.chromiumBare, { linux: true }))).toBe('chromium')
  })

  it('classifies Firefox as firefox, including on Linux', () => {
    expect(classifyGpuEnvironment(signals(UA.firefoxWindows))).toBe('firefox')
    // Firefox on Linux gets Firefox guidance (its own software fallback),
    // NOT the WebKitGTK env-var recipe.
    expect(classifyGpuEnvironment(signals(UA.firefoxLinux, { linux: true }))).toBe('firefox')
  })

  it('classifies the Linux Tauri shell as tauri-linux regardless of UA', () => {
    expect(classifyGpuEnvironment(signals(UA.webkitGtk, { linux: true, tauri: true }))).toBe(
      'tauri-linux',
    )
  })

  it('classifies a non-Linux Tauri shell as other, even with a Chrome UA token', () => {
    // Windows Tauri = WebView2, whose UA carries "Chrome/". Telling a desktop
    // app user to relaunch *Chrome* with a flag would be wrong.
    expect(classifyGpuEnvironment(signals(UA.edgeWindows, { tauri: true }))).toBe('other')
    expect(classifyGpuEnvironment(signals(UA.safariMac, { tauri: true }))).toBe('other')
  })

  it('classifies unrecognized browsers (Safari, WebKitGTK outside Tauri) as other', () => {
    expect(classifyGpuEnvironment(signals(UA.safariMac))).toBe('other')
    expect(classifyGpuEnvironment(signals(UA.webkitGtk, { linux: true }))).toBe('other')
    expect(classifyGpuEnvironment(signals(''))).toBe('other')
  })
})

describe('webglUnavailableMessage', () => {
  const environments: GpuEnvironment[] = ['chromium', 'firefox', 'tauri-linux', 'other']

  it('produces a title and at least one line for every environment', () => {
    for (const env of environments) {
      const message = webglUnavailableMessage(env)
      expect(message.title.length).toBeGreaterThan(0)
      expect(message.lines.length).toBeGreaterThan(0)
      for (const line of message.lines) expect(line.length).toBeGreaterThan(0)
    }
  })

  it('contains no external links (there is no support page yet)', () => {
    for (const env of environments) {
      for (const line of webglUnavailableMessage(env).lines) {
        expect(line).not.toMatch(/https?:\/\//)
      }
    }
  })

  it('chromium: names the policy, the flag, the lost software fallback, and Firefox', () => {
    const text = webglUnavailableMessage('chromium').lines.join(' ')
    expect(text).toContain('EnableUnsafeSwiftShader')
    expect(text).toContain('--enable-unsafe-swiftshader')
    expect(text).toMatch(/no longer fall/i)
    expect(text).toContain('Firefox')
  })

  it('chromium: never says "Chrome" — the bucket also holds Edge/Brave/Opera', () => {
    // The environment is detected from the "Chrome/" UA token, which every
    // Chromium derivative carries; the copy must speak of "this browser" and
    // attribute the policy/flag to Chromium-based browsers, not Chrome.
    // (\bChrome\b does not match "Chromium", and the flag/policy tokens
    // don't contain the bare word either.)
    const text = webglUnavailableMessage('chromium').lines.join(' ')
    expect(text).not.toMatch(/\bChrome\b/)
    expect(text).toContain('Chromium-based browsers')
    expect(text).toMatch(/this browser/i)
  })

  it('firefox: points at drivers and the acceleration settings', () => {
    const text = webglUnavailableMessage('firefox').lines.join(' ')
    expect(text).toMatch(/drivers/i)
    expect(text).toMatch(/hardware acceleration/i)
    expect(text).toContain('about:config')
  })

  it('tauri-linux: names all three rescue environment variables', () => {
    const text = webglUnavailableMessage('tauri-linux').lines.join(' ')
    expect(text).toContain('WEBKIT_DISABLE_DMABUF_RENDERER=1')
    expect(text).toContain('WEBKIT_DISABLE_COMPOSITING_MODE=1')
    expect(text).toContain('LIBGL_ALWAYS_SOFTWARE=1')
  })

  it('other: generic drivers/acceleration guidance mentioning WebGL2', () => {
    const text = webglUnavailableMessage('other').lines.join(' ')
    expect(text).toContain('WebGL2')
    expect(text).toMatch(/drivers/i)
  })
})

// ---------------------------------------------------------------------------
// Software-rasterizer decision (success path)
// ---------------------------------------------------------------------------

describe('isSoftwareRenderer', () => {
  it('is false on hardware: caveat probe passes, hardware renderer string', () => {
    expect(
      isSoftwareRenderer({
        caveatContextOk: true,
        rendererString: 'NVIDIA GeForce RTX 3060/PCIe/SSE2',
      }),
    ).toBe(false)
    expect(
      isSoftwareRenderer({
        caveatContextOk: true,
        rendererString: 'ANGLE (Apple, ANGLE Metal Renderer: Apple M2, Unspecified Version)',
      }),
    ).toBe(false)
  })

  it('is false when the caveat probe passes and the string is masked or empty', () => {
    // WebKitGTK masks the renderer as "Apple GPU" even on real hardware; the
    // caveat probe is the authority, so a masked string must NOT flag software.
    expect(isSoftwareRenderer({ caveatContextOk: true, rendererString: 'Apple GPU' })).toBe(false)
    expect(isSoftwareRenderer({ caveatContextOk: true, rendererString: '' })).toBe(false)
  })

  it('is true whenever the caveat probe fails, regardless of the string', () => {
    // The llvmpipe-under-WebKitGTK case: string masked, probe refused.
    expect(isSoftwareRenderer({ caveatContextOk: false, rendererString: 'Apple GPU' })).toBe(true)
    expect(isSoftwareRenderer({ caveatContextOk: false, rendererString: '' })).toBe(true)
    expect(
      isSoftwareRenderer({ caveatContextOk: false, rendererString: 'NVIDIA GeForce RTX 3060' }),
    ).toBe(true)
  })

  it('corroborates via known software renderer strings even if the probe passes', () => {
    const softwareStrings = [
      'Mesa llvmpipe (LLVM 15.0.7, 256 bits)',
      'Google SwiftShader',
      'ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver)',
      'softpipe',
      'Mesa/X.org swrast',
      'Microsoft Basic Render Driver',
    ]
    for (const rendererString of softwareStrings) {
      expect(isSoftwareRenderer({ caveatContextOk: true, rendererString })).toBe(true)
    }
  })
})

describe('renderSettingsFor', () => {
  it('software: antialias off, pixel ratio capped at 1', () => {
    expect(renderSettingsFor(true)).toEqual({ antialias: false, maxPixelRatio: 1 })
  })

  it('hardware: antialias on, pixel ratio unbounded', () => {
    const settings = renderSettingsFor(false)
    expect(settings.antialias).toBe(true)
    expect(settings.maxPixelRatio).toBe(Number.POSITIVE_INFINITY)
    // The viewport applies the cap as min(devicePixelRatio, maxPixelRatio):
    expect(Math.min(2, settings.maxPixelRatio)).toBe(2)
    expect(Math.min(2, renderSettingsFor(true).maxPixelRatio)).toBe(1)
  })
})

// ---------------------------------------------------------------------------
// DOM-less / storage-backed behavior (node env: no document, no localStorage)
// ---------------------------------------------------------------------------

class FakeStorage {
  private store = new Map<string, string>()
  getItem(key: string): string | null {
    return this.store.has(key) ? this.store.get(key)! : null
  }
  setItem(key: string, value: string): void {
    this.store.set(key, value)
  }
  removeItem(key: string): void {
    this.store.delete(key)
  }
  clear(): void {
    this.store.clear()
  }
}

class ThrowingStorage {
  getItem(): string | null {
    throw new Error('storage unavailable')
  }
  setItem(): void {
    throw new Error('storage unavailable')
  }
}

let originalLocalStorage: unknown

beforeEach(() => {
  originalLocalStorage = (globalThis as { localStorage?: unknown }).localStorage
  ;(globalThis as { localStorage?: unknown }).localStorage = new FakeStorage()
  resetSoftwareNoticeForTest()
})

afterEach(() => {
  ;(globalThis as { localStorage?: unknown }).localStorage = originalLocalStorage
  resetSoftwareNoticeForTest()
})

describe('probeGpuAcceleration (no DOM)', () => {
  it('reports a failed probe without throwing', () => {
    // In this node environment there is no `document`; the probe must swallow
    // that and report "no caveat-free context" — harmless, because the real
    // context creation fails too and takes the failure-overlay path.
    expect(probeGpuAcceleration()).toEqual({ caveatContextOk: false, rendererString: '' })
  })
})

describe('detectRenderProfile', () => {
  it('honors the hardware override without probing', () => {
    localStorage.setItem('hew.debug.gpuProfile', 'hardware')
    const profile = detectRenderProfile()
    expect(profile.software).toBe(false)
    expect(profile.antialias).toBe(true)
    expect(profile.maxPixelRatio).toBe(Number.POSITIVE_INFINITY)
    expect(profile.rendererString).toBe('forced:hardware')
  })

  it('honors the software override', () => {
    localStorage.setItem('hew.debug.gpuProfile', 'software')
    const profile = detectRenderProfile()
    expect(profile.software).toBe(true)
    expect(profile.antialias).toBe(false)
    expect(profile.maxPixelRatio).toBe(1)
    expect(profile.rendererString).toBe('forced:software')
  })

  it('ignores junk override values and falls through to the probe', () => {
    localStorage.setItem('hew.debug.gpuProfile', 'turbo')
    // No DOM here, so the fallthrough probe reports software — the point is
    // that the junk value neither throws nor forces a profile.
    const profile = detectRenderProfile()
    expect(profile.rendererString).not.toContain('forced')
    expect(profile.software).toBe(true)
  })
})

describe('software-notice show-once decision', () => {
  it('shows on the first session and persists the shown flag', () => {
    expect(shouldShowSoftwareNotice()).toBe(true)
    expect(localStorage.getItem('hew.notice.softwareRendering')).toBe('shown')
  })

  it('is stable within a session (StrictMode remounts must re-show, not flicker)', () => {
    expect(shouldShowSoftwareNotice()).toBe(true)
    // A second mount in the SAME session (React StrictMode double-invokes the
    // viewport effect in dev) must get the same answer, even though the
    // persisted flag is already set.
    expect(shouldShowSoftwareNotice()).toBe(true)
  })

  it('stays quiet in a later session once the flag is persisted', () => {
    shouldShowSoftwareNotice()
    resetSoftwareNoticeForTest() // "next launch": new session, same storage
    expect(shouldShowSoftwareNotice()).toBe(false)
  })

  it('shows (and never throws) when storage is unavailable', () => {
    ;(globalThis as { localStorage?: unknown }).localStorage = new ThrowingStorage()
    expect(shouldShowSoftwareNotice()).toBe(true)
    resetSoftwareNoticeForTest()
    expect(shouldShowSoftwareNotice()).toBe(true) // can't persist — shows again
  })
})
