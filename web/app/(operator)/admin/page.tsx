'use client'

import Link from 'next/link'
import { useEffect, useState } from 'react'

import {
  getStatus,
  getCompute,
  getMedia,
  getInteraction,
  getIntegrations,
  getDocuments,
  type MediaStatus,
  type InteractionStatus,
} from '@/lib/admin'
import { PageHead, usePoll } from '@/components/admin/ui'

function Tile({ n, label }: { n: number | string; label: string }) {
  return (
    <div className="flex flex-col gap-0.5 border px-4 py-3" style={{ borderColor: 'var(--line)', minInlineSize: '120px' }}>
      <span className="text-2xl font-medium tabular-nums">{n}</span>
      <span className="text-xs" style={{ color: 'var(--fg-muted)' }}>
        {label}
      </span>
    </div>
  )
}

/** A subsystem status chip — on/off/degraded, so the whole stack reads at a glance. */
function Chip({ label, state, detail }: { label: string; state: 'on' | 'off' | 'warn'; detail: string }) {
  const color = state === 'on' ? 'var(--ok, #2e7d32)' : state === 'warn' ? 'var(--warn, #b26a00)' : 'var(--fg-faint)'
  return (
    <div className="flex items-center gap-2 border px-3 py-2 text-sm" style={{ borderColor: 'var(--line)' }}>
      <span aria-hidden style={{ inlineSize: 9, blockSize: 9, borderRadius: '50%', background: color, display: 'inline-block' }} />
      <span className="font-medium">{label}</span>
      <span style={{ color: 'var(--fg-muted)' }}>{detail}</span>
    </div>
  )
}

const LINKS: [string, string, string][] = [
  ['Live', '/admin/live', 'the crawler as it runs'],
  ['Documents', '/admin/documents', 'what has been collected'],
  ['Sources', '/admin/sources', 'the seed list'],
  ['Source health', '/admin/sources/health', 'per-source quality'],
  ['Discovery yield', '/admin/discovery', 'per-channel funnel'],
  ['Weak coverage', '/admin/weak-coverage', 'gaps to fill'],
  ['Index queue', '/admin/queue', 'backlog & dead letters'],
  ['Compute', '/admin/compute', 'device & politeness'],
  ['Image AI', '/admin/media', 'OCR & image similarity'],
  ['Interaction', '/admin/interaction', 'anonymous use signals'],
  ['Maintenance', '/admin/maintenance', 'takedowns & destructive actions'],
]

