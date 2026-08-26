'use client'

import { useEffect, useState } from 'react'

import type { Messages } from '@/lib/i18n/messages'
import type { Relation } from '@/lib/relations'
import { Icon, type IconName } from '@/components/ui/Icon'

/**
 * A row of cards for a relation query (M8-T11): the cast of a film, the books of an author.
 *
 * Mounted only when the server decided the query is relation-shaped, fetched after paint like the
 * entity panel, and collapses to nothing when the list is empty. Each card links to an authority
 * by identifier — Wikipedia first, then IMDb, Goodreads, Open Library, Google Books — and shows a
 * rating only when an open source publishes one, named.
 */

type Card = {
  id: string
  title: string
  description: string | null
  year: string | null
  thumb: string | null
  links: { key: string; url: string }[]
  rating: { average: number; count: number; source: string } | null
}

type ListAnswer = {
  relation: Relation
  subject: { id: string; title: string }
  cards: Card[]
}

const LINK_NAMES: Record<string, string> = {
  wikipedia: 'Wikipedia',
  wikidata: 'Wikidata',
  imdb: 'IMDb',
  goodreads: 'Goodreads',
  openlibrary: 'Open Library',
  googlebooks: 'Google Books',
}

const LINK_MARK: Record<string, string> = {
  imdb: '#F5C518',
  goodreads: '#553B08',
  openlibrary: '#0C7DB0',
  googlebooks: '#4285F4',
}

const RELATION_ICON: Record<Relation, IconName> = {
  cast: 'users',
  books: 'book',
  films: 'film',
  albums: 'music',
}

export default function ListPanel({ q, lang, t }: { q: string; lang: string; t: Messages }) {
  const [answer, setAnswer] = useState<ListAnswer | null | undefined>(undefined)
  const [asking, setAsking] = useState(false)

  useEffect(() => {
    const controller = new AbortController()
    setAnswer(undefined)
    setAsking(true)
    fetch(`/api/knowledge-list?q=${encodeURIComponent(q)}&lang=${encodeURIComponent(lang)}`, {
      signal: controller.signal,
    })
      .then(async (res) => (res.ok && res.status !== 204 ? ((await res.json()) as ListAnswer) : null))
      .then(setAnswer)
      .catch(() => setAnswer(null))
    return () => controller.abort()
  }, [q, lang])

  if (!asking) return null
  if (answer === undefined) {
    return (
      <section aria-busy="true" aria-live="polite" className="mb-6 text-sm" style={{ color: 'var(--fg-muted)' }}>
        <Icon name="sparkle" size={14} style={{ color: 'var(--accent)' }} /> {t.knowledgeLoading}
      </section>
    )
  }
  if (!answer || answer.cards.length === 0) return null

  const headings: Record<Relation, string> = {
    cast: t.listCast,
    books: t.listBooks,
    films: t.listFilms,
    albums: t.listAlbums,
  }
  // The primary link is the first authority — Wikipedia when there is an article.
  return (
    <section className="rise mb-6" aria-label={headings[answer.relation]}>
      <h2 className="m-0 mb-2 flex items-center gap-2 text-base font-semibold" dir="auto">
        <span
          className="inline-flex items-center gap-1.5 rounded-[var(--radius-pill)] px-2 py-0.5 text-xs font-medium"
          style={{ background: 'var(--accent-wash)', color: 'var(--accent)' }}
        >
          <Icon name={RELATION_ICON[answer.relation]} size={14} />
          {headings[answer.relation]}
        </span>
        <bdi>{answer.subject.title}</bdi>
      </h2>
      <ul
        className="m-0 flex list-none gap-3 overflow-x-auto p-0 pb-2"
        style={{ scrollSnapType: 'x proximity' }}
      >
        {answer.cards.map((c) => {
          const primary = c.links[0]
          return (
            <li
              key={c.id}
              className="flex w-40 shrink-0 flex-col rounded-lg border"
              style={{ borderColor: 'var(--line)', background: 'var(--bg-sunk)', scrollSnapAlign: 'start' }}
              dir="auto"
            >
              <a href={primary?.url} target="_blank" rel="noopener noreferrer nofollow" className="block no-underline">
                {c.thumb ? (
                  // Proxied and signed like every other remote image.
                  // eslint-disable-next-line @next/next/no-img-element
                  <img
                    src={c.thumb}
                    alt=""
                    loading="lazy"
                    referrerPolicy="no-referrer"
                    className="block h-40 w-full rounded-t-lg object-cover"
                  />
                ) : (
                  <span
                    aria-hidden
                    className="flex h-40 w-full items-center justify-center rounded-t-lg"
                    style={{ color: 'var(--fg-faint)' }}
                  >
                    <Icon name={RELATION_ICON[answer.relation] === 'users' ? 'user' : RELATION_ICON[answer.relation]} size={28} />
                  </span>
                )}
                <span className="block px-2 pt-2 text-sm font-medium leading-snug" style={{ color: 'var(--fg)' }}>
                  <bdi>{c.title}</bdi>
                  {c.year && (
                    <span className="ms-1 text-xs font-normal" style={{ color: 'var(--fg-muted)' }}>
                      <bdi>{c.year}</bdi>
                    </span>
                  )}
                </span>
              </a>
              {c.description && (
                <span className="line-clamp-2 px-2 pt-0.5 text-xs" style={{ color: 'var(--fg-muted)' }}>
                  {c.description}
                </span>
              )}
              {c.rating && (
                <span className="flex items-center gap-1 px-2 pt-1 text-xs" title={c.rating.source}>
                  <Icon name="star" size={12} style={{ color: 'var(--accent)' }} />
                  <bdi>
                    {c.rating.average} · {c.rating.count} · {c.rating.source}
                  </bdi>
                </span>
              )}
              <span className="mt-auto flex flex-wrap gap-1 px-2 pb-2 pt-1.5">
                {c.links.map((l) => (
                  <a
                    key={l.key}
                    href={l.url}
                    target="_blank"
                    rel="noopener noreferrer nofollow"
                    className="inline-flex items-center gap-1 rounded-[var(--radius-pill)] border px-1.5 py-0.5 text-[11px] no-underline"
                    style={{ borderColor: 'var(--line)', color: 'var(--fg-muted)' }}
                  >
                    {LINK_MARK[l.key] && (
                      <span aria-hidden className="inline-block h-1.5 w-1.5 rounded-full" style={{ background: LINK_MARK[l.key] }} />
                    )}
                    <bdi>{LINK_NAMES[l.key] ?? l.key}</bdi>
                  </a>
                ))}
              </span>
            </li>
          )
        })}
      </ul>
      {answer.relation === 'books' && (
        <p className="m-0 mt-1 text-xs" style={{ color: 'var(--fg-faint)' }} dir="auto">
          {t.listRatingNote}
        </p>
      )}
    </section>
  )
}
