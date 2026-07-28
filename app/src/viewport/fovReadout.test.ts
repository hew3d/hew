import { describe, expect, it } from 'vitest'
import { activeCameraToolForName, formatFov, fovReadoutText } from './fovReadout'

describe('fovReadoutText — Zoom tool VCB persistence (playtest finding 4b)', () => {
  it('shows the current fov as soon as the Zoom camera mode is active, with no typing required', () => {
    expect(fovReadoutText('Zoom', 'perspective', 45)).toBe('45.0° · 43.5 mm')
  })

  it('keeps showing (and tracking) the current fov during a cursor drag — no typing ever starts', () => {
    // A dolly drag doesn't itself change fov (WalkTool.test.ts-style: the
    // Zoom tool's cursor gesture moves the camera, not the lens), but the
    // readout must still reflect whatever fov IS, continuously — not go
    // blank just because the user never typed a digit.
    expect(fovReadoutText('Zoom', 'perspective', 30)).toBe('30.0° · 67.2 mm')
    expect(fovReadoutText('Zoom', 'perspective', 30)).toBe('30.0° · 67.2 mm') // same tool session, still shown
  })

  it('updates immediately after a typed commit changes fovDeg', () => {
    expect(fovReadoutText('Zoom', 'perspective', 90)).toBe('90.0° · 18.0 mm')
  })

  it('is blank under parallel projection — no lens to report', () => {
    expect(fovReadoutText('Zoom', 'parallel', 45)).toBe('')
  })

  it('is blank whenever the Zoom camera mode is not the active one', () => {
    expect(fovReadoutText('Orbit', 'perspective', 45)).toBe('')
    expect(fovReadoutText('Pan', 'perspective', 45)).toBe('')
    expect(fovReadoutText('ZoomWindow', 'perspective', 45)).toBe('')
    expect(fovReadoutText(null, 'perspective', 45)).toBe('')
  })

  it('formats degrees to one decimal place, matching every other angle VCB in the app, plus the mm equivalent (camera-playtest2.md §1)', () => {
    expect(formatFov(45)).toBe('45.0° · 43.5 mm')
    expect(formatFov(45.06)).toBe('45.1° · 43.4 mm')
    expect(formatFov(1)).toBe('1.0° · 2062.6 mm')
  })

  it('degrees stay the primary unit — always first', () => {
    expect(formatFov(45).startsWith('45.0°')).toBe(true)
  })
})

describe('activeCameraToolForName — switchToolRef\'s single activeCameraTool source (finding C)', () => {
  it('maps the four camera modes to their activeCameraTool value', () => {
    expect(activeCameraToolForName('Orbit')).toBe('Orbit')
    expect(activeCameraToolForName('Pan')).toBe('Pan')
    expect(activeCameraToolForName('Zoom')).toBe('Zoom')
    expect(activeCameraToolForName('Zoom Window')).toBe('ZoomWindow')
  })

  it('Zoom → Rectangle clears it: a named tool the switch falls through to its own case still gets null, not the stale outgoing camera tool', () => {
    expect(activeCameraToolForName('Zoom')).toBe('Zoom')
    expect(activeCameraToolForName('Rectangle')).toBe(null)
  })

  it('Zoom → Escape clears it: Escape requests \'Select\' (the switch\'s default case)', () => {
    expect(activeCameraToolForName('Zoom')).toBe('Zoom')
    expect(activeCameraToolForName('Select')).toBe(null)
  })

  it('Zoom → Orbit → Zoom shows it again: re-entering Zoom after an intervening camera tool is not a one-way clear', () => {
    expect(activeCameraToolForName('Zoom')).toBe('Zoom')
    expect(activeCameraToolForName('Orbit')).toBe('Orbit')
    expect(activeCameraToolForName('Zoom')).toBe('Zoom')
  })

  it('every other named tool (walkthrough tools included) is null', () => {
    for (const name of ['Select', 'Move', 'Push/Pull', 'Position Camera', 'Look Around', 'Walk', 'Section Plane']) {
      expect(activeCameraToolForName(name)).toBe(null)
    }
  })
})
