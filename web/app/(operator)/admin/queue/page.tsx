'use client'

import { useState } from 'react'

import { getQueue, replayDlq, type QueueStatus } from '@/lib/admin'
import { PageHead, usePoll } from '@/components/admin/ui'

function Tile({ n, label }: { n: number | string; label: string }) {
  return (
    <div className="flex flex-col gap-0.5 border px-4 py-3" style={{ borderColor: 'var(--line)', minInlineSize: '140px' }}>
      <span className="text-2xl font-medium tabular-nums">{n}</span>
      <span className="text-xs" style={{ color: 'var(--fg-muted)' }}>{label}</span>
    </div>
  )
}

/**
 * The index queue and its dead letters (M4). The backlog is documents waiting to be indexed; the
 * dead letters are the ones the indexer gave up on — a growing count is data the index will never
 * get without intervention. Replay is deliberate: fix the cause first, then re-enqueue.
 */
export default function QueuePage() {
  const { data, error } = usePoll<QueueStatus>(getQueue, 5_000)
  const [msg, setMsg] = useState('')
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)

  async function replay() {
    setBusy(true)
    setMsg('')
    try {
      const r = await replayDlq()
      setMsg(`replayed ${r.replayed ?? 0} dead letters`)
      setConfirming(false)
    } catch (e) {
      setMsg((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  const dead = data?.dead ?? []

  return (
    <>
      <PageHead title="Index queue">
        Documents waiting to be indexed, and the ones the indexer gave up on. A rising dead-letter
        count is data loss in slow motion — fix the cause, then replay.
      </PageHead>

      {error ? (
        <p className="mb-4 text-sm" style={{ color: 'var(--warn)' }}>Could not reach the API: {error}</p>
      ) : null}
      {data?.unavailable ? (
        <p className="mb-4 text-sm" style={{ color: 'var(--warn)' }}>The queue is unreachable (Redis down).</p>
      ) : null}

      <div className="mb-8 flex flex-wrap gap-3">
        <Tile n={data ? (data.unavailable ? 'unknown' : (data.backlog ?? 0)) : '…'} label="backlog (waiting to index)" />
        <Tile n={data ? (data.unavailable ? 'unknown' : (data.dead_count ?? 0)) : '…'} label="dead letters" />
      </div>

      <div className="mb-8">
        <h2 className="mb-2 text-lg font-semibold">Dead letters</h2>
        {dead.length === 0 ? (
          <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>None — nothing has been given up on.</p>
        ) : (
          <>
            <table className="mb-4 w-full max-w-4xl border-collapse text-sm">
              <thead>
                <tr style={{ color: 'var(--fg-muted)' }}>
                  <th className="border-b py-1 text-start" style={{ borderColor: 'var(--line)' }}>url</th>
                  <th className="border-b py-1 text-start" style={{ borderColor: 'var(--line)' }}>reason</th>
                  <th className="border-b py-1 text-end" style={{ borderColor: 'var(--line)' }}>attempts</th>
                </tr>
              </thead>
              <tbody>
                {dead.map((d, i) => (
                  <tr key={`${d.url}-${i}`}>
                    <td className="border-b py-1" style={{ borderColor: 'var(--line)' }} title={d.url} dir="ltr">
                      <span className="line-clamp-1">{d.url || '(no url)'}</span>
                    </td>
                    <td className="border-b py-1 text-xs" style={{ borderColor: 'var(--line)' }}>{d.reason}</td>
                    <td className="border-b py-1 text-end tabular-nums" style={{ borderColor: 'var(--line)' }}>{d.attempts}</td>
                  </tr>
                ))}
              </tbody>
            </table>

            {!confirming ? (
              <button
                type="button"
                onClick={() => setConfirming(true)}
                className="min-h-10 rounded border px-4 text-sm"
                style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}
              >
                Replay dead letters
              </button>
            ) : (
              <div className="flex items-center gap-2">
                <span className="text-sm" style={{ color: 'var(--fg-muted)' }}>
                  Re-enqueue all dead letters? Do this only after fixing the cause.
                </span>
                <button type="button" disabled={busy} onClick={replay} className="min-h-10 rounded border px-4 text-sm" style={{ borderColor: 'var(--warn)', color: 'var(--fg)' }}>
                  {busy ? 'replaying…' : 'Yes, replay'}
                </button>
                <button type="button" onClick={() => setConfirming(false)} className="min-h-10 rounded border px-4 text-sm" style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}>
                  Cancel
                </button>
              </div>
            )}
          </>
        )}
        {msg ? <p className="mt-3 text-sm" style={{ color: 'var(--fg-muted)' }}>{msg}</p> : null}
      </div>
    </>
  )
}
