import { render } from '@testing-library/react'
import { expect, it } from 'vitest'
import { ProfileMenuIcon } from './ProfileMenuIcon'

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
