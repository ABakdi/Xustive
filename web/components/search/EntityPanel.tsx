'use client'

import { useEffect, useState } from 'react'

import { knowledgePanel, type EntityFact, type EntityPanel as Panel } from '@/lib/api'
import type { Messages } from '@/lib/i18n/messages'

/**
 * The entity panel (M8-T08).
 *
 * Fetched out of band, after paint, so the results never wait for it. Unlike the Wikipedia panel
 * it replaces, this one **does** show a loading state: it is wide, and a reader who is about to
 * get a face and five facts should be told they are coming. The three states are the ones
 * `Summary` established — loading, resolved-empty (collapse to nothing at all), resolved-full —
 * because a placeholder that never fills is worse than no placeholder.
 */
export default function EntityPanel({
  q,
  lang,
  t,
  className = '',
}: {
  q: string
  lang: string
  t: Messages
  className?: string
}) {
  // `undefined` is "still asking", `null` is "asked, and there is nothing" — a distinction the
  // panel's whole rendering depends on and which a single nullable would lose.
  const [panel, setPanel] = useState<Panel | null | undefined>(undefined)
  // Whether the fetch has actually started, which is only true once an effect has run — so only
  // ever in a browser with JavaScript. Without it the skeleton would be server-rendered into the
  // HTML and, with scripting off, would sit there forever: a permanently loading frame, which is
  // worse than no rail at all and is what M8-T08.5 exists to prevent.
  const [asking, setAsking] = useState(false)

  useEffect(() => {
    const controller = new AbortController()
    setPanel(undefined)
    setAsking(true)
    knowledgePanel(q, lang, controller.signal)
      // The store first — a millisecond, and the answer for anything harvested. Then the live
      // fallback through the web tier for what the store does not hold yet; the miss was
      // recorded, so the harvester catches up and this second hop stops being taken for it.
      .then(async (data) => {
        if (data) return data
        const res = await fetch(
          `/api/knowledge-live?q=${encodeURIComponent(q)}&lang=${encodeURIComponent(lang)}`,
          { signal: controller.signal },
        )
        return res.ok && res.status !== 204 ? ((await res.json()) as Panel) : null
      })
      .then((data) => setPanel(data))
      // An aborted fetch is a re-render, not a failure. Anything else is treated as "no panel",
      // because the rail is additive and nothing on the page depends on it.
      .catch(() => setPanel(null))
    return () => controller.abort()
  }, [q, lang])

  if (!asking) return null
  if (panel === undefined) return <PanelSkeleton className={className} label={t.knowledgeLoading} />
  if (!panel) return null

  const image = panel.images[0]
  const facts = panel.facts
  const sources = Array.from(
    new Set([
      ...facts.map((f) => f.provenance.source),
      ...(panel.extract ? [panel.extract.source] : []),
    ]),
  )

  return (
    <aside
      className={`rise rounded-lg border ${className}`.trim()}
      style={{ borderColor: 'var(--line)', background: 'var(--bg-sunk)' }}
      aria-label={panel.title ?? undefined}
    >
      {image && (
        // Proxied same-origin so the reader's address never reaches the image host, and a plain
        // img rather than next/image because that host is deliberately not a configured domain.
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src={`/api/wiki-image?u=${encodeURIComponent(image.url)}`}
          alt=""
          className="max-h-72 w-full rounded-t-lg object-cover"
          loading="lazy"
          referrerPolicy="no-referrer"
        />
      )}
      <div className="p-4">
        <h2 className="m-0 text-lg font-semibold tracking-tight" dir="auto">
          {panel.title}
        </h2>
        {panel.description && (
          <p className="mt-0.5 text-sm" style={{ color: 'var(--fg-muted)' }} dir="auto">
            {panel.description}
          </p>
        )}

        {facts.length > 0 && (
          <dl className="mt-3 grid gap-x-3 gap-y-1 text-sm" style={{ gridTemplateColumns: 'auto 1fr' }}>
            {groupByKey(facts).map(([key, group]) => (
              <FactRow key={key} factKey={key} group={group} t={t} lang={lang} />
            ))}
          </dl>
        )}

        {/* A written line, shown only where there is no encyclopedic paragraph and always marked
            as generated. Unlabelled machine prose beside human prose is the thing to avoid. */}
        {!panel.extract && panel.blurb && (
          <p className="mt-3 text-sm leading-relaxed" dir="auto">
            {panel.blurb.text}{' '}
            <span className="text-xs" style={{ color: 'var(--fg-faint)' }}>
              {t.entityGenerated}
            </span>
          </p>
        )}

        {panel.extract && (
          <p className="mt-3 text-sm leading-relaxed" dir="auto">
            {panel.extract.text}
          </p>
        )}

        {panel.authorities.length > 0 && (
          <ul className="mt-3 flex list-none flex-wrap gap-2 p-0 text-sm">
            {panel.authorities.map((a) => (
              <li key={a.key}>
                <a
                  href={a.url}
                  target="_blank"
                  rel="noopener nofollow noreferrer"
                  className="rounded border px-2 py-0.5"
                  style={{ borderColor: 'var(--line)' }}
                >
                  <bdi>{AUTHORITY_NAMES[a.key] ?? a.key}</bdi> ↗
                </a>
              </li>
            ))}
          </ul>
        )}

        {panel.also && (
          <p className="mt-3 text-sm" style={{ color: 'var(--fg-muted)' }} dir="auto">
            {t.entityAlso}: <span style={{ color: 'var(--fg)' }}>{panel.also.title}</span>
            {panel.also.description ? ` — ${panel.also.description}` : ''}
          </p>
        )}

        {/* Attribution is not optional decoration: the claims are CC0, the extract is share-alike,
            and each image carries its own licence. Naming the sources is what the licences ask for. */}
        <p className="mt-3 text-xs" style={{ color: 'var(--fg-faint)' }} dir="auto">
          {t.entitySources}: <bdi>{sources.join(' · ')}</bdi>
          {image?.credit_url && (
            <>
              {' · '}
              <a href={image.credit_url} target="_blank" rel="noopener nofollow noreferrer">
                <bdi>{image.licence}</bdi> ↗
              </a>
            </>
          )}
        </p>
      </div>
    </aside>
  )
}

