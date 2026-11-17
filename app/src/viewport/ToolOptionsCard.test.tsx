import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { ToolOptionsCard } from './ToolOptionsCard'

afterEach(() => vi.restoreAllMocks())

it('mounts without taking focus and pointer choices leave typed sizes in the viewport', () => {
  const viewport = document.createElement('button')
  document.body.append(viewport)
  viewport.focus()
  render(<ToolOptionsCard tool="Focus test" info="Help."><button>Choice</button></ToolOptionsCard>)
  expect(document.activeElement).toBe(viewport)
  expect(fireEvent.pointerDown(screen.getByRole('button', { name: 'Choice' }))).toBe(false)
  expect(document.activeElement).toBe(viewport)
  viewport.remove()
})

it('keeps only the title when collapsed, remembers per tool, and expands again', () => {
  const { unmount } = render(<ToolOptionsCard tool="Collapse test" info="Help."><span>Rows</span></ToolOptionsCard>)
  fireEvent.click(screen.getByRole('button', { name: 'Collapse Collapse test options' }))
  expect(screen.queryByText('Rows')).toBeNull()
  expect(screen.getByText('Collapse test')).toBeVisible()
  unmount()
  const second = render(<ToolOptionsCard tool="Collapse test" info="Help."><span>Rows</span></ToolOptionsCard>)
  expect(screen.queryByText('Rows')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: 'Expand Collapse test options' }))
  expect(screen.getByText('Rows')).toBeVisible()
  second.unmount()
  render(<ToolOptionsCard tool="Another tool" info="Help."><span>Other rows</span></ToolOptionsCard>)
  expect(screen.getByText('Other rows')).toBeVisible()
})

it('reserves dock space on the positioned viewport and restores it on unmount', () => {
  const viewport = document.createElement('div')
  viewport.style.setProperty('--tool-card-inset', '8px')
  vi.spyOn(HTMLElement.prototype, 'offsetParent', 'get').mockReturnValue(viewport)
  const { unmount } = render(<ToolOptionsCard tool="Inset test" info="Help.">Rows</ToolOptionsCard>)
  expect(viewport.style.getPropertyValue('--tool-card-inset')).toBe('272px')
  unmount()
  expect(viewport.style.getPropertyValue('--tool-card-inset')).toBe('8px')
})

it('opens text help and dismisses it with Escape or an outside press', () => {
  render(<ToolOptionsCard tool="Help test" info="One plain sentence.">Rows</ToolOptionsCard>)
  const button = screen.getByRole('button', { name: 'About Help test' })
  fireEvent.click(button)
  expect(screen.getByRole('note')).toHaveTextContent('One plain sentence.')
  fireEvent.keyDown(window, { key: 'Escape' })
  expect(screen.queryByRole('note')).toBeNull()
  fireEvent.click(button)
  fireEvent.pointerDown(document.body)
  expect(screen.queryByRole('note')).toBeNull()
})
