'use client'

import { useCallback, useEffect, useMemo, useState } from 'react'

import { addSource, CATEGORIES, getSources, removeSource, type Seed } from '@/lib/admin'
import { PageHead, Table, Td, Th } from '@/components/admin/ui'

/** Pretty label for a category slug. */
function label(cat: string): string {
  if (!cat) return 'Other'
  if (cat === 'science-tech') return 'Science & Tech'
  return cat.charAt(0).toUpperCase() + cat.slice(1)
}

/** A small dz / global region pill. */
function RegionPill({ region }: { region: string }) {
  const dz = region === 'dz'
  return (
    <span
      className="rounded px-1.5 py-0.5 text-[0.6875rem] font-medium"
      style={{
        color: dz ? 'var(--accent)' : 'var(--fg-muted)',
        border: `1px solid ${dz ? 'var(--accent)' : 'var(--line)'}`,
      }}
    >
      {dz ? 'DZ' : 'global'}
    </span>
  )
}

export default function SourcesPage() {
  const [seeds, setSeeds] = useState<Seed[]>([])
  const [url, setUrl] = useState('')
  const [trust, setTrust] = useState('B')
  const [category, setCategory] = useState('news')
  const [filter, setFilter] = useState('') // '' = all categories
  const [needle, setNeedle] = useState('')
  const [msg, setMsg] = useState('')

  const load = useCallback(() => {
    getSources()
      .then(setSeeds)
      .catch((e) => setMsg((e as Error).message))
  }, [])
  useEffect(() => load(), [load])

  // Group seeds by category, in CATEGORIES order, with anything unknown under "other" (empty slug).
  const groups = useMemo(() => {
    const visible = seeds.filter((x) => !needle || `${x.url} ${(x as { note?: string }).note ?? ''} ${(x as { category?: string }).category ?? ''}`.toLowerCase().includes(needle.toLowerCase()))
    const order = [...CATEGORIES, ''] as string[]
    const by = new Map<string, Seed[]>()
    for (const s of visible) {
      const key = order.includes(s.category) ? s.category : ''
      const list = by.get(key) ?? []
      list.push(s)
      by.set(key, list)
    }
    // dz first within a category, then by source id.
    for (const list of by.values()) {
      list.sort((a, b) =>
        a.region === b.region ? a.source_id.localeCompare(b.source_id) : a.region === 'dz' ? -1 : 1,
      )
    }
    return order.filter((c) => by.has(c)).map((c) => [c, by.get(c)!] as const)
  }, [seeds, needle])

  const shown = filter ? groups.filter(([c]) => c === filter) : groups

  return (
    <>
      <PageHead title="Sources">
        The crawl catalog, grouped by category. Algerian and global sources sit side by side; adding
        one queues it <strong>at the front</strong>, so it is crawled next rather than behind
        everything already known.
      </PageHead>

      <form
        className="mb-3 flex flex-wrap items-center gap-2"
        onSubmit={async (e) => {
          e.preventDefault()
          setMsg('adding…')
          try {
            const r = await addSource(url.trim(), trust, category)
            setMsg(
              r.already_listed
                ? 'already listed — queued to crawl next'
                : r.queued === false
                  ? `added as ${r.source_id} — but the frontier was unreachable, so it is NOT queued yet (it will seed on the next crawld start)`
                  : `added as ${r.source_id} and queued to crawl next`,
            )
            setUrl('')
            load()
          } catch (err) {
            setMsg((err as Error).message)
          }
        }}
      >
        <input
          type="url"
          required
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="https://example.dz/"
          className="min-h-10 flex-1 rounded border px-3 py-2"
          style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)', minInlineSize: '240px' }}
        />
        <select
          value={category}
          onChange={(e) => setCategory(e.target.value)}
          className="min-h-10 rounded border px-3 py-2"
          style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
        >
          {CATEGORIES.map((c) => (
            <option key={c} value={c}>
              {label(c)}
            </option>
          ))}
        </select>
        <select
          value={trust}
          onChange={(e) => setTrust(e.target.value)}
          className="min-h-10 rounded border px-3 py-2"
          style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
        >
          <option value="A">A — established, accountable</option>
          <option value="B">B — credible, narrower</option>
          <option value="C">C — user-generated</option>
        </select>
        <button type="submit" className="min-h-10 rounded border px-4" style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}>
          Add and crawl next
        </button>
      </form>
      <p className="mb-4 text-sm" style={{ color: 'var(--fg-muted)' }}>
        {msg}
      </p>

      {/* Category filter chips + totals. */}
      <div className="mb-5 flex flex-wrap items-center gap-1.5 text-sm">
        <input value={needle} onChange={(e) => setNeedle(e.target.value)} placeholder="url, note or category…" className="w-full rounded border px-2 py-1 text-sm sm:w-auto sm:w-full sm:w-auto sm:min-w-[220px]" style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)'}} aria-label="Filter sources" />
        <button
          type="button"
          onClick={() => setFilter('')}
          className="rounded border px-2.5 py-1"
          style={{
            borderColor: filter === '' ? 'var(--accent)' : 'var(--line)',
            color: filter === '' ? 'var(--fg)' : 'var(--fg-muted)',
          }}
        >
          All ({seeds.length})
        </button>
        {groups.map(([c, list]) => (
          <button
            key={c || 'other'}
            type="button"
            onClick={() => setFilter(c)}
            className="rounded border px-2.5 py-1"
            style={{
              borderColor: filter === c ? 'var(--accent)' : 'var(--line)',
              color: filter === c ? 'var(--fg)' : 'var(--fg-muted)',
            }}
          >
            {label(c)} ({list.length})
          </button>
        ))}
      </div>

      {shown.map(([cat, list]) => {
        const dz = list.filter((s) => s.region === 'dz').length
        return (
          <section key={cat || 'other'} className="mb-8">
            <h2 className="mb-1 text-lg font-semibold tracking-tight">{label(cat)}</h2>
            <p className="mb-2 text-sm" style={{ color: 'var(--fg-muted)' }}>
              {list.length} sources · {dz} Algerian · {list.length - dz} global
            </p>
            <Table
              head={
                <>
                  <Th>source</Th>
                  <Th>region</Th>
                  <Th>trust</Th>
                  <Th>url</Th>
                  <Th>note</Th>
                  <Th />
                </>
              }
            >
              {list.map((s) => (
                <tr key={s.url}>
                  <Td>{s.source_id}</Td>
                  <Td>
                    <RegionPill region={s.region} />
                  </Td>
                  <Td>{s.trust}</Td>
                  <Td title={s.url}>
                    <a href={s.url} rel="noopener nofollow" style={{ color: 'var(--accent)' }}>
                      {s.url}
                    </a>
                  </Td>
                  <Td>{s.note}</Td>
                  <Td>
                    <button
                      type="button"
                      className="rounded border px-2 py-1 text-xs"
                      style={{ borderColor: 'var(--line)', color: 'var(--fg-muted)' }}
                      onClick={async () => {
                        if (!confirm(`Stop crawling ${s.url}?\nDocuments already collected stay in the index.`)) return
                        try {
                          const rr = await removeSource(s.url)
                          setMsg(
                            `removed ${rr.removed ?? 1} entr${(rr.removed ?? 1) === 1 ? 'y' : 'ies'} — already-crawled documents remain`,
                          )
                          load()
                        } catch (err) {
                          setMsg((err as Error).message)
                        }
                      }}
                    >
                      remove
                    </button>
                  </Td>
                </tr>
              ))}
            </Table>
          </section>
        )
      })}

      <p className="mt-4 text-sm" style={{ color: 'var(--fg-muted)' }}>
        Removing a source stops it being crawled. Documents already collected from it stay in the
        index — those are separate actions.
      </p>
    </>
  )
}