export default function OverviewPage() {
  const { data: s, error } = usePoll(getStatus, 5_000)

  // Subsystem statuses change rarely; fetch once (and on a slow refresh) rather than every 5s.
  const [compute, setCompute] = useState<Record<string, unknown> | null>(null)
  const [media, setMedia] = useState<MediaStatus | null>(null)
  const [interaction, setInteraction] = useState<InteractionStatus | null>(null)
  const [integrations, setIntegrations] = useState<Awaited<ReturnType<typeof getIntegrations>> | null>(null)
  const [corpus, setCorpus] = useState<number | null>(null)
  useEffect(() => {
    const tick = () => {
      getCompute().then(setCompute).catch(() => {})
      getMedia().then(setMedia).catch(() => {})
      getInteraction().then(setInteraction).catch(() => {})
      getIntegrations().then(setIntegrations).catch(() => {})
      // Total corpus size = the index's own estimate for an empty query.
      getDocuments({}).then((d) => setCorpus(d.estimated_total)).catch(() => {})
    }
    tick()
    const id = setInterval(tick, 30_000)
    return () => clearInterval(id)
  }, [])

  const device = (compute?.device ?? {}) as { active?: string; fell_back?: boolean }
  // Summaries are available when a summariser model is present on disk.
  const modelRows = (compute?.models ?? []) as { spec: { role: string }; present: boolean }[]
  const summariesOn = modelRows.some((m) => m.spec.role === 'summariser' && m.present)
  const vec = media?.vector
  const stt = media?.stt

  return (
    <>
      <PageHead title="Overview">
        Is anything wrong, in one screen. Every number that could be <em>unknown</em> says so rather
        than showing zero, because a zero and an unreachable dependency look identical.
      </PageHead>

      {error ? (
        <p className="mb-4 text-sm" style={{ color: 'var(--warn)' }}>
          Could not reach the API: {error}
        </p>
      ) : null}

      {/* The corpus and the crawler, the two numbers that answer "is it working". */}
      <div className="mb-6 flex flex-wrap gap-3">
        <Tile n={corpus == null ? '…' : corpus.toLocaleString()} label="documents in the index" />
        <Tile n={s ? (s.unavailable ? 'unknown' : s.state) : '…'} label="crawler state" />
        <Tile n={s?.indexed ?? 0} label="indexed (session)" />
        <Tile n={s?.waiting ?? 0} label="frontier waiting" />
        <Tile n={s?.inflight ?? 0} label="in flight" />
        <Tile n={s?.failed ?? 0} label="failed" />
      </div>

      {/* The rest of the stack, at a glance — what is on, what is off, what needs attention. */}
      <h2 className="mb-2 text-lg font-semibold">Subsystems</h2>
      <div className="mb-8 flex flex-wrap gap-2">
        <Chip
          label="Compute"
          state={device.active === 'gpu' ? 'on' : device.fell_back ? 'warn' : 'on'}
          detail={device.active ? `on ${device.active}${device.fell_back ? ' (GPU unused)' : ''}` : '…'}
        />
        <Chip label="Summaries" state={summariesOn ? 'on' : 'off'} detail={summariesOn ? 'AI summaries' : 'off'} />
        <Chip
          label="Image AI"
          state={media ? (media.ocr.healthy ? 'on' : 'warn') : 'off'}
          detail={media ? `OCR: ${media.ocr.backend}` : '…'}
        />
        <Chip
          label="Image search"
          state={vec?.enabled ? (vec.qdrant_reachable ? 'on' : 'warn') : 'off'}
          detail={vec?.enabled ? `${vec.image_vectors?.toLocaleString() ?? '?'} vectors` : 'off'}
        />
        <Chip
          label="Voice"
          state={stt?.enabled ? (stt.healthy ? 'on' : 'warn') : 'off'}
          detail={stt?.enabled ? (stt.healthy ? 'STT up' : 'STT down') : 'off'}
        />
        <Chip
          label="Interaction"
          state={interaction?.enabled ? 'on' : 'off'}
          detail={interaction?.enabled ? `k=${interaction.k_anonymity}` : 'off'}
        />
        <Chip
          label="Federation"
          state={
            integrations?.federation?.enabled
              ? integrations.federation.reachable_from_api
                ? 'on'
                : 'warn'
              : 'off'
          }
          detail={
            integrations?.federation?.enabled
              ? integrations.federation.reachable_from_api
                ? 'SearXNG live'
                : 'gateway down'
              : 'off'
          }
        />
        <Chip
          label="Semantic search"
          state={
            integrations?.semantic?.configured
              ? integrations.semantic.reachable
                ? 'on'
                : 'warn'
              : 'off'
          }
          detail={
            integrations?.semantic?.configured
              ? integrations.semantic.documents_embedded == null
                ? 'embedder down'
                : `${integrations.semantic.documents_embedded.toLocaleString()} vectors`
              : 'off'
          }
        />
      </div>

      <h2 className="mb-3 text-lg font-semibold">Sections</h2>
      <div className="grid gap-3" style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))' }}>
        {LINKS.map(([label, href, hint]) => (
          <Link
            key={href}
            href={href}
            className="block border px-4 py-3 no-underline"
            style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}
          >
            <div className="font-medium">{label}</div>
            <div className="text-sm" style={{ color: 'var(--fg-muted)' }}>
              {hint}
            </div>
          </Link>
        ))}
      </div>
    </>
  )
}
