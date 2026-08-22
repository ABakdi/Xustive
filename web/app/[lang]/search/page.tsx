import { notFound, redirect } from 'next/navigation'

import { Filters } from '@/components/search/Filters'
import { Pagination } from '@/components/search/Pagination'
import { ResultCard } from '@/components/search/ResultCard'
import { InteractionBeacon } from '@/components/search/InteractionBeacon'
import { SearchBox } from '@/components/search/SearchBox'
import { Summary } from '@/components/search/Summary'
import { Verticals } from '@/components/search/Verticals'
import { KnowledgePanel } from '@/components/search/KnowledgePanel'
import { ToolCard } from '@/components/tools/ToolCard'
import { TranslateCard } from '@/components/tools/TranslateCard'
import { readDisabledTools } from '@/lib/tools'
import { translateLanguages } from '@/lib/api'
import { LangSwitcher } from '@/components/layout/LangSwitcher'
import { DensityToggle } from '@/components/layout/DensityToggle'
import { ThemeToggle } from '@/components/layout/ThemeToggle'
import { Wordmark } from '@/components/layout/Wordmark'
import { search, SearchFailed } from '@/lib/api'
import { isLocale, type Locale } from '@/lib/i18n/config'
import { formatNumber, plural } from '@/lib/i18n/format'
import { messages } from '@/lib/i18n/messages'
import { readDensity, readTheme } from '@/lib/theme'

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

  const vertical = one(sp, 'v')
  const query = new URLSearchParams({ q, page: String(page), hits_per_page: '20', ui: lang })
  for (const key of FILTER_PARAMS) {
    const value = one(sp, key)
    if (value) query.set(key, value)
  }
  if (vertical) query.set('v', vertical)

  let data
  try {
    data = await search(query)
  } catch (error) {
    const message = error instanceof SearchFailed ? error.message : t.errorTitle
    return (
      <Shell lang={lang} t={t} q={q}>
        <div className="py-16 text-center">
          <p className="text-xl">{t.errorTitle}</p>
          <p className="mt-2 text-sm" style={{ color: 'var(--fg-muted)' }}>
            {message}
          </p>
        </div>
      </Shell>
    )
  }

  const { pagination: p } = data
  // Read here rather than sent to the API. The set of tools someone has switched off is small,
  // stable and unusual enough to identify them across requests, so sending it would hand a
  // fingerprint to the one component that receives no preference data — a perverse outcome for a
  // privacy control. See lib/tools.ts.
  const disabledTools = await readDisabledTools()
  // Only fetched when a translation was actually asked for. The list is cached for an hour, but
  // an unconditional call would still put a request on every result page for a card that is
  // almost never shown.
  const languages = data.instant?.tool === 'translate' ? await translateLanguages() : []

  return (
    <Shell
      lang={lang}
      t={t}
      q={q}
      aside={
        <KnowledgePanel
          q={q}
          lang={lang}
          t={t}
          className="mt-6 self-start lg:mt-0 lg:sticky lg:top-20"
        />
      }
    >
      <p className="mb-5 text-sm" style={{ color: 'var(--fg-muted)' }}>
        {p.estimated ? `${t.resultsApprox} ` : ''}
        <bdi className="numeric">{formatNumber(lang, p.total_hits)}</bdi>{' '}
        {/* Arabic has six plural categories; a ternary is wrong for four of them. */}
        {plural(lang, p.total_hits, {
          zero: t.resultsCount,
          one: t.resultsCount,
          two: t.resultsCount,
          few: t.resultsCount,
          many: t.resultsCount,
          other: t.resultsCount,
        })}{' '}
        {/* The parentheses are the problem, not the number. Brackets are neutral characters that
            take direction from their surroundings, so in an Arabic line an unisolated `(12 ms)`
            renders with the brackets swapped — `(ms 12` — which reads as a typo rather than as a
            bidi artefact. Isolating the whole group keeps the pair together. */}
        <bdi>
          (<span className="numeric">{formatNumber(lang, data.took_ms)}</span> {t.took})
        </bdi>
      </p>

      <Verticals lang={lang} q={q} active={vertical} t={t} />

      {/* Above the results and below the search box. Rendered even when there are no results —
          `2+2` has an answer whether or not the corpus mentions arithmetic. */}
      {data.instant &&
        !disabledTools.has(data.instant.tool) &&
        // Translation is the one tool with its own card. It streams and must be cancellable,
        // which a server-rendered card cannot do — and the language pickers change the request
        // rather than navigating. Every other tool shares the one generic frame.
        (data.instant.tool === 'translate' ? (
          languages.length > 0 && (
            <TranslateCard
              detail={(data.instant.detail ?? {}) as never}
              t={t}
              uiLang={lang}
              languages={languages}
            />
          )
        ) : (
          <ToolCard answer={data.instant} t={t} locale={lang} />
        ))}

      {/* A question gets its answer first.
          Someone typing a topic wants a list of pages; someone asking a question wants an answer,
          and ten blue links above it makes them do the work themselves. The placement is the whole
          difference — the summary itself is identical, and it still cites its sources. */}
      {data.summary_token && data.is_question && (
        <Summary
          token={data.summary_token}
          note={t.summaryNote}
          loadingLabel={t.summaryLoading}
          sourcesLabel={t.sources}
          prominent
        />
      )}

      {data.results.length === 0 ? (
        <div className="py-16 text-center">
          {vertical && vertical !== 'all' ? (
            <>
              {/* Name the empty vertical, and offer the way out — the corpus may hold the answer
                  outside this vertical even when the vertical is empty. */}
              <p className="text-xl">{vertical === 'files' ? t.noFiles : t.noNews}</p>
              <p className="mt-2 text-sm">
                <a
                  href={`/${lang}/search?q=${encodeURIComponent(q)}`}
                  style={{ color: 'var(--accent)' }}
                >
                  {t.noNewsHint}
                </a>
              </p>
            </>
          ) : (
            <>
              <p className="text-xl">{t.noResults}</p>
              <p className="mt-2 text-sm" style={{ color: 'var(--fg-muted)' }}>
                {t.noResultsHint}
              </p>
            </>
          )}
        </div>
      ) : (
        <>
          <Filters lang={lang} t={t} facets={data.facets} active={sp} q={q} />

          {/* Degraded, not empty. When the backend drops facets under load the filter row is bare,
              and without this note that reads as "nothing here to filter by" rather than "filters
              are resting". Shown only for the real degradation, never for a genuinely unfacetable
              result. */}
          {data.facets_degraded && (
            <p className="mb-4 text-xs" style={{ color: 'var(--fg-faint)' }}>
              {t.filtersUnavailable}
            </p>
          )}

          {/* Below the filters for a topic. A question puts it above them instead — see the
              block before the filters. Fetched after paint either way: on CPU a summary takes
              tens of seconds and nothing on the page may wait for it. */}
          {data.summary_token && !data.is_question && (
            <Summary
              token={data.summary_token}
              note={t.summaryNote}
              loadingLabel={t.summaryLoading}
              sourcesLabel={t.sources}
            />
          )}

          {/* The result list. When interaction signals are on, the API returns an opaque token and
              the list is wrapped in the anonymous click beacon (a single delegated listener). With
              no token the beacon is absent entirely — no listener, nothing recorded. */}
          {data.interaction_token ? (
            <InteractionBeacon token={data.interaction_token}>
              <ol
                className="list-none p-0"
                style={{ display: 'grid', gap: 'var(--result-gap)', gridTemplateColumns: 'minmax(0, 1fr)' }}
              >
                {data.results.map((result) => (
                  <ResultCard key={result.id} result={result} t={t} locale={lang} />
                ))}
              </ol>
            </InteractionBeacon>
          ) : (
            <ol
              className="list-none p-0"
              style={{ display: 'grid', gap: 'var(--result-gap)', gridTemplateColumns: 'minmax(0, 1fr)' }}
            >
              {data.results.map((result) => (
                <ResultCard key={result.id} result={result} t={t} locale={lang} />
              ))}
            </ol>
          )}

          <Pagination lang={lang} t={t} pagination={p} params={sp} q={q} />
        </>
      )}
    </Shell>
  )
}

