import { LinkButton } from '@/components/ui/Button'

import type { SearchResponse } from '@/lib/api'
import type { Messages } from '@/lib/i18n/messages'

/**
 * Real links, so paging works with JavaScript disabled and every page is shareable.
 *
 * Words rather than `‹` and `›`. In an RTL layout a literal left-chevron for "previous" points
 * the wrong way, and flipping it with a CSS transform is worse than simply writing the word.
 */
export function Pagination({
  lang,
  t,
  pagination,
  params,
  q,
}: {
  lang: string
  t: Messages
  pagination: SearchResponse['pagination']
  params: Record<string, string | string[] | undefined>
  q: string
}) {
  const total = Math.max(pagination.total_pages, 1)
  if (total <= 1) return null

  const href = (page: number) => {
    const next = new URLSearchParams({ q })
    for (const key of ['lang', 'source', 'sentiment']) {
      const value = params[key]
      const single = Array.isArray(value) ? value[0] : value
      if (single) next.set(key, single)
    }
    if (page > 1) next.set('page', String(page))
    return `/${lang}/search?${next}`
  }

  const start = Math.max(1, pagination.page - 2)
  const pages = Array.from({ length: Math.min(5, total - start + 1) }, (_, i) => start + i)

  return (
    <nav className="mt-10 flex flex-wrap items-center gap-2" aria-label={t.page}>
      {pagination.page > 1 && <LinkButton href={href(pagination.page - 1)}>{t.previous}</LinkButton>}
      {pages.map((n) =>
        // The current page is a `<span>`, not a link. A link to the page you are already on is a
        // navigation that does nothing, and a screen reader announces it as a destination.
        n === pagination.page ? (
          <span key={n} className="chip chip-active numeric" aria-current="page">
            {n}
          </span>
        ) : (
          <LinkButton key={n} href={href(n)} className="numeric">
            {n}
          </LinkButton>
        ),
      )}
      {pagination.page < total && <LinkButton href={href(pagination.page + 1)}>{t.next}</LinkButton>}
    </nav>
  )
}
