import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '@lingui/react'
import { i18n } from '@lingui/core'
import type { DrawPlaneState } from '../tools/drawPlane'
import { ViewDie, type PipId, type ViewDieApi } from './ViewDie'
import { quatForEye, type Quat } from './viewDieGeometry'

i18n.loadAndActivate({ locale: 'en', messages: {} })

function planeState(pip: PipId, hold: DrawPlaneState['hold'], normal: [number, number, number]): DrawPlaneState {
  return {
    plane: { origin: [0, 0, 0], normal, u: [1, 0, 0], v: [0, 1, 0], ground: false },
    pip,
    hold,
    hover: null,
  }
}

function makeApi(opts: { q?: Quat; plane?: DrawPlaneState | null } = {}) {
  let cam: ((q: [number, number, number, number]) => void) | null = null
  let planeCb: ((s: DrawPlaneState | null) => void) | null = null
  const state = { plane: opts.plane ?? null }
  const api: ViewDieApi = {
    setViewDirection: vi.fn(),
    orbitDrag: vi.fn(),
    endOrbitDrag: vi.fn(),
    subscribeCameraOrientation: (cb) => {
      cam = cb
      const q = opts.q ?? quatForEye([1, -1, 1])
      cb([q[0], q[1], q[2], q[3]])
      return () => { cam = null }
    },
    getDrawPlaneState: () => state.plane,
    subscribeDrawPlaneState: (cb) => {
      planeCb = cb
      return () => { planeCb = null }
    },
    pipClick: vi.fn(),
    pipWillRelease: (pip) => state.plane !== null && state.plane.pip === pip && (state.plane.hold === 'locked' || state.plane.hold === 'pinned'),
    pipEnabled: (pip) => state.plane !== null && (pip !== 'custom' || state.plane.pip === 'custom'),
  }
  return {
    api,
    state,
    pushCamera: (q: Quat) => act(() => cam?.([q[0], q[1], q[2], q[3]])),
    pushPlane: (p: DrawPlaneState | null) => act(() => { state.plane = p; planeCb?.(p) }),
  }
}

function mount(h: ReturnType<typeof makeApi>) {
  const apiRef = { current: h.api as ViewDieApi | null }
  return render(
    <I18nProvider i18n={i18n}>
      <ViewDie apiRef={apiRef} />
    </I18nProvider>,
  )
}

// jsdom has no layout: give the die's SVG a 76 px box at the origin.
beforeEach(() => {
  Element.prototype.getBoundingClientRect = function () {
    return { left: 0, top: 0, right: 76, bottom: 76, width: 76, height: 76, x: 0, y: 0, toJSON: () => ({}) } as DOMRect
  }
  Element.prototype.setPointerCapture = () => {}
  Element.prototype.releasePointerCapture = () => {}
})

const svg = () => screen.getByTestId('view-die-svg')

