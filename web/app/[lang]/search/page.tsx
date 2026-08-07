import { notFound, redirect } from 'next/navigation'

import { Filters } from '@/components/search/Filters'
import { Pagination } from '@/components/search/Pagination'
import { ResultCard } from '@/components/search/ResultCard'
import { SearchBox } from '@/components/search/SearchBox'
import { Summary } from '@/components/search/Summary'
import { Wordmark } from '@/components/layout/Wordmark'
import { search, SearchFailed } from '@/lib/api'
import { isLocale } from '@/lib/i18n/config'
import { messages } from '@/lib/i18n/messages'

/**
 * The results page.
 *
 * A Server Component with no `'use client'` anywhere in its own tree. The results arrive as HTML
 * in the first response, which is not a preference: a meaningful share of Algerian traffic is on
 * connections where waiting for a bundle to parse before any content appears is the difference
 * between a usable engine and a blank screen.
 *
 * Only three things below are client components — the search box, the filter chips' enhancement,
 * and the summary. Everything else, including the entire result list, ships as markup.
 */
export const dynamic = 'force-dynamic'

const FILTER_PARAMS = ['lang', 'source', 'sentiment'] as const

type SearchParams = Record<string, string | string[] | undefined>

function one(params: SearchParams, key: string): string | undefined {
  const value = params[key]
  return Array.isArray(value) ? value[0] : value
}

export async function generateMetadata({ searchParams }: { searchParams: Promise<SearchParams> }) {
  const q = one(await searchParams, 'q')
  return { title: q ? q : 'Search' }
}

export default async function SearchPage({
  params,
  searchParams,
}: {
  params: Promise<{ lang: string }>
  searchParams: Promise<SearchParams>
}) {
  const { lang } = await params
  if (!isLocale(lang)) notFound()
  const sp = await searchParams
  const t = messages(lang)

  const q = one(sp, 'q')?.trim()
  // An empty query is the home page, not an empty results shell.
  if (!q) redirect(`/${lang}`)

  const page = Math.max(1, Number(one(sp, 'page') ?? 1) || 1)

  const query = new URLSearchParams({ q, page: String(page), hits_per_page: '20' })
  for (const key of FILTER_PARAMS) {
    const value = one(sp, key)
    if (value) query.set(key, value)
  }

  let data
  try {
    data = await search(query)
  } catch (error) {
    const message = error instanceof SearchFailed ? error.message : t.errorTitle
    return (
      <Shell lang={lang} t={t} q={q}>
        <div className="py-16 text-center">
          <p className="text-xl">{t.errorTitle}</p>
          <p className="mt-2 text-sm" style={{ color: 'var(--ink-muted)' }}>
            {message}
          </p>
        </div>
      </Shell>
    )
  }

  const { pagination: p } = data
  const nf = new Intl.NumberFormat(lang === 'ary' ? 'ar' : lang)

  return (
    <Shell lang={lang} t={t} q={q}>
      <p className="mb-5 text-sm" style={{ color: 'var(--ink-muted)' }}>
        {p.estimated ? `${t.resultsApprox} ` : ''}
        <span className="numeric">{nf.format(p.total_hits)}</span> {t.resultsCount} (
        <span className="numeric">{nf.format(data.took_ms)}</span> {t.took})
      </p>

      {data.results.length === 0 ? (
        <div className="py-16 text-center">
          <p className="text-xl">{t.noResults}</p>
          <p className="mt-2 text-sm" style={{ color: 'var(--ink-muted)' }}>
            {t.noResultsHint}
          </p>
        </div>
      ) : (
        <>
          <Filters lang={lang} t={t} facets={data.facets} active={sp} q={q} />

          {/* Fetched after paint. On CPU a summary takes tens of seconds; nothing may wait. */}
          {data.summary_token && <Summary token={data.summary_token} note={t.summaryNote} />}

          <ol
            className="list-none p-0"
            style={{ display: 'grid', gap: 'var(--result-gap)', gridTemplateColumns: 'minmax(0, 1fr)' }}
          >
            {data.results.map((result) => (
              <ResultCard key={result.id} result={result} t={t} locale={lang} />
            ))}
          </ol>

          <Pagination lang={lang} t={t} pagination={p} params={sp} q={q} />
        </>
      )}
    </Shell>
  )
}

function Shell({
  lang,
  t,
  q,
  children,
}: {
  lang: string
  t: ReturnType<typeof messages>
  q: string
  children: React.ReactNode
}) {
  return (
    <>
      <header
        className="sticky top-0 border-b"
        style={{
          borderColor: 'var(--rule)',
          background: 'var(--paper)',
          zIndex: 'var(--z-sticky)' as unknown as number,
        }}
      >
        <div className="mx-auto flex max-w-3xl items-center gap-6 px-6 py-3">
          <Wordmark lang={lang} size="sm" />
          <div className="min-w-0 flex-1">
            <SearchBox lang={lang} t={t} initialQuery={q} compact />
          </div>
        </div>
      </header>

      <main className="mx-auto min-w-0 max-w-3xl px-6 py-6">{children}</main>
    </>
  )
}
