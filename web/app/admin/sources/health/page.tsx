'use client'

import { getSourceHealth } from '@/lib/admin'
import { PageHead, StatusLine, Table, Td, Th, pct, usePoll } from '@/components/admin/ui'

/** A quality cell: em-dash when unknown, amber when outside its §7 healthy band. */
function QualityCell({ v, ok }: { v: number | null; ok: (x: number) => boolean }) {
  if (v == null) return <Td num>—</Td>
  return (
    <Td num warn={!ok(v)}>
      {pct(v)}
    </Td>
  )
}

export default function SourceHealthPage() {
  const { data, error } = usePoll(getSourceHealth, 10_000)
  const rows = data ?? []
  return (
    <>
      <PageHead title="Source health">
        Per-source quality, joined from the registry and the live crawl counters. A cell reads
        &mdash; until the source has data. Amber marks a value outside its healthy band (§7) — the
        same signal the lifecycle automation degrades on.
      </PageHead>
      <StatusLine>
        {error
          ? `Could not load source health: ${error}`
          : data
            ? `${rows.length} source(s). Refreshing every 10s.`
            : 'Loading…'}
      </StatusLine>
      <Table
        head={
          <>
            <Th>source</Th>
            <Th>state</Th>
            <Th>tier</Th>
            <Th num>fetched</Th>
            <Th num>indexed</Th>
            <Th num>fetch ok</Th>
            <Th num>extraction</Th>
            <Th num>duplicate</Th>
            <Th num>spam</Th>
            <Th num>date?</Th>
          </>
        }
      >
        {rows.map((s) => (
          <tr key={s.id}>
            <Td>
              {s.display_name || s.id}{' '}
              <span style={{ color: 'var(--fg-faint)' }}>{s.id}</span>
            </Td>
            <Td>{s.lifecycle || '—'}</Td>
            <Td>{s.trust_tier || '—'}</Td>
            <Td num>{s.counts.fetched}</Td>
            <Td num>{s.counts.indexed}</Td>
            <QualityCell v={s.quality.fetch_success_rate} ok={(x) => x > 0.95} />
            <QualityCell v={s.quality.extraction_success_rate} ok={(x) => x > 0.9} />
            <QualityCell v={s.quality.duplicate_ratio} ok={(x) => x < 0.3} />
            <QualityCell v={s.quality.spam_mean} ok={(x) => x < 0.2} />
            <QualityCell v={s.quality.date_unknown_ratio} ok={(x) => x < 0.1} />
          </tr>
        ))}
      </Table>
    </>
  )
}
