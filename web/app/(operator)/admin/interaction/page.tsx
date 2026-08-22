'use client'

import { getInteraction, type InteractionStatus } from '@/lib/admin'
import { PageHead, usePoll } from '@/components/admin/ui'

/**
 * The interaction-signals console (M6-T07).
 *
 * Top queries, category volumes, and hot re-crawl targets — every figure k-anonymous by
 * construction: the store only ever returns what has cleared the anonymity floor, so nothing on
 * this page can describe a single person's searching. The same "what this means / why it's safe"
 * note the weak-coverage page carries applies here.
 */
export default function InteractionPage() {
  const { data, error } = usePoll<InteractionStatus>(getInteraction, 15_000)

  return (
    <>
      <PageHead title="Interaction signals">
        Anonymous, aggregate use — the queries people run and the results they open — that feeds
        ranking and re-crawl. Every number here has passed the k-anonymity floor, so it reflects what
        many people did, never one person. No query log, no per-person record. Off by default.
      </PageHead>

      {error ? (
        <p className="mb-4 text-sm" style={{ color: 'var(--warn)' }}>
          Could not reach the API: {error}
        </p>
      ) : null}

      {data && !data.enabled ? (
        <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>
          Interaction signals are disabled. Set <code>[interaction] enabled = true</code> (k ≥ 20
          outside dev) to collect anonymous, aggregate use.
        </p>
      ) : null}

      {data?.enabled ? (
        <>
          <p className="mb-6 text-sm" style={{ color: 'var(--fg-muted)' }}>
            k-anonymity floor <strong>{data.k_anonymity}</strong> · window{' '}
            <strong>{data.window_days} days</strong>
          </p>

          <section className="mb-8">
            <h2 className="mb-2 text-base font-semibold">Top queries</h2>
            {data.top_queries && data.top_queries.length > 0 ? (
              <table className="w-full max-w-xl border-collapse text-sm">
                <thead>
                  <tr style={{ color: 'var(--fg-muted)' }}>
                    <th className="border-b py-1 text-start" style={{ borderColor: 'var(--line)' }}>Query</th>
                    <th className="border-b py-1 text-start" style={{ borderColor: 'var(--line)' }}>Category</th>
                    <th className="border-b py-1 text-end" style={{ borderColor: 'var(--line)' }}>Count</th>
                  </tr>
                </thead>
                <tbody>
                  {data.top_queries.map((q) => (
                    <tr key={q.query}>
                      <td className="border-b py-1" style={{ borderColor: 'var(--line)' }} dir="auto">{q.query}</td>
                      <td className="border-b py-1" style={{ borderColor: 'var(--line)' }}>{q.category}</td>
                      <td className="border-b py-1 text-end tabular-nums" style={{ borderColor: 'var(--line)' }}>{q.count}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>
                Nothing has crossed the anonymity floor yet.
              </p>
            )}
          </section>

          <section className="mb-8">
            <h2 className="mb-2 text-base font-semibold">Category volume</h2>
            {data.categories && Object.keys(data.categories).length > 0 ? (
              <ul className="m-0 max-w-xs list-none p-0 text-sm">
                {Object.entries(data.categories).map(([cat, n]) => (
                  <li key={cat} className="flex justify-between border-b py-1" style={{ borderColor: 'var(--line)' }}>
                    <span>{cat}</span>
                    <span className="tabular-nums">{n}</span>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>—</p>
            )}
          </section>

          <section>
            <h2 className="mb-2 text-base font-semibold">Hot re-crawl targets</h2>
            <p className="mb-2 text-xs" style={{ color: 'var(--fg-faint)' }}>
              Documents opened often enough to be pulled forward in the revisit schedule.
            </p>
            {data.hot_docs && data.hot_docs.length > 0 ? (
              <ul className="m-0 list-none p-0 text-sm">
                {data.hot_docs.map((d) => (
                  <li key={d} className="border-b py-1 font-mono text-xs" style={{ borderColor: 'var(--line)' }}>{d}</li>
                ))}
              </ul>
            ) : (
              <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>None yet.</p>
            )}
          </section>
        </>
      ) : null}
    </>
  )
}
