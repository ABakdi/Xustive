'use client'

import { useEffect, useState } from 'react'

import type { Messages } from '@/lib/i18n/messages'

type Entity = {
  title: string
  description: string | null
  extract: string
  thumb: string | null
  url: string
  lang: string
}

/**
 * The knowledge side-box.
 *
 * A right-hand card for entity queries — a person, a place, a thing, a concept — pulled from
 * Wikipedia through this app's own server (`/api/knowledge`), so the browser never contacts
 * Wikimedia and the reader's lookups stay on one origin. Absent by default: it appears only when the
 * server resolves the query to a real article, and collapses to nothing otherwise, so a query the
 * corpus answers as a plain list is not crowded by a guess.
 *
 * Fetched after paint like the summary — it must never delay the results.
 */
export function KnowledgePanel({
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
  const [entity, setEntity] = useState<Entity | null>(null)

  useEffect(() => {
    const controller = new AbortController()
    setEntity(null)
    fetch(`/api/knowledge?q=${encodeURIComponent(q)}&lang=${encodeURIComponent(lang)}`, {
      signal: controller.signal,
    })
      .then((res) => (res.ok ? res.json() : null))
      .then((data: Entity | null) => {
        if (data && data.extract) setEntity(data)
      })
      .catch(() => {})
    return () => controller.abort()
  }, [q, lang])

  // Silent until it resolves: the box either has an entity to show or takes no space at all. A
  // loading placeholder in a side rail that most queries never fill would just be noise — unlike the
  // AI summary, whose wait the reader is explicitly waiting on.
  if (!entity) return null

  return (
    <aside
      className={`rise rounded-lg border ${className}`.trim()}
      style={{ borderColor: 'var(--line)', background: 'var(--bg-sunk)' }}
      aria-label={entity.title}
    >
      {entity.thumb && (
        // Proxied through /api/wiki-image; a plain img (not next/image) because the source is a
        // remote host we deliberately do not configure as a Next image domain.
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src={entity.thumb}
          alt=""
          className="max-h-56 w-full rounded-t-lg object-cover"
          loading="lazy"
          referrerPolicy="no-referrer"
        />
      )}
      <div className="p-4">
        <h2 className="m-0 text-lg font-semibold tracking-tight" dir="auto">
          {entity.title}
        </h2>
        {entity.description && (
          <p className="mt-0.5 mb-0 text-xs" dir="auto" style={{ color: 'var(--fg-muted)' }}>
            {entity.description}
          </p>
        )}
        <p className="mt-3 mb-0 text-sm" dir="auto" style={{ lineHeight: 1.6 }}>
          {entity.extract}
        </p>
        <p className="mt-3 mb-0 text-xs">
          <a
            href={entity.url}
            target="_blank"
            rel="noopener nofollow"
            style={{ color: 'var(--accent)' }}
          >
            {t.knowledgeSource} ↗
          </a>
        </p>
      </div>
    </aside>
  )
}
