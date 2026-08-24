'use client'

import { useCallback, useEffect, useState } from 'react'

import {
  getIntegrations,
  setIntegration,
  type FederationStatus,
  type SemanticStatus,
  type ImageVectorStatus,
} from '@/lib/admin'
import { PageHead } from '@/components/admin/ui'

/**
 * External-tool integrations (M7-T09, ADR-0017).
 *
 * Today: query-time federation with a self-hosted SearXNG aggregator. The serving plane never talks
 * to SearXNG directly — it lives on the egress network behind the Federation Gateway — so this page
 * shows configuration and the runtime switch, not a live health probe. The blend and crawl-feed that
 * consume the switch are later increments; this is the control surface for them.
 */
export default function IntegrationsPage() {
  const [fed, setFed] = useState<FederationStatus | null>(null)
  const [semantic, setSemantic] = useState<SemanticStatus | null>(null)
  const [image, setImage] = useState<ImageVectorStatus | null>(null)
  const [msg, setMsg] = useState('')
  const [busy, setBusy] = useState(false)

  const load = useCallback(() => {
    getIntegrations()
      .then((d) => {
        setFed(d.federation)
        setSemantic(d.semantic)
        setImage(d.image)
      })
      .catch((e) => setMsg((e as Error).message))
  }, [])
  useEffect(() => load(), [load])

  async function toggle(enabled: boolean) {
    setBusy(true)
    setMsg('')
    try {
      await setIntegration('federation', enabled)
      load()
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
              onChange={(e) => toggle(e.target.checked)}
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
                ['Extra allowlist', fed.allowlist.length ? fed.allowlist.join(', ') : '(SearXNG host only)'],
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

          {/* Honest about what this switch does today: it is the control surface; the consumers that
              read it (the query-time blend and the crawler crawl-feed) are later increments. */}
          <p className="mb-6 max-w-2xl rounded border px-3 py-2 text-sm" style={{ borderColor: 'var(--line)', background: 'var(--bg-sunk)' }}>
            The blend into live results and the crawl-feed that indexes federated URLs are being
            built next. Enabling federation here arms the switch they read; until they land, this
            toggle changes configuration state without yet altering results.
          </p>
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

      {msg ? <p className="text-sm" style={{ color: 'var(--warn)' }}>{msg}</p> : null}
    </>
  )
}
