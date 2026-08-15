// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { isArQuickLookCandidate, launchArQuickLook } from './arQuickLook'

/** Real UA strings (trimmed of vendor cruft that doesn't matter here) for
 *  each row of the detection matrix. */
const UA = {
  iphoneSafari:
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1',
  // iPadOS 13+ masquerades as desktop Safari on "Macintosh" — indistinguishable
  // from a real Mac by UA string alone, hence the maxTouchPoints check.
  ipadMasquerade:
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_6) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15',
  macSafari:
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15',
  chromeAndroid:
    'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Mobile Safari/537.36',
  chromeIOS:
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/125.0.6422.80 Mobile/15E148 Safari/604.1',
} as const

/** Stub `navigator.userAgent`/`navigator.maxTouchPoints` for one call,
 *  restoring the originals after — mirrors `platform.test.ts`'s
 *  `withMatchMedia` stub-and-restore shape. jsdom's `navigator` properties
 *  are configurable, so `Object.defineProperty` can override the getters. */
function withNavigator<T>(userAgent: string, maxTouchPoints: number, fn: () => T): T {
  const uaDesc = Object.getOwnPropertyDescriptor(Navigator.prototype, 'userAgent')
  const touchDesc = Object.getOwnPropertyDescriptor(Navigator.prototype, 'maxTouchPoints')
  Object.defineProperty(navigator, 'userAgent', { value: userAgent, configurable: true })
  Object.defineProperty(navigator, 'maxTouchPoints', { value: maxTouchPoints, configurable: true })
  try {
    return fn()
  } finally {
    if (uaDesc) Object.defineProperty(navigator, 'userAgent', uaDesc)
    else delete (navigator as unknown as Record<string, unknown>).userAgent
    if (touchDesc) Object.defineProperty(navigator, 'maxTouchPoints', touchDesc)
    else delete (navigator as unknown as Record<string, unknown>).maxTouchPoints
  }
}

describe('isArQuickLookCandidate', () => {
  it('is true on iPhone Safari', () => {
    expect(withNavigator(UA.iphoneSafari, 5, () => isArQuickLookCandidate())).toBe(true)
  })

  it('is true on iPadOS masquerading as macOS Safari (maxTouchPoints > 1)', () => {
    expect(withNavigator(UA.ipadMasquerade, 5, () => isArQuickLookCandidate())).toBe(true)
  })

  it('is true in a home-screen standalone app (UA drops the Safari token)', () => {
    // "Add to Home Screen" standalone mode: same engine, same Quick Look
    // hand-off, but the UA ends at Mobile/15E148 with no trailing
    // Safari/xxx — navigator.standalone (Safari-only) is the signal.
    const standaloneUa =
      'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148'
    const desc = Object.getOwnPropertyDescriptor(navigator, 'standalone')
    Object.defineProperty(navigator, 'standalone', { value: true, configurable: true })
    try {
      expect(withNavigator(standaloneUa, 5, () => isArQuickLookCandidate())).toBe(true)
    } finally {
      if (desc) Object.defineProperty(navigator, 'standalone', desc)
      else delete (navigator as unknown as Record<string, unknown>).standalone
    }
  })

  it('is still false in a standalone-less iOS webview UA with no Safari token', () => {
    // Same token-less UA but navigator.standalone absent (an embedded
    // webview, not a home-screen app): no reliable Quick Look hand-off.
    const webviewUa =
      'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148'
    expect(withNavigator(webviewUa, 5, () => isArQuickLookCandidate())).toBe(false)
  })

  it('is false on real macOS Safari (same UA family, no touch points)', () => {
    expect(withNavigator(UA.macSafari, 0, () => isArQuickLookCandidate())).toBe(false)
  })

  it('is false on Chrome/Android', () => {
    expect(withNavigator(UA.chromeAndroid, 5, () => isArQuickLookCandidate())).toBe(false)
  })

  it('is false on Chrome for iOS (CriOS token, despite an iPhone UA)', () => {
    expect(withNavigator(UA.chromeIOS, 5, () => isArQuickLookCandidate())).toBe(false)
  })

  it('returns false rather than throwing when navigator is unavailable', () => {
    const original = globalThis.navigator
    // @ts-expect-error — simulating an environment without `navigator`
    delete globalThis.navigator
    try {
      expect(isArQuickLookCandidate()).toBe(false)
    } finally {
      Object.defineProperty(globalThis, 'navigator', { value: original, configurable: true })
    }
  })
})

describe('launchArQuickLook', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  it('clicks an <a rel="ar"> with an <img> child and the .usdz-suffixed download name', () => {
    let capturedAnchor: HTMLAnchorElement | null = null
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      // Capture the anchor mid-flight, before launchArQuickLook removes it
      // from the document again.
      capturedAnchor = this
    })
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock-url')
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})

    launchArQuickLook(new Uint8Array([1, 2, 3]), 'Cafe Table')

    expect(clickSpy).toHaveBeenCalledTimes(1)
    expect(capturedAnchor).not.toBeNull()
    const anchor = capturedAnchor as unknown as HTMLAnchorElement
    expect(anchor.getAttribute('rel')).toBe('ar')
    expect(anchor.download).toBe('Cafe Table.usdz')
    expect(anchor.querySelector('img')).not.toBeNull()
  })

  it('does not double-suffix a name that already ends in .usdz', () => {
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {})
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock-url')
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})

    let capturedAnchor: HTMLAnchorElement | null = null
    clickSpy.mockImplementation(function (this: HTMLAnchorElement) {
      capturedAnchor = this
    })

    launchArQuickLook(new Uint8Array([1]), 'Cafe Table.usdz')

    expect((capturedAnchor as unknown as HTMLAnchorElement).download).toBe('Cafe Table.usdz')
  })

  it('does not remove the anchor from the document before click() runs', () => {
    let wasConnectedAtClick = false
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      wasConnectedAtClick = this.isConnected
    })
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock-url')
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})

    launchArQuickLook(new Uint8Array([1]), 'Model')

    expect(wasConnectedAtClick).toBe(true)
  })

  it('schedules the object URL revocation on a delay rather than synchronously', () => {
    vi.useFakeTimers()
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {})
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock-url')
    const revokeSpy = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})

    launchArQuickLook(new Uint8Array([1]), 'Model')

    // Not revoked synchronously — Safari's Quick Look fetch of the blob URL
    // is asynchronous, so an immediate revoke would risk racing it.
    expect(revokeSpy).not.toHaveBeenCalled()
    vi.runAllTimers()
    expect(revokeSpy).toHaveBeenCalledWith('blob:mock-url')
  })
})
