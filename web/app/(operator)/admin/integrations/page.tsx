'use client'

import { useCallback, useEffect, useState } from 'react'

import {
  getIntegrations,
  setIntegration,
  type FederationStatus,
  type SemanticStatus,
  type ImageVectorStatus,
  type ExternalSummariserStatus,
  type IntegrationEffectiveness,
} from '@/lib/admin'
import { PageHead } from '@/components/admin/ui'

/**
 * External-tool integrations (M7-T09, ADR-0017).
 *
 * Query-time federation with a self-hosted SearXNG aggregator (mixed into results, eager-indexed,
 * crawl-fed), the semantic and image vector engines, and the opt-in external AI summariser. The
 * serving plane never talks to SearXNG or the LLM provider directly — both live behind the
 * Federation Gateway on the egress network; this page is the control surface and the health view.
 */
export default function IntegrationsPage() {
  const [fed, setFed] = useState<FederationStatus | null>(null)
  const [semantic, setSemantic] = useState<SemanticStatus | null>(null)
  const [image, setImage] = useState<ImageVectorStatus | null>(null)
  const [ext, setExt] = useState<ExternalSummariserStatus | null>(null)
  const [eff, setEff] = useState<IntegrationEffectiveness | null>(null)
  const [msg, setMsg] = useState('')
  const [busy, setBusy] = useState(false)

  const load = useCallback(
    () =>
      getIntegrations()
        .then((d) => {
          setFed(d.federation)
          setSemantic(d.semantic)
          setImage(d.image)
          setExt(d.external_summariser)
          setEff(d.effectiveness)
        })
        .catch((e) => setMsg((e as Error).message)),
    [],
  )
  useEffect(() => {
    void load()
  }, [load])

  async function toggle(integration: string, enabled: boolean) {
    setBusy(true)
    setMsg('')
    try {
      await setIntegration(integration, enabled)
      // Await the reload before re-enabling the controls (BUG-027): clearing `busy` while the
      // refetch was still in flight let a rapid second click toggle against stale state.
      await load()
    } catch (e) {
      setMsg((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <PageHead title="Integrations">
        External tools that feed the index and the answer. Federation borrows recall from a
        self-hosted SearXNG aggregator and indexes what it borrows, so the corpus converges toward
        answering on its own. Everything here is off by default and runs behind the Federation
        Gateway on the egress network — the serving plane never reaches these tools directly.
      </PageHead>

      <h2 className="mb-2 text-lg font-semibold">Query-time federation (SearXNG)</h2>

      {fed ? (
        <>
          <p className="mb-4 max-w-2xl text-sm" style={{ color: 'var(--fg-muted)' }}>
            {fed.configured ? (
              <>
                Gateway <code>{fed.federator_url}</code>, budget <strong>{fed.budget_ms} ms</strong>,
                up to <strong>{fed.max_hits}</strong> hits per query. Federation runs concurrently
                with the local index and never makes the answer wait past the budget. Start the{' '}
                <code>federation</code> compose profile, then flip the switch.
              </>
            ) : (
              <>
                No gateway configured. Set <code>federation.federator_url</code> in the config before
                enabling — a switch with no gateway to call would silently do nothing, so turning it
                on here is refused until it is set.
              </>
            )}
          </p>

          <label className="mb-4 flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={fed.enabled}
              disabled={busy || !fed.configured}
              onChange={(e) => toggle('federation', e.target.checked)}
            />
            Federation is <strong>{fed.enabled ? 'ON' : 'off'}</strong>
            {!fed.configured ? (
              <span style={{ color: 'var(--fg-faint)' }}> — configure an endpoint first</span>
            ) : null}
          </label>

          <table className="mb-6 w-full max-w-2xl border-collapse text-sm">
            <tbody>
              {[
                ['Configured', fed.configured ? 'yes' : 'no'],
                ['Runtime switch', fed.enabled ? 'on' : 'off'],
                ['Gateway (API calls this)', fed.federator_url || '—'],
                ['SearXNG (gateway calls this)', fed.searxng_url || '—'],
                ['Latency budget', `${fed.budget_ms} ms`],
                ['Max hits / query', String(fed.max_hits)],
                ['Gateway reachable', fed.reachable_from_api ? 'yes — healthy' : 'no — start the federation profile'],
                ['Circuit breaker', fed.breaker],
                ['Eager index', fed.eager_index ? 'on — results indexed immediately (thin), then crawled' : 'off — crawl-feed only (slower to appear)'],
              ].map(([k, v]) => (
                <tr key={k}>
                  <td className="border-b py-1 pr-4" style={{ borderColor: 'var(--line)', color: 'var(--fg-muted)' }}>
                    {k}
                  </td>
                  <td className="border-b py-1 font-mono text-xs" style={{ borderColor: 'var(--line)' }}>
                    {v}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>

          <p className="mb-6 max-w-2xl rounded border px-3 py-2 text-sm" style={{ borderColor: 'var(--line)', background: 'var(--bg-sunk)' }}>
            With federation on, web results are mixed into searches with a “from the web” badge,
            indexed immediately in the background, and fed to the crawler — so a page someone just
            searched for becomes a normal local result within seconds.
          </p>
        </>
      ) : (
        <p className="mb-6 text-sm" style={{ color: 'var(--fg-faint)' }}>Loading…</p>
      )}

      {/* External AI summariser (M7-T08) — the one integration that is third-party SaaS, said
          plainly: when on, query text and result excerpts leave this deployment. */}
      <h2 className="mb-2 mt-8 text-lg font-semibold">External AI summariser</h2>
      {ext ? (
        <>
          <p className="mb-3 max-w-2xl rounded border px-3 py-2 text-sm" style={{ borderColor: 'var(--warn)', background: 'var(--bg-sunk)' }}>
            <strong>Third-party service.</strong> When enabled, the search terms and result excerpts
            of summarised searches are sent to the AI provider configured on the gateway
            (<code>EXTERNAL_LLM_URL</code>) — unlike everything else on this page, data leaves this
            deployment. The local model stays the default and the fallback; external answers face
            the same citation and language validation. Off by default.
          </p>
          <label className="mb-4 flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={ext.enabled}
              disabled={busy || !ext.configured}
              onChange={(e) => toggle('external_summariser', e.target.checked)}
            />
            External summariser is <strong>{ext.enabled ? 'ON' : 'off'}</strong>
            {!ext.configured ? (
              <span style={{ color: 'var(--fg-faint)' }}> — configure the gateway first</span>
            ) : null}
          </label>
          <table className="mb-6 w-full max-w-2xl border-collapse text-sm">
            <tbody>
              {[
                ['Configured (gateway reachable path)', ext.configured ? 'yes' : 'no — set federation.federator_url'],
                ['Runtime switch', ext.enabled ? 'on' : 'off'],
                ['Provider endpoint & key', 'held by the gateway (EXTERNAL_LLM_URL / EXTERNAL_LLM_KEY env) — never on the serving plane'],
                ['Summaries served externally', ext.attempts_ok.toLocaleString()],
                ['Fell back to local', ext.attempts_failed.toLocaleString()],
              ].map(([k, v]) => (
                <tr key={k}>
                  <td className="border-b py-1 pr-4" style={{ borderColor: 'var(--line)', color: 'var(--fg-muted)' }}>{k}</td>
                  <td className="border-b py-1 font-mono text-xs" style={{ borderColor: 'var(--line)' }}>{v}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      ) : (
        <p className="mb-6 text-sm" style={{ color: 'var(--fg-faint)' }}>Loading…</p>
      )}

      {/* Semantic (dense) text search — a whole retrieval path that otherwise had no console. */}
      <h2 className="mb-2 mt-8 text-lg font-semibold">Semantic search (dense text)</h2>
      {semantic ? (
        semantic.configured ? (
          <table className="mb-6 w-full max-w-2xl border-collapse text-sm">
            <tbody>
              {[
                ['Enabled', semantic.enabled ? 'yes' : 'no (configured, switch off)'],
                ['Embedder', semantic.embedder_endpoint ?? '—'],
                ['Reachable', semantic.reachable ? 'yes — healthy' : 'no — start the semantic profile'],
                ['Circuit breaker', semantic.breaker ?? '—'],
                ['Collection', `${semantic.collection ?? '—'} (${semantic.dim ?? '?'}-d)`],
                [
                  'Documents embedded',
                  semantic.documents_embedded == null
                    ? 'Qdrant unreachable'
                    : semantic.documents_embedded.toLocaleString(),
                ],
              ].map(([k, v]) => (
                <tr key={k}>
                  <td className="border-b py-1 pr-4" style={{ borderColor: 'var(--line)', color: 'var(--fg-muted)' }}>{k}</td>
                  <td className="border-b py-1 font-mono text-xs" style={{ borderColor: 'var(--line)' }}>{v}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <p className="mb-6 text-sm" style={{ color: 'var(--fg-muted)' }}>
            Off. Enable <code>[vector] text_enabled</code> and start the <code>semantic</code> compose
            profile (bge-m3 embedder) to match queries by meaning, not just words.
          </p>
        )
      ) : null}

      {/* Image similarity (CLIP) — the other vector engine, shown for parity. */}
      <h2 className="mb-2 mt-4 text-lg font-semibold">Image similarity (CLIP)</h2>
      {image ? (
        image.configured ? (
          <table className="mb-6 w-full max-w-2xl border-collapse text-sm">
            <tbody>
              {[
                ['Embedder', image.embedder_endpoint ?? '—'],
                ['Reachable', image.reachable ? 'yes — healthy' : 'no — start the vector profile'],
                ['Collection', image.collection ?? '—'],
                [
                  'Images embedded',
                  image.images_embedded == null ? 'Qdrant unreachable' : image.images_embedded.toLocaleString(),
                ],
              ].map(([k, v]) => (
                <tr key={k}>
                  <td className="border-b py-1 pr-4" style={{ borderColor: 'var(--line)', color: 'var(--fg-muted)' }}>{k}</td>
                  <td className="border-b py-1 font-mono text-xs" style={{ borderColor: 'var(--line)' }}>{v}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <p className="mb-6 text-sm" style={{ color: 'var(--fg-muted)' }}>
            Off. Enable <code>[vector] enabled</code> and start the <code>vector</code> compose profile
            (CLIP embedder) for reverse-image / search-by-image.
          </p>
        )
      ) : null}

      {/* Effectiveness — read from the metrics registry, so it matches Prometheus/Grafana exactly. */}
      {eff ? (
        <>
          <h2 className="mb-2 mt-4 text-lg font-semibold">Effectiveness (since API start)</h2>
          <table className="mb-6 w-full max-w-2xl border-collapse text-sm">
            <tbody>
              {[
                [
                  'Federation contributed',
                  `${eff.federation_searches_hits.toLocaleString()} searches with hits · ${eff.federation_searches_empty.toLocaleString()} empty`,
                ],
                ['URLs fed to the index', eff.federation_urls_fed.toLocaleString()],
                [
                  'Blend share (convergence)',
                  eff.blend_cards_web + eff.blend_cards_local > 0
                    ? `${((100 * eff.blend_cards_web) / (eff.blend_cards_web + eff.blend_cards_local)).toFixed(1)}% from the web (${eff.blend_cards_web.toLocaleString()} web · ${eff.blend_cards_local.toLocaleString()} local) — falling means the index is catching up`
                    : 'no federation-armed searches yet',
                ],
                [
                  'Semantic added recall',
                  `${eff.semantic_fused_recall.toLocaleString()} searches · ${eff.semantic_fused_reinforce.toLocaleString()} only reinforced lexical`,
                ],
              ].map(([k, v]) => (
                <tr key={k}>
                  <td className="border-b py-1 pr-4" style={{ borderColor: 'var(--line)', color: 'var(--fg-muted)' }}>{k}</td>
                  <td className="border-b py-1 tabular-nums" style={{ borderColor: 'var(--line)' }}>{v}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="mb-6 text-xs" style={{ color: 'var(--fg-faint)' }}>
            Counters since the API last started. Full time-series live in Grafana (dev:{' '}
            <code>localhost:3001</code>).
          </p>
        </>
      ) : null}

      {msg ? <p className="text-sm" style={{ color: 'var(--warn)' }}>{msg}</p> : null}
    </>
  )
}
