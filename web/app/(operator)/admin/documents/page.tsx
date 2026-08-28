'use client'

import { useCallback, useEffect, useState } from 'react'

import {
  getDocuments,
  SEARX_CHANNELS,
  DISCOVERED_CHANNELS,
  type DocumentsPage,
} from '@/lib/admin'
import { PageHead, StatusLine, Table, Td, Th } from '@/components/admin/ui'

// SearXNG-sourced first (accent-tinted), then the crawler's own discovery.
const ALL_CHANNELS = [...SEARX_CHANNELS, ...DISCOVERED_CHANNELS] as const
const FROM_SEARX = new Set<string>(SEARX_CHANNELS)

export default function DocumentsPage() {
  const [q, setQ] = useState('')
  const [host, setHost] = useState('')
  const [lang, setLang] = useState('')
  const [channel, setChannel] = useState('')
  // `image` / `video` / '' — the media drill-in (M9), applied immediately like the channel one.
  const [media, setMedia] = useState('')
  // The order of the listing (M12-T03.5): newest, or what readers opened or reported most.
  const [sort, setSort] = useState<'' | 'opens' | 'reports' | 'endorsement'>('')
  // The *applied* filters, updated only on submit — so typing does not refetch per keystroke.
  const [applied, setApplied] = useState({ q: '', host: '', lang: '', channel: '', media: '' })
  const [page, setPage] = useState(1)
  const [data, setData] = useState<DocumentsPage | null>(null)
  const [error, setError] = useState<string | null>(null)

  // Apply a channel filter immediately (from a composition chip), bypassing the submit button.
  const pickChannel = (c: string) => {
    setChannel(c)
    setPage(1)
    setApplied((a) => ({ ...a, channel: c }))
  }
  const pickMedia = (m: string) => {
    setMedia(m)
    setPage(1)
    setApplied((a) => ({ ...a, media: m }))
  }

  const load = useCallback(() => {
    const controller = new AbortController()
    getDocuments({ ...applied, page, sort }, controller.signal)
      .then((d) => {
        setData(d)
        setError(null)
      })
      .catch((e) => {
        if ((e as Error).name !== 'AbortError') setError((e as Error).message)
      })
    return () => controller.abort()
  }, [applied, page, sort])

  useEffect(() => load(), [load])

  const perPage = data?.per_page ?? 20
  const total = data?.estimated_total ?? 0
  const totalPages = Math.max(1, Math.min(100, Math.ceil(total / perPage)))

  // Index composition by provenance: what a user search pulled from SearXNG and we indexed, vs what
  // the crawler discovered on its own. Unknown/legacy channels count under the crawler total.
  const comp = data?.composition ?? {}
  const searxTotal = Object.entries(comp)
    .filter(([c]) => FROM_SEARX.has(c))
    .reduce((n, [, v]) => n + v, 0)
  const crawlerTotal = Object.entries(comp)
    .filter(([c]) => !FROM_SEARX.has(c))
    .reduce((n, [, v]) => n + v, 0)
  const orderedComp = ALL_CHANNELS.filter((c) => comp[c])

  return (
    <>
      <PageHead title="Documents">
        Everything indexed, newest first. This is the section that answers whether the crawler is
        collecting the <em>right</em> things — a count rises the same whether we are finding news or
        four hundred copies of one calendar page.
      </PageHead>

      {/* Index composition by provenance (M7): what came from SearXNG (a user's searches) vs what
          the crawler discovered on its own, with a per-channel chip you can click to drill in. */}
      <div
        className="mb-4 rounded border px-3 py-3 text-sm"
        style={{ borderColor: 'var(--line)', background: 'var(--bg-sunk)' }}
      >
        <div className="mb-2 flex flex-wrap gap-x-6 gap-y-1">
          <span>
            <strong className="tabular-nums" style={{ color: 'var(--accent)' }}>
              {searxTotal.toLocaleString()}
            </strong>{' '}
            indexed from <strong>SearXNG</strong> (your searches)
          </span>
          <span>
            <strong className="tabular-nums">{crawlerTotal.toLocaleString()}</strong> discovered by
            the <strong>crawler</strong>
          </span>
        </div>
        <div className="flex flex-wrap gap-1.5">
          <button
            type="button"
            onClick={() => pickChannel('')}
            className="rounded border px-2 py-0.5 text-xs"
            style={{
              borderColor: applied.channel === '' ? 'var(--accent)' : 'var(--line)',
              color: 'var(--fg-muted)',
            }}
          >
            all
          </button>
          {orderedComp.map((c) => (
            <button
              key={c}
              type="button"
              onClick={() => pickChannel(c)}
              className="rounded border px-2 py-0.5 text-xs tabular-nums"
              style={{
                borderColor: applied.channel === c ? 'var(--accent)' : 'var(--line)',
                color: FROM_SEARX.has(c) ? 'var(--accent)' : 'var(--fg-muted)',
              }}
              title={FROM_SEARX.has(c) ? 'from SearXNG' : 'crawler-discovered'}
            >
              {c} {(comp[c] ?? 0).toLocaleString()}
            </button>
          ))}
        </div>
      </div>

      {/* Media enumerated apart from pages (M9): a gallery and an article are both "one page
          indexed", and only this says how many pages carry pictures or videos. A chip drills the
          list down to just those. */}
      <div
        className="mb-4 flex flex-wrap items-center gap-x-6 gap-y-2 rounded border px-3 py-3 text-sm"
        style={{ borderColor: 'var(--line)', background: 'var(--bg-sunk)' }}
      >
        <span>
          <strong className="tabular-nums">{(data?.media?.image ?? 0).toLocaleString()}</strong>{' '}
          pages with <strong>images</strong>
        </span>
        <span>
          <strong className="tabular-nums">{(data?.media?.video ?? 0).toLocaleString()}</strong>{' '}
          pages with <strong>videos</strong>
        </span>
        <span className="flex flex-wrap gap-1.5">
          {(
            [
              ['', 'all pages'],
              ['image', 'with images'],
              ['video', 'with videos'],
            ] as const
          ).map(([m, label]) => (
            <button
              key={m}
              type="button"
              onClick={() => pickMedia(m)}
              className="rounded border px-2 py-0.5 text-xs"
              style={{
                borderColor: applied.media === m ? 'var(--accent)' : 'var(--line)',
                color: 'var(--fg-muted)',
              }}
            >
              {label}
            </button>
          ))}
        </span>
      </div>

      <form
        className="mb-3 flex flex-wrap items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          setPage(1)
          setApplied({ q, host, lang, channel, media })
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
        <select
          value={channel}
          onChange={(e) => setChannel(e.target.value)}
          className="min-h-10 rounded border px-3 py-2"
          style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
          title="provenance: SearXNG vs crawler"
        >
          <option value="">any source</option>
          {ALL_CHANNELS.map((c) => (
            <option key={c} value={c}>
              {c}
              {FROM_SEARX.has(c) ? ' (SearXNG)' : ''}
            </option>
          ))}
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

      <p className="mb-3 flex flex-wrap items-center gap-2 text-sm" style={{ color: 'var(--fg-muted)' }}>
        <span>Order:</span>
        {([['', 'newest'], ['opens', 'most opened'], ['reports', 'most reported'], ['endorsement', 'most endorsed']] as const).map(([v, label]) => (
          <button key={v} type="button" className={`chip ${sort === v ? 'chip-active' : ''} cursor-pointer`} onClick={() => setSort(v)}>
            {label}
          </button>
        ))}
      </p>
      <Table
        head={
          <>
            <Th>title</Th>
            <Th>domain</Th>
            <Th>source</Th>
            <Th>language</Th>
            <Th num>length</Th>
            <Th num>media</Th>
            <Th>published</Th>
            <Th num>opens</Th>
            <Th num>reports</Th>
            <Th num>web</Th>
          </>
        }
      >
        {(data?.hits ?? []).map((h) => (
          <tr key={h.id}>
            {/* Hovering shows the excerpt — the preview that arrived on every row and was only
                ever used as a length fallback (PROB-003). */}
            <Td title={h.excerpt || h.title}>
              <a href={h.url} rel="noopener nofollow" style={{ color: 'var(--accent)' }}>
                {h.title || h.url}
              </a>
            </Td>
            <Td>{h.domain || ''}</Td>
            <Td>
              {h.discovery && h.discovery !== 'unknown' ? (
                <span
                  className="rounded px-1.5 py-0.5 text-xs"
                  style={{
                    background: 'var(--bg-sunk)',
                    color: FROM_SEARX.has(h.discovery) ? 'var(--accent)' : 'var(--fg-muted)',
                  }}
                  title={FROM_SEARX.has(h.discovery) ? 'from SearXNG' : 'crawler-discovered'}
                >
                  {h.discovery}
                </span>
              ) : (
                <span style={{ color: 'var(--fg-faint)' }}>—</span>
              )}
            </Td>
            <Td>{h.language || ''}</Td>
            <Td num>{h.body_len ?? (h.excerpt ? h.excerpt.length : '')}</Td>
            {/* The page's own media, enumerated: N images · M videos, or a dash. */}
            <Td num>
              {(() => {
                const img = (h.media ?? []).filter((m) => m.type === 'image').length
                const vid = (h.media ?? []).filter((m) => m.type === 'video').length
                return img || vid ? `${img} img · ${vid} vid` : '—'
              })()}
            </Td>
            <Td>
              {h.published_at
                ? new Date(h.published_at * 1000).toISOString().slice(0, 16).replace('T', ' ')
                : ''}
            </Td>
            <Td num>{h.hits?.opens ?? 0}</Td>
            <Td num>{h.hits?.reports ?? 0}</Td>
            <Td num>
              {h.web?.seen ? (
                <span title={`returned ${h.web.seen}× by the web, best rank ${h.web.best_rank ?? '?'} (${(h.web.engines ?? []).join(', ') || 'engine unnamed'})`} style={{ fontVariantNumeric: 'tabular-nums' }}>
                  ×{h.web.seen} · #{h.web.best_rank ?? '?'}
                </span>
              ) : (
                <span style={{ color: 'var(--fg-faint)' }}>—</span>
              )}
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