describe('ViewDie', () => {
  it('renders the die with no text on the widget but the face labels and four idle pips', () => {
    mount(makeApi())
    for (const pip of ['red', 'green', 'blue', 'custom']) {
      expect(screen.getByTestId(`view-die-pip-${pip}`)).toHaveAttribute('data-state', 'idle')
    }
    expect(svg().textContent).toContain('Top')
    expect(svg().textContent).toContain('Front')
    expect(svg().textContent).toContain('Right')
    expect(svg().textContent).not.toContain('Back')
  })

  it('draws the outline dashed off a standard view and solid on one', () => {
    const h = makeApi({ q: quatForEye([0.3, -1, 0.6]) })
    mount(h)
    expect(screen.getByTestId('view-die-silhouette')).toHaveAttribute('data-solid', 'false')
    h.pushCamera(quatForEye([0, -1, 0]))
    expect(screen.getByTestId('view-die-silhouette')).toHaveAttribute('data-solid', 'true')
    expect(screen.getByTestId('view-die')).toHaveAttribute('data-view', 'front')
    expect(screen.getByTestId('view-die-step-right')).toBeInTheDocument()
  })

  it('a click on a face asks the viewport to turn there, with a tween', () => {
    const h = makeApi({ q: quatForEye([0, -1, 0]) })
    mount(h)
    // Face-on Front: the centre of the die is the Front face.
    fireEvent.pointerDown(svg(), { clientX: 38, clientY: 38, pointerId: 1, button: 0 })
    fireEvent.pointerUp(svg(), { clientX: 38, clientY: 38, pointerId: 1, button: 0 })
    // Already on Front: flips to the opposite face.
    expect(h.api.setViewDirection).toHaveBeenCalledWith([0, 1, 0], { tween: true })
  })

  it('a 2 px press is a click and a 4 px press is a drag that orbits and never turns', () => {
    const h = makeApi({ q: quatForEye([0, -1, 0]) })
    mount(h)
    fireEvent.pointerDown(svg(), { clientX: 38, clientY: 38, pointerId: 1, button: 0 })
    fireEvent.pointerMove(svg(), { clientX: 40, clientY: 38, pointerId: 1 })
    fireEvent.pointerUp(svg(), { clientX: 40, clientY: 38, pointerId: 1, button: 0 })
    expect(h.api.setViewDirection).toHaveBeenCalledTimes(1)
    expect(h.api.orbitDrag).not.toHaveBeenCalled()

    fireEvent.pointerDown(svg(), { clientX: 38, clientY: 38, pointerId: 2, button: 0 })
    fireEvent.pointerMove(svg(), { clientX: 42, clientY: 38, pointerId: 2 })
    fireEvent.pointerMove(svg(), { clientX: 50, clientY: 41, pointerId: 2 })
    fireEvent.pointerUp(svg(), { clientX: 50, clientY: 41, pointerId: 2, button: 0 })
    expect(h.api.setViewDirection).toHaveBeenCalledTimes(1)
    expect(h.api.orbitDrag).toHaveBeenNthCalledWith(1, 4, 0)
    expect(h.api.orbitDrag).toHaveBeenNthCalledWith(2, 8, 3)
    expect(h.api.endOrbitDrag).toHaveBeenCalledTimes(1)
  })

  it('ends a drag when the pointer is cancelled', () => {
    const h = makeApi()
    mount(h)
    fireEvent.pointerDown(svg(), { clientX: 38, clientY: 38, pointerId: 3, button: 0 })
    fireEvent.pointerMove(svg(), { clientX: 60, clientY: 38, pointerId: 3 })
    fireEvent.pointerCancel(svg(), { pointerId: 3 })
    expect(h.api.endOrbitDrag).toHaveBeenCalled()
  })

  it('cancels the default of pointer and mouse presses so focus never moves', () => {
    mount(makeApi())
    const down = new MouseEvent('mousedown', { bubbles: true, cancelable: true, button: 0 })
    svg().dispatchEvent(down)
    expect(down.defaultPrevented).toBe(true)
    const pip = screen.getByTestId('view-die-pip-red')
    const pdown = new MouseEvent('mousedown', { bubbles: true, cancelable: true })
    pip.dispatchEvent(pdown)
    expect(pdown.defaultPrevented).toBe(true)
    expect(pip).not.toHaveAttribute('tabindex')
  })

  it('names the hovered target in a pill', () => {
    mount(makeApi({ q: quatForEye([0, -1, 0]) }))
    fireEvent.pointerMove(svg(), { clientX: 38, clientY: 38, pointerId: 1 })
    expect(screen.getByTestId('view-die-tooltip')).toHaveTextContent('Front')
    expect(screen.getByTestId('view-die-hover')).toBeInTheDocument()
  })

  describe('input ownership', () => {
    const press = (id: number, x = 38, y = 38, button = 0) =>
      fireEvent.pointerDown(svg(), { clientX: x, clientY: y, pointerId: id, button })

    it('lets a mousedown reach the document (an open menu closes) while cancelling its default', () => {
      mount(makeApi())
      const seen = vi.fn()
      document.addEventListener('mousedown', seen)
      try {
        for (const el of [svg(), screen.getByTestId('view-die-pip-red')]) {
          const down = new MouseEvent('mousedown', { bubbles: true, cancelable: true, button: 0 })
          el.dispatchEvent(down)
          expect(down.defaultPrevented).toBe(true)
        }
        expect(seen).toHaveBeenCalledTimes(2)
      } finally {
        document.removeEventListener('mousedown', seen)
      }
    })

    it('does not cancel the pointerdown itself (that would swallow the mousedown)', () => {
      mount(makeApi())
      const down = new Event('pointerdown', { bubbles: true, cancelable: true })
      Object.assign(down, { button: 0, pointerId: 1, clientX: 38, clientY: 38 })
      svg().dispatchEvent(down)
      expect(down.defaultPrevented).toBe(false)
    })

    it('ends a drag whose release was lost: a mouse move with no button held', () => {
      const h = makeApi()
      mount(h)
      press(1)
      fireEvent.pointerMove(svg(), { clientX: 60, clientY: 38, pointerId: 1, pointerType: 'mouse', buttons: 1 })
      expect(h.api.orbitDrag).toHaveBeenCalled()
      fireEvent.pointerMove(svg(), { clientX: 70, clientY: 38, pointerId: 1, pointerType: 'mouse', buttons: 0 })
      expect(h.api.endOrbitDrag).toHaveBeenCalledTimes(1)
      // The press is over: more moves do not orbit.
      fireEvent.pointerMove(svg(), { clientX: 80, clientY: 38, pointerId: 1, pointerType: 'mouse', buttons: 0 })
      expect(h.api.orbitDrag).toHaveBeenCalledTimes(1)
    })

    it('ends a drag when the window loses focus', () => {
      const h = makeApi()
      mount(h)
      press(1)
      fireEvent.pointerMove(svg(), { clientX: 60, clientY: 38, pointerId: 1 })
      act(() => { window.dispatchEvent(new Event('blur')) })
      expect(h.api.endOrbitDrag).toHaveBeenCalledTimes(1)
    })

    it('ignores a second pointer while a press is live', () => {
      const h = makeApi({ q: quatForEye([0, -1, 0]) })
      mount(h)
      press(1)
      press(2)
      fireEvent.pointerUp(svg(), { clientX: 38, clientY: 38, pointerId: 2, button: 0 })
      expect(h.api.setViewDirection).not.toHaveBeenCalled()
      fireEvent.pointerUp(svg(), { clientX: 38, clientY: 38, pointerId: 1, button: 0 })
      expect(h.api.setViewDirection).toHaveBeenCalledTimes(1)
    })

    it('settles a stale press of the same pointer before starting a new one', () => {
      const h = makeApi()
      mount(h)
      press(1)
      fireEvent.pointerMove(svg(), { clientX: 60, clientY: 38, pointerId: 1 })
      press(1)
      expect(h.api.endOrbitDrag).toHaveBeenCalledTimes(1)
    })

    it('acts only on the pressing button: another button\'s release is ignored', () => {
      const h = makeApi({ q: quatForEye([0, -1, 0]) })
      mount(h)
      press(1)
      fireEvent.pointerUp(svg(), { clientX: 38, clientY: 38, pointerId: 1, button: 1 })
      expect(h.api.setViewDirection).not.toHaveBeenCalled()
      fireEvent.pointerUp(svg(), { clientX: 38, clientY: 38, pointerId: 1, button: 0 })
      expect(h.api.setViewDirection).toHaveBeenCalledTimes(1)
    })

    it('the transparent box lets presses through: only shapes and pips take them', () => {
      mount(makeApi({ q: quatForEye([0, -1, 0]) }))
      expect(svg().style.pointerEvents).toBe('none')
      expect(screen.getByTestId('view-die').style.pointerEvents).toBe('none')
      expect(screen.getByTestId('view-die-pip-red').style.pointerEvents).toBe('auto')
      const base = document.querySelector('[data-face="front"] polygon') as SVGElement
      expect(base.style.pointerEvents).toBe('auto')
      expect(screen.getByTestId('view-die-step-right').style.pointerEvents).toBe('all')
    })
  })

  describe('pips', () => {
    it('repaints when only pipEnabled or pipWillRelease changes', () => {
      const h = makeApi({ plane: planeState('green', 'inferred', [0, -1, 0]) })
      let enabled = false
      h.api.pipEnabled = () => enabled
      mount(h)
      expect(screen.getByTestId('view-die-pip-custom')).toHaveAttribute('data-enabled', 'false')
      enabled = true
      // Same pip, hold and normal: only the enablement differs.
      h.pushPlane(planeState('green', 'inferred', [0, -1, 0]))
      expect(screen.getByTestId('view-die-pip-custom')).toHaveAttribute('data-enabled', 'true')
    })

    it('with no draw tool, pips are not clickable and show not-allowed', () => {
      const h = makeApi()
      mount(h)
      const blue = screen.getByTestId('view-die-pip-blue')
      expect(blue).toHaveStyle({ cursor: 'not-allowed' })
      fireEvent.click(blue)
      expect(h.api.pipClick).not.toHaveBeenCalled()
    })

    it('an inferred plane fills one pip and the matching face pair, no padlock', () => {
      const h = makeApi({ plane: planeState('green', 'inferred', [0, -1, 0]) })
      mount(h)
      expect(screen.getByTestId('view-die-pip-green')).toHaveAttribute('data-state', 'inferred')
      expect(screen.getByTestId('view-die-pip-red')).toHaveAttribute('data-state', 'idle')
      expect(screen.queryByTestId('view-die-lock-green')).toBeNull()
      expect(screen.queryByTestId('view-die-ring-green')).toBeNull()
      expect(screen.getByTestId('view-die-fill-front')).toBeInTheDocument()
      expect(screen.queryByTestId('view-die-fill-top')).toBeNull()
    })

    it('a locked or pinned plane shows the padlock and ring; anchored shows only the ring', () => {
      const h = makeApi({ plane: planeState('blue', 'locked', [0, 0, 1]) })
      mount(h)
      expect(screen.getByTestId('view-die-pip-blue')).toHaveAttribute('data-state', 'locked')
      expect(screen.getByTestId('view-die-lock-blue')).toBeInTheDocument()
      expect(screen.getByTestId('view-die-ring-blue')).toBeInTheDocument()
      h.pushPlane(planeState('blue', 'pinned', [0, 0, 1]))
      expect(screen.getByTestId('view-die-lock-blue')).toBeInTheDocument()
      h.pushPlane(planeState('green', 'anchored', [0, -1, 0]))
      expect(screen.getByTestId('view-die-ring-green')).toBeInTheDocument()
      expect(screen.queryByTestId('view-die-lock-green')).toBeNull()
    })

    it('a held plane fills the face stronger than an inferred one', () => {
      const h = makeApi({ plane: planeState('red', 'inferred', [1, 0, 0]) })
      mount(h)
      const alpha = () => Number(screen.getByTestId('view-die-fill-right').style.fillOpacity)
      expect(alpha()).toBeCloseTo(0.35)
      h.pushPlane(planeState('red', 'locked', [1, 0, 0]))
      expect(alpha()).toBeCloseTo(0.55)
    })

    it('a custom plane lights the violet pip and fills no face', () => {
      const n: [number, number, number] = [Math.sin(Math.PI / 6), -Math.cos(Math.PI / 6), 0]
      mount(makeApi({ plane: planeState('custom', 'inferred', n) }))
      expect(screen.getByTestId('view-die-pip-custom')).toHaveAttribute('data-state', 'inferred')
      expect(document.querySelector('[data-testid^="view-die-fill-"]')).toBeNull()
    })

    it('a plane parallel to no world axis fills no face even on a colored pip', () => {
      const n: [number, number, number] = [Math.sin(Math.PI / 6), -Math.cos(Math.PI / 6), 0]
      mount(makeApi({ plane: planeState('green', 'inferred', n) }))
      expect(document.querySelector('[data-testid^="view-die-fill-"]')).toBeNull()
    })

    it('clicking an enabled pip goes to the viewport', () => {
      const h = makeApi({ plane: planeState('green', 'inferred', [0, -1, 0]) })
      mount(h)
      fireEvent.click(screen.getByTestId('view-die-pip-blue'))
      expect(h.api.pipClick).toHaveBeenCalledWith('blue')
    })

    it('the custom pip is enabled only when the tracked plane is custom', () => {
      const h = makeApi({ plane: planeState('green', 'inferred', [0, -1, 0]) })
      mount(h)
      expect(screen.getByTestId('view-die-pip-custom')).toHaveAttribute('data-enabled', 'false')
      fireEvent.click(screen.getByTestId('view-die-pip-custom'))
      expect(h.api.pipClick).not.toHaveBeenCalled()
      h.pushPlane(planeState('custom', 'inferred', [0.5, -0.866, 0]))
      expect(screen.getByTestId('view-die-pip-custom')).toHaveAttribute('data-enabled', 'true')
    })

    it('tooltips say Lock, and Release once the pip is held', () => {
      const h = makeApi({ plane: planeState('blue', 'inferred', [0, 0, 1]) })
      mount(h)
      const tip = () => screen.getByTestId('view-die-tooltip').textContent
      fireEvent.pointerEnter(screen.getByTestId('view-die-pip-red'))
      expect(tip()).toBe('Lock red plane  →')
      fireEvent.pointerLeave(screen.getByTestId('view-die-pip-red'))
      fireEvent.pointerEnter(screen.getByTestId('view-die-pip-custom'))
      expect(tip()).toBe('Pin the plane under the cursor  Shift')
      fireEvent.pointerLeave(screen.getByTestId('view-die-pip-custom'))
      h.pushPlane(planeState('blue', 'locked', [0, 0, 1]))
      fireEvent.pointerEnter(screen.getByTestId('view-die-pip-blue'))
      expect(tip()).toBe('Release blue plane  ↑')
    })

    it('the plane survives a null-hover update (state read from the tool, not a cue)', () => {
      const h = makeApi({ plane: planeState('blue', 'locked', [0, 0, 1]) })
      mount(h)
      fireEvent.pointerEnter(screen.getByTestId('view-die'))
      expect(screen.getByTestId('view-die-pip-blue')).toHaveAttribute('data-state', 'locked')
    })
  })
})
