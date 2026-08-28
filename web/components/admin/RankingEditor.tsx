'use client'

import { useEffect, useMemo, useState } from 'react'

import { getSettings, patchSettings, type RankingWeights } from '@/lib/admin'
import { Action } from '@/components/admin/ui'

/**
 * The ranking weights, editable (M12-T02.4).
 *
 * Ten sliders and two integers, with the rule that keeps relevance dominant checked as you
 * drag — the same rule the API enforces, so Apply cannot be refused for a reason the page did
 * not show first: every weight in [0, 1]; relevance above the side signals together; the side
 * signals unable to bridge the relevance gap across twenty positions (a barely-relevant page
 * must not reach the top on freshness and trust alone). Apply changes ranking for the next
 * search and writes `config/runtime.toml`; Revert reloads what the API holds.
 */
const RELEVANCE_DECAY = 10
const SLIDERS: [keyof RankingWeights, string][] = [
  ['relevance', 'Relevance — the retrieval score; the base every other signal is a fraction of'],
  ['ui_language', 'Reader’s language — a result in the interface language, bounded (ADR-0026)'],
  ['freshness', 'Freshness — newer pages, decaying with age'],
  ['trust', 'Trust — the source’s tier in the registry'],
  ['authority', 'Authority — the domain’s curated prior and PageRank'],
  ['quality', 'Quality — length, structure, and no spam signals'],
  ['interaction', 'Interaction — the anonymous click-through term (ADR-0015)'],
  ['spam_penalty', 'Spam penalty — subtracted from pages the classifier flags'],
  ['unknown_date_factor', 'Unknown-date factor — multiplier for pages whose date was guessed'],
]

export function checkWeights(w: RankingWeights): string | null {
  for (const [k] of SLIDERS) {
    const v = w[k]
    if (typeof v !== 'number' || Number.isNaN(v) || v < 0 || v > 1) return `${k} must be between 0 and 1`
  }
  if (w.per_domain_cap < 1 || w.per_domain_cap > 20) return 'per-domain cap must be between 1 and 20'
  if (w.simhash_collapse_distance > 16) return 'simhash collapse distance must be at most 16'
  const side = w.freshness + w.trust + w.authority + w.quality + w.interaction
  if (w.relevance <= side) return `relevance (${w.relevance.toFixed(2)}) must stay above the side signals together (${side.toFixed(2)})`
  const gap20 = w.relevance * (1 - Math.exp(-20 / RELEVANCE_DECAY))
  if (side >= gap20) return `side signals (${side.toFixed(2)}) could bridge a 20-position relevance gap (${gap20.toFixed(2)}) — lower them or raise relevance`
  return null
}

export function RankingEditor() {
  const [held, setHeld] = useState<RankingWeights | null>(null)
  const [w, setW] = useState<RankingWeights | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [overridden, setOverridden] = useState(false)

  const load = () =>
    getSettings()
      .then((s) => {
        setHeld(s.ranking)
        setW(s.ranking)
        setOverridden(s.overridden.includes('ranking'))
      })
      .catch((e) => setNote(String(e)))
  useEffect(() => {
    void load()
  }, [])

  const problem = useMemo(() => (w ? checkWeights(w) : null), [w])
  const dirty = useMemo(() => JSON.stringify(w) !== JSON.stringify(held), [w, held])
  if (!w) return <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>{note ?? 'Loading…'}</p>

  const side = w.freshness + w.trust + w.authority + w.quality + w.interaction
  const gap20 = w.relevance * (1 - Math.exp(-20 / RELEVANCE_DECAY))
  const set = (k: keyof RankingWeights, v: number) => setW({ ...w, [k]: v })

  return (
    <div className="max-w-2xl">
      <div className="mb-3 flex flex-wrap items-center gap-2 text-xs" style={{ color: 'var(--fg-muted)' }}>
        <span>
          Side signals <strong style={{ fontVariantNumeric: 'tabular-nums' }}>{side.toFixed(2)}</strong> of a
          20-position gap of <strong style={{ fontVariantNumeric: 'tabular-nums' }}>{gap20.toFixed(2)}</strong>
        </span>
        {overridden && <span className="chip">runtime override in force</span>}
      </div>
      <div className="mb-2 h-1.5 w-full overflow-hidden rounded" style={{ background: 'var(--viz-seq-1)' }} aria-hidden>
        <div className="h-full rounded" style={{ width: `${Math.min(100, (side / Math.max(gap20, 0.001)) * 100)}%`, background: side >= gap20 ? 'var(--viz-critical)' : side > gap20 * 0.85 ? 'var(--viz-warning)' : 'var(--viz-1)' }} />
      </div>
      <ul className="m-0 flex flex-col gap-2 p-0" style={{ listStyle: 'none' }}>
        {SLIDERS.map(([k, hint]) => (
          <li key={k} className="grid items-center gap-3" style={{ gridTemplateColumns: '150px 1fr 56px' }}>
            <label htmlFor={`rk-${k}`} className="text-sm" title={hint}>
              {k.replace(/_/g, ' ')}
            </label>
            <input id={`rk-${k}`} type="range" min={0} max={1} step={0.01} value={w[k]} onChange={(e) => set(k, Number(e.target.value))} title={hint} />
            <input type="number" min={0} max={1} step={0.01} value={w[k]} onChange={(e) => set(k, Number(e.target.value))} className="w-14 rounded border px-1 py-0.5 text-end text-xs" style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)', fontVariantNumeric: 'tabular-nums' }} aria-label={`${k} value`} />
          </li>
        ))}
        {(['per_domain_cap', 'simhash_collapse_distance'] as const).map((k) => (
          <li key={k} className="grid items-center gap-3" style={{ gridTemplateColumns: '150px 1fr 56px' }}>
            <label htmlFor={`rk-${k}`} className="text-sm">{k.replace(/_/g, ' ')}</label>
            <span className="text-xs" style={{ color: 'var(--fg-faint)' }}>
              {k === 'per_domain_cap' ? 'results per domain on a page (1–20)' : 'near-duplicate pages collapsed within this simhash distance (0–16)'}
            </span>
            <input id={`rk-${k}`} type="number" min={k === 'per_domain_cap' ? 1 : 0} max={k === 'per_domain_cap' ? 20 : 16} step={1} value={w[k]} onChange={(e) => set(k, Number(e.target.value))} className="w-14 rounded border px-1 py-0.5 text-end text-xs" style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }} />
          </li>
        ))}
      </ul>
      <p className="mt-3 flex flex-wrap items-center gap-2 text-sm">
        <Action
          busy={busy}
          disabled={!dirty || problem !== null}
          onClick={async () => {
            setBusy(true)
            setNote(null)
            try {
              const r = await patchSettings({ ranking: w })
              setHeld(r.ranking)
              setW(r.ranking)
              setOverridden(true)
              setNote(r.persisted_to ? `Applied, and kept in ${r.persisted_to}.` : 'Applied for this run; could not write the override file.')
            } catch (e) {
              setNote((e as Error).message)
            } finally {
              setBusy(false)
            }
          }}
        >
          Apply
        </Action>
        <button type="button" className="chip cursor-pointer" disabled={!dirty} onClick={() => setW(held)}>
          Revert
        </button>
        {problem ? <span style={{ color: 'var(--viz-critical)' }}>{problem}</span> : note ? <span style={{ color: 'var(--fg-muted)' }}>{note}</span> : null}
      </p>
    </div>
  )
}
