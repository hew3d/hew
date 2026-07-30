import { describe, it, expect } from 'vitest'
import { CleanModifierTap } from './cleanModifierTap'

const isCtrl = (key: string) => key === 'Control'
const isCtrlOrMeta = (key: string) => key === 'Control' || key === 'Meta'

describe('CleanModifierTap (same-tool tap)', () => {
  it('fires when the same tool is still active at keyup', () => {
    const tap = new CleanModifierTap<string>(isCtrl)
    tap.onKeyDown({ key: 'Control', repeat: false }, 'Scale')
    const fired = tap.onKeyUp({ key: 'Control' }, 'Scale')
    expect(fired).toBe('Scale')
  })

  it('does not fire for a non-modifier key', () => {
    const tap = new CleanModifierTap<string>(isCtrl)
    tap.onKeyDown({ key: 'Control', repeat: false }, 'Scale')
    expect(tap.onKeyUp({ key: 'a' }, 'Scale')).toBeNull()
  })

  it('a chord (another key between down and up) is not a tap', () => {
    const tap = new CleanModifierTap<string>(isCtrl)
    tap.onKeyDown({ key: 'Control', repeat: false }, 'Scale')
    tap.onKeyDown({ key: 'z', repeat: false }, 'Scale') // Ctrl+Z
    expect(tap.onKeyUp({ key: 'Control' }, 'Scale')).toBeNull()
  })

  it('keydown autorepeat does not re-arm or disturb an already-armed tap', () => {
    const tap = new CleanModifierTap<string>(isCtrl)
    tap.onKeyDown({ key: 'Control', repeat: false }, 'Scale')
    tap.onKeyDown({ key: 'Control', repeat: true }, 'Scale')
    tap.onKeyDown({ key: 'Control', repeat: true }, 'Scale')
    expect(tap.onKeyUp({ key: 'Control' }, 'Scale')).toBe('Scale')
  })

  it('a second keyup after the tap already resolved is a no-op', () => {
    const tap = new CleanModifierTap<string>(isCtrl)
    tap.onKeyDown({ key: 'Control', repeat: false }, 'Scale')
    expect(tap.onKeyUp({ key: 'Control' }, 'Scale')).toBe('Scale')
    expect(tap.onKeyUp({ key: 'Control' }, 'Scale')).toBeNull()
  })

  it('watches either Control or Meta when configured to (Push/Pull chord)', () => {
    const tap = new CleanModifierTap<string>(isCtrlOrMeta)
    tap.onKeyDown({ key: 'Meta', repeat: false }, 'Push/Pull')
    expect(tap.onKeyUp({ key: 'Meta' }, 'Push/Pull')).toBe('Push/Pull')
  })
})

describe('CleanModifierTap (mid-hold tool switch — the adversarial-review finding)', () => {
  it('holding Ctrl on tool A, switching to tool B, releasing Ctrl fires NEITHER toggle', () => {
    const tap = new CleanModifierTap<string>(isCtrl)
    tap.onKeyDown({ key: 'Control', repeat: false }, 'Scale')
    // Mid-hold tool switch — no keyup in between, just the active tool changing.
    const fired = tap.onKeyUp({ key: 'Control' }, 'Push/Pull')
    expect(fired).toBeNull()
  })

  it('the reverse direction also fires nothing', () => {
    const tap = new CleanModifierTap<string>(isCtrl)
    tap.onKeyDown({ key: 'Control', repeat: false }, 'Push/Pull')
    const fired = tap.onKeyUp({ key: 'Control' }, 'Scale')
    expect(fired).toBeNull()
  })

  it('reset() clears an in-flight arm so a later keyup on the new tool cannot resolve it', () => {
    const tap = new CleanModifierTap<string>(isCtrl)
    tap.onKeyDown({ key: 'Control', repeat: false }, 'Scale')
    tap.reset() // e.g. Viewport's switchToolRef / beginDragMove
    expect(tap.onKeyUp({ key: 'Control' }, 'Push/Pull')).toBeNull()
  })

  it('after a mid-hold switch, a FRESH clean tap on the new tool still fires normally', () => {
    const tap = new CleanModifierTap<string>(isCtrl)
    tap.onKeyDown({ key: 'Control', repeat: false }, 'Scale')
    tap.onKeyUp({ key: 'Control' }, 'Push/Pull') // dropped, tool switched mid-hold
    // A brand-new tap, entirely on the new tool.
    tap.onKeyDown({ key: 'Control', repeat: false }, 'Push/Pull')
    expect(tap.onKeyUp({ key: 'Control' }, 'Push/Pull')).toBe('Push/Pull')
  })
})
