import { describe, it, expect } from 'vitest'
import {
  resolveShellMode,
  readShellModeOverride,
  writeShellModeOverride,
  parseRecvParams,
  SHELL_MODE_OVERRIDE_KEY,
  type ResolveShellModeInput,
} from './shellMode'

/** A phone-shaped, coarse-pointer, non-Tauri, no-hash, auto-override boot —
 *  the baseline every test overrides one field of. */
function baseInput(overrides: Partial<ResolveShellModeInput> = {}): ResolveShellModeInput {
  return {
    hash: '',
    isTauri: false,
    override: 'auto',
    coarsePointer: true,
    innerWidth: 390,
    innerHeight: 844,
    ...overrides,
  }
}

describe('resolveShellMode', () => {
  it('#shop forces Shop Mode even on a desktop-shaped, fine-pointer session', () => {
    expect(
      resolveShellMode(baseInput({ hash: '#shop', coarsePointer: false, innerWidth: 1920, innerHeight: 1080 })),
    ).toBe('shop')
  })

  it('#shop forces Shop Mode over an explicit "editor" override', () => {
    expect(resolveShellMode(baseInput({ hash: '#shop', override: 'editor' }))).toBe('shop')
  })

  it('the Tauri desktop shell never enters Shop Mode, even with an explicit "shop" override', () => {
    expect(resolveShellMode(baseInput({ isTauri: true, override: 'shop' }))).toBe('editor')
  })

  it('the Tauri desktop shell never enters Shop Mode under the auto heuristic either', () => {
    expect(resolveShellMode(baseInput({ isTauri: true }))).toBe('editor')
  })

  it('an explicit "editor" override wins over a phone-shaped auto heuristic', () => {
    expect(resolveShellMode(baseInput({ override: 'editor' }))).toBe('editor')
  })

  it('an explicit "shop" override wins even on a desktop-shaped, fine-pointer session', () => {
    expect(
      resolveShellMode(baseInput({ override: 'shop', coarsePointer: false, innerWidth: 1920, innerHeight: 1080 })),
    ).toBe('shop')
  })

  it('auto: coarse pointer + phone-sized viewport → shop', () => {
    expect(resolveShellMode(baseInput())).toBe('shop')
  })

  it('auto: coarse pointer but tablet-sized viewport → editor', () => {
    expect(resolveShellMode(baseInput({ innerWidth: 1024, innerHeight: 1366 }))).toBe('editor')
  })

  it('auto: fine pointer (a touchscreen desktop) → editor regardless of size', () => {
    expect(resolveShellMode(baseInput({ coarsePointer: false }))).toBe('editor')
  })

  it('auto: the SMALLER dimension governs — a phone in landscape still qualifies', () => {
    expect(resolveShellMode(baseInput({ innerWidth: 844, innerHeight: 390 }))).toBe('shop')
  })

  it('auto: right at the threshold is NOT phone-sized (strict less-than)', () => {
    expect(resolveShellMode(baseInput({ innerWidth: 600, innerHeight: 900 }))).toBe('editor')
  })

  // Adversarial-review finding 2: a bare `#recv=…` (the shape a camera-app
  // scan actually produces — QR codes never encode `#shop&recv=…`) must
  // reach Shop Mode's own receive handling even when something else would
  // otherwise route this boot to the editor.
  describe('a bare #recv= hash forces Shop Mode (finding 2)', () => {
    const RECV_HASH = `#recv=${'a'.repeat(22)}.${'k'.repeat(43)}.Bench`

    it('forces Shop Mode over an explicit "editor" override', () => {
      expect(resolveShellMode(baseInput({ hash: RECV_HASH, override: 'editor' }))).toBe('shop')
    })

    it('forces Shop Mode over the auto heuristic on a desktop-shaped, fine-pointer session', () => {
      expect(
        resolveShellMode(baseInput({ hash: RECV_HASH, coarsePointer: false, innerWidth: 1920, innerHeight: 1080 })),
      ).toBe('shop')
    })

    it('does NOT force Shop Mode under the Tauri desktop shell — that exclusion still wins', () => {
      expect(resolveShellMode(baseInput({ hash: RECV_HASH, isTauri: true }))).toBe('editor')
    })

    it('a malformed/incomplete #recv= hash still forces Shop Mode (prefix check, not full validation)', () => {
      expect(resolveShellMode(baseInput({ hash: '#recv=not-even-close', override: 'editor' }))).toBe('shop')
    })
  })

  // The new #recv= check must not widen what forces Shop Mode beyond that
  // one prefix — an unrelated hash still falls through to the ordinary
  // override/auto-heuristic precedence untouched.
  it('an unrelated hash (e.g. #settings) never force-enters Shop Mode on its own', () => {
    expect(resolveShellMode(baseInput({ hash: '#settings', override: 'editor' }))).toBe('editor')
    expect(
      resolveShellMode(baseInput({ hash: '#settings', coarsePointer: false, innerWidth: 1920, innerHeight: 1080 })),
    ).toBe('editor')
  })
})

