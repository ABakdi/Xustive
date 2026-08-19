'use client'

import { useCallback, useEffect, useState } from 'react'

import { addSource, getSources, removeSource, type Seed } from '@/lib/admin'
import { PageHead, Table, Td, Th } from '@/components/admin/ui'

export default function SourcesPage() {
  const [seeds, setSeeds] = useState<Seed[]>([])
  const [url, setUrl] = useState('')
  const [trust, setTrust] = useState('B')
  const [msg, setMsg] = useState('')

  const load = useCallback(() => {
    getSources()
      .then(setSeeds)
      .catch((e) => setMsg((e as Error).message))
  }, [])
  useEffect(() => load(), [load])

  return (
    <>
      <PageHead title="Sources">
        The seed list. Adding one queues it <strong>at the front</strong>, so it is crawled next
        rather than behind everything already known.
      </PageHead>

      <form
        className="mb-3 flex flex-wrap items-center gap-2"
        onSubmit={async (e) => {
          e.preventDefault()
          setMsg('adding…')
          try {
            const r = await addSource(url.trim(), trust)
            setMsg(r.already_listed ? 'already listed — queued to crawl next' : `added as ${r.source_id} and queued to crawl next`)
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
          style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)', minInlineSize: '260px' }}
        />
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

      <Table
        head={
          <>
            <Th>source</Th>
            <Th>trust</Th>
            <Th>url</Th>
            <Th>note</Th>
            <Th />
          </>
        }
      >
        {seeds.map((s) => (
          <tr key={s.url}>
            <Td>{s.source_id}</Td>
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
                    await removeSource(s.url)
                    setMsg('removed — already-crawled documents remain')
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
      <p className="mt-4 text-sm" style={{ color: 'var(--fg-muted)' }}>
        Removing a source stops it being crawled. Documents already collected from it stay in the
        index — those are separate actions.
      </p>
    </>
  )
}
