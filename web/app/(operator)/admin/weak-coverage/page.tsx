'use client'

import { useState } from 'react'

import { forgetWeakTerm, getWeakCoverage } from '@/lib/admin'
import { PageHead, StatusLine, Table, Td, Th, usePoll } from '@/components/admin/ui'

export default function WeakCoveragePage() {
  const { data, error } = usePoll(getWeakCoverage, 15_000)
  const [dismissed, setDismissed] = useState<Set<string>>(new Set())
  const k = data?.k_anonymity ?? 20
  const terms = data?.terms ?? []
  const entities = data?.entities ?? []
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
              <Th>{''}</Th>
            </>
          }
        >
          {terms.filter((t) => !dismissed.has(t.term)).map((t) => (
            <tr key={t.term}>
              {/* The term is a user's search text — React escapes it by rendering as a text node. */}
              <Td>{t.term}</Td>
              <Td num>{t.count}</Td>
              <Td>
                {/* Dismissal, not suppression (PROB-003): a real gap re-accumulates on its own. */}
                <button
                  type="button"
                  onClick={async () => {
                    try {
                      await forgetWeakTerm(t.term)
                      setDismissed((d) => new Set(d).add(t.term))
                    } catch {
                      // The next poll shows the truth either way.
                    }
                  }}
                  className="rounded border px-2 py-0.5 text-xs"
                  style={{ borderColor: 'var(--line)', color: 'var(--fg-muted)' }}
                >
                  forget
                </button>
              </Td>
            </tr>
          ))}
        </Table>
      ) : null}

      {data?.enabled && entities.length > 0 ? (
        <div className="mt-8">
          <h2 className="mb-2 text-lg font-semibold">Entities people asked for</h2>
          <StatusLine>
            {`${entities.length} name(s) searched ≥ ${k} times that the knowledge store does not hold. These want a harvest, not a crawl source — add the QID to data/knowledge/seeds.tsv.`}
          </StatusLine>
          <Table
            head={
              <>
                <Th>name</Th>
                <Th num>searches</Th>
              </>
            }
          >
            {entities.map((t) => (
              <tr key={t.term}>
                {/* User search text — React escapes it by rendering as a text node. */}
                <Td>{t.term}</Td>
                <Td num>{t.count}</Td>
              </tr>
            ))}
          </Table>
        </div>
      ) : null}
    </>
  )
}