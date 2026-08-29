'use client'

import { useState } from 'react'

import { takedown, type TakedownResult } from '@/lib/admin'
import { PageHead } from '@/components/admin/ui'

/**
 * Destructive maintenance — the takedown control (M4-T09.3).
 *
 * Removing already-indexed content is a genuine operational/compliance need, so it lives in the
 * console rather than only on the command line. It is deliberately two-step: a **preview** first
 * (how many documents match), then **execute**, which requires typing the exact domain again — the
 * same guard the CLI enforces with `--yes`. It does not stop future crawling; the page says so.
 */
export default function MaintenancePage() {
  const [domain, setDomain] = useState('')
  const [confirm, setConfirm] = useState('')
  const [preview, setPreview] = useState<TakedownResult | null>(null)
  const [result, setResult] = useState<TakedownResult | null>(null)
  const [msg, setMsg] = useState('')
  const [busy, setBusy] = useState(false)

  async function runPreview() {
    setBusy(true)
    setMsg('')
    setResult(null)
    try {
      setPreview(await takedown(domain.trim(), false, ''))
    } catch (e) {
      setMsg((e as Error).message)
      setPreview(null)
    } finally {
      setBusy(false)
    }
  }

  async function runExecute() {
    setBusy(true)
    setMsg('')
    try {
      const r = await takedown(domain.trim(), true, confirm.trim())
      setResult(r)
      setPreview(null)
      setConfirm('')
    } catch (e) {
      setMsg((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <PageHead title="Maintenance">
        Destructive actions, kept behind a confirmation. Removing content deletes it from the search
        index, the image vectors, and the stored page bodies — it does not stop future crawling, so
        pair a takedown with disabling the source on the Sources page.
      </PageHead>

      <h2 className="mb-2 text-lg font-semibold">Takedown a domain</h2>
      <p className="mb-3 max-w-2xl text-sm" style={{ color: 'var(--fg-muted)' }}>
        Removes every already-indexed document for a domain. Preview first to see how many match,
        then confirm by typing the domain again.
      </p>

      <div className="mb-4 flex flex-wrap items-end gap-2">
        <label className="flex flex-col gap-1 text-sm">
          Domain
          <input
            value={domain}
            onChange={(e) => {
              setDomain(e.target.value)
              setPreview(null)
              setResult(null)
            }}
            placeholder="example.dz"
            dir="ltr"
            autoComplete="off"
            className="min-h-10 w-full sm:w-auto sm:min-w-[260px] rounded border px-3 py-1.5"
            style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
            suppressHydrationWarning
          />
        </label>
        <button
          type="button"
          disabled={busy || !domain.trim()}
          onClick={runPreview}
          className="min-h-10 self-end rounded border px-4 text-sm"
          style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}
        >
          {busy && !preview ? 'checking…' : 'Preview'}
        </button>
      </div>

      {preview && !preview.executed ? (
        <div className="mb-4 max-w-2xl rounded border px-3 py-3 text-sm" style={{ borderColor: 'var(--warn)' }}>
          <p className="mb-2">
            <strong>{preview.matched ?? 0}</strong> document{preview.matched === 1 ? '' : 's'} match{' '}
            <code><bdi>{preview.domain}</bdi></code>. This deletes them from the index, image vectors, and stored
            bodies — permanently.
          </p>
          {(preview.matched ?? 0) > 0 ? (
            <div className="flex flex-wrap items-end gap-2">
              <label className="flex flex-col gap-1">
                Type <code><bdi>{preview.domain}</bdi></code> to confirm
                <input
                  value={confirm}
                  onChange={(e) => setConfirm(e.target.value)}
                  dir="ltr"
                  autoComplete="off"
                  className="min-h-10 w-full sm:w-auto sm:min-w-[260px] rounded border px-3 py-1.5"
                  style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
                  suppressHydrationWarning
                />
              </label>
              <button
                type="button"
                disabled={busy || confirm.trim() !== preview.domain}
                onClick={runExecute}
                className="min-h-10 self-end rounded border px-4 text-sm"
                style={{ borderColor: 'var(--warn)', color: 'var(--fg)' }}
              >
                {busy ? 'removing…' : 'Delete permanently'}
              </button>
            </div>
          ) : null}
        </div>
      ) : null}

      {result?.executed ? (
        <div className="mb-4 max-w-2xl rounded border px-3 py-3 text-sm" style={{ borderColor: 'var(--line)' }}>
          Removed for <code><bdi>{result.domain}</bdi></code>: {result.documents_removed ?? 0} documents,{' '}
          {result.vector_groups_removed ?? 0} vector groups, {result.raw_bodies_removed ?? 0} stored
          bodies.{' '}
          {/* The API's own note, verbatim, so the two cannot drift (PROB-003). */}
          {result.note ?? 'Future crawling is not blocked — disable the source to prevent re-indexing.'}
        </div>
      ) : null}

      {msg ? <p className="text-sm" style={{ color: 'var(--warn)' }}>{msg}</p> : null}
    </>
  )
}
