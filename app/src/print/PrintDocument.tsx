/**
 * The printed page as DOM (docs/design/printing.md §8): one `<section>` per
 * page sized in physical millimetres, holding the page bitmap (or a vector
 * drawing) exactly where the layout put it and an SVG overlay of the page
 * furniture. The SAME component draws the dialog preview (CSS-scaled) and
 * the print root (1:1, print media) — that is the WYSIWYG guarantee.
 */
import React, { useLayoutEffect } from 'react'
import { createPortal } from 'react-dom'
import type { FurnitureItem } from './furniture'
import type { PrintLayout, TileSpec } from './layout'

export interface PrintPageModel {
  tile: TileSpec
  furniture: FurnitureItem[]
  /** Object URL / data URL of the page bitmap; null while rendering. */
  imageUrl: string | null
  /** Vector drawing (Line art): inline SVG markup for the image rect, used
   * instead of `imageUrl` when present. */
  vectorSvg?: string | null
  /** A furniture-only page (cut list): no drawing box at all. */
  blank?: boolean
}

export const PRINT_ROOT_ID = 'hew-print-root'

const PAGE_FONT = '-apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif'

function gray(g: number): string {
  const v = Math.round(Math.max(0, Math.min(1, g)) * 255)
  return `rgb(${v},${v},${v})`
}

/** SVG overlay of the furniture, in mm user units over the whole page. */
export function FurnitureSvg({ layout, items }: { layout: PrintLayout; items: FurnitureItem[] }) {
  const { w, h } = layout.page.paper
  return (
    <svg
      className="hew-print-furniture"
      viewBox={`0 0 ${w} ${h}`}
      width={`${w}mm`}
      height={`${h}mm`}
      style={{ position: 'absolute', left: 0, top: 0, width: `${w}mm`, height: `${h}mm`, overflow: 'visible', pointerEvents: 'none' }}
      aria-hidden="true"
    >
      {items.map((it, i) => {
        if (it.kind === 'line') {
          return (
            <line
              key={i}
              data-role={it.role}
              x1={it.x1}
              y1={it.y1}
              x2={it.x2}
              y2={it.y2}
              stroke={gray(it.gray)}
              strokeWidth={it.widthMm}
              strokeDasharray={it.dash !== undefined ? it.dash.join(' ') : undefined}
              strokeLinecap="butt"
            />
          )
        }
        if (it.kind === 'rect') {
          return (
            <rect
              key={i}
              data-role={it.role}
              x={it.x}
              y={it.y}
              width={it.w}
              height={it.h}
              fill={it.fillGray !== undefined ? gray(it.fillGray) : 'none'}
              stroke={it.strokeMm !== undefined ? gray(it.gray ?? 0) : 'none'}
              strokeWidth={it.strokeMm ?? 0}
            />
          )
        }
        return (
          <text
            key={i}
            data-role={it.role}
            x={it.x}
            y={it.y}
            fontSize={it.sizeMm}
            fontFamily={PAGE_FONT}
            fontWeight={it.bold === true ? 600 : 400}
            fill={gray(it.gray)}
            textAnchor={it.align === 'center' ? 'middle' : it.align === 'right' ? 'end' : 'start'}
            transform={it.rotate !== undefined && it.rotate !== 0 ? `rotate(${it.rotate} ${it.x} ${it.y})` : undefined}
          >
            {it.text}
          </text>
        )
      })}
    </svg>
  )
}

