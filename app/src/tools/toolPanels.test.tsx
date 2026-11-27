import { render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'

afterEach(() => { vi.doUnmock('../preview/build'); vi.resetModules() })

it('mounts the Fillet card only for its active tool in Preview', async () => {
  vi.resetModules()
  vi.doMock('../preview/build', () => ({ previewBuild: true }))
  const { panelsForTool } = await import('./toolPanels')
  expect(panelsForTool('Line')).toHaveLength(0)
  const panels = panelsForTool('Fillet')
  expect(panels).toHaveLength(1)
  render(panels[0].render({ profileSections: [] }))
  expect(screen.getByRole('region', { name: 'Fillet options' })).toBeVisible()
})

it('exposes no Fillet card in production', async () => {
  vi.resetModules()
  vi.doMock('../preview/build', () => ({ previewBuild: false }))
  const { panelsForTool } = await import('./toolPanels')
  expect(panelsForTool('Fillet')).toHaveLength(0)
  expect(panelsForTool('Line')).toHaveLength(0)
})
