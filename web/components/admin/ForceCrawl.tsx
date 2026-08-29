'use client'

import { useState } from 'react'

import { enqueueUrl } from '@/lib/admin'

/**
 * Force-crawl a URL — the "crawl this now" control (M4 ops gap, backed by `/crawler/enqueue`).
 *
 * A URL an operator types is as trusted as a seed: it is pushed straight into the frontier, and
 * with "front of queue" it jumps ahead of everything waiting. The API still runs it through the
 * SSRF guard and the crawler-trap detector, so a bad URL is refused with a reason rather than
 * silently dropped.
 */
export function ForceCrawl() {
  const [url, setUrl] = useState('')
  const [front, setFront] = useState(true)
  const [msg, setMsg] = useState('')
  const [busy, setBusy] = useState(false)

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    const u = url.trim()
    if (!u) return
    setBusy(true)
    setMsg('')
    try {
      const r = await enqueueUrl(u, front)
      setMsg(
        r.error?.message
          ? `refused: ${r.error.message}`
          : r.already_known
            ? `${r.url ?? u} is already known — not re-queued`
            : `queued ${r.url ?? u}`,
      )
      if (!r.error) setUrl('')
    } catch (err) {
      setMsg((err as Error).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    // Password-manager extensions (1Password, LastPass, Dashlane…) inject `form_signature` /
    // `field_signature` attributes onto forms and inputs after the server HTML arrives but before
    // React hydrates, which React reports as a mismatch. It is not ours to fix — this suppresses the
    // warning for the element's own attributes only, exactly as the search box does.
    <form onSubmit={submit} className="mb-6 flex flex-wrap items-end gap-2" suppressHydrationWarning>
      <label className="flex flex-col gap-1 text-sm">
        Force-crawl a URL
        <input
          type="url"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="https://example.dz/page"
          dir="ltr"
          autoComplete="off"
          className="min-h-10 w-full sm:w-auto sm:min-w-[280px] rounded border px-3 py-1.5 text-sm"
          style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
          suppressHydrationWarning
        />
      </label>
      <label className="flex items-center gap-1.5 pb-2 text-sm">
        <input type="checkbox" checked={front} onChange={(e) => setFront(e.target.checked)} />
        front of queue
      </label>
      <button
        type="submit"
        disabled={busy || !url.trim()}
        className="min-h-10 self-end rounded border px-4 text-sm"
        style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}
      >
        {busy ? 'queuing…' : 'Crawl now'}
      </button>
      {msg ? (
        <span className="pb-2 text-sm" style={{ color: 'var(--fg-muted)' }}>
          {msg}
        </span>
      ) : null}
    </form>
  )
}
