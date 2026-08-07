'use client'

import { useEffect, useState } from 'react'

import { summarise, type SummaryResponse } from '@/lib/api'

/**
 * The AI summary.
 *
 * Fetched **after** the results have painted, never with them. On CPU a summary of real crawled
 * pages takes tens of seconds against a 40 ms search; blocking results on that would trade the
 * whole product for one feature.
 *
 * Reserves no height and stays absent until there is something to show. Most summaries never
 * arrive — the model refuses, the queue is full, the machine is slow — and a placeholder that
 * collapses would move the results out from under the reader.
 *
 * Carries the qalam rule, because this is the engine asserting something rather than listing what
 * someone else published.
 */
export function Summary({ token, note }: { token: string; note: string }) {
  const [data, setData] = useState<SummaryResponse | null>(null)

  useEffect(() => {
    const controller = new AbortController()
    summarise(token, controller.signal)
      .then(setData)
      // Including aborts. A missing summary is a normal outcome, not an error to show.
      .catch(() => {})
    return () => controller.abort()
  }, [token])

  if (!data?.summary) return null

  return (
    <section
      className="qalam rise mb-6 py-1"
      aria-label="Summary"
      // Announced once when it arrives, so a screen-reader user is not left unaware that content
      // appeared above where they are reading.
      aria-live="polite"
    >
      <p dir="auto" className="m-0 text-base" style={{ lineHeight: 1.75 }}>
        {renderCitations(data)}
      </p>
      <p className="mt-2 text-xs" dir="auto" style={{ color: 'var(--ink-muted)' }}>
        {note}
      </p>
    </section>
  )
}

/**
 * Turn `[1]` markers into links to the result they cite.
 *
 * The model's text is untrusted output derived from untrusted crawled pages, so it is rendered as
 * React children — escaped by construction — and only the markers we recognise become markup.
 * A marker with no matching citation is dropped rather than rendered as a link to nothing.
 */
function renderCitations(data: SummaryResponse) {
  const text = data.summary ?? ''
  const byNumber = new Map((data.citations ?? []).map((c) => [c.n, c.result_id]))

  return text.split(/(\[\d+\])/g).map((part, i) => {
    const match = /^\[(\d+)\]$/.exec(part)
    if (!match) return part
    const id = byNumber.get(Number(match[1]))
    if (!id) return null
    return (
      <a
        key={i}
        href={`#result-${id}`}
        aria-label={`Source ${match[1]}`}
        className="align-super text-xs no-underline hover:underline"
        style={{ color: 'var(--accent)' }}
      >
        {part}
      </a>
    )
  })
}
