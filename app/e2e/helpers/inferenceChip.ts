import type { Page } from '@playwright/test'

/**
 * The snap text of the inference chip under the cursor (its first row: the
 * snap's name plus any "projected" / "on x axis" words), without the
 * drawing-plane word and padlock the chip appends (view-die design §4) and
 * without the hover-detail row. Empty when no chip shows.
 */
export function inferenceChip(page: Page): Promise<string> {
  return page.evaluate(() => {
    const row = document.querySelector('[data-testid="inference-tip"] > div')
    if (row === null) return ''
    const copy = row.cloneNode(true) as HTMLElement
    copy.querySelector('[data-testid="inference-tip-plane"]')?.remove()
    return copy.textContent ?? ''
  })
}
