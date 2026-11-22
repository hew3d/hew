import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ProfileMenuIcon, profileOutline } from './ProfileMenuIcon'

describe('profile menu artwork', () => {
  it('keeps a saved profile’s supplied lines and arcs as crisp decorative SVG', () => {
    const shape = { b: 1, segs: [
      { kind: 'line', to: [0.8, 0] },
      { kind: 'arc', to: [0, 1], center: [0, 0], ccw: true },
    ] }
    render(<button>My curve<ProfileMenuIcon shape={shape} /></button>)
    const svg = screen.getByRole('button', { name: 'My curve' }).querySelector('svg')!
    expect(svg).toHaveAttribute('aria-hidden', 'true')
    expect(svg).toHaveAttribute('viewBox', '0 0 24 24')
    const path = svg.querySelector('path')!
    expect(path.getAttribute('d')).toContain('L0.8 0A')
    expect(path).toHaveAttribute('vector-effect', 'non-scaling-stroke')
  })

  it('does not invent artwork for unsupported data and retains the supplied thumbnail', () => {
    for (const shape of [null, {}, { b: -1, segs: [] }, { b: 1, segs: [{ kind: 'line', to: [NaN, 0] }] }]) {
      expect(profileOutline(shape)).toBeNull()
    }
    const { container } = render(<ProfileMenuIcon shape={{}} thumbnail="data:image/png;base64,AA==" />)
    expect(container.querySelector('svg')).toBeNull()
    expect(container.querySelector('img')).toHaveAttribute('width', '24')
    expect(container.querySelector('img')).toHaveAttribute('alt', '')
  })
})

it('uses distinct, matching decorative cross-sections for the four primary shapes', () => {
  const { container } = render(<>{(['round', 'chamfer', 'cove', 'bullnose'] as const).map(kind =>
    <button key={kind}>{kind}<ProfileMenuIcon kind={kind} /></button>,
  )}</>)
  const paths = Array.from(container.querySelectorAll('svg path'))
  expect(new Set(paths.map(path => path.getAttribute('d'))).size).toBe(4)
  for (const path of paths) {
    expect(path).toHaveAttribute('stroke', 'currentColor')
    expect(path).toHaveAttribute('stroke-width', '1.25')
    expect(path).toHaveAttribute('vector-effect', 'non-scaling-stroke')
    expect(path.parentElement).toHaveAttribute('aria-hidden', 'true')
    expect(path.parentElement).toHaveAttribute('width', '24')
  }
})
