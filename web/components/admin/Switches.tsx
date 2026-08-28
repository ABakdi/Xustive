'use client'

import { useEffect, useState } from 'react'

import { getSettings, patchSettings, type RuntimeSettings } from '@/lib/admin'
import { Toggle } from '@/components/admin/ui'

/** The runtime switches (M12-T02.5), one component each so a page mounts only its own. */
function useSettings() {
  const [s, setS] = useState<RuntimeSettings | null>(null)
  useEffect(() => {
    getSettings().then(setS).catch(() => {})
  }, [])
  return [s, setS] as const
}

export function SummariesSwitch() {
  const [s, setS] = useSettings()
  if (!s) return null
  return (
    <Toggle
      label="AI summaries"
      checked={s.ml.summaries_enabled}
      hint="off: no summary token is minted, the page shows results only"
      onChange={async (next) => setS(await patchSettings({ ml: { summaries_enabled: next } }))}
    />
  )
}

export function RerankerSwitch() {
  const [s, setS] = useState<RuntimeSettings | null>(null)
  useEffect(() => { getSettings().then(setS).catch(() => setS(null)) }, [])
  if (!s) return null
  return (
    <Toggle
      label="Cross-encoder reranker"
      checked={s.ml.reranker_enabled}
      hint="Qwen3-Reranker-0.6B re-reads the top of the page and its order is fused with ours by reciprocal rank (ADR-0032); needs services/reranker up, bounded by its timeout"
      onChange={async (next) => setS(await patchSettings({ ml: { reranker_enabled: next } }))}
    />
  )
}

export function CollectionSwitch() {
  const [s, setS] = useSettings()
  if (!s) return null
  return (
    <Toggle
      label="Keep search events"
      checked={s.collection.enabled}
      hint="ADR-0030 — on, the operator is a data controller (Legal §5)"
      onChange={async (next) => setS(await patchSettings({ collection: { enabled: next } }))}
    />
  )
}

export function InteractionSwitch() {
  const [s, setS] = useSettings()
  if (!s) return null
  return (
    <Toggle
      label="Anonymous interaction signals"
      checked={s.interaction.enabled}
      hint="k-anonymous click counters that feed the ranker (ADR-0015)"
      onChange={async (next) => setS(await patchSettings({ interaction: { enabled: next } }))}
    />
  )
}

/** Federation's budgets and the eager index, editable. */
export function FederationBudgets() {
  const [s, setS] = useSettings()
  const [note, setNote] = useState<string | null>(null)
  const [draft, setDraft] = useState<{ budget_ms: number; fetch_budget_ms: number; max_hits: number } | null>(null)
  useEffect(() => {
    if (s && !draft) setDraft({ budget_ms: s.federation.budget_ms, fetch_budget_ms: s.federation.fetch_budget_ms, max_hits: s.federation.max_hits })
  }, [s, draft])
  if (!s || !draft) return null
  const field = (k: keyof typeof draft, label: string, hint: string, min: number, max: number) => (
    <label className="flex items-center gap-3 text-sm">
      <span className="w-40">{label}</span>
      <input type="number" min={min} max={max} value={draft[k]} onChange={(e) => setDraft({ ...draft, [k]: Number(e.target.value) })} className="w-24 rounded border px-2 py-0.5 text-end text-xs" style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)', fontVariantNumeric: 'tabular-nums' }} />
      <span className="text-xs" style={{ color: 'var(--fg-faint)' }}>{hint}</span>
    </label>
  )
  const dirty = draft.budget_ms !== s.federation.budget_ms || draft.fetch_budget_ms !== s.federation.fetch_budget_ms || draft.max_hits !== s.federation.max_hits
  return (
    <div className="flex flex-col gap-2">
      {field('budget_ms', 'Live strip budget (ms)', 'how long a search waits for federated results before shipping index-only (100–5000)', 100, 5000)}
      {field('fetch_budget_ms', 'Fetch budget (ms)', 'how long the detached fetch may run to eager-index and feed the crawl (1000–30000)', 1000, 30000)}
      {field('max_hits', 'Max hits', 'federated results taken per search (1–50)', 1, 50)}
      <Toggle label="Eager index" checked={s.federation.eager_index} hint="index federated hits at once as thin documents, then crawl them" onChange={async (next) => setS(await patchSettings({ federation: { eager_index: next } }))} />
      <p className="m-0 flex items-center gap-2 text-sm">
        <button
          type="button"
          className="chip chip-active cursor-pointer"
          disabled={!dirty}
          onClick={async () => {
            setNote(null)
            try {
              const r = await patchSettings({ federation: draft })
              setS(r)
              setNote(r.persisted_to ? `Applied, kept in ${r.persisted_to}.` : 'Applied for this run.')
            } catch (e) {
              setNote((e as Error).message)
            }
          }}
        >
          Apply budgets
        </button>
        {note && <span style={{ color: note.startsWith('Applied') ? 'var(--fg-muted)' : 'var(--viz-critical)' }}>{note}</span>}
      </p>
    </div>
  )
}
