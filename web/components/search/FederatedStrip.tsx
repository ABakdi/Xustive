import type { FederatedHit } from '@/lib/api'

/**
 * The "from the web" strip (M7-T05, ADR-0017).
 *
 * Results borrowed live from federation (self-hosted SearXNG via the Federation Gateway), shown in a
 * clearly-labelled section **below and separate from** the ranked index results — never interleaved,
 * because a federated hit has none of the relevance/trust/freshness signals the real results are
 * scored on. Each URL is also queued for crawl server-side, so it becomes a real indexed result on a
 * later search. Rendered only when federation returned something.
 */
export function FederatedStrip({ hits, label }: { hits: FederatedHit[]; label: string }) {
  if (!hits.length) return null
  return (
    <section className="mt-8" aria-label={label}>
      <h2
        className="mb-3 text-sm font-semibold tracking-wide"
        style={{ color: 'var(--fg-muted)' }}
      >
        {label}
      </h2>
      <ol
        className="list-none p-0"
        style={{ display: 'grid', gap: 'var(--result-gap)', gridTemplateColumns: 'minmax(0, 1fr)' }}
      >
        {hits.map((h) => {
          let display = h.url
          try {
            display = new URL(h.url).host.replace(/^www\./, '')
          } catch {
            // Keep the raw URL if it does not parse — better a long string than a dropped result.
          }
          return (
            <li key={h.url}>
              <a href={h.url} rel="nofollow noopener" className="block no-underline">
                <div className="flex items-center gap-2 text-xs" style={{ color: 'var(--fg-faint)' }}>
                  <span dir="ltr">{display}</span>
                  {h.engine ? (
                    <span
                      className="rounded px-1.5 py-0.5"
                      style={{ background: 'var(--bg-sunk)', color: 'var(--fg-muted)' }}
                    >
                      {h.engine}
                    </span>
                  ) : null}
                </div>
                <div className="text-[1.05rem]" style={{ color: 'var(--accent)' }}>
                  {h.title || display}
                </div>
                {h.snippet ? (
                  <p className="mt-0.5 text-sm" style={{ color: 'var(--fg-muted)' }}>
                    {h.snippet}
                  </p>
                ) : null}
              </a>
            </li>
          )
        })}
      </ol>
    </section>
  )
}
