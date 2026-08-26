'use client'

import { useEffect, useRef, useState } from 'react'

import type { Messages } from '@/lib/i18n/messages'
import type { Relation } from '@/lib/relations'
import { Icon, type IconName } from '@/components/ui/Icon'

/**
 * A row of cards for a relation query (M8-T11): the cast of a film, the books of an author.
 *
 * Mounted only when the server decided the query is relation-shaped, fetched after paint like the
 * entity panel, and collapses to nothing when the list is empty. Each card links to an authority
 * by identifier — Wikipedia first, then IMDb, Goodreads, Open Library, Google Books.
 *
 * The row scrolls sideways without a scrollbar: the wheel and a swipe move it, and two arrows
 * appear at the edges while the pointer is over it — only the edge that has somewhere to go.
 */

type Card = {
  id: string
  title: string
  description: string | null
  year: string | null
  thumb: string | null
  links: { key: string; url: string }[]
}

type Related = { group: 'series' | 'seasons'; items: { id: string; title: string; year: string | null }[] }

type ListAnswer = {
  relation: Relation
  subject: { id: string; title: string }
  related: Related | null
  cards: Card[]
}

/** The event the row raises when the reader picks another part: the side panel follows it. */
export const SUBJECT_EVENT = 'xustive:subject'

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
  // A part or season the reader picked from "see also"; null is "whatever the query resolves to".
  const [picked, setPicked] = useState<string | null>(null)
  // The "see also" row survives a pick: it belongs to the family, not to the member shown.
  const [family, setFamily] = useState<Related | null>(null)
  const row = useRef<HTMLUListElement>(null)
  const [edges, setEdges] = useState<{ start: boolean; end: boolean }>({ start: false, end: false })

  // Which arrows to offer. Measured, not assumed: a row of three cards has nowhere to go, and
  // an arrow that does nothing is worse than none. In RTL the scroll position runs negative, so
  // distance from either edge is taken in absolute terms.
  const measure = () => {
    const el = row.current
    if (!el) return
    const travelled = Math.abs(el.scrollLeft)
    const room = el.scrollWidth - el.clientWidth
    setEdges({ start: travelled > 4, end: room - travelled > 4 })
  }
  useEffect(() => {
    measure()
    const el = row.current
    if (!el) return
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [answer])

  const nudge = (towards: 'start' | 'end') => {
    const el = row.current
    if (!el) return
    const rtl = getComputedStyle(el).direction === 'rtl'
    const step = Math.max(el.clientWidth * 0.8, 200)
    const sign = (towards === 'end' ? 1 : -1) * (rtl ? -1 : 1)
    el.scrollBy({ left: sign * step, behavior: 'smooth' })
  }

  useEffect(() => {
    setPicked(null)
    setFamily(null)
  }, [q, lang])

  useEffect(() => {
    const controller = new AbortController()
    setAnswer(undefined)
    setAsking(true)
    const pick = picked ? `&subject=${encodeURIComponent(picked)}` : ''
    fetch(`/api/knowledge-list?q=${encodeURIComponent(q)}&lang=${encodeURIComponent(lang)}${pick}`, {
      signal: controller.signal,
    })
      .then(async (res) => (res.ok && res.status !== 204 ? ((await res.json()) as ListAnswer) : null))
      .then((a) => {
        setAnswer(a)
        if (a?.related && !picked) setFamily(a.related)
      })
      .catch(() => setAnswer(null))
    return () => controller.abort()
  }, [q, lang, picked])

  const pick = (id: string, title: string) => {
    setPicked(id)
    // The side panel shows the same thing the row does, at once, without a page load.
    window.dispatchEvent(new CustomEvent(SUBJECT_EVENT, { detail: { id, title } }))
  }

  if (!asking) return null
  if (answer === undefined) {
    return (
      <section aria-busy="true" aria-live="polite" className="mb-6 text-sm" style={{ color: 'var(--fg-muted)' }}>
        <Icon name="sparkle" size={14} style={{ color: 'var(--accent)' }} /> {t.knowledgeLoading}
      </section>
    )
  }
  if ((!answer || answer.cards.length === 0) && !family) return null

  const headings: Record<Relation, string> = {
    cast: t.listCast,
    books: t.listBooks,
    films: t.listFilms,
    albums: t.listAlbums,
  }
  // The primary link is the first authority — Wikipedia when there is an article.
  const relation: Relation = answer?.relation ?? 'cast'
  const current = picked ?? answer?.subject.id ?? null
  return (
    <section className="rise mb-8" aria-label={headings[relation]}>
      {family && (
        <p className="m-0 mb-3 flex flex-wrap items-center gap-2 text-sm" dir="auto">
          <span style={{ color: 'var(--fg-muted)' }}>
            {t.listSeeAlso} · {family.group === 'seasons' ? t.listSeasons : t.listSeries}
          </span>
          {family.items.map((it) => {
            const active = it.id === current
            return (
              <button
                key={it.id}
                type="button"
                aria-pressed={active}
                onClick={() => pick(it.id, it.title)}
                className="rounded-[var(--radius-pill)] border px-2.5 py-1 text-xs transition-colors"
                style={{
                  borderColor: active ? 'var(--accent)' : 'var(--line)',
                  background: active ? 'var(--accent-wash)' : 'transparent',
                  color: active ? 'var(--accent)' : 'var(--fg)',
                }}
              >
                <bdi>{it.title}</bdi>
                {it.year && (
                  <span className="ms-1" style={{ color: 'var(--fg-muted)' }}>
                    {it.year}
                  </span>
                )}
              </button>
            )
          })}
        </p>
      )}
      {answer && (
      <h2 className="m-0 mb-3 flex items-center gap-2 text-base font-semibold" dir="auto">
        <span
          className="inline-flex items-center gap-1.5 rounded-[var(--radius-pill)] px-2 py-0.5 text-xs font-medium"
          style={{ background: 'var(--accent-wash)', color: 'var(--accent)' }}
        >
          <Icon name={RELATION_ICON[answer.relation]} size={14} />
          {headings[answer.relation]}
        </span>
        <bdi>{answer.subject.title}</bdi>
      </h2>
      )}
      <div className="group relative">
      <ul
        ref={row}
        onScroll={measure}
        className="list-row m-0 flex list-none gap-4 overflow-x-auto p-0 pb-1"
        style={{ scrollSnapType: 'x proximity' }}
      >
        {(answer?.cards ?? []).map((c) => {
          const primary = c.links[0]
          return (
            <li
              key={c.id}
              className="flex w-44 shrink-0 flex-col rounded-lg border"
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
                    className="block h-44 w-full rounded-t-lg object-cover"
                  />
                ) : (
                  <span
                    aria-hidden
                    className="flex h-44 w-full items-center justify-center rounded-t-lg"
                    style={{ color: 'var(--fg-faint)' }}
                  >
                    <Icon name={RELATION_ICON[relation] === 'users' ? 'user' : RELATION_ICON[relation]} size={28} />
                  </span>
                )}
                <span className="block px-3 pt-2.5 text-sm font-medium leading-snug" style={{ color: 'var(--fg)' }}>
                  <bdi>{c.title}</bdi>
                  {c.year && (
                    <span className="ms-1 text-xs font-normal" style={{ color: 'var(--fg-muted)' }}>
                      <bdi>{c.year}</bdi>
                    </span>
                  )}
                </span>
              </a>
              {c.description && (
                <span className="line-clamp-2 px-3 pt-1 text-xs leading-relaxed" style={{ color: 'var(--fg-muted)' }}>
                  {c.description}
                </span>
              )}
              <span className="mt-auto flex flex-wrap gap-1.5 px-3 pb-3 pt-2.5">
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
      {(['start', 'end'] as const).map((side) =>
        edges[side] ? (
          <button
            key={side}
            type="button"
            aria-label={side === 'start' ? t.listScrollBack : t.listScrollForward}
            onClick={() => nudge(side)}
            className={`absolute top-1/2 ${side === 'start' ? 'start-1' : 'end-1'} flex h-9 w-9 -translate-y-1/2 items-center justify-center rounded-full border opacity-0 shadow-sm transition-opacity focus-visible:opacity-100 group-hover:opacity-100`}
            style={{ background: 'var(--bg)', borderColor: 'var(--line)', color: 'var(--fg)' }}
          >
            <Icon name={side === 'start' ? 'chevron-start' : 'chevron-end'} size={16} className="rtl-flip" />
          </button>
        ) : null,
      )}
      </div>
    </section>
  )
}