async function Shell({
  lang,
  t,
  q,
  aside,
  children,
}: {
  lang: Locale
  t: ReturnType<typeof messages>
  q: string
  /** The right-hand knowledge rail, when the page has one. Absent on the error shell. */
  aside?: React.ReactNode
  children: React.ReactNode
}) {
  const [theme, density] = await Promise.all([readTheme(), readDensity()])
  return (
    <>
      <header
        className="sticky top-0 border-b"
        style={{
          borderColor: 'var(--line)',
          background: 'var(--bg)',
          zIndex: 'var(--z-sticky)' as unknown as number,
        }}
      >
        <div className="mx-auto flex max-w-3xl items-center gap-4 px-6 py-2.5">
          <Wordmark lang={lang} size="sm" />
          <div className="min-w-0 flex-1">
            <SearchBox lang={lang} t={t} initialQuery={q} compact />
          </div>
          <div className="flex shrink-0 items-center gap-1">
            <LangSwitcher current={lang} label={t.language} />
            <ThemeToggle
              current={theme}
              labels={{ system: t.themeSystem, light: t.themeLight, dark: t.themeDark }}
            />
<DensityToggle
              current={density}
              labels={{ comfortable: t.densityComfortable, compact: t.densityCompact }}
            />
          </div>
        </div>
      </header>

      <main className="mx-auto min-w-0 max-w-3xl px-6 py-6 lg:max-w-5xl">
        {aside ? (
          // Two columns on large screens: results (capped for readability) + a sticky knowledge
          // rail. One column below that, where the rail falls under the results.
          <div className="lg:grid lg:grid-cols-[minmax(0,1fr)_300px] lg:items-start lg:gap-8">
            <div className="min-w-0">{children}</div>
            {aside}
          </div>
        ) : (
          children
        )}
      </main>
    </>
  )
}
