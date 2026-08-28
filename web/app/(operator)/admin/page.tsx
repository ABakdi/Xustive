'use client'

import Link from 'next/link'
import { useEffect, useMemo, useState } from 'react'

import {
  getStatus,
  getCompute,
  getMedia,
  getInteraction,
  getIntegrations,
  getQueue,
  getDocuments,
  getTimeseries,
  setCrawlPaused,
  setIntegration,
  type MediaStatus,
  type InteractionStatus,
  type Timeseries,
} from '@/lib/admin'
import { Action, PageHead, Section, Status, usePoll } from '@/components/admin/ui'
import { LineChart, StatTile, compact } from '@/components/admin/charts'

/**
 * The overview (M12-T01.3): is anything wrong, and since when.
 *
 * A hero number, tiles with the last hour behind them, three lines over the window the
 * operator picks, a status strip where every state is a shape and a colour, the two actions
 * that are wanted from here, and a link to every page. Every number that could be unknown says
 * so rather than showing zero: a zero and an unreachable dependency look identical.
 */
const PAGES: [string, string, string][] = [
  ['Live', '/admin/live', 'the crawler as it runs'],
  ['Documents', '/admin/documents', 'what has been collected'],
  ['Sources', '/admin/sources', 'the seed list'],
  ['Source health', '/admin/sources/health', 'per-source quality and policy'],
  ['Discovery yield', '/admin/discovery', 'per-channel funnel'],
  ['Weak coverage', '/admin/weak-coverage', 'gaps to fill'],
  ['Evaluation', '/admin/evaluation', 'the golden set over time'],
  ['Integrations', '/admin/integrations', 'federation, external models'],
  ['Searches & hits', '/admin/searches', 'what people searched and opened'],
  ['Anonymous signals', '/admin/interaction', 'k-anonymous use counters'],
  ['Media & voice', '/admin/media', 'OCR, image search, STT'],
  ['Compute', '/admin/compute', 'device, ranking, logging'],
  ['Configuration', '/admin/config', 'the effective config'],
  ['Index queue', '/admin/queue', 'backlog & dead letters'],
  ['Maintenance', '/admin/maintenance', 'takedowns & destructive actions'],
]

const WINDOWS = [1, 3, 6, 12, 24]

/** Sum a field per bucket so a 6 h window is ~48 points, not 720. */
function bucket(ts: Timeseries | null, field: keyof Timeseries['points'][number], mode: 'sum' | 'last' | 'max' = 'sum') {
  const pts = ts?.points ?? []
  if (pts.length === 0) return { labels: [] as string[], values: [] as (number | null)[] }
  const per = Math.max(1, Math.ceil(pts.length / 48))
  const labels: string[] = []
  const values: (number | null)[] = []
  for (let i = 0; i < pts.length; i += per) {
    const slice = pts.slice(i, i + per)
    const raw = slice.map((p) => p[field] as number | null).filter((v): v is number => v !== null)
    const d = new Date(slice[0]!.at * 1000)
    labels.push(`${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`)
    if (raw.length === 0) values.push(null)
    else if (mode === 'sum') values.push(raw.reduce((a, b) => a + b, 0))
    else if (mode === 'max') values.push(Math.max(...raw))
    else values.push(raw[raw.length - 1]!)
  }
  return { labels, values }
}

