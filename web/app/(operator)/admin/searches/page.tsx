'use client'

import { useState } from 'react'

import {
  forgetVisitor,
  getEventsOverview,
  getVisitorEvents,
  type EventDocRow,
  type EventRow,
  type EventsOverview,
} from '@/lib/admin'
import { PageHead, StatusLine, Table, Td, Th, usePoll } from '@/components/admin/ui'
import { CollectionSwitch } from '@/components/admin/Switches'

/**
 * Searches & hits — the first-party events console ([[ADR-0030]], M11-T04).
 *
 * Not a dashboard of totals. Five lists, each with the action it implies: searches that got
 * nothing (a coverage gap or a missing synonym), searches with results nobody opened (the
 * ranking put the wrong thing first), results readers reported (look at the page, or the
 * query it came up for), the most opened documents (what people actually want), and the raw
 * recent events for the cases that need a look. A visitor lookup with a Forget button makes
 * the right to be forgotten something an operator can do, not promise.
 */
export default function SearchesPage() {
  const [days, setDays] = useState(7)
  const { data, error } = usePoll<EventsOverview>(getEventsOverview(days), 30_000)
  const [visitor, setVisitor] = useState('')
  const [visitorEvents, setVisitorEvents] = useState<EventRow[] | null>(null)
  const [note, setNote] = useState('')

  async function lookup() {
    setNote('')
    try {
      const r = await getVisitorEvents(visitor.trim())
      setVisitorEvents(r.events)
    } catch (e) {
      setNote(String(e))
    }
  }
  async function forget() {
    setNote('')
    try {
      const r = await forgetVisitor(visitor.trim())
      setNote(`Forgot ${r.deleted} events.`)
      setVisitorEvents([])
    } catch (e) {
      setNote(String(e))
    }
  }

  const t = data?.totals
  return (
    <>
      <PageHead title="Searches & hits">
        What people searched, what they were shown, what they opened, and what they said was not
        relevant — kept per event with a first-party visitor id, never shared. Each list below
        names the fix it points at. Data is deleted after {data?.retention_days ?? '—'} days.
      </PageHead>

      {error ? <StatusLine>Could not reach the API: {error}</StatusLine> : null}
      <div className="mb-6">
        <CollectionSwitch />
      </div>
      {data && !data.enabled ? (
        <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>
          Collection is off. Read Legal and Compliance §5 before turning it on: on, the operator
          is a data controller.
        </p>
      ) : null}

      {data?.enabled && t ? (
        <>
          <p className="mb-4 flex flex-wrap items-center gap-3 text-sm" style={{ color: 'var(--fg-muted)' }}>
            <span>Window:</span>
            {[1, 7, 30, 90].map((d) => (
              <button
                key={d}
                type="button"
                className={`chip ${d === days ? 'chip-active' : ''} cursor-pointer`}
                onClick={() => setDays(d)}
              >
                {d} d
              </button>
            ))}
            <span className="ms-auto">
              {data.written} written · {data.dropped} dropped since start
            </span>
          </p>

          <div className="mb-6 grid gap-3" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))' }}>
            {[
              ['Searches', t.searches],
              ['Distinct queries', t.distinct_queries],
              ['Visitors', t.visitors],
              ['Opened a result', `${t.searches_with_a_click} (${t.searches ? Math.round((100 * t.searches_with_a_click) / t.searches) : 0} %)`],
              ['Got nothing', `${t.zero_result_searches} (${t.searches ? Math.round((100 * t.zero_result_searches) / t.searches) : 0} %)`],
              ['Reported', t.reports],
            ].map(([k, v]) => (
              <div key={String(k)} className="rounded border p-3" style={{ borderColor: 'var(--line)' }}>
                <div className="text-xs" style={{ color: 'var(--fg-faint)' }}>{k}</div>
                <div className="text-lg font-semibold numeric">{v}</div>
              </div>
            ))}
          </div>

          <Section title="Searched, got nothing" hint="A coverage gap, a missing synonym, or a spelling — add the synonym, crawl the source, or both.">
            <QueryTable rows={data.zero_results ?? []} cols={['count']} />
          </Section>
          <Section title="Searched, never opened" hint="Results came back and nobody opened one, more than once: the ranking put the wrong thing first. Run the query.">
            <QueryTable rows={data.unopened ?? []} cols={['count', 'results']} />
          </Section>
          <Section title="Reported as not relevant" hint="Readers said this result was wrong for these queries. Look at the page, then at the query.">
            <DocTable rows={data.reported ?? []} />
          </Section>
          <Section title="Most opened" hint="What people actually want. Keep these fresh.">
            <DocTable rows={data.most_opened ?? []} />
          </Section>
          <Section title="Top queries" hint="Volume, results, and clicks — the shape of demand.">
            <QueryTable rows={data.top_queries ?? []} cols={['count', 'results', 'clicks']} />
          </Section>

          <Section title="One visitor" hint="Every event of one visitor id (the xv cookie), and the button that honours a deletion request.">
            <p className="mb-2 flex flex-wrap gap-2 text-sm">
              <input
                value={visitor}
                onChange={(e) => setVisitor(e.target.value)}
                placeholder="visitor id (26 characters)"
                className="rounded border px-2 py-1 font-mono text-xs"
                style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)', minWidth: 300 }}
              />
              <button type="button" className="chip chip-active cursor-pointer" onClick={() => void lookup()} disabled={visitor.trim().length !== 26}>
                Look up
              </button>
              <button type="button" className="chip cursor-pointer" onClick={() => void forget()} disabled={visitor.trim().length !== 26} style={{ color: 'var(--warn)' }}>
                Forget this visitor
              </button>
              {note && <span style={{ color: 'var(--fg-muted)' }}>{note}</span>}
            </p>
            {visitorEvents && <EventTable rows={visitorEvents} />}
          </Section>

          <Section title="Recent events" hint="The last hundred, newest first.">
            <EventTable rows={data.recent ?? []} />
          </Section>
        </>
      ) : null}
    </>
  )
}

