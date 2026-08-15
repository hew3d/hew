/**
 * orientation — Shop Mode's ONE decision point for portrait vs. landscape
 * (design_handoff_shop_mode/README.md §5 "Landscape" + the "Decisions"
 * bullet: "Portrait: chrome stacks top/bottom. Landscape: chrome moves to
 * the sides"). `ShopApp.tsx` calls `useShopOrientation()` once at its root
 * and threads the result down as a plain prop (`PartsSheet`, `DocumentMenu`,
 * `SettingsMenu`, `UnitPicker`) — no other component queries `window.innerWidth` or
 * `matchMedia` itself, so the whole chrome tree can never disagree about
 * which layout it's in.
 *
 * The decision is a pure function of the live viewport's aspect ratio
 * (`computeOrientation`, unit-tested directly) — `aspect > 1` (wider than
 * tall) reads as landscape, matching the prototype's own `orient` toggle
 * and the design's 844x390 landscape / 390x844 portrait targets. A SQUARE
 * viewport (aspect exactly 1) reads as portrait: the design has no
 * "square" layout, and Shop Mode's portrait chrome (stacked top/bottom) is
 * the one that degrades best when there's no room to spare on either axis.
 *
 * `useShopOrientation` re-evaluates on both `resize` (desktop window drags,
 * mobile browser chrome show/hide, and — practically — every real device
 * rotation, which fires `resize` well before or without ever firing
 * `orientationchange` on some browsers) and `orientationchange` (belt and
 * suspenders on the browsers that do still fire it, and on the odd device
 * that resizes AFTER `orientationchange` rather than before).
 */
import { useEffect, useState } from 'react'

export type ShopOrientation = 'portrait' | 'landscape'

/** Pure decision from live pixel dimensions — `width/height > 1` reads as
 *  landscape. Exported (not just the hook) so callers with an already-known
 *  size (a test, or a measured container rather than the whole window) can
 *  reuse the exact same rule without mounting anything. */
export function computeOrientation(widthPx: number, heightPx: number): ShopOrientation {
  return widthPx / heightPx > 1 ? 'landscape' : 'portrait'
}

function readWindowOrientation(): ShopOrientation {
  if (typeof window === 'undefined') return 'portrait'
  return computeOrientation(window.innerWidth, window.innerHeight)
}

/** Live `'portrait' | 'landscape'`, re-evaluated on `resize`/
 *  `orientationchange`. The single subscription point every Shop Mode
 *  chrome component's orientation prop ultimately derives from — see the
 *  module doc for why nothing else should call `matchMedia`/read
 *  `innerWidth` directly. */
export function useShopOrientation(): ShopOrientation {
  const [orientation, setOrientation] = useState<ShopOrientation>(readWindowOrientation)

  useEffect(() => {
    const update = () => setOrientation(readWindowOrientation())
    // Read once more on mount/effect-run — covers a jsdom/SSR-style first
    // render where `useState`'s initializer ran before `window` was the
    // real thing, and costs nothing extra in a real browser (same value).
    update()
    window.addEventListener('resize', update)
    window.addEventListener('orientationchange', update)
    return () => {
      window.removeEventListener('resize', update)
      window.removeEventListener('orientationchange', update)
    }
  }, [])

  return orientation
}
