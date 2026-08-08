import type { ResultCard as Result } from '@/lib/api'
import type { Locale } from '@/lib/i18n/config'
import { formatDate } from '@/lib/i18n/format'
import type { Messages } from '@/lib/i18n/messages'

const GLYPH: Record<string, string> = { positive: '▲', neutral: '●', negative: '▼' }

/**
 * One result.
 *
 * A pure Server Component — no interactivity, so no JavaScript ships for the thing that makes up
 * most of the page. `dir="auto"` per card because a French interface showing an Arabic result is
 * the normal case, not an edge one.
 *
 * Never carries the assert rule. That mark means the engine is asserting something; a result is
 * what somebody else published.
 */
export function ResultCard({
  result,
  t,
  locale,
}: {
  result: Result
  t: Messages
  locale: string
}) {
  const dated = result.published_at_precision !== 'unknown' && result.published_at > 0
  const date = dated ? formatDate(locale as Locale, result.published_at) : null

  const sourceLabel = (t as Record<string, string>)[result.source_type] ?? result.source_type

  return (
    // `min-w-0` and `overflow-hidden` are load-bearing, not cosmetic. A grid child defaults to
    // `min-width: auto`, so one unbreakable string — and a percent-encoded Arabic slug is a
    // 200-character unbreakable string — widens the whole column. It took the document to 4487px
    // and pushed everything off-screen.
    <li
      id={`result-${result.id}`}
      dir="auto"
      className="min-w-0 overflow-hidden scroll-mt-24"
    >
      <div
        className="mb-1 flex flex-wrap items-center gap-2 text-xs"
        style={{ color: 'var(--fg-muted)' }}
      >
        <span
          className="rounded-[var(--radius-sm)] border px-2 py-0.5"
          style={{ borderColor: 'var(--line)' }}
        >
          {sourceLabel}
        </span>
        {/* Isolated: a URL inside an RTL line is reordered into nonsense without this. */}
        <bdi className="max-w-full truncate">{result.display_url}</bdi>
        {/* Isolated for the same reason as the URL above. A formatted date is digits joined by
            neutral separators, and neutrals take their direction from what surrounds them — so
            `08/08/2026` sitting between an Arabic source label and an Arabic sentiment word is
            reordered into `2026/08/08`. The card is `dir="auto"`, which resolves the card as a
            whole; it does not isolate the runs inside it. */}
        {date ? (
          <bdi>
            <time dateTime={String(result.published_at)}>{date}</time>
          </bdi>
        ) : (
          <span>{t.dateUnknown}</span>
        )}
        {result.sentiment && (
          <span className="inline-flex items-center gap-1">
            <span aria-hidden>{GLYPH[result.sentiment.label]}</span>
            {(t as Record<string, string>)[result.sentiment.label]}
          </span>
        )}
      </div>

      <h2 className="m-0 text-lg leading-snug" style={{ overflowWrap: 'anywhere' }}>
        {/* `<em>` from the engine marks matched terms and is the only markup rendered. Everything
            else was escaped by the API before it left the process. */}
        <a
          href={result.url}
          rel="noopener nofollow"
          className="no-underline hover:underline"
          style={{ color: 'var(--accent)' }}
          dangerouslySetInnerHTML={{ __html: result.title }}
        />
      </h2>

      <p
        className="mt-1 text-sm"
        style={{ color: 'var(--fg-muted)', overflowWrap: 'anywhere' }}
        dangerouslySetInnerHTML={{ __html: result.excerpt }}
      />
    </li>
  )
}