function Section({ title, hint, children }: { title: string; hint: string; children: React.ReactNode }) {
  return (
    <section className="mb-8">
      <h2 className="m-0 text-base font-semibold">{title}</h2>
      <p className="m-0 mb-2 text-xs" style={{ color: 'var(--fg-faint)' }}>{hint}</p>
      {children}
    </section>
  )
}

function QueryTable({ rows, cols }: { rows: Record<string, string | number>[]; cols: string[] }) {
  if (rows.length === 0) return <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>Nothing in this window.</p>
  return (
    <Table head={<><Th>Query</Th>{cols.map((c) => <Th key={c} num>{c}</Th>)}</>}>
      {rows.map((r) => (
        <tr key={String(r.query)}>
          <Td><span dir="auto">{r.query}</span></Td>
          {cols.map((c) => <Td key={c} num>{r[c]}</Td>)}
        </tr>
      ))}
    </Table>
  )
}

function DocTable({ rows }: { rows: EventDocRow[] }) {
  if (rows.length === 0) return <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>Nothing in this window.</p>
  return (
    <Table head={<><Th>Document</Th><Th>For the queries</Th><Th num>opens</Th><Th num>reports</Th></>}>
      {rows.map((r) => (
        <tr key={r.doc}>
          <Td>
            {r.url ? (
              <a href={r.url} target="_blank" rel="noopener noreferrer" dir="auto">{r.title || r.url}</a>
            ) : (
              <code className="text-xs">{r.doc}</code>
            )}
          </Td>
          <Td><span dir="auto">{r.queries.map((q) => `${q.query} (${q.count})`).join(' · ')}</span></Td>
          <Td num>{r.opens}</Td>
          <Td num>{r.reports}</Td>
        </tr>
      ))}
    </Table>
  )
}

function EventTable({ rows }: { rows: EventRow[] }) {
  if (rows.length === 0) return <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>No events.</p>
  return (
    <Table head={<><Th>When</Th><Th>Kind</Th><Th>Query</Th><Th>Detail</Th><Th>Visitor</Th></>}>
      {rows.map((e) => (
        <tr key={e.id}>
          <Td><time dateTime={new Date(e.at * 1000).toISOString()}>{new Date(e.at * 1000).toLocaleString()}</time></Td>
          <Td>{e.kind}</Td>
          <Td><span dir="auto">{e.query}</span></Td>
          <Td>
            {e.kind === 'search'
              ? `${e.total_hits ?? 0} results · ${e.vertical ?? 'all'} · p${e.page ?? 1} · ${e.ui ?? ''} · ${e.latency_ms ?? 0} ms`
              : `${e.doc ?? ''}${e.rank ? ` · rank ${e.rank}` : ''}${e.reason ? ` · ${e.reason}` : ''}`}
          </Td>
          <Td><code className="text-xs">{(e.visitor ?? '').slice(0, 8)}</code></Td>
        </tr>
      ))}
    </Table>
  )
}
