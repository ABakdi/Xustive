'use client'

import { useEffect, useState } from 'react'

import { setCrawlPaused, type Snapshot } from '@/lib/admin'
import { PageHead, Table, Td, Th } from '@/components/admin/ui'
import { StatTile } from '@/components/admin/charts'
import { ForceCrawl } from '@/components/admin/ForceCrawl'

/** The kit's tile (M12-T01.4); the page adds a per-second rate behind the counters it can. */
function Tile({ n, label, trend }: { n: number | string; label: string; trend?: (number | null)[] }) {
  return <StatTile label={label} value={n} trend={trend} />
}

export default function LivePage() {
  const [pauseBusy, setPauseBusy] = useState(false)
  const [snap, setSnap] = useState<Snapshot | null>(null)
  const [down, setDown] = useState(false)
  // The last two minutes of frames, so the counters get a rate behind them (M12-T03.3): the
  // stream is absolute values once a second, and a difference per frame is a per-second rate.
  const [frames, setFrames] = useState<Snapshot[]>([])

  useEffect(() => {
    // The one live connection carries every number on the page — absolute values, never deltas, so
    // a missed frame loses nothing.
    const es = new EventSource('/api/v1/admin/crawler/events')
    es.onmessage = (e) => {
      try {
        const next = JSON.parse(e.data) as Snapshot
        setSnap(next)
        setFrames((f) => [...f.slice(-119), next])
        setDown(false)
      } catch {
        /* ignore a partial frame */
      }
    }
    es.onerror = () => setDown(true)
    return () => es.close()
  }, [])

  const s = snap
  // Per-second rates from consecutive frames, in ~10 s buckets: twelve points over two minutes.
  const rate = (field: 'fetched' | 'indexed' | 'discovered' | 'failed') => {
    if (frames.length < 2) return undefined
    const out: (number | null)[] = []
    for (let i = 0; i < frames.length - 1; i += 10) {
      const a = frames[i]!
      const b = frames[Math.min(i + 10, frames.length - 1)]!
      const span = Math.max(1, Math.min(i + 10, frames.length - 1) - i)
      out.push(Math.max(0, ((b[field] ?? 0) - (a[field] ?? 0)) / span))
    }
    return out
  }
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

      {/* The pause control (PROB-003): state was displayed everywhere and changeable nowhere.
          Pausing holds new claims only — in-flight fetches finish, the frontier is untouched. */}
      <div className="mb-4 flex items-center gap-3">
        {s?.paused ? (
          <p className="m-0 rounded border px-3 py-1.5 text-sm" style={{ borderColor: 'var(--warn)', color: 'var(--warn)' }}>
            Crawl paused by operator — workers hold new claims until resumed.
          </p>
        ) : null}
        <button
          type="button"
          disabled={pauseBusy}
          onClick={async () => {
            setPauseBusy(true)
            try {
              await setCrawlPaused(!s?.paused)
            } catch {
              // The next SSE frame reflects reality either way.
            } finally {
              setPauseBusy(false)
            }
          }}
          className="min-h-10 rounded border px-4 text-sm"
          style={{ borderColor: s?.paused ? 'var(--line)' : 'var(--warn)', color: 'var(--fg)' }}
        >
          {pauseBusy ? '…' : s?.paused ? 'Resume crawl' : 'Pause crawl'}
        </button>
      </div>

      <div className="mb-6 mt-1 flex flex-wrap gap-3">
        <Tile n={s?.paused ? 'paused' : (s?.state ?? '…')} label="state" />
        <Tile n={s?.fetched ?? 0} label="fetched" trend={rate('fetched')} />
        <Tile n={s?.revisited ?? 0} label="revisited" />
        <Tile n={s?.parsed ?? 0} label="parsed" />
        {/* Media enumerated apart from pages (M9): a page is one "parsed" however many pictures
            it carries, and the count of pictures is a different fact about the crawl. */}
        <Tile n={s?.images ?? 0} label="images found" />
        <Tile n={s?.videos ?? 0} label="videos found" />
        <Tile n={s?.indexed ?? 0} label="indexed" trend={rate('indexed')} />
        <Tile n={s?.discovered ?? 0} label="discovered" trend={rate('discovered')} />
        <Tile n={s?.failed ?? 0} label="failed" trend={rate('failed')} />
        <Tile n={s?.waiting ?? 0} label="waiting" />
        <Tile n={s?.inflight ?? 0} label="in flight" />
        <Tile n={s?.deferred ?? 0} label="deferred (revisit)" />
      </div>

      <h2 className="mb-2 mt-8 text-lg font-semibold">Recent URLs</h2>
      <Table head={<><Th>at</Th><Th>outcome</Th><Th>host</Th>
              <Th num>media</Th><Th>url</Th><Th num>words</Th></>}>
        {(s?.recent ?? []).map((r, i) => (
          <tr key={`${r.url}-${i}`}>
            {/* host and at arrived in every frame and were dropped (PROB-003). */}
            <Td>{r.at ? new Date(r.at * 1000).toLocaleTimeString() : '—'}</Td>
            <Td>{r.outcome}</Td>
            <Td><bdi>{r.host}</bdi></Td>
            <Td num>
              {r.images || r.videos
                ? `${r.images ?? 0} img · ${r.videos ?? 0} vid`
                : '—'}
            </Td>
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
