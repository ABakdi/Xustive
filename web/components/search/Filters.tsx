import { LinkButton } from '@/components/ui/Button'

import type { Locale } from '@/lib/i18n/config'
import { formatNumber } from '@/lib/i18n/format'
import type { Messages } from '@/lib/i18n/messages'

/**
 * Facet chips.
 *
 * Links, not script-driven controls — so narrowing works with JavaScript disabled, and every
 * filtered view is a URL you can share or bookmark. A filter that needs script disappears on
 * exactly the connection where narrowing results matters most.
 *
 * Three behaviours that each came from a bug in the previous implementation:
 *
 *  - Clicking an active chip **clears** it. A filter you cannot undo by clicking the thing you
 *    clicked is one people leave the page to escape.
 *  - Each link **preserves the other active filters**. Narrowing by language then by tone was
 *    silently dropping the language.
 *  - A facet with one value is hidden **unless it is the active one**, in which case that single
 *    value *is* the filter and hiding it strands the user with no way back.
 */

const GROUPS = [
  { facet: 'language', param: 'lang', label: 'language', prefix: 'lang_' },
  { facet: 'source_type', param: 'source', label: 'source', prefix: '' },
  { facet: 'sentiment.label', param: 'sentiment', label: 'tone', prefix: '' },
] as const

type Params = Record<string, string | string[] | undefined>

function one(params: Params, key: string): string | undefined {
  const value = params[key]
  return Array.isArray(value) ? value[0] : value
}

export function Filters({
  lang,
  t,
  facets,
  active,
  q,
}: {
  lang: string
  t: Messages
  facets: Record<string, Record<string, number>> | null
  active: Params
  q: string
}) {
  const nf = { format: (n: number) => formatNumber(lang as Locale, n) }
  const current = new Map<string, string>()
  for (const { param } of GROUPS) {
    const value = one(active, param)
    if (value) current.set(param, value)
  }

  const href = (param: string, value: string | null) => {
    const next = new URLSearchParams({ q })
    for (const [key, held] of current) {
      if (key !== param) next.set(key, held)
    }
    if (value) next.set(param, value)
    return `/${lang}/search?${next}`
  }

  const groups = GROUPS.map((group) => {
    const counts = facets?.[group.facet]
    if (!counts) return null
    const values = Object.entries(counts)
      .filter(([, n]) => n > 0)
      .sort((a, b) => b[1] - a[1])

    const selected = current.get(group.param)
    if (values.length < 2 && !selected) return null

    return (
      <div key={group.param} role="group" aria-label={t[group.label as keyof Messages]} className="flex flex-wrap items-center gap-2">
        <span className="text-xs" style={{ color: 'var(--fg-muted)' }}>
          {t[group.label as keyof Messages]}
        </span>
        {values.map(([value, count]) => {
          const on = selected === value
          const label = t[`${group.prefix}${value}` as keyof Messages] ?? value
          return (
            <LinkButton
              key={value}
              href={href(group.param, on ? null : value)}
              variant={on ? 'emphasis' : 'default'}
              {...(on ? { 'aria-current': 'true' as const } : {})}
            >
              {label} <span className="numeric text-xs opacity-70">{nf.format(count)}</span>
            </LinkButton>
          )
        })}
      </div>
    )
  }).filter(Boolean)

  if (groups.length === 0) return null

  return (
    <div
      className="mb-5 flex flex-wrap items-center gap-x-3 gap-y-2"
    >
      {groups}
      {current.size > 0 && (
        <LinkButton href={`/${lang}/search?q=${encodeURIComponent(q)}`} className="chip-clear">
          {t.clearFilters}
        </LinkButton>
      )}
    </div>
  )
}