/** Authority names are brands, not translatable strings — IMDb is IMDb in every language. */
const AUTHORITY_NAMES: Record<string, string> = {
  imdb: 'IMDb',
  rotten_tomatoes: 'Rotten Tomatoes',
  tmdb: 'TMDB',
  metacritic: 'Metacritic',
  musicbrainz: 'MusicBrainz',
  facebook: 'Facebook',
  x: 'X',
}

/** Several values of one key render as one row, not several — "Genre: drama, war, historical". */
function groupByKey(facts: EntityFact[]): [string, EntityFact[]][] {
  const out: [string, EntityFact[]][] = []
  for (const f of facts) {
    const existing = out.find(([k]) => k === f.key)
    if (existing) existing[1].push(f)
    else out.push([f.key, [f]])
  }
  return out
}

function FactRow({
  factKey,
  group,
  t,
  lang,
}: {
  factKey: string
  group: EntityFact[]
  t: Messages
  lang: string
}) {
  const label = (t as unknown as Record<string, string>)[`f_${factKey}`]
  // A fact with no translated label is not rendered. Showing a raw machine key would be worse
  // than showing nothing, and the missing translation is a build-time thing to fix, not a
  // runtime thing to paper over.
  if (!label) return null
  return (
    <>
      <dt style={{ color: 'var(--fg-muted)' }} dir="auto">
        {label}
      </dt>
      <dd className="m-0" dir="auto">
        {group.map((f, i) => (
          <span key={i}>
            {i > 0 && '، '.replace('، ', lang === 'fr' || lang === 'en' ? ', ' : '، ')}
            <FactValue fact={f} lang={lang} />
          </span>
        ))}
      </dd>
    </>
  )
}

function FactValue({ fact, lang }: { fact: EntityFact; lang: string }) {
  const v = fact.value
  switch (v.type) {
    case 'text':
      return <bdi>{v.v}</bdi>
    case 'entity':
      return <bdi>{v.v.label}</bdi>
    case 'number':
      return <bdi>{new Intl.NumberFormat(lang).format(v.v)}</bdi>
    case 'quantity':
      return (
        <bdi>
          {new Intl.NumberFormat(lang).format(v.v.amount)} {v.v.unit}
        </bdi>
      )
    case 'score':
      // The reviewer is part of the fact, never dropped: 99/100 means something different from
      // Metacritic than from an audience, which is why an unattributed score is never stored.
      return (
        <bdi>
          {v.v.value}/{v.v.best} — {v.v.reviewer}
        </bdi>
      )
    case 'date':
      return <bdi>{formatDate(v.v.at, v.v.precision, lang)}</bdi>
  }
}

/** Render only the precision the publisher asserted — a year-precision date is a year. */
function formatDate(at: number, precision: 'year' | 'month' | 'day', lang: string) {
  const d = new Date(at * 1000)
  const opts: Intl.DateTimeFormatOptions =
    precision === 'year'
      ? { year: 'numeric', timeZone: 'UTC' }
      : precision === 'month'
        ? { year: 'numeric', month: 'long', timeZone: 'UTC' }
        : { year: 'numeric', month: 'long', day: 'numeric', timeZone: 'UTC' }
  return new Intl.DateTimeFormat(lang, opts).format(d)
}

/**
 * The waiting state. Sized like a real panel so nothing on the page moves when the answer lands —
 * a rail that jumps as it fills is worse than one that appears late.
 */
function PanelSkeleton({ className, label }: { className: string; label: string }) {
  return (
    <aside
      className={`rounded-lg border ${className}`.trim()}
      style={{ borderColor: 'var(--line)', background: 'var(--bg-sunk)' }}
      aria-busy="true"
      aria-live="polite"
      aria-label={label}
    >
      <div className="p-4">
        <span className="sr-only">{label}</span>
        <div
          className="animate-pulse rounded"
          style={{ blockSize: '1.25rem', inlineSize: '60%', background: 'var(--line)' }}
        />
        <div
          className="mt-2 animate-pulse rounded"
          style={{ blockSize: '0.75rem', inlineSize: '85%', background: 'var(--line)' }}
        />
        <div
          className="mt-4 animate-pulse rounded"
          style={{ blockSize: '0.75rem', inlineSize: '70%', background: 'var(--line)' }}
        />
        <div
          className="mt-2 animate-pulse rounded"
          style={{ blockSize: '0.75rem', inlineSize: '55%', background: 'var(--line)' }}
        />
      </div>
    </aside>
  )
}
