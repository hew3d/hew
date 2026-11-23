/**
 * CatalogView — "Browse the catalog" (docs/design/shared-library-sources.md
 * 7.5): the sources the Hew catalog lists, one row each with name, author,
 * description, license, item count and an Add button. A row opens a preview
 * of the source from the catalog's own snapshot. An empty catalog says so and
 * points to "Add a source…" and the publishing guide; a catalog that cannot be
 * read shows the one message and nothing else.
 */

import { useEffect, useState } from 'react'
import { plural } from '@lingui/core/macro'
import { Trans, useLingui } from '@lingui/react/macro'
import { isTauri } from '../../io/fileHost'
import { catalogHost } from '../../library/catalogUrl'
import { failureMessage, sourcesOffText, type SourceMessage } from '../../library/sourceMessages'
import type { SourcesBinding } from '../../library/sourcesDeps'
import type { CatalogSourceEntry } from '../../library/sources'
import { ExternalLink } from './ExternalLink'

export const PUBLISHING_GUIDE_URL = 'https://hew3d.com/learn/publishing-a-source'

export interface CatalogViewProps {
  binding: SourcesBinding
  /** Canonical references of the sources already added. */
  added: ReadonlySet<string>
  /** The catalog was read (its removed list may have changed what the sidebar shows). */
  onLoaded(): void
  onPreview(entry: CatalogSourceEntry): void
  onAdd(entry: CatalogSourceEntry): void
  onAddSource(): void
  /** Why the last Add failed, when it did. */
  addError?: string | null
}

type Phase = { phase: 'loading' } | { phase: 'ok'; sources: CatalogSourceEntry[] } | { phase: 'failed'; message: SourceMessage }

export function CatalogView({ binding, added, onLoaded, onPreview, onAdd, onAddSource, addError = null }: CatalogViewProps) {
  const { t } = useLingui()
  const [state, setState] = useState<Phase>({ phase: 'loading' })

  useEffect(() => {
    let cancelled = false
    setState({ phase: 'loading' })
    void binding.sources.catalog().then((r) => {
      if (cancelled) return
      if ('off' in r) setState({ phase: 'failed', message: { text: sourcesOffText() } })
      else if ('ok' in r && r.ok) setState({ phase: 'ok', sources: r.catalog.sources })
      else if ('ok' in r) setState({ phase: 'failed', message: failureMessage(r, { web: !isTauri, host: catalogHost() }) })
      onLoaded()
    })
    return () => {
      cancelled = true
    }
    // The catalog is read once each time the view opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [binding])

  return (
    <div className="hwlib__catalog" data-testid="library-catalog" aria-label={t`Catalog`} role="region">
      {addError !== null && (
        <div className="hwlib__action-error" role="alert" data-testid="library-catalog-add-error">
          {addError}
        </div>
      )}
      {state.phase === 'loading' && <div className="hwlib__src-empty"><Trans>Loading…</Trans></div>}
      {state.phase === 'failed' && (
        <div className="hwlib__src-empty" role="alert" data-testid="library-catalog-error">
          {state.message.text}
        </div>
      )}
      {state.phase === 'ok' && state.sources.length === 0 && (
        <div className="hwlib__src-empty" data-testid="library-catalog-empty">
          <p style={{ margin: '0 0 10px' }}><Trans>No sources are listed yet.</Trans></p>
          <p style={{ margin: '0 0 12px' }}>
            <button type="button" className="hwlib__btn-secondary" onClick={onAddSource}>
              <Trans>Add a source…</Trans>
            </button>
          </p>
          <p style={{ margin: 0 }}>
            <ExternalLink href={PUBLISHING_GUIDE_URL}><Trans>How to publish a source</Trans></ExternalLink>
          </p>
        </div>
      )}
      {state.phase === 'ok' &&
        state.sources.map((s) => {
          const isAdded = added.has(s.source)
          const license = s.license ?? t`No license stated`
          const count = plural(s.items, { one: '# item', other: '# items' })
          const name = s.name
          return (
            <div
              key={s.source}
              className="hwlib__catalog-row"
              role="button"
              tabIndex={0}
              data-testid="library-catalog-row"
              aria-label={t`Preview ${name}`}
              onClick={() => onPreview(s)}
              onKeyDown={(e) => {
                if (e.target !== e.currentTarget) return
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault()
                  onPreview(s)
                }
              }}
            >
              <div className="hwlib__catalog-main">
                <div className="hwlib__catalog-name">{s.name}</div>
                {s.author != null && s.author !== '' && <div className="hwlib__catalog-sub">{s.author}</div>}
                {s.description != null && s.description !== '' && <div className="hwlib__catalog-sub">{s.description}</div>}
                <div className="hwlib__catalog-sub">{license} · {count}</div>
              </div>
              <button
                type="button"
                className={isAdded ? 'hwlib__btn-secondary hwlib__catalog-add' : 'hwlib__btn-primary hwlib__catalog-add'}
                disabled={isAdded}
                aria-label={isAdded ? t`${name} is added` : t`Add ${name}`}
                onClick={(e) => {
                  e.stopPropagation()
                  onAdd(s)
                }}
              >
                {isAdded ? t`Added` : t`Add`}
              </button>
            </div>
          )
        })}
    </div>
  )
}
