'use client'

import { Fragment, useState } from 'react'

import { editRegistry, getSourceHealth, type SourceHealthRow } from '@/lib/admin'
import { PageHead, StatusLine, Table, Td, Th, pct, usePoll } from '@/components/admin/ui'

/** A quality cell: em-dash when unknown, amber when outside its §7 healthy band. */
function QualityCell({ v, ok }: { v: number | null; ok: (x: number) => boolean }) {
  if (v == null) return <Td num>—</Td>
  return (
    <Td num warn={!ok(v)}>
      {pct(v)}
    </Td>
  )
}

const btn = { borderColor: 'var(--line)', color: 'var(--fg-muted)' }

/**
 * The lifecycle/policy editor for one registry source (PROB-003). Mirrors
 * `xustive registry approve|activate|disable` plus the policy knobs the CLI has no verb for.
 * Floors live server-side (delay ≥ 500 ms, depth 1–10) — this form just carries the numbers.
 */
function EditRow({ row, cols, onDone }: { row: SourceHealthRow; cols: number; onDone: (msg: string) => void }) {
  const p = row.policy
  const [frequency, setFrequency] = useState(p?.frequency ?? 'daily')
  const [delay, setDelay] = useState(String(p?.crawl_delay_ms ?? 1500))
  const [depth, setDepth] = useState(String(p?.depth_limit ?? 3))
  const [maxDocs, setMaxDocs] = useState(String(p?.max_docs_per_run ?? 500))
  const [reason, setReason] = useState('')
  const [busy, setBusy] = useState(false)

  async function send(edit: Parameters<typeof editRegistry>[0]) {
    setBusy(true)
    try {
      const r = await editRegistry(edit)
      onDone(`${row.id}: ${r.changed.join(', ')} — now ${r.lifecycle}${r.crawlable ? '' : ' (not crawlable)'}. ${r.note ?? ''}`)
    } catch (e) {
      onDone((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <tr>
      <td colSpan={cols} className="border-b px-3 py-3" style={{ borderColor: 'var(--line)', background: 'var(--bg-raised, transparent)' }}>
        <div className="flex flex-wrap items-end gap-4 text-xs">
          <span className="flex gap-1">
            <button type="button" disabled={busy} onClick={() => send({ id: row.id, action: 'approve' })} className="rounded border px-2 py-1" style={btn}>
              approve
            </button>
            <button type="button" disabled={busy} onClick={() => send({ id: row.id, action: 'activate' })} className="rounded border px-2 py-1" style={btn}>
              activate
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => send({ id: row.id, action: 'disable', reason: reason || undefined })}
              className="rounded border px-2 py-1"
              style={{ borderColor: 'var(--warn)', color: 'var(--warn)' }}
            >
              disable
            </button>
            <input
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              placeholder="reason (kept in the registry)"
              className="rounded border px-2 py-1"
              style={{ ...btn, minInlineSize: '180px' }}
            />
          </span>
          {p ? (
            <span className="flex items-end gap-2">
              <label className="flex flex-col gap-0.5">
                frequency
                <select value={frequency} onChange={(e) => setFrequency(e.target.value as typeof frequency)} className="rounded border px-1 py-1" style={btn}>
                  <option value="realtime">realtime</option>
                  <option value="hourly">hourly</option>
                  <option value="daily">daily</option>
                  <option value="weekly">weekly</option>
                </select>
              </label>
              <label className="flex flex-col gap-0.5">
                delay ms (≥500)
                <input value={delay} onChange={(e) => setDelay(e.target.value)} inputMode="numeric" className="rounded border px-1 py-1" style={{ ...btn, inlineSize: '80px' }} />
              </label>
              <label className="flex flex-col gap-0.5">
                depth (1–10)
                <input value={depth} onChange={(e) => setDepth(e.target.value)} inputMode="numeric" className="rounded border px-1 py-1" style={{ ...btn, inlineSize: '60px' }} />
              </label>
              <label className="flex flex-col gap-0.5">
                docs/run
                <input value={maxDocs} onChange={(e) => setMaxDocs(e.target.value)} inputMode="numeric" className="rounded border px-1 py-1" style={{ ...btn, inlineSize: '80px' }} />
              </label>
              <button
                type="button"
                disabled={busy}
                onClick={() =>
                  send({
                    id: row.id,
                    policy: {
                      frequency,
                      crawl_delay_ms: Number(delay) || undefined,
                      depth_limit: Number(depth) || undefined,
                      max_docs_per_run: Number(maxDocs) || undefined,
                    },
                  })
                }
                className="rounded border px-2 py-1"
                style={btn}
              >
                {busy ? 'saving…' : 'save policy'}
              </button>
            </span>
          ) : (
            <span style={{ color: 'var(--fg-faint)' }}>no registry record — a TSV-only seed has no policy to edit</span>
          )}
        </div>
      </td>
    </tr>
  )
}

export default function SourceHealthPage() {
  const { data, error } = usePoll(getSourceHealth, 10_000)
  const [open, setOpen] = useState<string | null>(null)
  const [msg, setMsg] = useState('')
  // Filters (M12): a text match on id/domain, the lifecycle, the tier. Client-side over the
  // registry, which is small by construction.
  const [needle, setNeedle] = useState('')
  const [lifecycle, setLifecycle] = useState('')
  const [tier, setTier] = useState('')
  const all = data ?? []
  const rows = all.filter(
    (r) =>
      (!needle || `${r.id} ${(r as { domain?: string }).domain ?? ''}`.toLowerCase().includes(needle.toLowerCase())) &&
      (!lifecycle || String((r as { lifecycle?: string }).lifecycle ?? '') === lifecycle) &&
      (!tier || String((r as { trust_tier?: string; tier?: string }).trust_tier ?? (r as { tier?: string }).tier ?? '') === tier),
  )
  const lifecycles = Array.from(new Set(all.map((r) => String((r as { lifecycle?: string }).lifecycle ?? '')).filter(Boolean))).sort()
  const tiers = Array.from(new Set(all.map((r) => String((r as { trust_tier?: string; tier?: string }).trust_tier ?? (r as { tier?: string }).tier ?? '')).filter(Boolean))).sort()
  const COLS = 14
  return (
    <>
      <PageHead title="Source health">
        Per-source quality, joined from the registry and the live crawl counters. A cell reads
        &mdash; until the source has data. Amber marks a value outside its healthy band (§7) — the
        same signal the lifecycle automation degrades on. &ldquo;edit&rdquo; opens the registry
        controls: the same transitions as <code>xustive registry</code>, plus the crawl policy.
      </PageHead>
      <StatusLine>
        {error
          ? `Could not load source health: ${error}`
          : data
            ? `${rows.length} source(s). Refreshing every 10s.`
            : 'Loading…'}
      </StatusLine>
      {msg ? <p className="mb-3 text-sm" style={{ color: 'var(--fg-muted)' }}>{msg}</p> : null}
      <p className="mb-3 flex flex-wrap items-center gap-2 text-sm">
        <input value={needle} onChange={(e) => setNeedle(e.target.value)} placeholder="source id or domain…" className="rounded border px-2 py-1" style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)', minWidth: 220 }} aria-label="Filter sources" />
        <select value={lifecycle} onChange={(e) => setLifecycle(e.target.value)} className="rounded border px-2 py-1" style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }} aria-label="Lifecycle">
          <option value="">any lifecycle</option>
          {lifecycles.map((l) => <option key={l} value={l}>{l}</option>)}
        </select>
        <select value={tier} onChange={(e) => setTier(e.target.value)} className="rounded border px-2 py-1" style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }} aria-label="Tier">
          <option value="">any tier</option>
          {tiers.map((t) => <option key={t} value={t}>tier {t}</option>)}
        </select>
        <span className="text-xs" style={{ color: 'var(--fg-faint)' }}>{rows.length} of {all.length}</span>
      </p>
      <Table
        head={
          <>
            <Th>source</Th>
            <Th>state</Th>
            <Th>tier</Th>
            <Th num>fetched</Th>
            <Th num>failed</Th>
            <Th num>indexed</Th>
            <Th num>thin</Th>
            <Th num>dup</Th>
            <Th num>fetch ok</Th>
            <Th num>extraction</Th>
            <Th num>duplicate</Th>
            <Th num>spam</Th>
            <Th num>date?</Th>
            <Th>{''}</Th>
          </>
        }
      >
        {rows.map((s) => (
          <Fragment key={s.id}>
            <tr>
              <Td>
                {s.display_name || s.id}{' '}
                <span style={{ color: 'var(--fg-faint)' }}>{s.id}</span>
              </Td>
              {/* crawlable folds approval + lifecycle into the one bit that matters: will the
                  next crawl include this source? Returned since M2, rendered now (PROB-003). */}
              <Td>
                {s.lifecycle || '—'}
                {s.crawlable === false ? (
                  <span style={{ color: 'var(--warn)' }}> · not crawlable</span>
                ) : null}
              </Td>
              <Td>{s.trust_tier || '—'}</Td>
              <Td num>{s.counts.fetched}</Td>
              <Td num>{s.counts.failed}</Td>
              <Td num>{s.counts.indexed}</Td>
              <Td num>{s.counts.thin}</Td>
              <Td num>{s.counts.duplicate}</Td>
              <QualityCell v={s.quality.fetch_success_rate} ok={(x) => x > 0.95} />
              <QualityCell v={s.quality.extraction_success_rate} ok={(x) => x > 0.9} />
              <QualityCell v={s.quality.duplicate_ratio} ok={(x) => x < 0.3} />
              <QualityCell v={s.quality.spam_mean} ok={(x) => x < 0.2} />
              <QualityCell v={s.quality.date_unknown_ratio} ok={(x) => x < 0.1} />
              <Td>
                <button
                  type="button"
                  onClick={() => setOpen(open === s.id ? null : s.id)}
                  className="rounded border px-2 py-0.5 text-xs"
                  style={btn}
                >
                  {open === s.id ? 'close' : 'edit'}
                </button>
              </Td>
            </tr>
            {open === s.id ? (
              <EditRow
                row={s}
                cols={COLS}
                onDone={(m) => {
                  setMsg(m)
                  setOpen(null)
                }}
              />
            ) : null}
          </Fragment>
        ))}
      </Table>
    </>
  )
}