describe('readShellModeOverride / writeShellModeOverride', () => {
  function makeStorage(): Storage {
    const map = new Map<string, string>()
    return {
      getItem: (k: string) => map.get(k) ?? null,
      setItem: (k: string, v: string) => {
        map.set(k, v)
      },
      removeItem: (k: string) => {
        map.delete(k)
      },
      clear: () => map.clear(),
      key: () => null,
      length: 0,
    }
  }

  it('defaults to "auto" when nothing is stored', () => {
    expect(readShellModeOverride(makeStorage())).toBe('auto')
  })

  it('round-trips a written value', () => {
    const storage = makeStorage()
    writeShellModeOverride('shop', storage)
    expect(readShellModeOverride(storage)).toBe('shop')
    expect(storage.getItem(SHELL_MODE_OVERRIDE_KEY)).toBe('shop')
  })

  it('falls back to "auto" for an unrecognized stored value', () => {
    const storage = makeStorage()
    storage.setItem(SHELL_MODE_OVERRIDE_KEY, 'bogus')
    expect(readShellModeOverride(storage)).toBe('auto')
  })

  it('read tolerates a storage that throws', () => {
    const throwing: Pick<Storage, 'getItem'> = {
      getItem: () => {
        throw new Error('storage disabled')
      },
    }
    expect(readShellModeOverride(throwing)).toBe('auto')
  })

  it('write tolerates a storage that throws (best-effort)', () => {
    const throwing: Pick<Storage, 'setItem'> = {
      setItem: () => {
        throw new Error('quota exceeded')
      },
    }
    expect(() => writeShellModeOverride('editor', throwing)).not.toThrow()
  })

  it('defaults to "auto" when no storage is available at all (SSR-like)', () => {
    expect(readShellModeOverride(undefined)).toBe('auto')
    expect(() => writeShellModeOverride('shop', undefined)).not.toThrow()
  })
})

describe('parseRecvParams', () => {
  // 22-char base64url (128 bits) and 43-char base64url (256 bits) — the
  // exact shapes share-relay's token and shareCrypto's key respectively
  // produce (TOKEN_RE/KEY_RE's own doc comments).
  const TOKEN = 'a'.repeat(22)
  const KEY = 'k'.repeat(43)

  it('parses a well-formed bare fragment', () => {
    expect(parseRecvParams(`#recv=${TOKEN}.${KEY}.Bench`)).toEqual({
      token: TOKEN,
      key: KEY,
      name: 'Bench',
    })
  })

  it('parses the identical fragment carried on a full absolute URL', () => {
    expect(parseRecvParams(`https://app.hew3d.com/#recv=${TOKEN}.${KEY}.Bench`)).toEqual({
      token: TOKEN,
      key: KEY,
      name: 'Bench',
    })
  })

  // ShopApp.tsx's boot-time effect preserves `#shop` (rather than stripping
  // the hash bare) when it was already part of the incoming hash — this is
  // the shape that would produce, mirroring the retired LAN-era grammar's
  // identical `#shop&open=…` prefix.
  it('also accepts an optional leading "shop&" before recv= (the #shop&recv=… shape)', () => {
    expect(parseRecvParams(`#shop&recv=${TOKEN}.${KEY}.Bench`)).toEqual({
      token: TOKEN,
      key: KEY,
      name: 'Bench',
    })
  })

  it('percent-decodes spaces and unicode in the name', () => {
    expect(parseRecvParams(`#recv=${TOKEN}.${KEY}.Caf%C3%A9%20Table`)).toEqual({
      token: TOKEN,
      key: KEY,
      name: 'Café Table',
    })
  })

  // shareCrypto.ts's module doc: encodeURIComponent never escapes '.', so a
  // document name containing a literal '.' (e.g. "v2.hew") rides the name
  // segment verbatim — this must NOT be mistaken for a third '.'-separated
  // field. Only the FIRST TWO dots (token, then key) are split on.
  it('a name containing literal dots is never re-split', () => {
    expect(parseRecvParams(`#recv=${TOKEN}.${KEY}.v2.hew`)).toEqual({
      token: TOKEN,
      key: KEY,
      name: 'v2.hew',
    })
  })

  it('a percent-encoded name with dots decodes to the same literal-dot name', () => {
    expect(parseRecvParams(`#recv=${TOKEN}.${KEY}.v2%2Ehew`)).toEqual({
      token: TOKEN,
      key: KEY,
      name: 'v2.hew',
    })
  })

  it('returns null with no # at all', () => {
    expect(parseRecvParams('')).toBeNull()
    expect(parseRecvParams(`recv=${TOKEN}.${KEY}.Bench`)).toBeNull()
  })

  it('returns null for a fragment not starting with recv=', () => {
    expect(parseRecvParams('#shop')).toBeNull()
    expect(parseRecvParams('#settings')).toBeNull()
  })

  it('returns null for missing segments (no key, or no name)', () => {
    expect(parseRecvParams(`#recv=${TOKEN}`)).toBeNull()
    expect(parseRecvParams(`#recv=${TOKEN}.${KEY}`)).toBeNull()
    expect(parseRecvParams(`#recv=${TOKEN}.${KEY}.`)).toBeNull()
  })

  it('rejects an undersized or oversized token', () => {
    expect(parseRecvParams(`#recv=${TOKEN.slice(0, 21)}.${KEY}.Bench`)).toBeNull()
    expect(parseRecvParams(`#recv=${TOKEN}a.${KEY}.Bench`)).toBeNull()
  })

  it('rejects an undersized or oversized key', () => {
    expect(parseRecvParams(`#recv=${TOKEN}.${KEY.slice(0, 42)}.Bench`)).toBeNull()
    expect(parseRecvParams(`#recv=${TOKEN}.${KEY}k.Bench`)).toBeNull()
  })

  it('rejects a token/key containing characters outside the base64url alphabet', () => {
    expect(parseRecvParams(`#recv=${'+'.repeat(22)}.${KEY}.Bench`)).toBeNull()
    expect(parseRecvParams(`#recv=${TOKEN}.${'/'.repeat(43)}.Bench`)).toBeNull()
  })

  it('rejects an empty name segment', () => {
    expect(parseRecvParams(`#recv=${TOKEN}.${KEY}.`)).toBeNull()
  })

  it('rejects a name segment that is not valid percent-encoding', () => {
    expect(parseRecvParams(`#recv=${TOKEN}.${KEY}.%E0%A4%A`)).toBeNull()
  })
})