/** One page. `scale` (CSS) shrinks it for the preview; the print root uses 1. */
export function PrintPage({ layout, page, scale = 1, pageNumber }: { layout: PrintLayout; page: PrintPageModel; scale?: number; /** Global 1-based page number for `data-page` (defaults to the tile's own). */ pageNumber?: number }) {
  const { w, h } = layout.page.paper
  const r = page.tile.imageRectMm
  const outer: React.CSSProperties =
    scale === 1
      ? { width: `${w}mm`, height: `${h}mm` }
      : { width: `${w}mm`, height: `${h}mm`, transform: `scale(${scale})`, transformOrigin: 'top left' }
  return (
    <section
      className="hew-print-page"
      data-tile={page.tile.id}
      data-page={pageNumber ?? page.tile.page + 1}
      style={{ position: 'relative', overflow: 'hidden', background: '#fff', boxSizing: 'border-box', ...outer }}
    >
      {page.blank === true ? null : page.vectorSvg !== undefined && page.vectorSvg !== null ? (
        <div
          className="hew-print-drawing"
          data-kind="vector"
          style={{ position: 'absolute', left: `${r.x}mm`, top: `${r.y}mm`, width: `${r.w}mm`, height: `${r.h}mm`, overflow: 'hidden' }}
          dangerouslySetInnerHTML={{ __html: page.vectorSvg }}
        />
      ) : page.imageUrl !== null ? (
        <img
          className="hew-print-drawing"
          data-kind="raster"
          alt=""
          src={page.imageUrl}
          style={{ position: 'absolute', left: `${r.x}mm`, top: `${r.y}mm`, width: `${r.w}mm`, height: `${r.h}mm`, display: 'block' }}
        />
      ) : (
        <div
          className="hew-print-drawing"
          data-kind="pending"
          style={{ position: 'absolute', left: `${r.x}mm`, top: `${r.y}mm`, width: `${r.w}mm`, height: `${r.h}mm`, background: '#f2f2f2' }}
        />
      )}
      <FurnitureSvg layout={layout} items={page.furniture} />
    </section>
  )
}

/** The `@page` + print-media rules for a layout: the sheet size, zero
 * margins (Hew lays out its own), and "hide the app, show the pages". */
export function printCss(layout: PrintLayout): string {
  const { w, h } = layout.page.paper
  return [
    `@page { size: ${w}mm ${h}mm; margin: 0; }`,
    `@media screen { #${PRINT_ROOT_ID} { display: none; } }`,
    `@media print {`,
    `  body > *:not(#${PRINT_ROOT_ID}) { display: none !important; }`,
    `  html, body { margin: 0 !important; padding: 0 !important; background: #fff !important; height: auto !important; overflow: visible !important; }`,
    `  #${PRINT_ROOT_ID} { display: block; }`,
    `  #${PRINT_ROOT_ID} .hew-print-page { page-break-after: always; break-after: page; print-color-adjust: exact; -webkit-print-color-adjust: exact; }`,
    `  #${PRINT_ROOT_ID} .hew-print-page:last-child { page-break-after: auto; break-after: auto; }`,
    `}`,
  ].join('\n')
}

/**
 * The print root: portals every page into `document.body` (outside the React
 * app root, so the print CSS can hide the app wholesale). Mounted only while
 * a print is in flight; unmounting revokes nothing — the caller owns the
 * image URLs.
 */
export function PrintRoot({ layout, pages, title }: { layout: PrintLayout; pages: PrintPageModel[]; title: string }) {
  useLayoutEffect(() => {
    const prev = document.title
    document.title = title
    return () => {
      document.title = prev
    }
  }, [title])
  if (typeof document === 'undefined') return null
  return createPortal(
    <div id={PRINT_ROOT_ID} data-title={title}>
      <style>{printCss(layout)}</style>
      {pages.map((p, i) => (
        <PrintPage key={`${i}#${p.tile.id}`} layout={layout} page={p} pageNumber={i + 1} />
      ))}
    </div>,
    document.body,
  )
}

/** Resolve when every page image in the print root has decoded (Chrome's
 * preview otherwise races the first paint). */
export async function waitForPrintImages(root: HTMLElement | null = document.getElementById(PRINT_ROOT_ID)): Promise<void> {
  if (root === null) return
  const imgs = Array.from(root.querySelectorAll('img'))
  await Promise.all(
    imgs.map((img) =>
      typeof img.decode === 'function'
        ? img.decode().catch(() => undefined)
        : new Promise<void>((resolve) => {
            if (img.complete) resolve()
            else {
              img.onload = () => resolve()
              img.onerror = () => resolve()
            }
          }),
    ),
  )
}
