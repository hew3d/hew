import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import * as rec from './inputRecorder'

// A minimal PointerEvent-ish stub (jsdom isn't enabled; we only read fields).
function ptr(
  over: Partial<PointerEvent> & { button?: number; buttons?: number } = {},
): PointerEvent {
  return {
    button: over.button ?? 0,
    buttons: over.buttons ?? 0,
    shiftKey: over.shiftKey ?? false,
    altKey: over.altKey ?? false,
    ctrlKey: over.ctrlKey ?? false,
    metaKey: over.metaKey ?? false,
  } as PointerEvent
}

describe('inputRecorder', () => {
  let now = 0
  beforeEach(() => {
    now = 1000
    rec.__setClockForTest(() => now)
    rec.stop()
    rec.take() // clear any residue
  })
  afterEach(() => rec.__setClockForTest())

  it('is a no-op until start()', () => {
    rec.recordPointer('pointermove', 1, 2, ptr())
    expect(rec.isActive()).toBe(false)
    expect(rec.take()).toEqual([])
  })

  it('captures pointer events with canvas coords, buttons, and mods', () => {
    rec.start()
    now = 1005
    rec.recordPointer('pointerdown', 10, 20, ptr({ button: 0, buttons: 1, shiftKey: true }))
    const ev = rec.take()
    expect(ev).toHaveLength(1)
    const p = ev[0]
    expect(p.kind).toBe('pointerdown')
    if (p.kind === 'pointerdown') {
      expect([p.x, p.y]).toEqual([10, 20])
      expect(p.buttons).toBe(1)
      expect(p.mods.shift).toBe(true)
      expect(p.t).toBe(5) // 1005 - 1000 (start origin)
    }
  })

  it('assigns a strictly increasing global seq across kinds', () => {
    rec.start()
    rec.recordPointer('pointerdown', 0, 0, ptr({ buttons: 1 }))
    rec.recordCamera([1, 2, 3], [0, 0, 0], [0, 0, 1], 45)
    rec.recordKey('keydown', { key: 'Shift', shiftKey: true, altKey: false, ctrlKey: false, metaKey: false } as KeyboardEvent)
    const seqs = rec.take().map((e) => e.seq)
    expect(seqs).toEqual([0, 1, 2])
  })

  it('bumps the gesture id on each pointerdown; moves/up inherit it', () => {
    rec.start()
    rec.recordPointer('pointerdown', 0, 0, ptr({ buttons: 1 })) // gesture 1
    rec.recordPointer('pointermove', 1, 1, ptr({ buttons: 1 }))
    rec.recordPointer('pointerup', 1, 1, ptr({ button: 0 }))
    rec.recordPointer('pointerdown', 2, 2, ptr({ buttons: 1 })) // gesture 2
    const g = rec.take().map((e) => e.gesture)
    expect(g).toEqual([1, 1, 1, 2])
  })

  it('camera/key events between gestures carry the current gesture id', () => {
    rec.start()
    rec.recordCamera([0, 0, 5], [0, 0, 0], [0, 0, 1], 45) // before any down → gesture 0
    rec.recordPointer('pointerdown', 0, 0, ptr({ buttons: 1 })) // → gesture 1
    rec.recordCamera([0, 0, 6], [0, 0, 0], [0, 0, 1], 45) // → gesture 1
    const g = rec.take().map((e) => e.gesture)
    expect(g).toEqual([0, 1, 1])
  })

  it('start() resets seq, gesture, and the clock origin; take() clears', () => {
    rec.start()
    rec.recordPointer('pointerdown', 0, 0, ptr({ buttons: 1 }))
    expect(rec.take()).toHaveLength(1)
    expect(rec.take()).toEqual([]) // cleared
    now = 2000
    rec.start() // new origin
    now = 2003
    rec.recordPointer('pointerdown', 9, 9, ptr({ buttons: 1 }))
    const ev = rec.take()
    expect(ev[0].seq).toBe(0)
    expect(ev[0].gesture).toBe(1)
    expect(ev[0].t).toBe(3)
  })

  it('stop() halts capture but keeps the buffer for take()', () => {
    rec.start()
    rec.recordPointer('pointerdown', 0, 0, ptr({ buttons: 1 }))
    rec.stop()
    rec.recordPointer('pointermove', 5, 5, ptr({ buttons: 1 })) // ignored
    expect(rec.isActive()).toBe(false)
    expect(rec.take()).toHaveLength(1)
  })
})
