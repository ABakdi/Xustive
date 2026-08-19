'use client'

import { useCallback, useEffect, useState } from 'react'

import { getDocuments, type DocumentsPage } from '@/lib/admin'
import { PageHead, StatusLine, Table, Td, Th } from '@/components/admin/ui'

export default function DocumentsPage() {
  const [q, setQ] = useState('')
  const [host, setHost] = useState('')
  const [lang, setLang] = useState('')
  // The *applied* filters, updated only on submit — so typing does not refetch per keystroke.
  const [applied, setApplied] = useState({ q: '', host: '', lang: '' })
  const [page, setPage] = useState(1)
  const [data, setData] = useState<DocumentsPage | null>(null)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(() => {
    const controller = new AbortController()
    getDocuments({ ...applied, page }, controller.signal)
      .then((d) => {
        setData(d)
        setError(null)
      })
      .catch((e) => {
        if ((e as Error).name !== 'AbortError') setError((e as Error).message)
      })
    return () => controller.abort()
  }, [applied, page])

  useEffect(() => load(), [load])

  const perPage = data?.per_page ?? 20
  const total = data?.estimated_total ?? 0
  const totalPages = Math.max(1, Math.min(100, Math.ceil(total / perPage)))

  return (
    <>
      <PageHead title="Documents">
        Everything indexed, newest first. This is the section that answers whether the crawler is
        collecting the <em>right</em> things — a count rises the same whether we are finding news or
        four hundred copies of one calendar page.
      </PageHead>

      <form
        className="mb-3 flex flex-wrap items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          setPage(1)
          setApplied({ q, host, lang })
        }}
      >
        <input
          type="search"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="search title, url, body"
          className="min-h-10 flex-1 rounded border px-3 py-2"
          style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)', minInlineSize: '260px' }}
        />
        <input
          type="text"
          value={host}
          onChange={(e) => setHost(e.target.value)}
          placeholder="domain, e.g. aps.dz"
          className="min-h-10 rounded border px-3 py-2"
          style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
        />
        <select
          value={lang}
          onChange={(e) => setLang(e.target.value)}
          className="min-h-10 rounded border px-3 py-2"
          style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
        >
          <option value="">any language</option>
          <option value="ar">العربية</option>
          <option value="ary">الدارجة</option>
          <option value="fr">Français</option>
          <option value="en">English</option>
        </select>
        <button
          type="submit"
          className="min-h-10 rounded border px-4"
          style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}
        >
          Filter
        </button>
      </form>

      <StatusLine>
        {error
          ? `Could not reach the index: ${error}`
          : !data
            ? 'Loading…'
            : `${total} document${total === 1 ? '' : 's'}${
                total > perPage
                  ? ` — showing ${(page - 1) * perPage + 1}–${Math.min(page * perPage, total)}`
                  : ''
              }`}
      </StatusLine>

      <Table
        head={
          <>
            <Th>title</Th>
            <Th>domain</Th>
            <Th>language</Th>
            <Th num>length</Th>
            <Th>published</Th>
          </>
        }
      >
        {(data?.hits ?? []).map((h) => (
          <tr key={h.id}>
            <Td title={h.title}>
              <a href={h.url} rel="noopener nofollow" style={{ color: 'var(--accent)' }}>
                {h.title || h.url}
              </a>
            </Td>
            <Td>{h.domain || ''}</Td>
            <Td>{h.language || ''}</Td>
            <Td num>{h.body_len ?? (h.excerpt ? h.excerpt.length : '')}</Td>
            <Td>
              {h.published_at
                ? new Date(h.published_at * 1000).toISOString().slice(0, 16).replace('T', ' ')
                : ''}
            </Td>
          </tr>
        ))}
      </Table>

      {totalPages > 1 ? (
        <div className="mb-8 mt-3 flex items-center gap-4">
          <button
            type="button"
            disabled={page <= 1}
            onClick={() => {
              setPage((p) => Math.max(1, p - 1))
              window.scrollTo(0, 0)
            }}
            className="min-h-9 rounded border px-4 disabled:opacity-40"
            style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}
          >
            ← Prev
          </button>
          <span className="text-sm tabular-nums" style={{ color: 'var(--fg-muted)' }}>
            Page {page} of {totalPages}
          </span>
          <button
            type="button"
            disabled={page >= totalPages}
            onClick={() => {
              setPage((p) => Math.min(totalPages, p + 1))
              window.scrollTo(0, 0)
            }}
            className="min-h-9 rounded border px-4 disabled:opacity-40"
            style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}
          >
            Next →
          </button>
        </div>
      ) : null}
    </>
  )
}