export default function OverviewPage() {
  const { data: s, error } = usePoll(getStatus, 5_000)
  const [hours, setHours] = useState(6)
  const { data: ts } = usePoll(getTimeseries(hours), 30_000)

  const [compute, setCompute] = useState<Record<string, unknown> | null>(null)
  const [media, setMedia] = useState<MediaStatus | null>(null)
  const [interaction, setInteraction] = useState<InteractionStatus | null>(null)
  const [integrations, setIntegrations] = useState<Awaited<ReturnType<typeof getIntegrations>> | null>(null)
  const [corpus, setCorpus] = useState<number | null>(null)
  const [queue, setQueue] = useState<Awaited<ReturnType<typeof getQueue>> | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const refresh = () => {
    getCompute().then(setCompute).catch(() => {})
    getMedia().then(setMedia).catch(() => {})
    getInteraction().then(setInteraction).catch(() => {})
    getIntegrations().then(setIntegrations).catch(() => {})
    getQueue().then(setQueue).catch(() => {})
    getDocuments({}).then((d) => setCorpus(d.estimated_total)).catch(() => {})
  }
  useEffect(() => {
    refresh()
    const id = setInterval(refresh, 30_000)
    return () => clearInterval(id)
  }, [])

  const device = (compute?.device ?? {}) as { active?: string; fell_back?: boolean }
  const modelRows = (compute?.models ?? []) as { spec: { role: string }; present: boolean }[]
  const summariesOn = modelRows.some((m) => m.spec.role === 'summariser' && m.present)
  const vec = media?.vector
  const stt = media?.stt

  // The last hour, for the tiles: totals and a 12-point trend.
  const lastHour = useMemo(() => {
    const pts = (ts?.points ?? []).slice(-120)
    const sum = (f: keyof Timeseries['points'][number]) => pts.reduce((a, p) => a + ((p[f] as number) ?? 0), 0)
    const trend = (f: keyof Timeseries['points'][number], mode: 'sum' | 'last' = 'sum') => {
      const per = Math.max(1, Math.ceil(pts.length / 12))
      const out: (number | null)[] = []
      for (let i = 0; i < pts.length; i += per) {
        const sl = pts.slice(i, i + per)
        const raw = sl.map((p) => p[f] as number | null).filter((v): v is number => v !== null)
        out.push(raw.length ? (mode === 'sum' ? raw.reduce((a, b) => a + b, 0) : raw[raw.length - 1]!) : null)
      }
      return out
    }
    const p95s = pts.map((p) => p.search_p95_ms).filter((v): v is number => v !== null)
    return {
      minutes: (pts.length * (ts?.step_seconds ?? 30)) / 60,
      searches: sum('searches'),
      zero: sum('zero_results'),
      p95: p95s.length ? Math.max(...p95s) : null,
      fetched: sum('fetched'),
      indexed: sum('indexed'),
      events: sum('events_written'),
      dropped: sum('events_dropped'),
      t: { searches: trend('searches'), p95: trend('search_p95_ms', 'last'), fetched: trend('fetched'), waiting: trend('frontier_waiting', 'last'), events: trend('events_written') },
    }
  }, [ts])

  const searches = bucket(ts, 'searches')
  const zero = bucket(ts, 'zero_results')
  const p95 = bucket(ts, 'search_p95_ms', 'max')
  const sumP95 = bucket(ts, 'summary_p95_ms', 'max')
  const fetched = bucket(ts, 'fetched')
  const indexed = bucket(ts, 'indexed')
  const waiting = bucket(ts, 'frontier_waiting', 'last')
  const empty = (ts?.points.length ?? 0) < 2

  const paused = s?.paused === true
  const fedOn = integrations?.federation?.enabled === true
  const zeroRate = lastHour.searches ? lastHour.zero / lastHour.searches : 0

  return (
    <>
      <PageHead title="Overview">
        Is anything wrong, and since when. The tiles carry the last hour; the lines the window you
        pick; the strip says what is on. Every number that could be unknown says so rather than
        showing zero.
      </PageHead>

      {error ? (
        <p className="mb-4 text-sm" style={{ color: 'var(--viz-critical)' }}>
          Could not reach the API: {error}
        </p>
      ) : null}

      {/* The hero: the one number the product is. */}
      <div className="mb-6 flex flex-wrap items-end gap-6">
        <div>
          <div className="text-xs" style={{ color: 'var(--fg-faint)' }}>
            documents in the index
          </div>
          <div className="text-5xl font-semibold leading-none" style={{ fontVariantNumeric: 'normal' }}>
            {corpus == null ? '…' : corpus.toLocaleString('en')}
          </div>
        </div>
        <div className="ms-auto flex flex-wrap gap-2">
          <Action busy={busy === 'pause'} onClick={async () => { setBusy('pause'); try { await setCrawlPaused(!paused) } finally { setBusy(null) } }}>
            {paused ? 'Resume crawler' : 'Pause crawler'}
          </Action>
          <Action busy={busy === 'fed'} disabled={integrations?.federation?.configured === false} onClick={async () => { setBusy('fed'); try { await setIntegration('federation', !fedOn); refresh() } finally { setBusy(null) } }}>
            {fedOn ? 'Federation off' : 'Federation on'}
          </Action>
        </div>
      </div>

      <div className="mb-6 grid gap-3" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(170px, 1fr))' }}>
        <StatTile label="Searches, last hour" value={lastHour.searches} trend={lastHour.t.searches} />
        <StatTile label="Got nothing" value={lastHour.searches ? `${Math.round(zeroRate * 100)} %` : '—'} status={zeroRate > 0.3 ? 'warning' : undefined} />
        <StatTile label="Search p95, worst 30 s" value={lastHour.p95 === null ? '—' : compact(lastHour.p95)} unit="ms" status={lastHour.p95 !== null && lastHour.p95 > 1500 ? 'warning' : undefined} trend={lastHour.t.p95} upIsGood={false} />
        <StatTile label="Crawled, last hour" value={lastHour.fetched} trend={lastHour.t.fetched} />
        <StatTile label="Frontier waiting" value={s?.waiting ?? '…'} trend={lastHour.t.waiting} status={queue?.capacity?.redis_pct != null && queue.capacity.redis_pct >= 80 ? 'critical' : undefined} />
        <StatTile label="Events kept, last hour" value={lastHour.events} trend={lastHour.t.events} status={lastHour.dropped > 0 ? 'warning' : undefined} />
      </div>

      <div className="mb-3 flex flex-wrap items-center gap-2 text-sm" style={{ color: 'var(--fg-muted)' }}>
        <span>Window:</span>
        {WINDOWS.map((h) => (
          <button key={h} type="button" className={`chip ${h === hours ? 'chip-active' : ''} cursor-pointer`} onClick={() => setHours(h)}>
            {h} h
          </button>
        ))}
        {empty && <span className="ms-2" style={{ color: 'var(--fg-faint)' }}>The vitals ring fills from the API's start; the first points arrive within a minute.</span>}
      </div>
      <div className="mb-8 grid gap-3" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(300px, 1fr))' }}>
        <LineChart title="Searches" labels={searches.labels} series={[{ name: 'searches', values: searches.values }, { name: 'got nothing', values: zero.values }]} />
        <LineChart title="Latency, p95 (ms)" labels={p95.labels} series={[{ name: 'search', values: p95.values }, { name: 'summary', values: sumP95.values }]} unit="ms" />
        <LineChart title="Crawl" labels={fetched.labels} series={[{ name: 'fetched', values: fetched.values }, { name: 'indexed', values: indexed.values }]} />
        <LineChart title="Frontier waiting" labels={waiting.labels} series={[{ name: 'waiting', values: waiting.values }]} area />
      </div>

      <Section title="Subsystems" hint="What is on, what is off, what needs attention — each a shape and a colour.">
        <div className="flex flex-wrap gap-2">
          <Status label="Crawler" state={s ? (s.unavailable ? 'off' : paused ? 'warn' : 'on') : 'off'} detail={s ? (s.unavailable ? 'unknown' : paused ? 'paused' : s.state) : '…'} />
          <Status label="Compute" state={device.active === 'gpu' ? 'on' : device.fell_back ? 'warn' : 'on'} detail={device.active ? `on ${device.active}${device.fell_back ? ' (GPU unused)' : ''}` : '…'} />
          <Status label="Summaries" state={summariesOn ? 'on' : 'off'} detail={summariesOn ? 'AI summaries' : 'off'} />
          <Status label="Image AI" state={media ? (media.ocr.healthy ? 'on' : 'warn') : 'off'} detail={media ? `OCR: ${media.ocr.backend}` : '…'} />
          <Status label="Image search" state={vec?.enabled ? (vec.qdrant_reachable ? 'on' : 'warn') : 'off'} detail={vec?.enabled ? `${vec.image_vectors?.toLocaleString() ?? '?'} vectors` : 'off'} />
          <Status label="Voice" state={stt?.enabled ? (stt.healthy ? 'on' : 'warn') : 'off'} detail={stt?.enabled ? (stt.healthy ? 'STT up' : 'STT down') : 'off'} />
          <Status label="Interaction" state={interaction?.enabled ? 'on' : 'off'} detail={interaction?.enabled ? `k=${interaction.k_anonymity}` : 'off'} />
          <Status label="Federation" state={fedOn ? (integrations?.federation?.reachable_from_api ? 'on' : 'warn') : 'off'} detail={fedOn ? (integrations?.federation?.reachable_from_api ? 'SearXNG live' : 'gateway down') : 'off'} />
          <Status label="Semantic" state={integrations?.semantic?.configured ? (integrations.semantic.reachable ? 'on' : 'warn') : 'off'} detail={integrations?.semantic?.configured ? (integrations.semantic.documents_embedded == null ? 'embedder down' : `${integrations.semantic.documents_embedded.toLocaleString()} vectors`) : 'off'} />
          <Status label="Capacity" state={queue?.capacity?.redis_pct != null ? (queue.capacity.redis_pct >= 85 ? 'critical' : queue.capacity.redis_pct >= 80 ? 'warn' : 'on') : 'off'} detail={queue?.capacity ? (queue.capacity.redis_pct != null ? `Redis ${queue.capacity.redis_pct}%` : 'Redis uncapped') : '…'} />
        </div>
      </Section>

      <Section title="Pages">
        <div className="grid gap-3" style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))' }}>
          {PAGES.map(([label, href, hint]) => (
            <Link key={href} href={href} className="block rounded border px-4 py-3 no-underline" style={{ borderColor: 'var(--line)', color: 'var(--fg)', background: 'var(--surface)' }}>
              <div className="font-medium">{label}</div>
              <div className="text-sm" style={{ color: 'var(--fg-muted)' }}>{hint}</div>
            </Link>
          ))}
        </div>
      </Section>
    </>
  )
}
