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
