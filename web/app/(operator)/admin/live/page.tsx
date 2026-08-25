'use client'

import { useEffect, useState } from 'react'

import type { Snapshot } from '@/lib/admin'
import { PageHead, Table, Td, Th } from '@/components/admin/ui'
import { ForceCrawl } from '@/components/admin/ForceCrawl'

function Tile({ n, label }: { n: number | string; label: string }) {
  return (
    <div className="flex flex-col gap-0.5 border px-4 py-3" style={{ borderColor: 'var(--line)', minInlineSize: '110px' }}>
      <span className="text-2xl font-medium tabular-nums">{n}</span>
      <span className="text-xs" style={{ color: 'var(--fg-muted)' }}>
        {label}
      </span>
    </div>
  )
}

export default function LivePage() {
  const [snap, setSnap] = useState<Snapshot | null>(null)
  const [down, setDown] = useState(false)

  useEffect(() => {
    // The one live connection carries every number on the page — absolute values, never deltas, so
    // a missed frame loses nothing.
    const es = new EventSource('/api/v1/admin/crawler/events')
    es.onmessage = (e) => {
      try {
        setSnap(JSON.parse(e.data))
        setDown(false)
      } catch {
        /* ignore a partial frame */
      }
    }
    es.onerror = () => setDown(true)
    return () => es.close()
  }, [])

  const s = snap
  const skips = s ? Object.entries(s.skipped).sort((a, b) => b[1] - a[1]) : []
  const hosts = s ? Object.entries(s.hosts).sort((a, b) => b[1] - a[1]).slice(0, 20) : []

  return (
    <>
      <PageHead title="Live">
        The crawler as it runs, one frame a second. Counters are cumulative — a number that stops
        moving is a crawler that stopped, not a dropped frame.
      </PageHead>

      <ForceCrawl />

      {down ? (
        <p className="mb-4 text-sm" style={{ color: 'var(--warn)' }}>
          The live stream disconnected — the API may be restarting. It reconnects on its own.
        </p>
      ) : null}
      {s?.unavailable ? (
        <p className="mb-4 text-sm" style={{ color: 'var(--warn)' }}>
          Cannot read the crawler counters (Redis unreachable). This is different from an idle crawler.
        </p>
      ) : null}

      <div className="mb-6 mt-1 flex flex-wrap gap-3">
        <Tile n={s?.state ?? '…'} label="state" />
        <Tile n={s?.fetched ?? 0} label="fetched" />
        <Tile n={s?.revisited ?? 0} label="revisited" />
        <Tile n={s?.parsed ?? 0} label="parsed" />
        <Tile n={s?.indexed ?? 0} label="indexed" />
        <Tile n={s?.discovered ?? 0} label="discovered" />
        <Tile n={s?.failed ?? 0} label="failed" />
        <Tile n={s?.waiting ?? 0} label="waiting" />
        <Tile n={s?.inflight ?? 0} label="in flight" />
        <Tile n={s?.deferred ?? 0} label="deferred (revisit)" />
      </div>

      <h2 className="mb-2 mt-8 text-lg font-semibold">Recent URLs</h2>
      <Table head={<><Th>at</Th><Th>outcome</Th><Th>host</Th><Th>url</Th><Th num>words</Th></>}>
        {(s?.recent ?? []).map((r, i) => (
          <tr key={`${r.url}-${i}`}>
            {/* host and at arrived in every frame and were dropped (PROB-003). */}
            <Td>{r.at ? new Date(r.at * 1000).toLocaleTimeString() : '—'}</Td>
            <Td>{r.outcome}</Td>
            <Td>{r.host}</Td>
            <Td title={r.url}>{r.url}</Td>
            <Td num>{r.words}</Td>
          </tr>
        ))}
      </Table>

      <div className="mt-8 grid gap-8" style={{ gridTemplateColumns: 'minmax(0,1fr) minmax(0,1fr)' }}>
        <div>
          <h2 className="mb-2 text-lg font-semibold">Skips</h2>
          <Table head={<><Th>reason</Th><Th num>count</Th></>}>
            {skips.map(([k, v]) => (
              <tr key={k}>
                <Td>{k}</Td>
                <Td num>{v}</Td>
              </tr>
            ))}
          </Table>
        </div>
        <div>
          <h2 className="mb-2 text-lg font-semibold">Busiest hosts</h2>
          <Table head={<><Th>host</Th><Th num>last fetch</Th></>}>
            {hosts.map(([h, at]) => (
              <tr key={h}>
                <Td>{h}</Td>
                <Td num>{new Date(at * 1000).toISOString().slice(11, 19)}</Td>
              </tr>
            ))}
          </Table>
        </div>
      </div>
    </>
  )
}
