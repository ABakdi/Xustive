'use client'

import { getWeakCoverage } from '@/lib/admin'
import { PageHead, StatusLine, Table, Td, Th, usePoll } from '@/components/admin/ui'

export default function WeakCoveragePage() {
  const { data, error } = usePoll(getWeakCoverage, 15_000)
  const k = data?.k_anonymity ?? 20
  const terms = data?.terms ?? []
  return (
    <>
      <PageHead title="Weak coverage">
        Searches the corpus could not answer — the precise gaps worth finding sources for.{' '}
        <strong>k-anonymous</strong>: a term appears only once at least {k} searches have hit it, so
        nothing here identifies a query or a person. When a resolution source is configured (SERP or
        Brave), the crawler chases these automatically.
      </PageHead>
      {/* Whether that "automatically" is actually true here (PROB-003): the page used to promise
          resolution with no way to see that no source was wired. */}
      {data?.resolution ? (
        <p className="mb-3 text-sm" style={{ color: 'var(--fg-muted)' }}>
          Resolution:{' '}
          {data.resolution.serp_enabled ? (
            <strong>direct SERP enabled</strong>
          ) : data.resolution.brave_usable ? (
            <strong>Brave enabled</strong>
          ) : (
            <strong style={{ color: 'var(--warn)' }}>
              no source configured — these terms are collected but nothing chases them
            </strong>
          )}
          .
        </p>
      ) : null}
      <StatusLine>
        {error
          ? `Could not load weak coverage: ${error}`
          : !data
            ? 'Loading…'
            : !data.enabled
              ? 'Query-driven discovery is off. Set discovery.weak_coverage_enabled to collect gaps.'
              : terms.length
                ? `${terms.length} coverage gap(s), each searched ≥ ${k} times.`
                : 'No coverage gaps yet — search for something the corpus does not have.'}
      </StatusLine>
      {data?.enabled && terms.length > 0 ? (
        <Table
          head={
            <>
              <Th>term</Th>
              <Th num>searches</Th>
            </>
          }
        >
          {terms.map((t) => (
            <tr key={t.term}>
              {/* The term is a user's search text — React escapes it by rendering as a text node. */}
              <Td>{t.term}</Td>
              <Td num>{t.count}</Td>
            </tr>
          ))}
        </Table>
      ) : null}
    </>
  )
}
